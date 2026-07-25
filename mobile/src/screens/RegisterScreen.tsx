import React, { useState } from 'react';
import { Alert, StyleSheet, Text, TouchableOpacity, View, ActivityIndicator } from 'react-native';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { RootStackParamList } from '../App';
import { setRegistered, setPassportName } from '../chain/citizenState';

type Props = NativeStackScreenProps<RootStackParamList, 'Register'>;

type Step = 'idle' | 'nfc' | 'liveness' | 'proving' | 'submitting' | 'done';

const STEPS = [
  { id: 'nfc',        label: 'Scan passport NFC chip',        detail: 'Hold your phone to your biometric passport' },
  { id: 'liveness',   label: 'Face verification',             detail: 'On-device face match + liveness check' },
  { id: 'proving',    label: 'Generate ZK proof',             detail: 'Proof generated on device — nothing leaves your phone' },
  { id: 'submitting', label: 'Submit to blockchain',          detail: 'Proof posted to Agora chain; no personal data stored' },
];

const STEP_ORDER: Step[] = ['idle', 'nfc', 'liveness', 'proving', 'submitting', 'done'];

export default function RegisterScreen({ navigation }: Props) {
  const [step, setStep] = useState<Step>('idle');

  async function start() {
    try {
      setStep('nfc');
      // TODO: const scan = await RarimoSDK.scanPassport();
      setStep('liveness');
      // TODO: await FaceMatch.verify(scan.faceImage);
      setStep('proving');
      // TODO: const { nullifier, proof, publicInputs } = await RarimoSDK.generateProof(scan);
      setStep('submitting');
      // TODO: await registerCitizen(pair, { nullifier, zkProof: proof, publicInputs });
      // In the real system the name is extracted from the passport MRZ during the NFC scan above.
      setPassportName('Jordan Lee');
      setRegistered(true);
      setStep('done');
    } catch (e: any) {
      Alert.alert('Registration failed', e.message);
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
        <TouchableOpacity style={s.btn} onPress={start}>
          <Text style={s.btnText}>Begin Registration</Text>
        </TouchableOpacity>
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
});
