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
import { Proposal, fetchProposals, voteOnReferendum } from '../chain/governance';
import { getSigningKeypair } from '../chain/identity';
import { useAppModal } from '../components/AppModal';
import { colors } from '../theme';

export default function ProposalsScreen() {
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [voting, setVoting] = useState<number | null>(null);
  const { showInfo, showError, showConfirm } = useAppModal();

  const load = useCallback(async () => {
    try {
      const data = await fetchProposals();
      setProposals(data);
    } catch (e: any) {
      showError("Couldn't load proposals", e);
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
      showInfo('Vote cast', `You voted ${inFavor ? 'for' : 'against'} proposal #${id}.`);
      load();
    } catch (e: any) {
      showError('Vote failed', e, 'Your vote could not be submitted. Please check your connection and try again.');
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
      renderItem={({ item }) => (
        <View style={s.card}>
          <View style={s.cardHeader}>
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

          <Text style={s.hash} numberOfLines={1}>{item.topicHash}</Text>

          <View style={s.tally}>
            <Text style={s.forVotes}>▲ {item.votesFor} for</Text>
            <Text style={s.againstVotes}>▼ {item.votesAgainst} against</Text>
          </View>

          {item.state === 'Voting' && (
            <View style={s.voteRow}>
              <TouchableOpacity
                style={[s.voteBtn, s.voteBtnFor]}
                onPress={() => vote(item.id, true)}
                disabled={voting === item.id}
                accessibilityRole="button"
                accessibilityLabel={`Vote for proposal ${item.id}`}
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
              >
                {voting === item.id
                  ? <ActivityIndicator color={colors.textPrimary} size="small" />
                  : <Text style={s.voteBtnText}>Vote Against</Text>}
              </TouchableOpacity>
            </View>
          )}
        </View>
      )}
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
  hash: { fontSize: 11, color: colors.textDim, fontFamily: 'monospace', marginBottom: 10 },
  tally: { flexDirection: 'row', gap: 16, marginBottom: 14 },
  forVotes: { color: colors.success, fontSize: 14, fontWeight: '600' },
  againstVotes: { color: colors.danger, fontSize: 14, fontWeight: '600' },
  voteRow: { flexDirection: 'row', gap: 10 },
  voteBtn: { flex: 1, paddingVertical: 10, borderRadius: 10, alignItems: 'center' },
  voteBtnFor: { backgroundColor: colors.successSolid },
  voteBtnAgainst: { backgroundColor: colors.dangerSolid },
  voteBtnText: { color: colors.textPrimary, fontWeight: '600', fontSize: 14 },
});
