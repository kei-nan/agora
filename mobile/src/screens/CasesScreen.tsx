/**
 * Courts tab — lists every on-chain case (`pallet-courts`), with per-case
 * appeal / jury-vote actions where the caller is eligible.
 *
 * Registered as a Tab.Screen (see App.tsx's MainTabs), so it reaches the
 * stack-only `FileCase` route the same way DelegateScreen.tsx does: a typed
 * `NativeStackNavigationProp<RootStackParamList>` via useNavigation<Nav>(),
 * not a screen prop.
 *
 * Eager-vs-lazy case-detail fetching: `fetchAllCases()` only returns
 * `CaseSummary` (no `juryPool`/`appealDeadlineBlock`/etc.), but this screen
 * needs `CaseDetail` for every case to decide which actions to show. Case
 * lists here are expected to stay small (real-world court dockets, not a
 * high-frequency feed), so this eagerly calls `fetchCaseDetail(caseId)` for
 * every case on every load rather than lazily fetching on expand/interact —
 * simpler code, and the extra chain reads are cheap at this scale. Revisit
 * if case lists grow large enough for this to matter.
 */
import React, { useCallback, useState } from 'react';
import {
  ActivityIndicator,
  FlatList,
  RefreshControl,
  StyleSheet,
  Text,
  TouchableOpacity,
  View,
} from 'react-native';
import { useFocusEffect, useNavigation } from '@react-navigation/native';
import { NativeStackNavigationProp } from '@react-navigation/native-stack';
import { RootStackParamList } from '../App';
import { getApi } from '../chain/api';
import { getSigningKeypair } from '../chain/identity';
import {
  CaseDetail,
  CaseStatus,
  CaseSubject,
  Verdict,
  appealRuling,
  castJuryVote,
  fetchAllCases,
  fetchCaseDetail,
  getOracleMembers,
  hasJurorVoted,
  isFilerOrOracle,
  isRuledAgainstParty,
} from '../chain/courts';
import { useAppModal } from '../components/AppModal';
import IpfsContentBox from '../components/IpfsContentBox';
import { colors } from '../theme';

type Nav = NativeStackNavigationProp<RootStackParamList>;

/**
 * Mirrors `pallet_courts::Config::OracleApprovalNumerator`/`OracleApprovalDenominator` as wired
 * in `runtime/src/configs/mod.rs` (`ConstU32<1>` / `ConstU32<2>` at time of writing — i.e. an
 * oracle action needs *strictly more than* half the council to approve). These are compile-time
 * `Get<u32>` runtime constants, not on-chain storage, so there is no chain read for them — same
 * hardcoded approach and same caveat as desktop's `fetch_oracle_council_info`
 * (`desktop/src-tauri/src/commands/chain.rs`): if the runtime config ever changes these values,
 * update the constants here (and there) to match.
 */
const ORACLE_APPROVAL_NUMERATOR = 1;
const ORACLE_APPROVAL_DENOMINATOR = 2;

/** Smallest approval count that satisfies the pallet's strict `approvals/council > numerator/denominator` check. */
function minOracleApprovalsNeeded(councilSize: number): number {
  return Math.floor((councilSize * ORACLE_APPROVAL_NUMERATOR) / ORACLE_APPROVAL_DENOMINATOR) + 1;
}

/**
 * Human-readable "time left" for the appeal window, mirroring the blocks-to-time
 * conversion `RegistrationStatusScreen.tsx`'s `estimateTimeRemaining` already uses
 * (`minimumPeriod * 2` for block time — same pattern desktop's `AuthPage.tsx` uses
 * for QR expiry, just block-denominated instead of a wall-clock timestamp).
 */
function formatBlocksRemaining(blocksRemaining: number, blockTimeMs: number): string {
  const msRemaining = blocksRemaining * blockTimeMs;
  const hoursRemaining = msRemaining / (1000 * 60 * 60);
  if (hoursRemaining < 1) {
    const minutes = Math.max(1, Math.round(msRemaining / (1000 * 60)));
    return `~${minutes} minute${minutes === 1 ? '' : 's'} left to appeal`;
  }
  if (hoursRemaining < 24) {
    const hours = Math.max(1, Math.round(hoursRemaining));
    return `~${hours} hour${hours === 1 ? '' : 's'} left to appeal`;
  }
  const days = Math.max(1, Math.round(hoursRemaining / 24));
  return `~${days} day${days === 1 ? '' : 's'} left to appeal`;
}

