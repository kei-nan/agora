import React, { useEffect, useRef, useState } from 'react';
import { Platform, StyleSheet, Text, TextInput, TouchableOpacity, View, ActivityIndicator, ScrollView } from 'react-native';
import { Buffer } from 'buffer';
import DateTimePicker, { DateTimePickerEvent } from '@react-native-community/datetimepicker';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { KeyringPair } from '@polkadot/keyring/types';
import { RootStackParamList } from '../App';
import { readPassport, cancelPendingScan, RawPassportData } from '../native/nfcPassportReader';
import FaceCameraView from '../native/FaceCameraView';
import {
  isFaceMatchAvailable,
  hasCameraPermission,
  requestCameraPermission,
  capturePhoto,
  deleteCaptureFile,
  matchAgainstPassport,
  CapturedPhoto,
  LivenessChallenge,
} from '../native/faceMatch';
import { buildCircuitInputs } from '../chain/sodParser';
import { shouldBlockOnFaceMismatch } from './faceMatchGating';
import {
  TEST_PASSPORT_DG1_BASE64,
  TEST_PASSPORT_DG15_BASE64,
  TEST_PASSPORT_SOD_BASE64,
} from '../chain/__fixtures__/testPassport';
import { useAppModal } from '../components/AppModal';
import { getSigningKeypair } from '../chain/identity';
import { writeRegistrationStatus } from '../chain/registrationState';
import { colors } from '../theme';

// setRegistered/setPassportName (../chain/citizenState) intentionally not
// imported here anymore — this screen can no longer honestly claim
// registration completed (see the `throw` in `start()` below), so it
// shouldn't call the side effects that mark a citizen as registered. Wire
// those back in once the proving pipeline this screen is blocked on
// actually exists.

type Props = NativeStackScreenProps<RootStackParamList, 'Register'>;

type Step = 'idle' | 'nfc' | 'liveness' | 'proving' | 'submitting' | 'done';

const STEPS = [
  { id: 'nfc',        label: 'Scan passport NFC chip',        detail: 'Hold your phone to your biometric passport' },
  { id: 'liveness',   label: 'Face verification',             detail: 'On-device face match + liveness check' },
  { id: 'proving',    label: 'Generate ZK proof',             detail: 'Proof generated on device — nothing leaves your phone' },
  { id: 'submitting', label: 'Submit to blockchain',          detail: 'Proof posted to Agora chain; no personal data stored' },
];

const STEP_ORDER: Step[] = ['idle', 'nfc', 'liveness', 'proving', 'submitting', 'done'];

/** Distinguishes "we got further than before but the rest isn't built" from an actual failure. */
class NotImplementedError extends Error {}

/** MRZ dates are `YYMMDD` per ICAO Doc 9303 — this is what the native BAC key derivation needs, not a display format. */
function toMrz(d: Date): string {
  const yy = String(d.getFullYear() % 100).padStart(2, '0');
  const mm = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  return `${yy}${mm}${dd}`;
}

function formatDate(d: Date): string {
  return d.toLocaleDateString(undefined, { day: '2-digit', month: 'short', year: 'numeric' });
}

const TODAY = new Date();
const MIN_BIRTH_DATE = new Date(TODAY.getFullYear() - 120, 0, 1);
const MAX_EXPIRY_DATE = new Date(TODAY.getFullYear() + 15, 11, 31);

/**
 * A synthetic but genuinely-valid ICAO-shaped SOD (see
 * scripts/generate-test-passport-fixture.js) — a real self-signed test
 * certificate really signs a real LDSSecurityObject over a real hash of the
 * DG1 bytes below, so it exercises buildCircuitInputs() exactly like a real
 * passport would. Lets registration be exercised on the emulator (no NFC
 * hardware) and without a physical passport. __DEV__-gated: never available
 * in a release build.
 */
