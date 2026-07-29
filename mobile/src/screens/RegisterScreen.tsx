import React, { useState } from 'react';
import { Alert, Platform, StyleSheet, Text, TextInput, TouchableOpacity, View, ActivityIndicator } from 'react-native';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { RootStackParamList } from '../App';
import { readPassport, RawPassportData } from '../native/nfcPassportReader';
import { buildCircuitInputs } from '../chain/sodParser';

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

export default function RegisterScreen({ navigation }: Props) {
  const [step, setStep] = useState<Step>('idle');
  // MRZ fields feed BAC key derivation for the NFC read (see
  // ../native/nfcPassportReader.ts). Dates are YYMMDD, matching the Android
  // native module's BACKey requirement (confirmed from JMRTD source, HANDOFF
  // log #58) — not validated/masked here yet, that's real form-UX work for
  // whoever picks this up next.
  const [documentNumber, setDocumentNumber] = useState('');
  const [dateOfBirth, setDateOfBirth] = useState('');
  const [dateOfExpiry, setDateOfExpiry] = useState('');
  const [rawPassport, setRawPassport] = useState<RawPassportData | null>(null);

  const mrzComplete = documentNumber.length > 0 && dateOfBirth.length === 6 && dateOfExpiry.length === 6;

  async function start() {
    if (Platform.OS !== 'android') {
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
      // circuit witness inputs.
      const raw = await readPassport({ documentNumber, dateOfBirth, dateOfExpiry });
      setRawPassport(raw);
      setStep('liveness');
      // TODO: await FaceMatch.verify(scan.faceImage);
      setStep('proving');
      // Real as of this session (../chain/sodParser.ts) — parses the SOD's
      // CMS SignedData structure and assembles the RegisterIdentityBuilder
      // circuit's dg1/dg15/encapsulatedContent/signedAttributes/pubkey/
      // signature inputs, and identifies which circuit variant this
      // passport needs. Still stops here, though, blocked on two things
      // buildCircuitInputs deliberately does NOT produce (see its module
      // doc comment for why both are real, unresolved blockers, not
      // oversights):
      //  (1) `skIdentity` — must be a genuine locally-generated secret, not
      //      derived from public passport bytes.
      //  (2) `slaveMerkleRoot` / `slaveMerkleInclusionBranches` — an
      //      inclusion proof against Rarimo's live `CertificatesSMT`
      //      registry; no documented client API for that was found yet.
      // ...plus obtaining a `.wcd` witness graph + the ~515MB proving key
      // for the specific variant identified below (see ../chain/
      // zkProving.ts's module doc + HANDOFF item 8/log #56).
      // Once all of that exists, this step becomes roughly:
      //   const graphPath = await fetchZkAsset(WCD_GRAPH_HASH_FOR[variant.name]);
      //   const zkeyPath = await fetchZkAsset(PROVING_KEY_HASH_FOR[variant.name]);
      //   const fullInputs = { ...inputs, skIdentity, slaveMerkleRoot, slaveMerkleInclusionBranches };
      //   const witness = await computeWitness(JSON.stringify(fullInputs), graphPath);
      //   const { proof, pub_signals } = await generateProof(zkeyPath, toBase64(witness));
      //   const zkProof = encodeGroth16Proof(proof as SnarkjsGroth16Proof, 0);
      const { variant } = buildCircuitInputs(raw.dg1, raw.dg15, raw.sod);
      throw new NotImplementedError(
        `Passport chip read succeeded (DG1: ${raw.dg1.length}B, DG15: ${raw.dg15.length}B, ` +
          `SOD: ${raw.sod.length}B) and circuit inputs were assembled for variant "${variant.name}" — ` +
          'but proof generation still needs a real identity secret, a Merkle inclusion proof from ' +
          "Rarimo's certificate registry, and this variant's proving key/witness graph, none of which " +
          'exist yet. See RegisterScreen.tsx TODOs.',
      );
    } catch (e: any) {
      if (e instanceof NotImplementedError) {
        Alert.alert('Scan succeeded — registration not complete yet', e.message);
      } else {
        Alert.alert('Registration failed', e.message);
      }
      setStep('idle');
    }
  }

  const activeIndex = STEP_ORDER.indexOf(step);

  return (
    <View style={s.container}>
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
                  <ActivityIndicator size="small" color="#fff" />
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
              placeholderTextColor="#4b5563"
              autoCapitalize="characters"
            />
            <Text style={s.mrzLabel}>Date of birth (YYMMDD)</Text>
            <TextInput
              style={s.mrzInput}
              value={dateOfBirth}
              onChangeText={setDateOfBirth}
              placeholder="e.g. 740812"
              placeholderTextColor="#4b5563"
              keyboardType="number-pad"
              maxLength={6}
            />
            <Text style={s.mrzLabel}>Date of expiry (YYMMDD)</Text>
            <TextInput
              style={s.mrzInput}
              value={dateOfExpiry}
              onChangeText={setDateOfExpiry}
              placeholder="e.g. 341231"
              placeholderTextColor="#4b5563"
              keyboardType="number-pad"
              maxLength={6}
            />
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
            onPress={start}
            disabled={!mrzComplete}
          >
            <Text style={s.btnText}>Begin Registration</Text>
          </TouchableOpacity>
        </>
      )}

      {step === 'done' && (
        <View style={s.successBox}>
          <Text style={s.successIcon}>✓</Text>
          <Text style={s.successText}>You are now a registered citizen</Text>
          <TouchableOpacity style={[s.btn, { marginTop: 24 }]} onPress={() => navigation.goBack()}>
            <Text style={s.btnText}>Go to Home</Text>
          </TouchableOpacity>
        </View>
      )}
    </View>
  );
}