/**
 * `identity.ts` exposes `isCitizen(address)` (a boolean) but nothing that
 * hands back the raw `CitizenNullifier` bytes this screen needs for
 * `isRuledAgainstParty` — so, per the task's guidance, this queries the same
 * `identity.citizenNullifier` storage item directly rather than modifying
 * identity.ts for a single caller.
 */
async function fetchMyCitizenNullifier(address: string): Promise<Uint8Array | null> {
  const api = await getApi();
  const nullifier = await api.query.identity.citizenNullifier(address);
  return (nullifier as any).isSome ? (nullifier as any).unwrap().toU8a() : null;
}

function subjectDescription(subject: CaseSubject): string {
  if ('General' in subject) return 'General dispute';
  if ('LawChallenge' in subject) return `Law challenge (law #${subject.LawChallenge.law_id})`;
  if ('TreasuryDispute' in subject) {
    return `Treasury dispute (dept #${subject.TreasuryDispute.department_id})`;
  }
  // Distinct copy from "Law challenge" even though the on-chain remedy is identical
  // (T::LawEnforcer::invalidate_law) — this is a claim that the law was enacted at the
  // wrong constitutional tier, not a substantive challenge to its content.
  if ('TierConflict' in subject) return `Tier conflict (law #${subject.TierConflict.law_id})`;
  return 'Citizen conduct case';
}

const STATUS_LABEL: Record<CaseStatus, string> = {
  Filed: 'Filed',
  AIRulingIssued: 'AI ruling issued',
  InJuryAppeal: 'In jury appeal',
  JurySeated: 'Jury seated',
  FinalRuling: 'Final ruling',
  Enforced: 'Enforced',
};

function statusStyle(status: CaseStatus): { bg: any; text: any } {
  switch (status) {
    case 'InJuryAppeal':
    case 'JurySeated':
      return { bg: s.badgeWarningBg, text: s.badgeWarningText };
    case 'FinalRuling':
    case 'Enforced':
      return { bg: s.badgeSuccessBg, text: s.badgeSuccessText };
    case 'Filed':
    case 'AIRulingIssued':
    default:
      return { bg: s.badgeNeutralBg, text: s.badgeNeutralText };
  }
}

type PendingAction = 'appeal' | 'Upheld' | 'Overturned';

