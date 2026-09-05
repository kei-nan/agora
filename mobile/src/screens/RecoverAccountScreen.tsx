/**
 * Mobile UI for `recover_account` (call index 20, `pallets/pallet-identity/src/lib.rs`).
 * Structurally mirrors `RegisterScreen.tsx`'s re-scan flow (same MRZ form, NFC scan,
 * liveness/face-match gate, and proving step) since recovery re-proves the same
 * "I hold this passport" fact registration does — see that screen's own comments for
 * the detail on each step. It stops at the exact same wall: the ZKPassport proving
 * pipeline (certificate-registry inclusion proof, on-device commitment salts, a
 * proving key/witness graph) doesn't exist yet in this codebase, so this screen can
 * exercise the scan + liveness steps for real but cannot produce a real proof to hand
 * to `recoverAccount()` (`../chain/identity.ts`) — that wrapper exists and is unit
 * tested directly (`identity.test.ts`), but nothing in this screen can drive it
 * end-to-end without a live OPRF committee and a working prover, neither of which
 * exist in this environment (see docs/project/next-steps.md).
 *
 * What's new here, not present in RegisterScreen: a mandatory, plainly-worded
 * disclosure that must be visible and explicitly acknowledged before the citizen can
 * even start the re-scan. `recover_account` is instant, irreversible, and has no
 * dispute window (see the pallet's own doc comment on the extrinsic) — the old account
 * stops working the moment this succeeds. The chain now REJECTS the call outright (see
 * `Error::RecoveryBlocked*` on the pallet) if the old account still holds a nonzero AGR
 * balance, a registered pallet-elections delegate persona, a legislature seat, or a
 * cabinet role — so those specific losses can no longer happen silently, but the citizen
 * still needs to divest/resign each one via its own normal flow before recovery will go
 * through, and this screen has no UI yet to guide them through that (surfacing the
 * rejected-call error as-is is the current state). Two things are NOT chain-guarded and
 * still silently orphan on a successful recovery: pallet-voting delegations/budget and
 * anticorruption conflict-registry entries (see `docs/project/pallets/identity.md`'s
 * "Cross-pallet orphaning" section and `../chain/identity.ts`'s `recoverAccount` doc
 * comment). Undersell either of those here and a citizen could lose access with no
 * warning.
 */
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
  pickRandomChallenge,
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

type Props = NativeStackScreenProps<RootStackParamList, 'RecoverAccount'>;

type Step = 'disclosure' | 'form' | 'nfc' | 'liveness' | 'proving' | 'submitting' | 'done';

const STEPS = [
  { id: 'nfc',        label: 'Scan passport NFC chip',        detail: 'Hold your phone to the same biometric passport your old account was registered with' },
  { id: 'liveness',   label: 'Face verification',             detail: 'On-device face match + liveness check' },
  { id: 'proving',    label: 'Generate recovery proof',       detail: 'Proof generated on device — nothing leaves your phone' },
  { id: 'submitting', label: 'Submit to blockchain',          detail: 'Rebinds your citizen registration to this account' },
];

const STEP_ORDER: Step[] = ['disclosure', 'form', 'nfc', 'liveness', 'proving', 'submitting', 'done'];

/** Distinguishes "we got further than before but the rest isn't built" from an actual failure. Mirrors RegisterScreen.tsx's own class of the same name. */
class NotImplementedError extends Error {}

/** MRZ dates are `YYMMDD` per ICAO Doc 9303 — mirrors RegisterScreen.tsx's `toMrz`. */
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

/** Same synthetic-but-valid test fixture RegisterScreen.tsx uses — see that file's `getTestPassportData` doc comment. __DEV__-gated: never available in a release build. */
function getTestPassportData(): RawPassportData {
  return {
    dg1: new Uint8Array(Buffer.from(TEST_PASSPORT_DG1_BASE64, 'base64')),
    dg15: new Uint8Array(Buffer.from(TEST_PASSPORT_DG15_BASE64, 'base64')),
    sod: new Uint8Array(Buffer.from(TEST_PASSPORT_SOD_BASE64, 'base64')),
    dg2: new Uint8Array(0),
    dg2MimeType: '',
  };
}

