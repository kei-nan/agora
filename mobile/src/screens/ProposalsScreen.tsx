import React, { useCallback, useEffect, useState } from 'react';
import {
  ActivityIndicator,
  FlatList,
  RefreshControl,
  StyleSheet,
  Text,
  TouchableOpacity,
  View,
} from 'react-native';
import { Proposal, fetchProposals, hasVotedOnReferendum, voteOnReferendum } from '../chain/governance';
import { getSigningKeypair } from '../chain/identity';
import { useAppModal } from '../components/AppModal';
import IpfsContentBox from '../components/IpfsContentBox';
import { colors } from '../theme';

/**
 * pallet-voting's `dispatchError.toString()` surfaces as raw module-error
 * text (module name + Rust error variant, e.g. `voting.AlreadyVotedInReferendum:
 * ...`, per submitExtrinsic.ts) — not something a citizen should have to
 * parse to understand why their vote didn't go through. Maps the errors
 * `vote_referendum` can actually return (pallets/pallet-voting/src/lib.rs's
 * `Error<T>` enum) to plain language; anything unrecognized falls back to a
 * generic message. The real error is never discarded — callers still show it
 * via a "Details" button.
 */
function translateVoteError(raw: string): string {
  if (raw.includes('AlreadyVotedInReferendum')) {
    return 'You have already voted on this proposal.';
  }
  if (raw.includes('VotingEpochNotActive')) {
    return 'Voting is not open right now — this proposal is outside the current voting epoch.';
  }
  if (raw.includes('ReferendumNotActive')) {
    return 'Voting has closed on this proposal.';
  }
  if (raw.includes('ReferendumNotFound')) {
    return 'This proposal could not be found on-chain.';
  }
  if (raw.includes('CitizenNotActive')) {
    return 'Your citizen registration is not active — re-verify your passport to vote.';
  }
  return 'The chain rejected this vote. Please try again.';
}

