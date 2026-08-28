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
import { useTranslation } from 'react-i18next';
import { RootStackParamList } from '../App';
import { CaseSubject, fileCase } from '../chain/courts';
import { getSigningKeypair } from '../chain/identity';
import { useAppModal } from '../components/AppModal';
import { colors } from '../theme';

type Props = NativeStackScreenProps<RootStackParamList, 'FileCase'>;
type Step = 'idle' | 'submitting' | 'done';
type SubjectKind = 'General' | 'LawChallenge' | 'TreasuryDispute' | 'CitizenConduct' | 'TierConflict';

// Order of case-type options; the display label for each is looked up from
// the `fileCase` i18n namespace via KIND_LABEL_KEYS below, not stored here.
const KIND_ORDER: SubjectKind[] = [
  'General',
  'LawChallenge',
  'TreasuryDispute',
  'CitizenConduct',
  'TierConflict',
];

const KIND_LABEL_KEYS: Record<SubjectKind, string> = {
  General: 'kindOptions.general',
  LawChallenge: 'kindOptions.lawChallenge',
  TreasuryDispute: 'kindOptions.treasuryDispute',
  CitizenConduct: 'kindOptions.citizenConduct',
  TierConflict: 'kindOptions.tierConflict',
};

/**
 * `pallet_courts::file_case` requires a ZK citizenship proof for these three case types
 * (`Error::MissingZkProof` otherwise — see `courts.ts`'s `CaseFilingProof` doc comment).
 * No Noir prover native module is registered in this project yet (`chain/zkProving.ts`'s
 * documented blocker, the same one gating registration/reverification/migration/delegate-
 * persona proving), so this screen cannot build one — these are shown disabled, with a short
 * explanatory note, rather than omitted outright or left selectable for a submission that's
 * guaranteed to fail on-chain.
 */
const PROOF_REQUIRED_KINDS = new Set<SubjectKind>(['LawChallenge', 'TreasuryDispute', 'TierConflict']);

