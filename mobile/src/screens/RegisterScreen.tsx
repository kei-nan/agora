import React, { useEffect, useState } from 'react';
import { Alert, Modal, Platform, StyleSheet, Text, TextInput, TouchableOpacity, View, ActivityIndicator, ScrollView } from 'react-native';
import { Buffer } from 'buffer';
import DateTimePicker, { DateTimePickerEvent } from '@react-native-community/datetimepicker';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { RootStackParamList } from '../App';
import { readPassport, cancelPendingScan, RawPassportData } from '../native/nfcPassportReader';
import { buildCircuitInputs } from '../chain/sodParser';
import {
  TEST_PASSPORT_DG1_BASE64,
  TEST_PASSPORT_DG15_BASE64,
  TEST_PASSPORT_SOD_BASE64,
} from '../chain/__fixtures__/testPassport';
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
  };
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
  const [resultModal, setResultModal] = useState<{ title: string; message: string; devDetails?: string } | null>(null);

  const mrzComplete = documentNumber.length > 0 && dateOfBirth !== null && dateOfExpiry !== null;

  // If the user navigates away mid-scan (e.g. taps back while "Scan passport
  // NFC chip" is active), cancel the native side's pending read rather than
  // leaving it occupying NfcPassportModule's single pending-read slot until
  // its own timeout elapses.
  useEffect(() => {
    return () => {
      cancelPendingScan();
    };
  }, []);

  async function start(useTestPassport: boolean = false) {
    if (!useTestPassport && Platform.OS !== 'android') {
      Alert.alert(
        'Not available on this device',
        'Passport NFC reading is only implemented for Android so far. See HANDOFF.md item 8.',
      );
      return;
    }
    try {
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
      // TODO: await FaceMatch.verify(scan.faceImage);
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
      throw new NotImplementedError(
        `Passport chip read succeeded (DG1: ${raw.dg1.length}B, DG15: ${raw.dg15.length}B, ` +
          `SOD: ${raw.sod.length}B) and circuit inputs were assembled for variant "${variant.name}" — ` +
          'but proof generation still needs a certificate-registry inclusion proof, on-device ' +
          "commitment salts, and this variant's proving key/witness graph, none of which exist yet. " +
          'See RegisterScreen.tsx TODOs.',
      );
    } catch (e: any) {
      if (e instanceof NotImplementedError) {
        setResultModal({
          title: 'Almost there',
          message:
            "Your passport was read and verified successfully. We're still building the last part " +
            "of the system that turns that into a proof for the chain, so registration can't finish " +
            'in this version yet — please check back in a future update.',
          devDetails: __DEV__ ? e.message : undefined,
        });
      } else {
        setResultModal({
          title: "Registration didn't complete",
          message: 'Something went wrong while reading or processing your passport. Please try again.',
          devDetails: __DEV__ ? e.message : undefined,
        });
      }
      setStep('idle');
    }
  }

  const activeIndex = STEP_ORDER.indexOf(step);

  return (
    <>
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

    <Modal
      visible={resultModal !== null}
      transparent
      animationType="fade"
      onRequestClose={() => setResultModal(null)}
    >
      <View style={s.modalBackdrop}>
        <View style={s.modalCard}>
          <Text style={s.modalTitle}>{resultModal?.title}</Text>
          <Text style={s.modalMessage}>{resultModal?.message}</Text>
          {resultModal?.devDetails && (
            <>
              <View style={s.modalDivider} />
              <Text style={s.modalDevLabel}>DEV DETAILS</Text>
              <Text style={s.modalDevText}>{resultModal.devDetails}</Text>
            </>
          )}
          <TouchableOpacity
            style={s.modalBtn}
            onPress={() => setResultModal(null)}
            accessibilityRole="button"
            accessibilityLabel="OK"
          >
            <Text style={s.btnText}>OK</Text>
          </TouchableOpacity>
        </View>
      </View>
    </Modal>
    </>
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
  stepDone: { backgroundColor: '#166534' },
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
  modalBackdrop: {
    flex: 1,
    backgroundColor: 'rgba(0,0,0,0.65)',
    justifyContent: 'center',
    alignItems: 'center',
    padding: 24,
  },
  modalCard: {
    width: '100%',
    maxWidth: 400,
    backgroundColor: colors.card,
    borderRadius: 16,
    padding: 24,
    borderWidth: 1,
    borderColor: colors.border,
  },
  modalTitle: { fontSize: 18, fontWeight: '700', color: colors.textPrimary, marginBottom: 10 },
  modalMessage: { fontSize: 14, color: '#d1d5db', lineHeight: 20 },
  modalDivider: { height: 1, backgroundColor: colors.border, marginVertical: 16 },
  modalDevLabel: { fontSize: 10, fontWeight: '700', color: colors.textMuted, letterSpacing: 0.8, marginBottom: 6 },
  modalDevText: { fontSize: 12, color: colors.textMuted, lineHeight: 17 },
  modalBtn: {
    marginTop: 20,
    backgroundColor: colors.accent,
    paddingVertical: 12,
    borderRadius: 10,
    alignItems: 'center',
  },
});
