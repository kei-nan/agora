import React, { useCallback, useState } from 'react';
import { ActivityIndicator, ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { useFocusEffect } from '@react-navigation/native';
import { RootStackParamList } from '../App';
import { getApi } from '../chain/api';
import { getSigningKeypair } from '../chain/identity';
import { RegistrationStatus, clearRegistrationStatus } from '../chain/registrationState';
import { ReconciliationResult, reconcileRegistrationStatus } from '../chain/registrationReconciler';
import { useAppModal } from '../components/AppModal';
import { colors } from '../theme';

type Props = NativeStackScreenProps<RootStackParamList, 'RegistrationStatus'>;

type OprfProgress = ReconciliationResult['oprfProgress'];

/** The three pipeline stages that carry `slaExpiresAtBlock` — the only ones a time-remaining estimate applies to. */
type SlaStage = Extract<
  RegistrationStatus,
  { stage: 'OprfQuerySubmitted' | 'AwaitingCommitteeRound1' | 'AwaitingCommitteeRound2' }
>;

function isSlaStage(status: RegistrationStatus): status is SlaStage {
  return (
    status.stage === 'OprfQuerySubmitted' ||
    status.stage === 'AwaitingCommitteeRound1' ||
    status.stage === 'AwaitingCommitteeRound2'
  );
}

/** Stages a terminal chain-confirmed citizen status card covers — these are never persisted locally (see registrationState.ts). */
const CITIZEN_STAGES: RegistrationStatus['stage'][] = ['Active', 'ReverificationDue', 'Suspended'];

interface Cluster {
  label: string;
  detail: string;
  stages: RegistrationStatus['stage'][];
}

const CLUSTERS: Cluster[] = [
  {
    label: 'Passport & proof assembly',
    detail: 'Scan your passport and assemble the ZK proof inputs on-device',
    stages: ['NotStarted', 'PassportScanned', 'ProofMaterialAssembled'],
  },
  {
    label: 'OPRF committee query',
    detail: 'Committee members jointly derive your identity anchor across two rounds',
    stages: ['OprfQuerySubmitted', 'AwaitingCommitteeRound1', 'AwaitingCommitteeRound2', 'ProofCombining'],
  },
  {
    label: 'Proof & submission',
    detail: 'Finalize your proof and submit it to the chain',
    stages: ['ProofReady', 'ChainSubmissionPending'],
  },
];

/** Index of the cluster `stage` belongs to, or -1 if it's not a pipeline stage (Failed, or a terminal citizen stage). */
function clusterIndexOf(stage: RegistrationStatus['stage']): number {
  return CLUSTERS.findIndex((c) => c.stages.includes(stage));
}

/** Live OPRF round-response lines for the committee-query cluster, once `oprfProgress` is available. */
function oprfProgressLines(status: RegistrationStatus, progress: OprfProgress): string[] {
  if (!progress) return [];
  const totalCommittees = progress.slots.length;
  const round1Committees = progress.slots.filter((s) => s.round1Count >= progress.threshold).length;

  if (status.stage === 'OprfQuerySubmitted' || status.stage === 'AwaitingCommitteeRound1') {
    return [
      `Round 1: ${round1Committees} of ${totalCommittees} committees responded (each needs ${progress.threshold} of ${progress.threshold} members)`,
    ];
  }
  if (status.stage === 'AwaitingCommitteeRound2' || status.stage === 'ProofCombining') {
    const round2Committees = progress.slots.filter((s) => s.round2Count >= progress.threshold).length;
    return [
      `Round 1: ${round1Committees} of ${totalCommittees} committees responded (each needs ${progress.threshold} of ${progress.threshold} members)`,
      `Round 2: ${round2Committees} of ${totalCommittees} committees responded (each needs ${progress.threshold} of ${progress.threshold} members)`,
    ];
  }
  return [];
}

/**
 * Approximate wall-clock time left before `status.slaExpiresAtBlock`, derived
 * from the real `SlotDuration` the way `runtime/src/configs/mod.rs` computes
 * it (`minimumPeriod * 2`). Returns `null` (never throws) if `status` doesn't
 * carry an SLA deadline or the chain read fails for any reason — omitting the
 * estimate is preferable to crashing this screen over it.
 */
async function estimateTimeRemaining(status: RegistrationStatus): Promise<string | null> {
  if (!isSlaStage(status)) return null;
  try {
    const api = await getApi();
    const header = await api.rpc.chain.getHeader();
    const currentBlock = header.number.toNumber();
    const blocksRemaining = status.slaExpiresAtBlock - currentBlock;
    if (blocksRemaining <= 0) return null; // Past due — the next reconcile will flip this to Failed.

    const blockTimeMs = (api.consts.timestamp.minimumPeriod as any).toNumber() * 2;
    const msRemaining = blocksRemaining * blockTimeMs;
    const hoursRemaining = msRemaining / (1000 * 60 * 60);

    if (hoursRemaining < 24) {
      const hours = Math.max(1, Math.round(hoursRemaining));
      return `~${hours} hour${hours === 1 ? '' : 's'} remaining`;
    }
    const days = Math.max(1, Math.round(hoursRemaining / 24));
    return `~${days} day${days === 1 ? '' : 's'} remaining`;
  } catch {
    return null;
  }
}

/**
 * Best-effort estimated wall-clock date for a target block number, using the
 * same `minimumPeriod * 2` block-time approximation `estimateTimeRemaining`
 * above uses. Returns `null` (never throws) on any chain-read failure, or if
 * the target block is already in the past — an estimate is only meaningful
 * for a still-future block, and a stale/past estimate would be misleading.
 */
async function estimateBlockDate(targetBlock: number): Promise<string | null> {
  try {
    const api = await getApi();
    const header = await api.rpc.chain.getHeader();
    const currentBlock = header.number.toNumber();
    const blocksRemaining = targetBlock - currentBlock;
    if (blocksRemaining <= 0) return null;

    const blockTimeMs = (api.consts.timestamp.minimumPeriod as any).toNumber() * 2;
    const estimatedDate = new Date(Date.now() + blocksRemaining * blockTimeMs);
    return estimatedDate.toLocaleDateString(undefined, { day: '2-digit', month: 'short', year: 'numeric' });
  } catch {
    return null;
  }
}

/** Short, human-readable label per stage for the Suspended/ReverificationDue-adjacent card headers. */
function citizenCardTitle(stage: 'Active' | 'ReverificationDue' | 'Suspended'): string {
  switch (stage) {
    case 'Active':
      return '✓ Active citizen';
    case 'ReverificationDue':
      return 'Reverification needed';
    case 'Suspended':
      return 'Account suspended';
  }
}

export default function RegistrationStatusScreen({ navigation }: Props) {
  const [address, setAddress] = useState<string | null>(null);
  const [status, setStatus] = useState<RegistrationStatus | null>(null);
  const [oprfProgress, setOprfProgress] = useState<OprfProgress>(undefined);
  const [timeRemaining, setTimeRemaining] = useState<string | null>(null);
  const [suspendedUntilDate, setSuspendedUntilDate] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const { showError, showConfirm } = useAppModal();

  const refresh = useCallback(
    async (addr: string, opts: { manual: boolean; isCancelled?: () => boolean }) => {
      const { manual, isCancelled = () => false } = opts;
      if (manual) setRefreshing(true);
      try {
        const result = await reconcileRegistrationStatus(addr);
        if (isCancelled()) return;
        setStatus(result.status);
        setOprfProgress(result.oprfProgress);
        const timeText = await estimateTimeRemaining(result.status);
        if (isCancelled()) return;
        setTimeRemaining(timeText);
        const suspendedDateText =
          result.status.stage === 'Suspended' && result.status.until !== null
            ? await estimateBlockDate(result.status.until)
            : null;
        if (isCancelled()) return;
        setSuspendedUntilDate(suspendedDateText);
      } catch (e) {
        if (isCancelled()) return;
        // Initial focus-driven reconciles degrade gracefully offline, same
        // convention as HomeScreen.tsx — silently keep whatever was already
        // shown. A manual refresh is a deliberate user action, so it gets an
        // explicit error instead of failing silently.
        if (manual) showError("Couldn't refresh status", e);
      } finally {
        if (!isCancelled()) {
          if (manual) setRefreshing(false);
          setLoading(false);
        }
      }
    },
    [showError],
  );

  useFocusEffect(
    useCallback(() => {
      let cancelled = false;
      setLoading(true);
      (async () => {
        try {
          const { keypair } = await getSigningKeypair();
          if (cancelled) return;
          setAddress(keypair.address);
          await refresh(keypair.address, { manual: false, isCancelled: () => cancelled });
        } catch {
          if (!cancelled) setLoading(false);
        }
      })();
      return () => {
        cancelled = true;
      };
    }, [refresh]),
  );

  async function handleRetry() {
    if (!address) return;
    await clearRegistrationStatus(address);
    // .replace, not .navigate — so a Failed -> Register -> Failed cycle
    // doesn't grow the back stack indefinitely.
    navigation.replace('Register');
  }

  async function handleManualRefresh() {
    if (!address) return;
    await refresh(address, { manual: true });
  }

  /**
   * User-triggered privacy control: deletes this device's locally-saved
   * registration progress (which stage was reached, whether face match
   * passed or failed) right now, rather than relying on the OS to evict it
   * from cache storage eventually (see registrationState.ts's doc comment).
   * Purely local — never touches chain state, so a genuinely registered
   * citizen just sees their status re-derive from the chain on next check.
   */
  function handleClearStatus() {
    if (!address) return;
    showConfirm({
      title: 'Clear saved status?',
      message:
        "This deletes the registration progress saved on this device — including which step " +
        "you'd reached and whether your face match passed or failed — right away, instead of " +
        "waiting for it to eventually age out on its own. It doesn't touch anything on the " +
        "chain: if you're actually a registered citizen, checking again will still show that.",
      confirmLabel: 'Clear',
      destructive: true,
      onConfirm: () => {
        void (async () => {
          try {
            await clearRegistrationStatus(address);
            await refresh(address, { manual: false });
          } catch (e: any) {
            showError('Could not clear status', e, 'Your saved registration status could not be cleared. Please try again.');
          }
        })();
      },
    });
  }

  if (loading && !status) {
    return (
      <View style={s.loadingContainer}>
        <ActivityIndicator size="large" color={colors.accent} />
      </View>
    );
  }

  if (!status) {
    return (
      <ScrollView style={s.container} contentContainerStyle={s.scrollContent}>
        <Text style={s.title}>Registration Status</Text>
        <Text style={s.subtitle}>Couldn't load your registration status.</Text>
        <TouchableOpacity
          style={s.refreshBtn}
          onPress={handleManualRefresh}
          accessibilityRole="button"
          accessibilityLabel="Refresh status"
        >
          {refreshing ? (
            <ActivityIndicator size="small" color={colors.textPrimary} />
          ) : (
            <Text style={s.refreshBtnText}>Refresh status</Text>
          )}
        </TouchableOpacity>
      </ScrollView>
    );
  }

  const isFailed = status.stage === 'Failed';
  const isCitizenStage = CITIZEN_STAGES.includes(status.stage);
  // Once the chain confirms Active/ReverificationDue/Suspended, the whole
  // local pipeline is behind us — every cluster reads as done.
  const currentClusterIdx = isFailed ? -1 : isCitizenStage ? CLUSTERS.length : clusterIndexOf(status.stage);

  return (
    <ScrollView style={s.container} contentContainerStyle={s.scrollContent}>
      <Text style={s.title}>Registration Status</Text>
      <Text style={s.subtitle}>
        Your citizen registration moves through passport scanning, an OPRF committee round-trip, and
        final chain submission.
      </Text>

      {isFailed && status.stage === 'Failed' ? (
        <View style={s.failedCard}>
          <Text style={s.failedTitle}>Registration failed</Text>
          <Text style={s.failedReason}>{status.reason}</Text>
          {status.retryable && (
            <TouchableOpacity
              style={s.retryBtn}
              onPress={handleRetry}
              accessibilityRole="button"
              accessibilityLabel="Retry"
            >
              <Text style={s.retryBtnText}>Retry</Text>
            </TouchableOpacity>
          )}
        </View>
      ) : (
        <>
          <View style={s.stepList}>
            {CLUSTERS.map((cluster, i) => {
              const isDone = currentClusterIdx > i;
              const isActive = currentClusterIdx === i;
              const progressLines = isActive ? oprfProgressLines(status, oprfProgress) : [];
              const showTimeRemaining = isActive && isSlaStage(status) && timeRemaining;
              return (
                <View key={cluster.label} style={s.stepRow}>
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
                    <Text style={[s.stepLabel, (isActive || isDone) ? s.stepLabelActive : {}]}>
                      {cluster.label}
                    </Text>
                    <Text style={s.stepDetail}>{cluster.detail}</Text>
                    {progressLines.map((line) => (
                      <Text key={line} style={s.stepProgress}>
                        {line}
                      </Text>
                    ))}
                    {showTimeRemaining && <Text style={s.stepProgress}>{timeRemaining}</Text>}
                  </View>
                </View>
              );
            })}
          </View>

          {isCitizenStage && (status.stage === 'Active' || status.stage === 'ReverificationDue' || status.stage === 'Suspended') && (
            <View
              style={[
                s.citizenCard,
                status.stage === 'Active' && s.citizenCardSuccess,
                status.stage === 'ReverificationDue' && s.citizenCardWarning,
                status.stage === 'Suspended' && s.citizenCardDanger,
              ]}
            >
              <Text
                style={[
                  s.citizenTitle,
                  status.stage === 'Active' && s.citizenTitleSuccess,
                  status.stage === 'ReverificationDue' && s.citizenTitleWarning,
                  status.stage === 'Suspended' && s.citizenTitleDanger,
                ]}
              >
                {citizenCardTitle(status.stage)}
              </Text>
              {status.stage === 'Active' && (
                <Text style={s.citizenSub}>Your registration is confirmed on-chain.</Text>
              )}
              {status.stage === 'ReverificationDue' && (
                <Text style={s.citizenSub}>
                  Your reverification window has passed. Reverify your passport to keep your citizen
                  status active.
                </Text>
              )}
              {status.stage === 'Suspended' && (
                <>
                  <Text style={s.citizenSub}>
                    {status.until === null
                      ? 'Suspended indefinitely — check back later.'
                      : suspendedUntilDate
                        ? `Suspended — expected to lift around ${suspendedUntilDate}. Check back then.`
                        : 'Suspended — check back later.'}
                  </Text>
                  {status.until !== null && (
                    <Text style={s.citizenDebug}>
                      Technical reference: block #{status.until.toLocaleString()}
                    </Text>
                  )}
                </>
              )}
            </View>
          )}
        </>
      )}

      <TouchableOpacity
        style={s.refreshBtn}
        onPress={handleManualRefresh}
        accessibilityRole="button"
        accessibilityLabel="Refresh status"
      >
        {refreshing ? (
          <ActivityIndicator size="small" color={colors.textPrimary} />
        ) : (
          <Text style={s.refreshBtnText}>Refresh status</Text>
        )}
      </TouchableOpacity>

      {!isCitizenStage && (
        <TouchableOpacity
          style={s.clearBtn}
          onPress={handleClearStatus}
          accessibilityRole="button"
          accessibilityLabel="Clear saved status"
        >
          <Text style={s.clearBtnText}>Clear saved status</Text>
        </TouchableOpacity>
      )}
    </ScrollView>
  );
}

const s = StyleSheet.create({
  container: { flex: 1, backgroundColor: colors.bg },
  loadingContainer: { flex: 1, backgroundColor: colors.bg, alignItems: 'center', justifyContent: 'center' },
  scrollContent: { padding: 24, paddingBottom: 48 },
  title: { fontSize: 24, fontWeight: '700', color: colors.textPrimary, marginBottom: 8 },
  subtitle: { fontSize: 14, color: colors.textMuted, lineHeight: 20, marginBottom: 32 },
  stepList: { gap: 20, marginBottom: 32 },
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
  stepProgress: { fontSize: 12, color: colors.accent, fontWeight: '600', marginTop: 4 },
  citizenCard: {
    borderRadius: 16,
    padding: 20,
    borderWidth: 1,
    marginBottom: 24,
    backgroundColor: colors.card,
    borderColor: colors.border,
  },
  citizenCardSuccess: { backgroundColor: colors.successBg, borderColor: colors.successSolid },
  citizenCardWarning: { backgroundColor: colors.warningBg, borderColor: colors.warningBorder },
  citizenCardDanger: { backgroundColor: colors.card, borderColor: colors.danger },
  citizenTitle: { fontSize: 16, fontWeight: '700', marginBottom: 6, color: colors.textPrimary },
  citizenTitleSuccess: { color: colors.success },
  citizenTitleWarning: { color: colors.warningTextStrong },
  citizenTitleDanger: { color: colors.danger },
  citizenSub: { fontSize: 13, color: colors.textBody, lineHeight: 18 },
  citizenDebug: { fontSize: 11, color: colors.textFaint, marginTop: 6 },
  failedCard: {
    backgroundColor: colors.card,
    borderRadius: 16,
    padding: 20,
    borderWidth: 1,
    borderColor: colors.danger,
    marginBottom: 24,
  },
  failedTitle: { fontSize: 16, fontWeight: '700', color: colors.danger, marginBottom: 8 },
  failedReason: { fontSize: 13, color: colors.textBody, lineHeight: 18, marginBottom: 16 },
  retryBtn: {
    backgroundColor: colors.dangerSolid,
    paddingVertical: 14,
    borderRadius: 12,
    alignItems: 'center',
  },
  retryBtnText: { color: colors.textPrimary, fontWeight: '700', fontSize: 15 },
  refreshBtn: {
    borderWidth: 1,
    borderColor: colors.border,
    paddingVertical: 14,
    borderRadius: 12,
    alignItems: 'center',
  },
  refreshBtnText: { color: colors.textSecondary, fontWeight: '600', fontSize: 14 },
  clearBtn: {
    marginTop: 12,
    paddingVertical: 10,
    alignItems: 'center',
  },
  clearBtnText: { color: colors.textMuted, fontWeight: '600', fontSize: 13, textDecorationLine: 'underline' },
});
