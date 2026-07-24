import React, { useCallback, useEffect, useState } from 'react';
import {
  ActivityIndicator,
  Alert,
  FlatList,
  RefreshControl,
  StyleSheet,
  Text,
  TouchableOpacity,
  View,
} from 'react-native';
import { Proposal, fetchProposals, voteOnReferendum } from '../chain/governance';
import { getSigningKeypair } from '../chain/identity';

export default function ProposalsScreen() {
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [voting, setVoting] = useState<number | null>(null);

  const load = useCallback(async () => {
    try {
      const data = await fetchProposals();
      setProposals(data);
    } catch (e: any) {
      Alert.alert('Error', e.message);
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  async function vote(id: number, inFavor: boolean) {
    setVoting(id);
    try {
      const { keypair } = await getSigningKeypair();
      await voteOnReferendum(keypair, id, inFavor);
      Alert.alert('Vote cast', `You voted ${inFavor ? 'for' : 'against'} proposal #${id}.`);
      load();
    } catch (e: any) {
      Alert.alert('Vote failed', e.message);
    } finally {
      setVoting(null);
    }
  }

  if (loading) {
    return <View style={s.center}><ActivityIndicator color="#6C63FF" /></View>;
  }

  return (
    <FlatList
      style={s.list}
      data={proposals}
      keyExtractor={(p) => String(p.id)}
      refreshControl={<RefreshControl refreshing={refreshing} onRefresh={() => { setRefreshing(true); load(); }} tintColor="#6C63FF" />}
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
              >
                {voting === item.id
                  ? <ActivityIndicator color="#fff" size="small" />
                  : <Text style={s.voteBtnText}>Vote For</Text>}
              </TouchableOpacity>
              <TouchableOpacity
                style={[s.voteBtn, s.voteBtnAgainst]}
                onPress={() => vote(item.id, false)}
                disabled={voting === item.id}
              >
                <Text style={s.voteBtnText}>Vote Against</Text>
              </TouchableOpacity>
            </View>
          )}
        </View>
      )}
    />
  );
}

const s = StyleSheet.create({
  list: { flex: 1, backgroundColor: '#0f1117', padding: 16 },
  center: { flex: 1, backgroundColor: '#0f1117', alignItems: 'center', justifyContent: 'center' },
  empty: { color: '#6b7280', textAlign: 'center', marginTop: 40 },
  card: {
    backgroundColor: '#161b27',
    borderRadius: 14,
    padding: 16,
    marginBottom: 12,
    borderWidth: 1,
    borderColor: '#1f2937',
  },
  cardHeader: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', marginBottom: 8 },
  chips: { flexDirection: 'row', gap: 6 },
  chip: { fontSize: 11, fontWeight: '600', paddingHorizontal: 8, paddingVertical: 3, borderRadius: 6 },
  chipActive: { backgroundColor: '#1a3a1a', color: '#22c55e' },
  chipDone: { backgroundColor: '#1f2937', color: '#9ca3af' },
  chipConst: { backgroundColor: '#1e1040', color: '#a78bfa' },
  id: { fontSize: 12, color: '#6b7280' },
  hash: { fontSize: 11, color: '#4b5563', fontFamily: 'monospace', marginBottom: 10 },
  tally: { flexDirection: 'row', gap: 16, marginBottom: 14 },
  forVotes: { color: '#22c55e', fontSize: 14, fontWeight: '600' },
  againstVotes: { color: '#ef4444', fontSize: 14, fontWeight: '600' },
  voteRow: { flexDirection: 'row', gap: 10 },
  voteBtn: { flex: 1, paddingVertical: 10, borderRadius: 10, alignItems: 'center' },
  voteBtnFor: { backgroundColor: '#166534' },
  voteBtnAgainst: { backgroundColor: '#7f1d1d' },
  voteBtnText: { color: '#ffffff', fontWeight: '600', fontSize: 14 },
});
