import React, { useState } from 'react';
import {
  ActivityIndicator, ScrollView, StyleSheet,
  Text, TextInput, TouchableOpacity, View,
} from 'react-native';
// TextInput kept for the display-name and bio fields
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { RootStackParamList } from '../App';
import { getOrCreateDelegatePersonaKeypair } from '../chain/keystoreWallet';
import { getPassportName } from '../chain/citizenState';
import { useAppModal } from '../components/AppModal';
import { colors } from '../theme';

type Props = NativeStackScreenProps<RootStackParamList, 'RegisterDelegate'>;
type Step = 'idle' | 'submitting' | 'done';

/**
 * Thrown at the exact point real delegate-persona proving would need to run — see `submit()`.
 * Mirrors `RegisterScreen.tsx`/`RecoverAccountScreen.tsx`'s own local `NotImplementedError`:
 * a precise "everything real up to here worked, this specific next step doesn't exist yet"
 * signal, not a generic failure.
 */
class NotImplementedError extends Error {}

export default function RegisterDelegateScreen({ navigation }: Props) {
  const isRegisteredCitizen = !!getPassportName();
  const [displayName, setDisplayName] = useState('');
  const [bio, setBio] = useState('');
  const [step, setStep] = useState<Step>('idle');
  const { showError, showInfo } = useAppModal();

  const canSubmit = isRegisteredCitizen && displayName.trim().length > 0 && step === 'idle';

  async function submit() {
    if (!canSubmit) return;
    setStep('submitting');
    try {
      // Real: the delegate-persona account is a second, independent keypair — never the
      // citizen's main wallet — generated/loaded the same hardware-backed way
      // `keystoreWallet.ts` already does for it. This is the actual account the proof below
      // would bind and that would sign register_as_delegate.
      const personaKeypair = await getOrCreateDelegatePersonaKeypair();

      // Real, once it exists: a fresh delegate-persona ZK proof (`zkProving.ts`'s
      // `proveDelegatePersona`, circuit `circuits/oprf-identity-anchor/delegate-persona`)
      // binding `personaKeypair.address` via a dedicated 5-committee OPRF round-trip, then
      // `registerAsDelegate(personaKeypair, { ... })` (`governance.ts`) submitting it. Neither
      // step can run yet: proving needs a real on-device Noir prover
      // (`zkProving.ts`'s `NoirProverUnavailableError`) AND a real OPRF committee to answer
      // the query round (`oprfCombine.ts`'s `combineCommitteeSlotResponses` is a documented,
      // deliberate stub) — the same two dependencies ordinary citizen registration itself is
      // blocked on (`RegisterScreen.tsx`).
      throw new NotImplementedError(
        `Delegate-persona account ready (${personaKeypair.address}) — but proving the ` +
          'delegate-persona circuit needs both a real on-device Noir prover and a real OPRF ' +
          'committee to answer its query round, neither of which exist in this build yet.',
      );
    } catch (e: any) {
      if (e instanceof NotImplementedError) {
        showInfo(
          'Almost there',
          "Delegate registration needs the same on-device proving system as citizen " +
            "registration, which isn't finished yet — please check back in a future update.",
          __DEV__ ? e.message : undefined,
        );
      } else {
        showError('Registration failed', e, 'Your delegate registration could not be submitted. Please try again.');
      }
    } finally {
      setStep('idle');
    }
  }

  return (
    <ScrollView style={s.container} contentContainerStyle={s.content} keyboardShouldPersistTaps="handled">
      <Text style={s.title}>Become a Delegate</Text>
      <Text style={s.subtitle}>
        Becoming a delegate now uses a genuinely separate on-chain identity — a second account,
        proven by a dedicated ZK circuit to belong to a citizen in good standing, without
        revealing which one. Your delegate persona's on-chain activity (this registration,
        receiving vote delegations, term history) is not linkable back to your personal citizen
        account by the cryptography itself. What this does NOT hide: your chosen display name
        below is whatever you type — if you use your real name, or something recognizable, that
        is a public choice, not something the protocol protects. And no cryptography can prevent
        ordinary chain-analysis clues — funding this new persona account from your known citizen
        account, or registering right after other identifiable activity, can still let an
        outside observer connect the two. Finally, this whole feature — like every part of this
        app gated on identity proofs — cannot actually be exercised yet: it depends on a real
        OPRF committee that does not exist yet (see the in-app status note on the Identity
        screen), so registering as a delegate isn't possible in this build.
      </Text>

      <View style={s.infoBox}>
        <InfoRow icon="🕶️" text="A separate persona account — not your citizen account" />
        <InfoRow icon="✍️" text="Your display name is whatever you choose to make public" />
        <InfoRow icon="📊" text="You need 50 citizen backers to become active" />
        <InfoRow icon="⏱" text="Max 3 consecutive terms, then a mandatory break" />
      </View>

      <Text style={s.label}>Display Name</Text>
      <TextInput
        style={s.input}
        value={displayName}
        onChangeText={setDisplayName}
        placeholder="Chosen public name for this delegate persona"
        placeholderTextColor={colors.textDim}
        maxLength={64}
        editable={step === 'idle'}
        accessibilityLabel="Delegate display name"
      />
      {!isRegisteredCitizen && (
        <Text style={s.notCitizenNote}>You must be a registered citizen to become a delegate.</Text>
      )}

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
  notCitizenNote: { fontSize: 12, color: colors.warning, marginBottom: 20 },
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
