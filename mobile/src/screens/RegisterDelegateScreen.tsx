import React, { useState } from 'react';
import {
  ActivityIndicator, ScrollView, StyleSheet,
  Text, TextInput, TouchableOpacity, View,
} from 'react-native';
// TextInput kept for the bio field
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { RootStackParamList } from '../App';
import { registerAsDelegate } from '../chain/governance';
import { getSigningKeypair } from '../chain/identity';
import { getPassportName } from '../chain/citizenState';
import { useAppModal } from '../components/AppModal';
import { colors } from '../theme';

type Props = NativeStackScreenProps<RootStackParamList, 'RegisterDelegate'>;
type Step = 'idle' | 'submitting' | 'done';

export default function RegisterDelegateScreen({ navigation }: Props) {
  const passportName = getPassportName();
  const [bio, setBio] = useState('');
  const [step, setStep] = useState<Step>('idle');
  const { showError } = useAppModal();

  const canSubmit = !!passportName && step === 'idle';

  async function submit() {
    if (!passportName) return;
    setStep('submitting');
    try {
      const { keypair } = await getSigningKeypair();
      await registerAsDelegate(keypair, passportName, bio.trim());
      setStep('done');
    } catch (e: any) {
      showError('Registration failed', e, 'Your delegate registration could not be submitted. Please try again.');
      setStep('idle');
    }
  }

  return (
    <ScrollView style={s.container} contentContainerStyle={s.content} keyboardShouldPersistTaps="handled">
      <Text style={s.title}>Become a Delegate</Text>
      <Text style={s.subtitle}>
        As a delegate your real name is publicly visible on-chain. Delegate registration uses
        this same account you already vote with — there is no separate delegate identity, so
        your full on-chain activity is publicly linkable to your public name once you register.
      </Text>

      <View style={s.infoBox}>
        <InfoRow icon="👤" text="Your display name is permanently public" />
        <InfoRow icon="🔗" text="Links your public name to this account's full on-chain activity" />
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
        placeholderTextColor={colors.textDim}
        multiline
        numberOfLines={5}
        maxLength={500}
        editable={step === 'idle'}
        accessibilityLabel="Bio or policy positions, optional"
      />
      <Text style={s.charCount}>{bio.length}/500</Text>

      {step === 'done' ? (
        <View style={s.successBox}>
          <Text style={s.successIcon}>✓</Text>
          <Text style={s.successTitle}>Registered as delegate</Text>
          <Text style={s.successSub}>
            You are now in Pending status. Gather 50 backers to become Active and receive delegations.
          </Text>
          <TouchableOpacity
            style={s.doneBtn}
            onPress={() => navigation.goBack()}
            accessibilityRole="button"
            accessibilityLabel="Back to Delegates"
          >
            <Text style={s.doneBtnText}>Back to Delegates</Text>
          </TouchableOpacity>
        </View>
      ) : (
        <TouchableOpacity
          style={[s.submitBtn, !canSubmit && s.submitBtnDisabled]}
          onPress={submit}
          disabled={!canSubmit}
          accessibilityRole="button"
          accessibilityLabel="Register as Delegate"
          accessibilityState={{ disabled: !canSubmit }}
        >
          {step === 'submitting'
            ? <ActivityIndicator color={colors.textPrimary} />
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
  container: { flex: 1, backgroundColor: colors.bg },
  content: { padding: 24, paddingBottom: 40 },
  title: { fontSize: 22, fontWeight: '700', color: colors.textPrimary, marginBottom: 8 },
  subtitle: { fontSize: 14, color: colors.textMuted, lineHeight: 20, marginBottom: 20 },
  infoBox: {
    backgroundColor: colors.card, borderRadius: 14, padding: 16,
    borderWidth: 1, borderColor: colors.border, marginBottom: 28, gap: 12,
  },
  infoRow: { flexDirection: 'row', alignItems: 'center', gap: 12 },
  infoIcon: { fontSize: 18, width: 24 },
  infoText: { fontSize: 13, color: colors.textBody, flex: 1, lineHeight: 18 },
  label: { fontSize: 12, fontWeight: '600', color: colors.textSecondary, textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 8 },
  optional: { fontWeight: '400', textTransform: 'none', letterSpacing: 0 },
  input: {
    backgroundColor: colors.card, borderWidth: 1, borderColor: colors.border,
    borderRadius: 12, padding: 14, color: colors.textPrimary, fontSize: 14, marginBottom: 4,
  },
  bioInput: { height: 120, textAlignVertical: 'top', marginBottom: 4 },
  passportNameBox: {
    backgroundColor: colors.card, borderWidth: 1, borderColor: colors.border,
    borderRadius: 12, padding: 14, flexDirection: 'row', alignItems: 'center',
    marginBottom: 20, gap: 12,
  },
  passportNameLeft: { flex: 1 },
  passportName: { fontSize: 16, fontWeight: '600', color: colors.textPrimary, marginBottom: 3 },
  passportNameSub: { fontSize: 11, color: colors.textDim },
  passportLock: { fontSize: 18 },
  charCount: { fontSize: 11, color: colors.textDim, textAlign: 'right', marginBottom: 28 },
  submitBtn: {
    backgroundColor: colors.accent, paddingVertical: 16,
    borderRadius: 14, alignItems: 'center',
  },
  submitBtnDisabled: { opacity: 0.4 },
  submitBtnText: { color: colors.textPrimary, fontWeight: '700', fontSize: 16 },
  successBox: { alignItems: 'center', paddingTop: 16, gap: 12 },
  successIcon: { fontSize: 56, color: colors.success },
  successTitle: { fontSize: 20, fontWeight: '700', color: colors.success },
  successSub: { fontSize: 14, color: colors.textMuted, textAlign: 'center', lineHeight: 20 },
  doneBtn: {
    marginTop: 8, backgroundColor: colors.accent, paddingVertical: 14,
    paddingHorizontal: 32, borderRadius: 12,
  },
  doneBtnText: { color: colors.textPrimary, fontWeight: '600', fontSize: 15 },
});
