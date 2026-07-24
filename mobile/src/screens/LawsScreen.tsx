import React, { useCallback, useEffect, useState } from 'react';
import {
  ActivityIndicator,
  FlatList,
  RefreshControl,
  StyleSheet,
  Text,
  View,
} from 'react-native';
import { Law, fetchLaws } from '../chain/governance';

export default function LawsScreen() {
  const [laws, setLaws] = useState<Law[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);

  const load = useCallback(async () => {
    try {
      setLaws(await fetchLaws());
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  if (loading) {
    return <View style={s.center}><ActivityIndicator color="#6C63FF" /></View>;
  }

  return (
    <FlatList
      style={s.list}
      data={laws}
      keyExtractor={(l) => String(l.id)}
      refreshControl={
        <RefreshControl
          refreshing={refreshing}
          onRefresh={() => { setRefreshing(true); load(); }}
          tintColor="#6C63FF"
        />
      }
      ListEmptyComponent={<Text style={s.empty}>No laws enacted yet.</Text>}
      renderItem={({ item }) => (
        <View style={s.card}>
          <View style={s.header}>
            <Text style={[s.statusChip,
              item.status === 'Active' ? s.active
              : item.status === 'Paused' ? s.paused
              : s.repealed
            ]}>
              {item.status}
            </Text>
            {item.tier === 'Constitutional' && (
              <Text style={[s.statusChip, s.constitutional]}>constitutional</Text>
            )}
            <Text style={s.idText}>Law #{item.id} · v{item.version}</Text>
          </View>
          <Text style={s.hash} numberOfLines={1}>{item.contentHash}</Text>
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
    marginBottom: 10,
    borderWidth: 1,
    borderColor: '#1f2937',
  },
  header: { flexDirection: 'row', alignItems: 'center', gap: 8, marginBottom: 8, flexWrap: 'wrap' },
  statusChip: { fontSize: 11, fontWeight: '600', paddingHorizontal: 8, paddingVertical: 3, borderRadius: 6 },
  active: { backgroundColor: '#1a3a1a', color: '#22c55e' },
  paused: { backgroundColor: '#2a2a1a', color: '#f59e0b' },
  repealed: { backgroundColor: '#3a1a1a', color: '#ef4444' },
  constitutional: { backgroundColor: '#1e1040', color: '#a78bfa' },
  idText: { fontSize: 12, color: '#6b7280', marginLeft: 'auto' },
  hash: { fontSize: 11, color: '#4b5563', fontFamily: 'monospace' },
});