function getTestPassportData(): RawPassportData {
  return {
    dg1: new Uint8Array(Buffer.from(TEST_PASSPORT_DG1_BASE64, 'base64')),
    dg15: new Uint8Array(Buffer.from(TEST_PASSPORT_DG15_BASE64, 'base64')),
    sod: new Uint8Array(Buffer.from(TEST_PASSPORT_SOD_BASE64, 'base64')),
    // No DG2 fixture exists (see chain/__fixtures__/testPassport.ts) — same
    // "empty" convention that file already uses for DG15/Active
    // Authentication. An empty dg2MimeType makes the liveness gate below
    // take its normal "unsupported/undecodable DG2 photo" skip path, which
    // is the honest behavior for a fixture that never had a face photo.
    dg2: new Uint8Array(0),
    dg2MimeType: '',
  };
}

type LivenessSubstep = 'baseline' | 'challenge';

interface LivenessUiState {
  substep: LivenessSubstep;
  /** Only meaningful once `substep === 'challenge'` — chosen once per attempt so a static photo/video can't be pre-prepared for a known challenge. */
  challenge: LivenessChallenge;
  error: string | null;
  capturing: boolean;
}

/**
 * Thresholds below are unvalidated placeholders (no real capture corpus to
 * calibrate a false-accept/false-reject tradeoff against) — same honesty
 * standard as `FaceMatchModule.kt`'s `MATCH_THRESHOLD`. This is a 2-shot
 * challenge-response liveness check, not continuous video analysis: a
 * prepared attacker with video of the real person could plausibly defeat
 * it — a deliberate, documented scope boundary (see
 * `docs/project/changelog/087.md`), not an oversight.
 */
const EYES_OPEN_THRESHOLD = 0.5;
const EYES_CLOSED_THRESHOLD = 0.3;
const FRONTAL_ANGLE_MAX_DEGREES = 15;
const TURN_ANGLE_MIN_DEGREES = 15;

function baselinePassed(photo: CapturedPhoto): boolean {
  return (
    photo.leftEyeOpenProbability >= EYES_OPEN_THRESHOLD &&
    photo.rightEyeOpenProbability >= EYES_OPEN_THRESHOLD &&
    Math.abs(photo.headEulerAngleY) <= FRONTAL_ANGLE_MAX_DEGREES
  );
}

function challengePassed(challenge: LivenessChallenge, photo: CapturedPhoto): boolean {
  if (challenge === 'blink') {
    return photo.leftEyeOpenProbability <= EYES_CLOSED_THRESHOLD && photo.rightEyeOpenProbability <= EYES_CLOSED_THRESHOLD;
  }
  return Math.abs(photo.headEulerAngleY) >= TURN_ANGLE_MIN_DEGREES;
}

function pickRandomChallenge(): LivenessChallenge {
  return Math.random() < 0.5 ? 'blink' : 'turn';
}

