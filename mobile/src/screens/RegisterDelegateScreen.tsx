import React, { useState } from 'react';
import {
  ActivityIndicator, Alert, ScrollView, StyleSheet,
  Text, TextInput, TouchableOpacity, View,
} from 'react-native';
// TextInput kept for the bio field
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { RootStackParamList } from '../App';
import { registerAsDelegate } from '../chain/governance';
import { getSigningKeypair } from '../chain/identity';
import { getPassportName } from '../chain/citizenState';

type Props = NativeStackScreenProps<RootStackParamList, 'RegisterDelegate'>;
type Step = 'idle' | 'submitting' | 'done';

export default function RegisterDelegateScreen({ navigation }: Props) {
  const passportName = getPassportName();
  const [bio, setBio] = useState('');
  const [step, setStep] = useState<Step>('idle');

  const canSubmit = !!passportName && step === 'idle';

  async function submit() {
    if (!passportName) return;
    setStep('submitting');
    try {
      const { keypair } = await getSigningKeypair();
      await registerAsDelegate(keypair, passportName, bio.trim());
      setStep('done');
    } catch (e: any) {
      Alert.alert('Registration failed', e.message);
      setStep('idle');
    }
  }

  return (
    <ScrollView style={s.container} contentContainerStyle={s.content} keyboardShouldPersistTaps="handled">
      <Text style={s.title}>Become a Delegate</Text>
      <Text style={s.subtitle}>
        As a delegate your real name is publicly visible on-chain. Your delegate identity is
        cryptographically separate from your citizen identity — you remain anonymous as a voter.
      </Text>

      <View style={s.infoBox}>
        <InfoRow icon="👤" text="Your display name is permanently public" />
        <InfoRow icon="🔑" text="Uses a separate ZK identity from your citizen account" />
        <InfoRow icon="📊" text="You need 50 citizen backers to become active" />
        <InfoRow icon="⏱" text="Max 3 consecutive terms, then a mandatory break" />
      </View>

      <Text style={s.label}>Public Name</Text>
      <View style={s.passportNameBox}>
        <View style={s.passportNameLeft}>
          <Text style={s.passportName}>{passportName ?? '—'}</Text>
          <Text style={s.passportNameSub}>Verified from your passport · cannot be changed</Text>
        </View>
        <Text style={s.passportLock}>🔒</Text>
      </View>

      <Text style={s.label}>Bio / Policy Positions <Text style={s.optional}>(optional)</Text></Text>
      <TextInput
        style={[s.input, s.bioInput]}
        value={bio}
        onChangeText={setBio}
        placeholder="Describe your positions and why citizens should delegate to you…"
        placeholderTextColor="#4b5563"
        multiline
        numberOfLines={5}
        maxLength={500}
        editable={step === 'idle'}
      />
      <Text style={s.charCount}>{bio.length}/500</Text>

      {step === 'done' ? (
        <View style={s.successBox}>
          <Text style={s.successIcon}>✓</Text>
          <Text style={s.successTitle}>Registered as delegate</Text>
          <Text style={s.successSub}>
            You are now in Pending status. Gather 50 backers to become Active and receive delegations.
          </Text>
          <TouchableOpacity style={s.doneBtn} onPress={() => navigation.goBack()}>
            <Text style={s.doneBtnText}>Back to Delegates</Text>
          </TouchableOpacity>
        </View>
      ) : (
        <TouchableOpacity
          style={[s.submitBtn, !canSubmit && s.submitBtnDisabled]}
          onPress={submit}
          disabled={!canSubmit}
        >
          {step === 'submitting'
            ? <ActivityIndicator color="#fff" />
            : <Text style={s.submitBtnText}>Register as Delegate</Text>}
        </TouchableOpacity>
      )}
    </ScrollView>
  );
}

function InfoRow({ icon, text }: { icon: string; text: string }) {
  return (
    <View style={s.infoRow}>
      <Text style={s.infoIcon}>{icon}</Text>
      <Text style={s.infoText}>{text}</Text>
    </View>
  );
}

const s = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#0f1117' },
  content: { padding: 24, paddingBottom: 40 },
  title: { fontSize: 22, fontWeight: '700', color: '#ffffff', marginBottom: 8 },
  subtitle: { fontSize: 14, color: '#6b7280', lineHeight: 20, marginBottom: 20 },
  infoBox: {
    backgroundColor: '#161b27', borderRadius: 14, padding: 16,
    borderWidth: 1, borderColor: '#1f2937', marginBottom: 28, gap: 12,
  },
  infoRow: { flexDirection: 'row', alignItems: 'center', gap: 12 },
  infoIcon: { fontSize: 18, width: 24 },
  infoText: { fontSize: 13, color: '#d1d5db', flex: 1, lineHeight: 18 },
  label: { fontSize: 12, fontWeight: '600', color: '#9ca3af', textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 8 },
  optional: { fontWeight: '400', textTransform: 'none', letterSpacing: 0 },
  input: {
    backgroundColor: '#161b27', borderWidth: 1, borderColor: '#1f2937',
    borderRadius: 12, padding: 14, color: '#ffffff', fontSize: 14, marginBottom: 4,
  },
  bioInput: { height: 120, textAlignVertical: 'top', marginBottom: 4 },
  passportNameBox: {
    backgroundColor: '#161b27', borderWidth: 1, borderColor: '#1f2937',
    borderRadius: 12, padding: 14, flexDirection: 'row', alignItems: 'center',
    marginBottom: 20, gap: 12,
  },
  passportNameLeft: { flex: 1 },
  passportName: { fontSize: 16, fontWeight: '600', color: '#ffffff', marginBottom: 3 },
  passportNameSub: { fontSize: 11, color: '#4b5563' },
  passportLock: { fontSize: 18 },
  charCount: { fontSize: 11, color: '#4b5563', textAlign: 'right', marginBottom: 28 },
  submitBtn: {
    backgroundColor: '#6C63FF', paddingVertical: 16,
    borderRadius: 14, alignItems: 'center',
  },
  submitBtnDisabled: { opacity: 0.4 },
  submitBtnText: { color: '#fff', fontWeight: '700', fontSize: 16 },
  successBox: { alignItems: 'center', paddingTop: 16, gap: 12 },
  successIcon: { fontSize: 56, color: '#22c55e' },
  successTitle: { fontSize: 20, fontWeight: '700', color: '#22c55e' },
  successSub: { fontSize: 14, color: '#6b7280', textAlign: 'center', lineHeight: 20 },
  doneBtn: {
    marginTop: 8, backgroundColor: '#6C63FF', paddingVertical: 14,
    paddingHorizontal: 32, borderRadius: 12,
  },
  doneBtnText: { color: '#fff', fontWeight: '600', fontSize: 15 },
});
