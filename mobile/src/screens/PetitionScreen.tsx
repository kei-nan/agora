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

const SIGNATURE_THRESHOLD = 1000;

export default function PetitionScreen() {
  const [petitions, setPetitions] = useState<Petition[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [signing, setSigning] = useState<number | null>(null);

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
    setSigning(petitionId);
    try {
      const { keypair } = await getSigningKeypair();
      await signPetition(keypair, petitionId);
      Alert.alert('Signed', `You signed petition #${petitionId}.`);
      load();
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
        const pct = Math.min(100, Math.round((item.sigCount / SIGNATURE_THRESHOLD) * 100));
        return (
          <View style={s.card}>
            <View style={s.cardHeader}>
              <Text style={s.id}>Petition #{item.id}</Text>
              <Text style={s.sigCount}>{item.sigCount} / {SIGNATURE_THRESHOLD} signatures</Text>
            </View>

            <Text style={s.hash} numberOfLines={1}>{item.topicHash}</Text>

            {/* Progress bar */}
            <View style={s.barBg}>
              <View style={[s.barFill, { width: `${pct}%` as any }]} />
            </View>
            <Text style={s.pct}>{pct}% to referendum</Text>

            <TouchableOpacity
              style={s.signBtn}
              onPress={() => handleSign(item.id)}
              disabled={signing === item.id}
            >
              {signing === item.id
                ? <ActivityIndicator color="#fff" size="small" />
                : <Text style={s.signBtnText}>Sign this petition</Text>}
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
  cardHeader: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', marginBottom: 8 },
  id: { fontSize: 14, fontWeight: '700', color: '#ffffff' },
  sigCount: { fontSize: 12, color: '#6b7280' },
  hash: { fontSize: 11, color: '#4b5563', fontFamily: 'monospace', marginBottom: 12 },
  barBg: { height: 6, backgroundColor: '#1f2937', borderRadius: 3, marginBottom: 6 },
  barFill: { height: 6, backgroundColor: '#6C63FF', borderRadius: 3 },
  pct: { fontSize: 12, color: '#9ca3af', marginBottom: 14 },
  signBtn: {
    backgroundColor: '#6C63FF',
    paddingVertical: 12,
    borderRadius: 10,
    alignItems: 'center',
  },
  signBtnText: { color: '#ffffff', fontWeight: '600', fontSize: 14 },
});