export default function RegisterScreen({ navigation }: Props) {
  const [step, setStep] = useState<Step>('idle');
  // MRZ fields feed BAC key derivation for the NFC read (see
  // ../native/nfcPassportReader.ts) via toMrz() below, matching the Android
  // native module's BACKey requirement (confirmed from JMRTD source, HANDOFF
  // log #58).
  const [documentNumber, setDocumentNumber] = useState('');
  const [dateOfBirth, setDateOfBirth] = useState<Date | null>(null);
  const [dateOfExpiry, setDateOfExpiry] = useState<Date | null>(null);
  const [activePicker, setActivePicker] = useState<'dob' | 'expiry' | null>(null);
  const [rawPassport, setRawPassport] = useState<RawPassportData | null>(null);
  const [livenessUi, setLivenessUi] = useState<LivenessUiState | null>(null);
  const baselineUriRef = useRef<string | null>(null);
  const livenessResolveRef = useRef<((result: { faceMatched: boolean; matchSkippedReason?: string }) => void) | null>(null);
  const livenessRejectRef = useRef<((e: Error) => void) | null>(null);
  const { showInfo } = useAppModal();

  const mrzComplete = documentNumber.length > 0 && dateOfBirth !== null && dateOfExpiry !== null;

  /**
   * Shows the capture UI (rendered below, gated on `step === 'liveness' &&
   * livenessUi`) and blocks until `handleLivenessCapture` — driven by the
   * user tapping the on-screen "Capture" button, so this genuinely cannot be
   * a plain sequential `await` chain — settles the returned promise. Mirrors
   * `start()`'s own error-handling shape: throws a plain `Error` (camera
   * unavailable/permission denied) that falls into `start()`'s existing
   * generic "didn't complete, try again" branch, same as any other failure
   * in this screen.
   */
  async function runLivenessGate(): Promise<{ faceMatched: boolean; matchSkippedReason?: string }> {
    if (!isFaceMatchAvailable()) {
      throw new Error('Face match/liveness is not available on this device/build.');
    }
    const granted = (await hasCameraPermission()) || (await requestCameraPermission());
    if (!granted) {
      throw new Error('Camera permission is required to verify your face and liveness.');
    }
    baselineUriRef.current = null;
    setLivenessUi({ substep: 'baseline', challenge: pickRandomChallenge(), error: null, capturing: false });
    try {
      return await new Promise<{ faceMatched: boolean; matchSkippedReason?: string }>((resolve, reject) => {
        livenessResolveRef.current = resolve;
        livenessRejectRef.current = reject;
      });
    } finally {
      setLivenessUi(null);
      livenessResolveRef.current = null;
      livenessRejectRef.current = null;
    }
  }

  /**
   * The baseline shot only gates on eyes-open/frontal-angle; the challenge
   * shot's face-match comparison against the passport's DG2 photo happens
   * here too (using the baseline capture, not the challenge one — a
   * frontal, eyes-open frame is what the embedding model expects, see
   * `FaceMatchModule.kt`), once liveness itself has already passed.
   */
  async function handleLivenessCapture() {
    if (!livenessUi || livenessUi.capturing) return;
    setLivenessUi({ ...livenessUi, capturing: true, error: null });
    try {
      if (livenessUi.substep === 'baseline') {
        const photo = await capturePhoto('baseline');
        if (!baselinePassed(photo)) {
          // This shot failed the liveness heuristic and will never be passed to
          // matchAgainstPassport (whose own finally-block sweep is the only other
          // thing that ever cleans these up) — the retry below captures a brand-new
          // file, so this one is now permanently orphaned unless deleted here.
          await deleteCaptureFile(photo.uri);
          setLivenessUi({ ...livenessUi, capturing: false, error: 'Look straight at the camera with both eyes open, then try again.' });
          return;
        }
        baselineUriRef.current = photo.uri;
        setLivenessUi({ substep: 'challenge', challenge: livenessUi.challenge, error: null, capturing: false });
        return;
      }
      const photo = await capturePhoto(livenessUi.challenge);
      if (!challengePassed(livenessUi.challenge, photo)) {
        // Same reasoning as the baseline branch above — this failed challenge shot
        // is never reused, so clean it up now rather than leaving it for the next
        // app-launch sweep. Note: only this challenge shot, not baselineUriRef.current
        // — the passed baseline is still needed for the eventual matchAgainstPassport call.
        await deleteCaptureFile(photo.uri);
        setLivenessUi({ ...livenessUi, capturing: false, error: "That didn't register — try again." });
        return;
      }
      const dg2 = rawPassport?.dg2 ?? new Uint8Array(0);
      const dg2MimeType = rawPassport?.dg2MimeType ?? '';
      const match = await matchAgainstPassport(dg2, dg2MimeType, baselineUriRef.current!);
      // A real mismatch must not silently let registration continue. Mirror
      // the challenge-failure branch above: reset back to the baseline
      // substep with an error and don't resolve runLivenessGate()'s promise,
      // so `start()` simply never proceeds past `await runLivenessGate()`
      // until this passes (or is legitimately skipped) — no separate gate
      // needed in `start()` itself. See faceMatchGating.ts for what counts
      // as a block vs. a legitimate skip.
      if (shouldBlockOnFaceMismatch(match)) {
        baselineUriRef.current = null;
        setLivenessUi({
          substep: 'baseline',
          challenge: pickRandomChallenge(),
          error: "That doesn't look like a match for your passport photo. Please try again.",
          capturing: false,
        });
        return;
      }
      livenessResolveRef.current?.({
        faceMatched: match.matched,
        ...(match.skipped ? { matchSkippedReason: match.reason ?? 'unsupported passport photo format' } : {}),
      });
    } catch (e: any) {
      livenessRejectRef.current?.(e instanceof Error ? e : new Error(String(e)));
    }
  }

  // If the user navigates away mid-scan (e.g. taps back while "Scan passport
  // NFC chip" is active), cancel the native side's pending read rather than
  // leaving it occupying NfcPassportModule's single pending-read slot until
  // its own timeout elapses.
  useEffect(() => {
    return () => {
      cancelPendingScan();
    };
  }, []);

  // Mirrors the cleanup above for the liveness step: if the user abandons
  // registration after the baseline shot has passed (so it's held in
  // baselineUriRef, see handleLivenessCapture) but before finishing the
  // challenge substep, that file would otherwise never reach
  // matchAgainstPassport's sweep and would sit on disk until the next app
  // launch's startup sweep. `baselineUriRef.current` is read at unmount
  // time (not captured by this closure), so it reflects whatever the flow
  // last set it to. deleteCaptureFile is a no-op if the ref is already
  // null (never captured, or already consumed by a completed match) or
  // points to a file already swept by matchAgainstPassport's own finally.
  useEffect(() => {
    return () => {
      if (baselineUriRef.current) {
        deleteCaptureFile(baselineUriRef.current);
      }
    };
  }, []);

  async function start(useTestPassport: boolean = false) {
    if (!useTestPassport && Platform.OS !== 'android') {
      showInfo(
        'Not available on this device',
        'Passport NFC reading is only implemented for Android so far. See HANDOFF.md item 8.',
      );
      return;
    }
    // Declared outside the try block (rather than `const { keypair } =` right
    // inside it) so the catch block below can still reach `keypair.address`
    // for the Failed-status write even when the failure happened *before*
    // getSigningKeypair() resolved — in which case it's simply left
    // `undefined` and the write is skipped (see the inner try/catch around
    // that write below).
    let keypair: KeyringPair | undefined;
    try {
      // New dependency introduced by this screen: previously RegisterScreen
      // had zero reliance on identity.ts/signing key material. Keystore
      // unavailability is therefore a genuinely new failure mode partway
      // through this flow — it's covered by this same try/catch, falling
      // into the generic "didn't complete" branch below like any other error
      // here.
      keypair = (await getSigningKeypair()).keypair;
      setStep('nfc');
      // Real as of HANDOFF log #58 — Android only (see readPassport's own
      // doc comment; iOS needs a Swift module wrapping AndyQ/NFCPassportReader,
      // not built yet, no ios/ project exists). Returns raw EF.DG1/EF.DG15/
      // EF.SOD bytes via BAC, not parsed fields — these are the actual ZK
      // circuit witness inputs. The test-passport path skips this entirely
      // (see getTestPassportData() above) — MRZ fields are irrelevant there
      // since no real BAC handshake happens.
      const raw = useTestPassport
        ? getTestPassportData()
        : await readPassport({
            documentNumber,
            dateOfBirth: toMrz(dateOfBirth!),
            dateOfExpiry: toMrz(dateOfExpiry!),
          });
      setRawPassport(raw);
      setStep('liveness');
      await writeRegistrationStatus(keypair.address, { stage: 'PassportScanned' });
      // Real as of this session (../native/faceMatch.ts + the capture UI
      // rendered below, gated on `step === 'liveness' && livenessUi`) — a
      // 2-shot randomized challenge-response (frontal-eyes-open baseline,
      // then blink or turn) read via ML Kit, plus a MobileFaceNet embedding
      // comparison against `raw.dg2`. See runLivenessGate()'s doc comment
      // and docs/project/changelog/087.md for the full design/limitations.
      const { faceMatched, matchSkippedReason } = await runLivenessGate();
      await writeRegistrationStatus(keypair.address, {
        stage: 'LivenessVerified',
        faceMatched,
        ...(matchSkippedReason ? { matchSkippedReason } : {}),
      });
      setStep('proving');
      // Real as of this session (../chain/sodParser.ts) — parses the SOD's
      // CMS SignedData structure and assembles the ZKPassport sig-check/
      // data-check circuits' dg1/eContent/signedAttributes/pubkey/signature
      // inputs, and identifies which circuit variant this passport needs.
      // Still stops here, though, blocked on what buildCircuitInputs
      // deliberately does NOT produce (see its module doc comment /
      // `UnresolvedInputs` for why these are real, unstarted work rather
      // than oversights):
      //  (1) `cscCertificate` — the CSC (country signing) certificate that
      //      signed this passport's DSC, plus its Barrett-reduction params.
      //  (2) `certificateTreeProof` — the DSC's inclusion proof in Agora's
      //      certificate registry tree (certificateTree.ts already builds
      //      this tree — changelog entry 66 — but nothing wires the two
      //      together yet).
      //  (3) `commitmentSalts` — must be freshly random per proof, generated
      //      on-device, not derived from passport bytes.
      //  (4) `serviceScope` / `serviceSubscope` — chain-level constants.
      // ...plus obtaining a `.wcd` witness graph + proving key for the
      // specific variant identified below (see ../chain/zkProving.ts's
      // module doc + HANDOFF item 8/log #56).
      const { variant } = buildCircuitInputs(raw.dg1, raw.dg15, raw.sod);
      await writeRegistrationStatus(keypair.address, { stage: 'ProofMaterialAssembled' });
      throw new NotImplementedError(
        `Passport chip read succeeded (DG1: ${raw.dg1.length}B, DG15: ${raw.dg15.length}B, ` +
          `SOD: ${raw.sod.length}B) and circuit inputs were assembled for variant "${variant.name}" — ` +
          'but proof generation still needs a certificate-registry inclusion proof, on-device ' +
          "commitment salts, and this variant's proving key/witness graph, none of which exist yet. " +
          'See RegisterScreen.tsx TODOs.',
      );
    } catch (e: any) {
      if (e instanceof NotImplementedError) {
        showInfo(
          'Almost there',
          "Your passport was read and verified successfully. We're still building the last part " +
            "of the system that turns that into a proof for the chain, so registration can't finish " +
            'in this version yet — please check back in a future update.',
          __DEV__ ? e.message : undefined,
        );
      } else {
        try {
          // Best-effort: if keypair never resolved (e.g. the failure was
          // getSigningKeypair() itself), `keypair` is still undefined here
          // and `.address` throws — caught right below, so a failure to
          // persist the failure record can't mask the original error or
          // crash this error-handling path.
          await writeRegistrationStatus(keypair!.address, {
            stage: 'Failed',
            failedStage: step === 'idle' ? 'PassportScanned' : step,
            reason: e.message ?? String(e),
            retryable: true,
          });
        } catch {
          // Swallowed — see comment above.
        }
        showInfo(
          "Registration didn't complete",
          'Something went wrong while reading or processing your passport. Please try again.',
          __DEV__ ? e.message : undefined,
        );
      }
      setStep('idle');
    }
  }

  const activeIndex = STEP_ORDER.indexOf(step);

  return (
    <ScrollView
      style={s.container}
      contentContainerStyle={s.scrollContent}
      keyboardShouldPersistTaps="handled"
    >
      <Text style={s.title}>Citizen Registration</Text>
      <Text style={s.subtitle}>
        Your identity is verified using your biometric passport. Nothing leaves your phone — only a
        cryptographic proof is submitted to the chain.
      </Text>

      <View style={s.stepList}>
        {STEPS.map((st, i) => {
          const stepIndex = i + 1; // idle=0, nfc=1, liveness=2, ...
          const isActive = activeIndex === stepIndex;
          const isDone = activeIndex > stepIndex || step === 'done';
          return (
            <View key={st.id} style={s.stepRow}>
              <View style={[s.stepNum, isDone ? s.stepDone : isActive ? s.stepActive : s.stepPending]}>
                {isDone ? (
                  <Text style={s.stepNumText}>✓</Text>
                ) : isActive ? (
                  <ActivityIndicator size="small" color={colors.textPrimary} />
                ) : (
                  <Text style={s.stepNumText}>{i + 1}</Text>
                )}
              </View>
              <View style={s.stepText}>
                <Text style={[s.stepLabel, (isActive || isDone) ? s.stepLabelActive : {}]}>{st.label}</Text>
                <Text style={s.stepDetail}>{st.detail}</Text>
              </View>
            </View>
          );
        })}
      </View>

      {step === 'idle' && (
        <>
          <View style={s.mrzForm}>
            <Text style={s.mrzLabel}>Passport number</Text>
            <TextInput
              style={s.mrzInput}
              value={documentNumber}
              onChangeText={setDocumentNumber}
              placeholder="e.g. L898902C3"
              placeholderTextColor={colors.textDim}
              autoCapitalize="characters"
              accessibilityLabel="Passport number"
            />
            <Text style={s.mrzLabel}>Date of birth</Text>
            <TouchableOpacity
              style={s.mrzInput}
              onPress={() => setActivePicker('dob')}
              accessibilityRole="button"
              accessibilityLabel={dateOfBirth ? `Date of birth: ${formatDate(dateOfBirth)}` : 'Select date of birth'}
            >
              <Text style={dateOfBirth ? s.dateValue : s.datePlaceholder}>
                {dateOfBirth ? formatDate(dateOfBirth) : 'Select date of birth'}
              </Text>
            </TouchableOpacity>

            <Text style={s.mrzLabel}>Date of expiry</Text>
            <TouchableOpacity
              style={s.mrzInput}
              onPress={() => setActivePicker('expiry')}
              accessibilityRole="button"
              accessibilityLabel={dateOfExpiry ? `Date of expiry: ${formatDate(dateOfExpiry)}` : 'Select date of expiry'}
            >
              <Text style={dateOfExpiry ? s.dateValue : s.datePlaceholder}>
                {dateOfExpiry ? formatDate(dateOfExpiry) : 'Select date of expiry'}
              </Text>
            </TouchableOpacity>

            {activePicker && (
              <DateTimePicker
                value={(activePicker === 'dob' ? dateOfBirth : dateOfExpiry) ?? TODAY}
                mode="date"
                display="default"
                minimumDate={activePicker === 'dob' ? MIN_BIRTH_DATE : TODAY}
                maximumDate={activePicker === 'dob' ? TODAY : MAX_EXPIRY_DATE}
                onChange={(_event: DateTimePickerEvent, selected?: Date) => {
                  const picker = activePicker;
                  setActivePicker(null);
                  if (!selected) return;
                  if (picker === 'dob') setDateOfBirth(selected);
                  else if (picker === 'expiry') setDateOfExpiry(selected);
                }}
              />
            )}

            <Text style={s.mrzHint}>
              These three fields, printed in your passport's machine-readable zone, derive the key
              your phone uses to unlock the chip (Basic Access Control) — they never leave your device.
            </Text>
          </View>
          {rawPassport && (
            <Text style={s.scanResult}>
              Last scan: DG1 {rawPassport.dg1.length}B · DG15 {rawPassport.dg15.length}B · SOD{' '}
              {rawPassport.sod.length}B
            </Text>
          )}
          <TouchableOpacity
            style={[s.btn, !mrzComplete && s.btnDisabled]}
            onPress={() => start()}
            disabled={!mrzComplete}
            accessibilityRole="button"
            accessibilityLabel="Begin Registration"
            accessibilityState={{ disabled: !mrzComplete }}
          >
            <Text style={s.btnText}>Begin Registration</Text>
          </TouchableOpacity>

          {__DEV__ && (
            <TouchableOpacity
              style={s.testBtn}
              onPress={() => start(true)}
              accessibilityRole="button"
              accessibilityLabel="Use test passport, dev only, no NFC needed"
            >
              <Text style={s.testBtnText}>Use test passport (dev only, no NFC needed)</Text>
            </TouchableOpacity>
          )}
        </>
      )}

      {step === 'liveness' && livenessUi && (
        <View style={s.livenessBox}>
          <View style={s.cameraFrame}>
            <FaceCameraView style={s.cameraPreview} />
          </View>
          <Text style={s.livenessInstruction}>
            {livenessUi.substep === 'baseline'
              ? 'Look straight at the camera with both eyes open.'
              : livenessUi.challenge === 'blink'
                ? 'Now blink.'
                : 'Now turn your head to the side.'}
          </Text>
          {livenessUi.error && <Text style={s.livenessError}>{livenessUi.error}</Text>}
          <TouchableOpacity
            style={[s.btn, livenessUi.capturing && s.btnDisabled]}
            onPress={handleLivenessCapture}
            disabled={livenessUi.capturing}
            accessibilityRole="button"
            accessibilityLabel="Capture"
          >
            {livenessUi.capturing ? (
              <ActivityIndicator size="small" color={colors.textPrimary} />
            ) : (
              <Text style={s.btnText}>Capture</Text>
            )}
          </TouchableOpacity>
        </View>
      )}

      {step === 'done' && (
        <View style={s.successBox}>
          <Text style={s.successIcon}>✓</Text>
          <Text style={s.successText}>You are now a registered citizen</Text>
          <TouchableOpacity
            style={[s.btn, { marginTop: 24 }]}
            onPress={() => navigation.goBack()}
            accessibilityRole="button"
            accessibilityLabel="Go to Home"
          >
            <Text style={s.btnText}>Go to Home</Text>
          </TouchableOpacity>
        </View>
      )}
    </ScrollView>
  );
}

