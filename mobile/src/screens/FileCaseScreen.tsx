/**
 * "File a case" modal stack screen (see App.tsx — registered as a
 * `presentation: 'modal'` Stack.Screen, same category as Register/
 * RegisterDelegate). Reachable from CasesScreen's "File a case" button.
 *
 * Mirrors RegisterDelegateScreen.tsx's single-form-submission structure:
 * Step = 'idle' | 'submitting' | 'done', getSigningKeypair() -> wrapper call
 * -> useAppModal().showError on failure, disabled submit button with
 * ActivityIndicator while submitting, success view with return-navigation.
 */
import React, { useState } from 'react';
import {
  ActivityIndicator,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  TouchableOpacity,
  View,
} from 'react-native';
import { hexToU8a, isHex } from '@polkadot/util';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { RootStackParamList } from '../App';
import { CaseSubject, fileCase } from '../chain/courts';
import { getSigningKeypair } from '../chain/identity';
import { useAppModal } from '../components/AppModal';
import { colors } from '../theme';

type Props = NativeStackScreenProps<RootStackParamList, 'FileCase'>;
type Step = 'idle' | 'submitting' | 'done';
type SubjectKind = 'General' | 'LawChallenge' | 'TreasuryDispute' | 'CitizenConduct';

const KIND_OPTIONS: { kind: SubjectKind; label: string }[] = [
  { kind: 'General', label: 'General' },
  { kind: 'LawChallenge', label: 'Law Challenge' },
  { kind: 'TreasuryDispute', label: 'Treasury Dispute' },
  { kind: 'CitizenConduct', label: 'Citizen Conduct' },
];

export default function FileCaseScreen({ navigation }: Props) {
  const [kind, setKind] = useState<SubjectKind>('General');
  const [lawId, setLawId] = useState('');
  const [departmentId, setDepartmentId] = useState('');
  // Citizen Conduct target: entered as raw hex rather than looked up by
  // account address. There is no on-chain lookup this app can use today to
  // resolve "this citizen's account address" -> "their registered
  // CitizenNullifier" (identity.ts only ever queries a nullifier for the
  // caller's own address, e.g. isCitizen()) — nullifiers exist precisely so
  // an account can't be trivially linked back to their identity from the
  // outside. A hex text input is a stopgap: it's accurate (it's literally
  // the on-chain value fileCase needs) but bad UX — nobody has a citizen's
  // 32-byte nullifier memorized or easily copyable. A real fix would need a
  // dedicated on-chain or off-chain lookup flow, which doesn't exist yet.
  const [nullifierHex, setNullifierHex] = useState('');
  const [suspensionBlocks, setSuspensionBlocks] = useState('');
  const [step, setStep] = useState<Step>('idle');
  const { showError } = useAppModal();

  function buildSubject(): CaseSubject | null {
    switch (kind) {
      case 'General':
        return { General: null };
      case 'LawChallenge': {
        const id = Number(lawId);
        if (!lawId.trim() || !Number.isInteger(id) || id < 0) return null;
        return { LawChallenge: { law_id: id } };
      }
      case 'TreasuryDispute': {
        const id = Number(departmentId);
        if (!departmentId.trim() || !Number.isInteger(id) || id < 0) return null;
        return { TreasuryDispute: { department_id: id } };
      }
      case 'CitizenConduct': {
        const hex = nullifierHex.trim();
        if (!hex || !isHex(hex)) return null;
        let bytes: Uint8Array;
        try {
          bytes = hexToU8a(hex);
        } catch {
          return null;
        }
        if (bytes.length !== 32) return null;

        let suspension: number | null = null;
        if (suspensionBlocks.trim()) {
          const n = Number(suspensionBlocks);
          if (!Number.isInteger(n) || n < 0) return null;
          suspension = n;
        }
        return { CitizenConduct: { nullifier: bytes, suspension_blocks: suspension } };
      }
    }
  }

  const subjectPreview = buildSubject();
  const canSubmit = step === 'idle' && subjectPreview !== null;

  async function submit() {
    const subject = buildSubject();
    if (!subject) return;
    setStep('submitting');
    try {
      const { keypair } = await getSigningKeypair();
      await fileCase(keypair, subject);
      setStep('done');
    } catch (e: any) {
      showError('Filing failed', e, 'Your case could not be filed. Please try again.');
      setStep('idle');
    }
  }

  if (step === 'done') {
    return (
      <View style={s.successContainer}>
        <Text style={s.successIcon}>✓</Text>
        <Text style={s.successTitle}>Case filed</Text>
        <Text style={s.successSub}>
          Your case has been submitted. An AI judge will issue an initial ruling; you'll be able to
          track its status and appeal from the Courts tab.
        </Text>
        <TouchableOpacity
          style={s.doneBtn}
          onPress={() => navigation.goBack()}
          accessibilityRole="button"
          accessibilityLabel="Back to Courts"
        >
          <Text style={s.doneBtnText}>Back to Courts</Text>
        </TouchableOpacity>
      </View>
    );
  }

  return (
    <ScrollView style={s.container} contentContainerStyle={s.content} keyboardShouldPersistTaps="handled">
      <Text style={s.title}>File a Case</Text>
      <Text style={s.subtitle}>
        Cases are first reviewed by an AI judge. Rulings may be appealed to a citizen jury.
      </Text>

      <Text style={s.label}>Case type</Text>
      <View style={s.kindGrid}>
        {KIND_OPTIONS.map((opt) => (
          <TouchableOpacity
            key={opt.kind}
            style={[s.kindBtn, kind === opt.kind && s.kindBtnActive]}
            onPress={() => setKind(opt.kind)}
            disabled={step !== 'idle'}
            accessibilityRole="button"
            accessibilityLabel={`Case type: ${opt.label}`}
            accessibilityState={{ selected: kind === opt.kind }}
          >
            <Text style={[s.kindBtnText, kind === opt.kind && s.kindBtnTextActive]}>{opt.label}</Text>
          </TouchableOpacity>
        ))}
      </View>

      {kind === 'LawChallenge' && (
        <>
          <Text style={s.label}>Law ID</Text>
          <TextInput
            style={s.input}
            value={lawId}
            onChangeText={setLawId}
            placeholder="e.g. 3"
            placeholderTextColor={colors.textDim}
            keyboardType="number-pad"
            editable={step === 'idle'}
            accessibilityLabel="Law ID"
          />
        </>
      )}

      {kind === 'TreasuryDispute' && (
        <>
          <Text style={s.label}>Department ID</Text>
          <TextInput
            style={s.input}
            value={departmentId}
            onChangeText={setDepartmentId}
            placeholder="e.g. 2"
            placeholderTextColor={colors.textDim}
            keyboardType="number-pad"
            editable={step === 'idle'}
            accessibilityLabel="Department ID"
          />
        </>
      )}

      {kind === 'CitizenConduct' && (
        <>
          <Text style={s.label}>Citizen nullifier (hex)</Text>
          <TextInput
            style={s.input}
            value={nullifierHex}
            onChangeText={setNullifierHex}
            placeholder="0x…(32 bytes)"
            placeholderTextColor={colors.textDim}
            autoCapitalize="none"
            autoCorrect={false}
            editable={step === 'idle'}
            accessibilityLabel="Citizen nullifier, hex encoded"
          />
          <Text style={s.hint}>
            Raw 32-byte identity nullifier of the citizen this case concerns. There's no in-app lookup
            from an account address to a nullifier yet — this must be supplied directly.
          </Text>

          <Text style={s.label}>
            Suspension length (blocks) <Text style={s.optional}>(optional)</Text>
          </Text>
          <TextInput
            style={s.input}
            value={suspensionBlocks}
            onChangeText={setSuspensionBlocks}
            placeholder="Leave blank for no specific request"
            placeholderTextColor={colors.textDim}
            keyboardType="number-pad"
            editable={step === 'idle'}
            accessibilityLabel="Requested suspension length in blocks, optional"
          />
        </>
      )}

      <TouchableOpacity
        style={[s.submitBtn, !canSubmit && s.submitBtnDisabled]}
        onPress={submit}
        disabled={!canSubmit}
        accessibilityRole="button"
        accessibilityLabel="File case"
        accessibilityState={{ disabled: !canSubmit }}
      >
        {step === 'submitting' ? (
          <ActivityIndicator color={colors.textPrimary} />
        ) : (
          <Text style={s.submitBtnText}>File case</Text>
        )}
      </TouchableOpacity>
    </ScrollView>
  );
}