const s = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#0f1117', padding: 24 },
  title: { fontSize: 24, fontWeight: '700', color: '#ffffff', marginBottom: 8 },
  subtitle: { fontSize: 14, color: '#6b7280', lineHeight: 20, marginBottom: 32 },
  stepList: { gap: 20, marginBottom: 40 },
  stepRow: { flexDirection: 'row', alignItems: 'flex-start', gap: 14 },
  stepNum: {
    width: 36, height: 36, borderRadius: 18,
    alignItems: 'center', justifyContent: 'center', marginTop: 2,
  },
  stepPending: { backgroundColor: '#1f2937' },
  stepActive: { backgroundColor: '#6C63FF' },
  stepDone: { backgroundColor: '#166534' },
  stepNumText: { color: '#ffffff', fontWeight: '700', fontSize: 14 },
  stepText: { flex: 1 },
  stepLabel: { fontSize: 15, fontWeight: '600', color: '#6b7280', marginBottom: 2 },
  stepLabelActive: { color: '#ffffff' },
  stepDetail: { fontSize: 12, color: '#4b5563', lineHeight: 17 },
  btn: {
    backgroundColor: '#6C63FF',
    paddingVertical: 16,
    borderRadius: 14,
    alignItems: 'center',
  },
  btnText: { color: '#ffffff', fontWeight: '700', fontSize: 16 },
  successBox: { alignItems: 'center', gap: 12 },
  successIcon: { fontSize: 56, color: '#22c55e' },
  successText: { fontSize: 18, fontWeight: '600', color: '#22c55e', textAlign: 'center' },
  mrzForm: { marginBottom: 20, gap: 6 },
  mrzLabel: { fontSize: 12, fontWeight: '600', color: '#9ca3af', marginTop: 10 },
  mrzInput: {
    borderWidth: 1,
    borderColor: '#1f2937',
    borderRadius: 10,
    paddingHorizontal: 14,
    paddingVertical: 10,
    fontSize: 15,
    color: '#ffffff',
    backgroundColor: '#161a23',
  },
  mrzHint: { fontSize: 11, color: '#4b5563', lineHeight: 15, marginTop: 10 },
  scanResult: { fontSize: 12, color: '#22c55e', textAlign: 'center', marginBottom: 12 },
  btnDisabled: { backgroundColor: '#374151' },
});