export default function FileCaseScreen({ navigation }: Props) {
  const { t } = useTranslation('fileCase');
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
      case 'TierConflict':
        // Unreachable in practice — this kind is disabled in the picker below (see
        // PROOF_REQUIRED_KINDS) because filing it requires an on-device ZK proof
        // this app cannot generate yet. Handled explicitly for exhaustiveness
        // rather than falling through. (LawChallenge/TreasuryDispute above are
        // likewise unreachable while disabled, but keep their real branches since
        // that logic predates this fix and costs nothing to leave in place.)
        return null;
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
      showError(t('filingFailedTitle'), e, t('filingFailedMessage'));
      setStep('idle');
    }
  }

  if (step === 'done') {
    return (
      <View style={s.successContainer}>
        <Text style={s.successIcon}>✓</Text>
        <Text style={s.successTitle}>{t('successTitle')}</Text>
        <Text style={s.successSub}>{t('successSubtitle')}</Text>
        <TouchableOpacity
          style={s.doneBtn}
          onPress={() => navigation.goBack()}
          accessibilityRole="button"
          accessibilityLabel={t('backToCourts')}
        >
          <Text style={s.doneBtnText}>{t('backToCourts')}</Text>
        </TouchableOpacity>
      </View>
    );
  }

  return (
    <ScrollView style={s.container} contentContainerStyle={s.content} keyboardShouldPersistTaps="handled">
      <Text style={s.title}>{t('title')}</Text>
      <Text style={s.subtitle}>{t('subtitle')}</Text>

      <Text style={s.label}>{t('caseTypeLabel')}</Text>
      <View style={s.kindGrid}>
        {KIND_ORDER.map((optKind) => {
          const label = t(KIND_LABEL_KEYS[optKind]);
          const requiresProof = PROOF_REQUIRED_KINDS.has(optKind);
          return (
            <TouchableOpacity
              key={optKind}
              style={[
                s.kindBtn,
                kind === optKind && s.kindBtnActive,
                requiresProof && s.kindBtnDisabled,
              ]}
              onPress={() => {
                if (!requiresProof) setKind(optKind);
              }}
              disabled={step !== 'idle' || requiresProof}
              accessibilityRole="button"
              accessibilityLabel={
                requiresProof
                  ? t('caseTypeAccessibilityLabelDisabled', { label })
                  : t('caseTypeAccessibilityLabel', { label })
              }
              accessibilityState={{ selected: kind === optKind, disabled: requiresProof }}
            >
              <Text
                style={[
                  s.kindBtnText,
                  kind === optKind && s.kindBtnTextActive,
                  requiresProof && s.kindBtnTextDisabled,
                ]}
              >
                {label}
              </Text>
            </TouchableOpacity>
          );
        })}
      </View>
      {KIND_ORDER.some((optKind) => PROOF_REQUIRED_KINDS.has(optKind)) && (
        <Text style={s.proofRequiredNote}>{t('proofRequiredNote')}</Text>
      )}

      {kind === 'LawChallenge' && (
        <>
          <Text style={s.label}>{t('lawIdLabel')}</Text>
          <TextInput
            style={s.input}
            value={lawId}
            onChangeText={setLawId}
            placeholder={t('lawIdPlaceholder')}
            placeholderTextColor={colors.textDim}
            keyboardType="number-pad"
            editable={step === 'idle'}
            accessibilityLabel={t('lawIdLabel')}
          />
        </>
      )}

      {kind === 'TreasuryDispute' && (
        <>
          <Text style={s.label}>{t('departmentIdLabel')}</Text>
          <TextInput
            style={s.input}
            value={departmentId}
            onChangeText={setDepartmentId}
            placeholder={t('departmentIdPlaceholder')}
            placeholderTextColor={colors.textDim}
            keyboardType="number-pad"
            editable={step === 'idle'}
            accessibilityLabel={t('departmentIdLabel')}
          />
        </>
      )}

      {kind === 'CitizenConduct' && (
        <>
          <Text style={s.label}>{t('nullifierLabel')}</Text>
          <TextInput
            style={s.input}
            value={nullifierHex}
            onChangeText={setNullifierHex}
            placeholder={t('nullifierPlaceholder')}
            placeholderTextColor={colors.textDim}
            autoCapitalize="none"
            autoCorrect={false}
            editable={step === 'idle'}
            accessibilityLabel={t('nullifierAccessibilityLabel')}
          />
          <Text style={s.hint}>{t('nullifierHint')}</Text>

          <Text style={s.label}>
            {t('suspensionLengthLabel')} <Text style={s.optional}>{t('optional')}</Text>
          </Text>
          <TextInput
            style={s.input}
            value={suspensionBlocks}
            onChangeText={setSuspensionBlocks}
            placeholder={t('suspensionPlaceholder')}
            placeholderTextColor={colors.textDim}
            keyboardType="number-pad"
            editable={step === 'idle'}
            accessibilityLabel={t('suspensionAccessibilityLabel')}
          />
        </>
      )}

      <TouchableOpacity
        style={[s.submitBtn, !canSubmit && s.submitBtnDisabled]}
        onPress={submit}
        disabled={!canSubmit}
        accessibilityRole="button"
        accessibilityLabel={t('submitAccessibilityLabel')}
        accessibilityState={{ disabled: !canSubmit }}
      >
        {step === 'submitting' ? (
          <ActivityIndicator color={colors.textPrimary} />
        ) : (
          <Text style={s.submitBtnText}>{t('submitButton')}</Text>
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
  kindBtnDisabled: { opacity: 0.4 },
  kindBtnText: { color: colors.textSecondary, fontWeight: '600', fontSize: 13 },
  kindBtnTextActive: { color: colors.textPrimary },
  kindBtnTextDisabled: { color: colors.textDim },
  proofRequiredNote: { fontSize: 11, color: colors.textDim, lineHeight: 16, marginTop: -10, marginBottom: 20 },
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