const s = StyleSheet.create({
  container: { flex: 1, backgroundColor: colors.bg },
  content: { padding: 24, paddingBottom: 40 },
  title: { fontSize: 22, fontWeight: '700', color: colors.textPrimary, marginBottom: 8 },
  subtitle: { fontSize: 14, color: colors.textMuted, lineHeight: 20, marginBottom: 24 },
  label: { fontSize: 12, fontWeight: '600', color: colors.textSecondary, textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 8, marginTop: 4 },
  optional: { fontWeight: '400', textTransform: 'none', letterSpacing: 0 },
  kindGrid: { flexDirection: 'row', flexWrap: 'wrap', gap: 10, marginBottom: 20 },
  kindBtn: {
    paddingVertical: 10,
    paddingHorizontal: 14,
    borderRadius: 10,
    borderWidth: 1,
    borderColor: colors.border,
    backgroundColor: colors.card,
  },
  kindBtnActive: { backgroundColor: colors.accent, borderColor: colors.accent },
  kindBtnText: { color: colors.textSecondary, fontWeight: '600', fontSize: 13 },
  kindBtnTextActive: { color: colors.textPrimary },
  input: {
    backgroundColor: colors.card, borderWidth: 1, borderColor: colors.border,
    borderRadius: 12, padding: 14, color: colors.textPrimary, fontSize: 14, marginBottom: 8,
  },
  hint: { fontSize: 11, color: colors.textDim, lineHeight: 16, marginBottom: 16 },
  submitBtn: {
    backgroundColor: colors.accent, paddingVertical: 16,
    borderRadius: 14, alignItems: 'center', marginTop: 16,
  },
  submitBtnDisabled: { opacity: 0.4 },
  submitBtnText: { color: colors.textPrimary, fontWeight: '700', fontSize: 16 },
  successContainer: { flex: 1, backgroundColor: colors.bg, alignItems: 'center', justifyContent: 'center', padding: 24, gap: 12 },
  successIcon: { fontSize: 56, color: colors.success },
  successTitle: { fontSize: 20, fontWeight: '700', color: colors.success },
  successSub: { fontSize: 14, color: colors.textMuted, textAlign: 'center', lineHeight: 20 },
  doneBtn: {
    marginTop: 8, backgroundColor: colors.accent, paddingVertical: 14,
    paddingHorizontal: 32, borderRadius: 12,
  },
  doneBtnText: { color: colors.textPrimary, fontWeight: '600', fontSize: 15 },
});