export default function ProposalsScreen() {
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [voting, setVoting] = useState<number | null>(null);
  // Mirrors PetitionScreen.tsx's `signed` Set — local, per-item "already
  // acted" tracking so the buttons can't be re-tapped once a vote lands.
  // Seeded from chain state in load() (see hasVotedOnReferendum) so it
  // reflects votes cast in earlier sessions too, not just this one.
  const [voted, setVoted] = useState<Set<number>>(new Set());
  const { showInfo, showError, showConfirm } = useAppModal();

  const load = useCallback(async () => {
    let data: Proposal[];
    try {
      data = await fetchProposals();
      setProposals(data);
    } catch (e: any) {
      showError("Couldn't load proposals", e);
      setLoading(false);
      setRefreshing(false);
      return;
    }

    // Seeding the already-voted set is a best-effort UX nicety, not the
    // primary load — if it fails (e.g. no signing keypair yet), the
    // proposals list should still render rather than showing an error for a
    // problem unrelated to fetching proposals. Worst case, `voted` just
    // stays at its previous value and a repeat vote is caught by the chain
    // (and translateVoteError) instead of being pre-empted in the UI.
    try {
      const { keypair } = await getSigningKeypair();
      const votingIds = data.filter((p) => p.state === 'Voting').map((p) => p.id);
      const results = await Promise.all(
        votingIds.map((id) => hasVotedOnReferendum(keypair.address, id)),
      );
      setVoted(new Set(votingIds.filter((_, i) => results[i])));
    } catch {
      // ignore — see comment above
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, [showError]);

  useEffect(() => { load(); }, [load]);

  function vote(id: number, inFavor: boolean) {
    showConfirm({
      title: inFavor ? 'Vote for this proposal?' : 'Vote against this proposal?',
      message: `You're about to cast an on-chain vote ${inFavor ? 'for' : 'against'} proposal #${id}. Votes cannot be changed once submitted.`,
      confirmLabel: inFavor ? 'Vote For' : 'Vote Against',
      destructive: !inFavor,
      onConfirm: () => castVote(id, inFavor),
    });
  }

  async function castVote(id: number, inFavor: boolean) {
    setVoting(id);
    try {
      const { keypair } = await getSigningKeypair();
      await voteOnReferendum(keypair, id, inFavor);
      setVoted((prev) => new Set(prev).add(id));
      showInfo('Vote cast', `You voted ${inFavor ? 'for' : 'against'} proposal #${id}.`);
      load();
    } catch (e: any) {
      const rawMessage = e.message ?? String(e);
      // translateVoteError still supplies the specific, per-error-code friendly message;
      // showError's own devDetails mechanism now covers what the old "Details" button did,
      // gated to __DEV__ instead of always offered.
      showError('Vote failed', e, translateVoteError(rawMessage));
    } finally {
      setVoting(null);
    }
  }

  if (loading) {
    return <View style={s.center}><ActivityIndicator color={colors.accent} /></View>;
  }

  return (
    <FlatList
      style={s.list}
      data={proposals}
      keyExtractor={(p) => String(p.id)}
      refreshControl={<RefreshControl refreshing={refreshing} onRefresh={() => { setRefreshing(true); load(); }} tintColor={colors.accent} />}
      ListEmptyComponent={<Text style={s.empty}>No proposals on-chain yet.</Text>}
      renderItem={({ item }) => {
        const hasVoted = voted.has(item.id);
        return (
          <View style={s.card}>
            <View
              style={s.cardHeader}
              accessible
              accessibilityLabel={`Proposal ${item.id}, ${item.tier}, ${item.state}`}
            >
              <View style={s.chips}>
                <Text style={[s.chip, item.state === 'Voting' ? s.chipActive : s.chipDone]}>
                  {item.state}
                </Text>
                {item.tier === 'Constitutional' && (
                  <Text style={[s.chip, s.chipConst]}>constitutional</Text>
                )}
              </View>
              <Text style={s.id}>#{item.id}</Text>
            </View>

            <View style={s.contentBox}>
              <IpfsContentBox hashHex={item.topicHash} />
            </View>

            <View
              style={s.tally}
              accessible
              accessibilityLabel={`${item.votesFor} votes for, ${item.votesAgainst} votes against${hasVoted ? ', you already voted on this proposal' : ''}`}
            >
              <Text style={s.forVotes}>▲ {item.votesFor} for</Text>
              <Text style={s.againstVotes}>▼ {item.votesAgainst} against</Text>
            </View>

            {item.state === 'Voting' && (
              hasVoted ? (
                <View style={s.votedRow}>
                  <Text style={s.votedText}>✓ You voted on this proposal</Text>
                </View>
              ) : (
                <View style={s.voteRow}>
                  <TouchableOpacity
                    style={[s.voteBtn, s.voteBtnFor]}
                    onPress={() => vote(item.id, true)}
                    disabled={voting === item.id}
                    accessibilityRole="button"
                    accessibilityLabel={`Vote for proposal ${item.id}`}
                    accessibilityState={{ disabled: voting === item.id }}
                  >
                    {voting === item.id
                      ? <ActivityIndicator color={colors.textPrimary} size="small" />
                      : <Text style={s.voteBtnText}>Vote For</Text>}
                  </TouchableOpacity>
                  <TouchableOpacity
                    style={[s.voteBtn, s.voteBtnAgainst]}
                    onPress={() => vote(item.id, false)}
                    disabled={voting === item.id}
                    accessibilityRole="button"
                    accessibilityLabel={`Vote against proposal ${item.id}`}
                    accessibilityState={{ disabled: voting === item.id }}
                  >
                    <Text style={s.voteBtnText}>Vote Against</Text>
                  </TouchableOpacity>
                </View>
              )
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
  chips: { flexDirection: 'row', gap: 6 },
  chip: { fontSize: 11, fontWeight: '600', paddingHorizontal: 8, paddingVertical: 3, borderRadius: 6 },
  chipActive: { backgroundColor: colors.successBg, color: colors.success },
  chipDone: { backgroundColor: colors.border, color: colors.textSecondary },
  chipConst: { backgroundColor: '#1e1040', color: '#a78bfa' },
  id: { fontSize: 12, color: colors.textMuted },
  contentBox: { marginBottom: 10 },
  tally: { flexDirection: 'row', gap: 16, marginBottom: 14 },
  forVotes: { color: colors.success, fontSize: 14, fontWeight: '600' },
  againstVotes: { color: colors.danger, fontSize: 14, fontWeight: '600' },
  voteRow: { flexDirection: 'row', gap: 10 },
  voteBtn: { flex: 1, paddingVertical: 10, borderRadius: 10, alignItems: 'center' },
  voteBtnFor: { backgroundColor: colors.successSolid },
  voteBtnAgainst: { backgroundColor: colors.dangerSolid },
  voteBtnText: { color: colors.textPrimary, fontWeight: '600', fontSize: 14 },
  votedRow: { paddingVertical: 10, alignItems: 'center', backgroundColor: colors.border, borderRadius: 10 },
  votedText: { color: colors.success, fontWeight: '600', fontSize: 14 },
});