export default function CasesScreen() {
  const navigation = useNavigation<Nav>();
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [details, setDetails] = useState<Record<number, CaseDetail>>({});
  const [myAddress, setMyAddress] = useState<string | null>(null);
  const [oracleMembers, setOracleMembers] = useState<string[]>([]);
  const [myNullifier, setMyNullifier] = useState<Uint8Array | null>(null);
  const [currentBlock, setCurrentBlock] = useState<number | null>(null);
  const [blockTimeMs, setBlockTimeMs] = useState<number | null>(null);
  const [votedByMe, setVotedByMe] = useState<Record<number, boolean>>({});
  const [pending, setPending] = useState<Record<number, PendingAction>>({});
  const { showError } = useAppModal();

  const load = useCallback(
    async (opts: { manual: boolean } = { manual: false }) => {
      try {
        const api = await getApi();
        const [{ keypair }, summaries, oracle, header] = await Promise.all([
          getSigningKeypair(),
          fetchAllCases(),
          getOracleMembers(),
          api.rpc.chain.getHeader(),
        ]);
        const address = keypair.address;
        setMyAddress(address);
        setOracleMembers(oracle);
        setCurrentBlock(header.number.toNumber());
        setBlockTimeMs((api.consts.timestamp.minimumPeriod as any).toNumber() * 2);

        const [nullifier, detailList] = await Promise.all([
          fetchMyCitizenNullifier(address),
          Promise.all(summaries.map((c) => fetchCaseDetail(c.caseId))),
        ]);
        setMyNullifier(nullifier);

        const detailMap: Record<number, CaseDetail> = {};
        detailList.forEach((d) => {
          if (d) detailMap[d.caseId] = d;
        });
        setDetails(detailMap);

        // For any JurySeated case this caller is in the jury pool for, check
        // whether they've already voted (gates showing the vote buttons).
        const jurySeatedMine = Object.values(detailMap).filter(
          (d) => d.status === 'JurySeated' && d.juryPool.includes(address),
        );
        const votedEntries = await Promise.all(
          jurySeatedMine.map(async (d) => [d.caseId, await hasJurorVoted(d.caseId, address)] as const),
        );
        setVotedByMe(Object.fromEntries(votedEntries));
      } catch (e: any) {
        showError("Couldn't load cases", e);
      } finally {
        setLoading(false);
        setRefreshing(false);
      }
    },
    [showError],
  );

  useFocusEffect(
    useCallback(() => {
      load();
    }, [load]),
  );

  async function handleAppeal(caseId: number) {
    setPending((p) => ({ ...p, [caseId]: 'appeal' }));
    try {
      const { keypair } = await getSigningKeypair();
      await appealRuling(keypair, caseId);
      await load();
    } catch (e: any) {
      showError('Appeal failed', e, 'Your appeal could not be submitted. Please try again.');
    } finally {
      setPending((p) => {
        const next = { ...p };
        delete next[caseId];
        return next;
      });
    }
  }

  async function handleJuryVote(caseId: number, verdict: Verdict) {
    setPending((p) => ({ ...p, [caseId]: verdict }));
    try {
      const { keypair } = await getSigningKeypair();
      await castJuryVote(keypair, caseId, verdict);
      await load();
    } catch (e: any) {
      showError('Vote failed', e, 'Your jury vote could not be submitted. Please try again.');
    } finally {
      setPending((p) => {
        const next = { ...p };
        delete next[caseId];
        return next;
      });
    }
  }

  if (loading) {
    return (
      <View style={s.center}>
        <ActivityIndicator color={colors.accent} />
      </View>
    );
  }

  const cases = Object.values(details).sort((a, b) => a.caseId - b.caseId);

  return (
    <FlatList
      style={s.list}
      data={cases}
      keyExtractor={(c) => String(c.caseId)}
      refreshControl={
        <RefreshControl
          refreshing={refreshing}
          onRefresh={() => {
            setRefreshing(true);
            load({ manual: true });
          }}
          tintColor={colors.accent}
        />
      }
      ListHeaderComponent={
        <View style={s.header}>
          <View>
            <Text style={s.title}>Courts</Text>
            {oracleMembers.length > 0 && (
              <Text style={s.oracleInfo}>
                Oracle Council: {oracleMembers.length} member{oracleMembers.length === 1 ? '' : 's'}, needs{' '}
                {minOracleApprovalsNeeded(oracleMembers.length)} to approve a ruling
              </Text>
            )}
          </View>
          <TouchableOpacity
            style={s.fileBtn}
            onPress={() => navigation.navigate('FileCase')}
            accessibilityRole="button"
            accessibilityLabel="File a case"
          >
            <Text style={s.fileBtnText}>File a case</Text>
          </TouchableOpacity>
        </View>
      }
      ListEmptyComponent={<Text style={s.empty}>No cases yet.</Text>}
      renderItem={({ item }) => {
        const badge = statusStyle(item.status);
        const isPending = pending[item.caseId];

        const eligibleParty =
          isFilerOrOracle(item, myAddress ?? '', myNullifier, oracleMembers) ||
          isRuledAgainstParty(item, myNullifier);
        const appealWindowOpen =
          item.appealDeadlineBlock !== null && currentBlock !== null && currentBlock <= item.appealDeadlineBlock;
        const canAppeal = item.status === 'AIRulingIssued' && eligibleParty && appealWindowOpen;

        const inJuryPool = !!myAddress && item.juryPool.includes(myAddress);
        const alreadyVoted = !!votedByMe[item.caseId];
        const canVote = item.status === 'JurySeated' && inJuryPool && !alreadyVoted;

        return (
          <View style={s.card}>
            <View style={s.cardHeader}>
              <Text style={s.id}>Case #{item.caseId}</Text>
              <View style={[s.badge, badge.bg]}>
                <Text style={[s.badgeText, badge.text]}>{STATUS_LABEL[item.status]}</Text>
              </View>
            </View>

            <Text style={s.subject}>{subjectDescription(item.subject)}</Text>

            {item.rulingIpfsHash && (
              <View style={s.reasoningBox}>
                <Text style={s.reasoningLabel}>AI judge's reasoning</Text>
                <IpfsContentBox hashHex={item.rulingIpfsHash} />
              </View>
            )}

            {item.ruling && (
              <Text style={[s.ruling, item.ruling === 'Upheld' ? s.rulingUpheld : s.rulingOverturned]}>
                Ruling: {item.ruling}
              </Text>
            )}

            {item.status === 'AIRulingIssued' && item.appealDeadlineBlock !== null && currentBlock !== null && (
              <Text style={s.deadline}>
                {currentBlock <= item.appealDeadlineBlock
                  ? blockTimeMs !== null
                    ? formatBlocksRemaining(item.appealDeadlineBlock - currentBlock, blockTimeMs)
                    : 'Appeal window still open'
                  : 'Appeal window has closed'}
              </Text>
            )}

            {canAppeal && (
              <TouchableOpacity
                style={s.actionBtn}
                onPress={() => handleAppeal(item.caseId)}
                disabled={!!isPending}
                accessibilityRole="button"
                accessibilityLabel={`Appeal case ${item.caseId}`}
              >
                {isPending === 'appeal' ? (
                  <ActivityIndicator color={colors.textPrimary} size="small" />
                ) : (
                  <Text style={s.actionBtnText}>Appeal ruling</Text>
                )}
              </TouchableOpacity>
            )}

            {canVote && (
              <View style={s.voteRow}>
                <TouchableOpacity
                  style={[s.voteBtn, s.voteBtnUphold]}
                  onPress={() => handleJuryVote(item.caseId, 'Upheld')}
                  disabled={!!isPending}
                  accessibilityRole="button"
                  accessibilityLabel={`Vote to uphold case ${item.caseId}`}
                >
                  {isPending === 'Upheld' ? (
                    <ActivityIndicator color={colors.textPrimary} size="small" />
                  ) : (
                    <Text style={s.voteBtnText}>Uphold</Text>
                  )}
                </TouchableOpacity>
                <TouchableOpacity
                  style={[s.voteBtn, s.voteBtnOverturn]}
                  onPress={() => handleJuryVote(item.caseId, 'Overturned')}
                  disabled={!!isPending}
                  accessibilityRole="button"
                  accessibilityLabel={`Vote to overturn case ${item.caseId}`}
                >
                  {isPending === 'Overturned' ? (
                    <ActivityIndicator color={colors.textPrimary} size="small" />
                  ) : (
                    <Text style={s.voteBtnText}>Overturn</Text>
                  )}
                </TouchableOpacity>
              </View>
            )}
          </View>
        );
      }}
    />
  );
}