const s = StyleSheet.create({
  container: { flex: 1, backgroundColor: colors.bg },
  scrollContent: { padding: 24, paddingBottom: 48 },
  title: { fontSize: 24, fontWeight: '700', color: colors.textPrimary, marginBottom: 8 },
  subtitle: { fontSize: 14, color: colors.textMuted, lineHeight: 20, marginBottom: 32 },
  stepList: { gap: 20, marginBottom: 40 },
  stepRow: { flexDirection: 'row', alignItems: 'flex-start', gap: 14 },
  stepNum: {
    width: 36, height: 36, borderRadius: 18,
    alignItems: 'center', justifyContent: 'center', marginTop: 2,
  },
  stepPending: { backgroundColor: colors.border },
  stepActive: { backgroundColor: colors.accent },
  stepDone: { backgroundColor: colors.successSolid },
  stepNumText: { color: colors.textPrimary, fontWeight: '700', fontSize: 14 },
  stepText: { flex: 1 },
  stepLabel: { fontSize: 15, fontWeight: '600', color: colors.textMuted, marginBottom: 2 },
  stepLabelActive: { color: colors.textPrimary },
  stepDetail: { fontSize: 12, color: colors.textDim, lineHeight: 17 },
  btn: {
    backgroundColor: colors.accent,
    paddingVertical: 16,
    borderRadius: 14,
    alignItems: 'center',
  },
  btnText: { color: colors.textPrimary, fontWeight: '700', fontSize: 16 },
  testBtn: {
    marginTop: 12,
    paddingVertical: 12,
    borderRadius: 14,
    alignItems: 'center',
    borderWidth: 1,
    borderColor: colors.textFaint,
    borderStyle: 'dashed',
  },
  testBtnText: { color: colors.textSecondary, fontWeight: '600', fontSize: 13 },
  successBox: { alignItems: 'center', gap: 12 },
  successIcon: { fontSize: 56, color: colors.success },
  successText: { fontSize: 18, fontWeight: '600', color: colors.success, textAlign: 'center' },
  mrzForm: { marginBottom: 20, gap: 6 },
  mrzLabel: { fontSize: 12, fontWeight: '600', color: colors.textSecondary, marginTop: 10 },
  mrzInput: {
    borderWidth: 1,
    borderColor: colors.border,
    borderRadius: 10,
    paddingHorizontal: 14,
    paddingVertical: 10,
    fontSize: 15,
    color: colors.textPrimary,
    backgroundColor: '#161a23',
  },
  dateValue: { fontSize: 15, color: colors.textPrimary },
  datePlaceholder: { fontSize: 15, color: colors.textDim },
  mrzHint: { fontSize: 11, color: colors.textDim, lineHeight: 15, marginTop: 10 },
  scanResult: { fontSize: 12, color: colors.success, textAlign: 'center', marginBottom: 12 },
  btnDisabled: { backgroundColor: colors.textFaint },
  livenessBox: { alignItems: 'center', gap: 16 },
  cameraFrame: {
    width: 240,
    height: 240,
    borderRadius: 120,
    overflow: 'hidden',
    borderWidth: 2,
    borderColor: colors.accent,
    backgroundColor: '#161a23',
  },
  cameraPreview: { width: '100%', height: '100%' },
  livenessInstruction: { fontSize: 16, fontWeight: '600', color: colors.textPrimary, textAlign: 'center' },
  livenessError: { fontSize: 13, color: colors.danger, textAlign: 'center' },
});