type LivenessSubstep = 'baseline' | 'challenge';

interface LivenessUiState {
  substep: LivenessSubstep;
  challenge: LivenessChallenge;
  error: string | null;
  capturing: boolean;
}

// Same unvalidated-placeholder thresholds as RegisterScreen.tsx — see that file's
// comment on `EYES_OPEN_THRESHOLD` and docs/project/changelog/087.md.
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

export default function RecoverAccountScreen({ navigation }: Props) {
  const [step, setStep] = useState<Step>('disclosure');
  const [documentNumber, setDocumentNumber] = useState('');
  const [dateOfBirth, setDateOfBirth] = useState<Date | null>(null);
  const [dateOfExpiry, setDateOfExpiry] = useState<Date | null>(null);
  const [activePicker, setActivePicker] = useState<'dob' | 'expiry' | null>(null);
  const [rawPassport, setRawPassport] = useState<RawPassportData | null>(null);
  const [livenessUi, setLivenessUi] = useState<LivenessUiState | null>(null);
  const baselineUriRef = useRef<string | null>(null);
  const livenessResolveRef = useRef<((result: { faceMatched: boolean; matchSkippedReason?: string }) => void) | null>(null);
  const livenessRejectRef = useRef<((e: Error) => void) | null>(null);
  const { showInfo, showConfirm } = useAppModal();

  const mrzComplete = documentNumber.length > 0 && dateOfBirth !== null && dateOfExpiry !== null;

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

  // If the user navigates away mid-scan, cancel the native side's pending read rather
  // than leaving it occupying NfcPassportModule's single pending-read slot until its
  // own timeout elapses. Mirrors RegisterScreen.tsx's identical cleanup effect.
  useEffect(() => {
    return () => {
      cancelPendingScan();
    };
  }, []);

  // Mirrors RegisterScreen.tsx's identical cleanup effect: if the user abandons
  // recovery after the baseline shot has passed (so it's held in baselineUriRef,
  // see handleLivenessCapture) but before finishing the challenge substep, that
  // file would otherwise never reach matchAgainstPassport's sweep and would sit
  // on disk until the next app launch's startup sweep. `baselineUriRef.current`
  // is read at unmount time (not captured by this closure), so it reflects
  // whatever the flow last set it to. deleteCaptureFile is a no-op if the ref is
  // already null (never captured, or already consumed by a completed match) or
  // points to a file already swept by matchAgainstPassport's own finally.
  useEffect(() => {
    return () => {
      if (baselineUriRef.current) {
        deleteCaptureFile(baselineUriRef.current);
      }
    };
  }, []);

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
          // Mirrors RegisterScreen.tsx's identical branch.
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
        // Mirrors RegisterScreen.tsx's identical branch.
        await deleteCaptureFile(photo.uri);
        setLivenessUi({ ...livenessUi, capturing: false, error: "That didn't register — try again." });
        return;
      }
      const dg2 = rawPassport?.dg2 ?? new Uint8Array(0);
      const dg2MimeType = rawPassport?.dg2MimeType ?? '';
      const match = await matchAgainstPassport(dg2, dg2MimeType, baselineUriRef.current!);
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

  async function start(useTestPassport: boolean = false) {
    if (!useTestPassport && Platform.OS !== 'android') {
      showInfo(
        'Not available on this device',
        'Passport NFC reading is only implemented for Android so far. See HANDOFF.md item 8.',
      );
      return;
    }
    let keypair: KeyringPair | undefined;
    try {
      keypair = (await getSigningKeypair()).keypair;
      setStep('nfc');
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
      const { faceMatched, matchSkippedReason } = await runLivenessGate();
      await writeRegistrationStatus(keypair.address, {
        stage: 'LivenessVerified',
        faceMatched,
        ...(matchSkippedReason ? { matchSkippedReason } : {}),
      });
      setStep('proving');
      // Same real-but-incomplete step as RegisterScreen.tsx: parses the SOD and
      // assembles what buildCircuitInputs can, but recover_account needs the exact
      // same still-missing proving material registration does (certificate-registry
      // inclusion proof, on-device commitment salts, a proving key/witness graph for
      // this passport's circuit variant) — see that screen's own comment for the
      // itemized list. recoverAccount() (../chain/identity.ts) is the wrapper that
      // would submit the finished proof; nothing here can produce one yet.
      const { variant } = buildCircuitInputs(raw.dg1, raw.dg15, raw.sod);
      await writeRegistrationStatus(keypair.address, { stage: 'ProofMaterialAssembled' });
      throw new NotImplementedError(
        `Passport chip read succeeded (DG1: ${raw.dg1.length}B, DG15: ${raw.dg15.length}B, ` +
          `SOD: ${raw.sod.length}B) and circuit inputs were assembled for variant "${variant.name}" — ` +
          'but proof generation still needs a certificate-registry inclusion proof, on-device ' +
          "commitment salts, and this variant's proving key/witness graph, none of which exist yet. " +
          'See RecoverAccountScreen.tsx / RegisterScreen.tsx TODOs.',
      );
    } catch (e: any) {
      if (e instanceof NotImplementedError) {
        showInfo(
          'Almost there',
          "Your passport was read and verified successfully. We're still building the last part " +
            "of the system that turns that into a proof for the chain, so recovery can't finish " +
            'in this version yet — please check back in a future update. Your old account has NOT ' +
            'been touched.',
          __DEV__ ? e.message : undefined,
        );
      } else {
        try {
          await writeRegistrationStatus(keypair!.address, {
            stage: 'Failed',
            failedStage: step === 'disclosure' || step === 'form' ? 'PassportScanned' : step,
            reason: e.message ?? String(e),
            retryable: true,
          });
        } catch {
          // Swallowed — best-effort only, mirrors RegisterScreen.tsx.
        }
        showInfo(
          "Recovery didn't complete",
          'Something went wrong while reading or processing your passport. Your old account has NOT ' +
            'been touched. Please try again.',
          __DEV__ ? e.message : undefined,
        );
      }
      setStep('form');
    }
  }

  function confirmAndBegin() {
    showConfirm({
      title: 'Recover this account?',
      message:
        'This cannot be undone. As soon as recovery succeeds, your old account stops working ' +
        'immediately. If your old account still holds an AGR balance, a registered delegate ' +
        'persona, a legislature seat, or a cabinet role, the chain will reject this recovery — ' +
        'you must divest/resign each one from the old account first. Pallet-voting delegations ' +
        'or budget tied to the old account are NOT chain-guarded and will still be lost.',
      confirmLabel: 'I understand, continue',
      destructive: true,
      onConfirm: () => setStep('form'),
    });
  }

  const activeIndex = STEP_ORDER.indexOf(step);

  return (
    <ScrollView
      style={s.container}
      contentContainerStyle={s.scrollContent}
      keyboardShouldPersistTaps="handled"
    >
      <Text style={s.title}>Recover Citizen Account</Text>

      {step === 'disclosure' && (
        <>
          <View style={s.warnBox}>
            <Text style={s.warnTitle}>Read this before you continue</Text>
            <Text style={s.warnItem}>
              • Recovery is instant and irreversible. There is no undo and no dispute window.
            </Text>
            <Text style={s.warnItem}>
              • Your old account stops working the moment recovery succeeds — even if you still have
              access to it.
            </Text>
            <Text style={s.warnItem}>
              • None of this is transferred to the new account. For an AGR token balance, a
              registered delegate persona, a legislature seat, or a cabinet role tied to the old
              account, the chain will refuse to recover until you divest/resign each one from the
              old account first — nothing here is silently lost. Pallet-voting delegations/budget
              tied to the old account are the exception: those are NOT chain-checked and will be
              lost.
            </Text>
            <Text style={s.warnItem}>
              • Only your core citizen identity (passport-linked registration) moves to this device.
            </Text>
            <Text style={s.warnItem}>
              • Use this only if you have genuinely lost access to your old device/account — not to
              move an account you can still use normally (use "Migrate" flows for that instead where
              available).
            </Text>
          </View>
          <TouchableOpacity
            style={s.btn}
            onPress={confirmAndBegin}
            accessibilityRole="button"
            accessibilityLabel="I understand, continue to recovery"
          >
            <Text style={s.btnText}>I understand, continue</Text>
          </TouchableOpacity>
        </>
      )}

      {step !== 'disclosure' && (
        <>
          <Text style={s.subtitle}>
            Re-scan the same biometric passport your old account was registered with. Nothing leaves
            your phone except a cryptographic proof.
          </Text>

          <View style={s.stepList}>
            {STEPS.map((st, i) => {
              const stepIndex = i + 2; // disclosure=0, form=1, nfc=2, liveness=3, ...
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
        </>
      )}

      {step === 'form' && (
        <>
          <View style={s.noticeBox}>
            <Text style={s.noticeTitle}>Recovery can't finish yet</Text>
            <Text style={s.noticeText}>
              You can scan your passport and complete face verification below, but the last
              step — turning that into a proof the chain accepts — isn't built yet, so recovery
              won't actually complete in this version. If you'd rather not spend the few minutes
              that takes right now, check back in a future update. Either way, your old account
              will not be touched.
            </Text>
          </View>
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
            <Text style={s.expiryNote}>
              Your passport must not be expired to recover your account. Enter its real expiry
              date here even if that date has already passed — we'll tell you if it's a problem.
            </Text>

            {activePicker && (
              <DateTimePicker
                value={(activePicker === 'dob' ? dateOfBirth : dateOfExpiry) ?? TODAY}
                mode="date"
                display="default"
                // Deliberately no minimumDate on the expiry picker: a citizen whose
                // passport already expired still needs to be able to scroll to and
                // enter their real (past) expiry date. Chain-side proof/registration
                // logic is what actually determines validity, not this picker — see
                // the expiryNote text above.
                minimumDate={activePicker === 'dob' ? MIN_BIRTH_DATE : undefined}
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
          <TouchableOpacity
            style={[s.btn, !mrzComplete && s.btnDisabled]}
            onPress={() => start()}
            disabled={!mrzComplete}
            accessibilityRole="button"
            accessibilityLabel="Begin Recovery"
            accessibilityState={{ disabled: !mrzComplete }}
          >
            <Text style={s.btnText}>Begin Recovery</Text>
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
          <Text style={s.successText}>Your account has been recovered</Text>
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
  title: { fontSize: 24, fontWeight: '700', color: colors.textPrimary, marginBottom: 16 },
  subtitle: { fontSize: 14, color: colors.textMuted, lineHeight: 20, marginBottom: 32 },
  warnBox: {
    backgroundColor: colors.warningBg,
    borderWidth: 1,
    borderColor: colors.warningBorder,
    borderRadius: 14,
    padding: 18,
    marginBottom: 24,
  },
  warnTitle: { fontSize: 15, fontWeight: '700', color: colors.warningTextStrong, marginBottom: 10 },
  warnItem: { fontSize: 13, color: colors.warningTextStrong, lineHeight: 19, marginBottom: 8 },
  noticeBox: {
    backgroundColor: colors.warningBg,
    borderWidth: 1,
    borderColor: colors.warningBorder,
    borderRadius: 14,
    padding: 16,
    marginBottom: 20,
  },
  noticeTitle: { fontSize: 14, fontWeight: '700', color: colors.warningTextStrong, marginBottom: 6 },
  noticeText: { fontSize: 13, color: colors.warningTextStrong, lineHeight: 18 },
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
  expiryNote: { fontSize: 12, color: colors.warning, lineHeight: 16, marginTop: 6 },
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