const s = StyleSheet.create({
  list: { flex: 1, backgroundColor: colors.bg, padding: 16 },
  center: { flex: 1, backgroundColor: colors.bg, alignItems: 'center', justifyContent: 'center' },
  header: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 },
  title: { fontSize: 24, fontWeight: '700', color: colors.textPrimary },
  oracleInfo: { fontSize: 12, color: colors.textMuted, marginTop: 2 },
  fileBtn: { backgroundColor: colors.accent, paddingVertical: 10, paddingHorizontal: 16, borderRadius: 10 },
  fileBtnText: { color: colors.textPrimary, fontWeight: '600', fontSize: 13 },
  empty: { color: colors.textMuted, textAlign: 'center', marginTop: 40 },
  card: {
    backgroundColor: colors.card,
    borderRadius: 14,
    padding: 16,
    marginBottom: 12,
    borderWidth: 1,
    borderColor: colors.border,
  },
  cardHeader: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', marginBottom: 8 },
  id: { fontSize: 12, color: colors.textDim, fontWeight: '600' },
  badge: { paddingHorizontal: 8, paddingVertical: 3, borderRadius: 6 },
  badgeText: { fontSize: 11, fontWeight: '600' },
  badgeNeutralBg: { backgroundColor: colors.border },
  badgeNeutralText: { color: colors.textSecondary },
  badgeWarningBg: { backgroundColor: colors.warningBg },
  badgeWarningText: { color: colors.warningTextStrong },
  badgeSuccessBg: { backgroundColor: colors.successBg },
  badgeSuccessText: { color: colors.success },
  subject: { fontSize: 15, fontWeight: '600', color: colors.textPrimary, marginBottom: 8 },
  reasoningBox: { marginBottom: 8 },
  reasoningLabel: { fontSize: 11, fontWeight: '600', color: colors.textDim, marginBottom: 4 },
  ruling: { fontSize: 13, fontWeight: '600', marginBottom: 8 },
  rulingUpheld: { color: colors.success },
  rulingOverturned: { color: colors.danger },
  deadline: { fontSize: 12, color: colors.textMuted, marginBottom: 8 },
  actionBtn: {
    backgroundColor: colors.accent,
    paddingVertical: 11,
    borderRadius: 10,
    alignItems: 'center',
    marginTop: 4,
  },
  actionBtnText: { color: colors.textPrimary, fontWeight: '600', fontSize: 13 },
  voteRow: { flexDirection: 'row', gap: 10, marginTop: 4 },
  voteBtn: { flex: 1, paddingVertical: 11, borderRadius: 10, alignItems: 'center' },
  voteBtnUphold: { backgroundColor: colors.successSolid },
  voteBtnOverturn: { backgroundColor: colors.dangerSolid },
  voteBtnText: { color: colors.textPrimary, fontWeight: '600', fontSize: 13 },
});
