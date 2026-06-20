/**
 * Citizen registration screen.
 *
 * Guides the user through:
 *  1. NFC passport scan (via Rarimo SDK — TODO: install native module)
 *  2. On-device face-match liveness check (Apple Vision / MobileFaceNet)
 *  3. ZK proof generation (on device — nothing leaves the phone)
 *  4. Submission to the chain
 */
import React, { useState } from 'react';
import { Alert, Button, StyleSheet, Text, View } from 'react-native';

type Step = 'idle' | 'nfc' | 'liveness' | 'proving' | 'submitting' | 'done';

export default function RegisterScreen() {
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
      setStep('done');
    } catch (e: any) {
      Alert.alert('Registration failed', e.message);
      setStep('idle');
    }
  }

  const label: Record<Step, string> = {
    idle: 'Tap to register as a citizen',
    nfc: 'Scanning passport NFC chip…',
    liveness: 'Verifying face match…',
    proving: 'Generating ZK proof on device…',
    submitting: 'Submitting to blockchain…',
    done: 'Registration complete!',
  };

  return (
    <View style={styles.container}>
      <Text style={styles.title}>Citizen Registration</Text>
      <Text style={styles.status}>{label[step]}</Text>
      {step === 'idle' && <Button title="Start" onPress={start} />}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, alignItems: 'center', justifyContent: 'center', padding: 24 },
  title: { fontSize: 24, fontWeight: '700', marginBottom: 16 },
  status: { fontSize: 16, marginBottom: 32, textAlign: 'center' },
});
