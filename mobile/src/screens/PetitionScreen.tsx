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
import { Petition, fetchPetitions, signPetition } from '../chain/governance';
import { getSigningKeypair } from '../chain/identity';
import { getRegistered } from '../chain/citizenState';

export default function PetitionScreen() {
  const [petitions, setPetitions] = useState<Petition[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [signing, setSigning] = useState<number | null>(null);
  const [signed, setSigned] = useState<Set<number>>(new Set());

  const load = useCallback(async () => {
    try {
      setPetitions(await fetchPetitions());
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  async function handleSign(petitionId: number) {
    if (!getRegistered()) {
      Alert.alert('Not registered', 'You must be a registered citizen to sign petitions.');
      return;
    }
    setSigning(petitionId);
    try {
      const { keypair } = await getSigningKeypair();
      await signPetition(keypair, petitionId);
      setSigned(prev => new Set(prev).add(petitionId));
    } catch (e: any) {
      Alert.alert('Failed to sign', e.message);
    } finally {
      setSigning(null);
    }
  }

  if (loading) {
    return <View style={s.center}><ActivityIndicator color="#6C63FF" /></View>;
  }

  return (
    <FlatList
      style={s.list}
      data={petitions}
      keyExtractor={(p) => String(p.id)}
      refreshControl={
        <RefreshControl
          refreshing={refreshing}
          onRefresh={() => { setRefreshing(true); load(); }}
          tintColor="#6C63FF"
        />
      }
      ListEmptyComponent={<Text style={s.empty}>No active petitions.</Text>}
      renderItem={({ item }) => {
        const pct = Math.min(100, Math.round((item.sigCount / item.threshold) * 100));
        const reached = item.sigCount >= item.threshold;
        const hasSigned = signed.has(item.id);
        return (
          <View style={s.card}>
            <View style={s.cardHeader}>
              <Text style={s.id}>Petition #{item.id}</Text>
              {reached && (
                <View style={s.reachedBadge}>
                  <Text style={s.reachedText}>Referendum pending</Text>
                </View>
              )}
            </View>

            <Text style={s.title}>{item.title}</Text>
            <Text style={s.description}>{item.description}</Text>

            <View style={s.sigRow}>
              <Text style={s.sigCount}>{item.sigCount.toLocaleString()} / {item.threshold.toLocaleString()} signatures</Text>
              <Text style={s.pct}>{pct}%</Text>
            </View>
            <View style={s.barBg}>
              <View style={[s.barFill, { width: `${pct}%` as any },
                reached && s.barFillReached]} />
            </View>

            <TouchableOpacity
              style={[s.signBtn, (hasSigned || reached) && s.signBtnDone]}
              onPress={() => handleSign(item.id)}
              disabled={signing === item.id || hasSigned || reached}
            >
              {signing === item.id
                ? <ActivityIndicator color="#fff" size="small" />
                : <Text style={s.signBtnText}>
                    {hasSigned ? '✓ Signed' : reached ? 'Threshold reached' : 'Sign this petition'}
                  </Text>}
            </TouchableOpacity>
          </View>
        );
      }}
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
  cardHeader: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', marginBottom: 10 },
  id: { fontSize: 12, color: '#4b5563', fontWeight: '600' },
  reachedBadge: { backgroundColor: '#052e16', paddingHorizontal: 8, paddingVertical: 3, borderRadius: 6 },
  reachedText: { fontSize: 11, color: '#22c55e', fontWeight: '600' },
  title: { fontSize: 16, fontWeight: '700', color: '#ffffff', marginBottom: 8, lineHeight: 22 },
  description: { fontSize: 13, color: '#9ca3af', lineHeight: 19, marginBottom: 14 },
  sigRow: { flexDirection: 'row', justifyContent: 'space-between', marginBottom: 6 },
  sigCount: { fontSize: 12, color: '#6b7280' },
  pct: { fontSize: 12, color: '#6b7280' },
  barBg: { height: 6, backgroundColor: '#1f2937', borderRadius: 3, marginBottom: 14 },
  barFill: { height: 6, backgroundColor: '#6C63FF', borderRadius: 3 },
  barFillReached: { backgroundColor: '#22c55e' },
  signBtn: { backgroundColor: '#6C63FF', paddingVertical: 12, borderRadius: 10, alignItems: 'center' },
  signBtnDone: { backgroundColor: '#1f2937' },
  signBtnText: { color: '#ffffff', fontWeight: '600', fontSize: 14 },
});
