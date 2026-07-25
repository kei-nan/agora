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
          <View style={s.chips}>
            <View style={[s.chip,
              item.status === 'Active' ? s.chipActive
              : item.status === 'Paused' ? s.chipPaused
              : s.chipRepealed
            ]}>
              <Text style={[s.chipText,
                item.status === 'Active' ? s.chipTextActive
                : item.status === 'Paused' ? s.chipTextPaused
                : s.chipTextRepealed
              ]}>{item.status}</Text>
            </View>
            {item.tier === 'Constitutional' && (
              <View style={[s.chip, s.chipConstitutional]}>
                <Text style={[s.chipText, s.chipTextConstitutional]}>Constitutional</Text>
              </View>
            )}
            <Text style={s.meta}>Law #{item.id} · v{item.version}</Text>
          </View>
          <Text style={s.title}>{item.title}</Text>
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
  chips: { flexDirection: 'row', alignItems: 'center', gap: 6, marginBottom: 10, flexWrap: 'wrap' },
  chip: { paddingHorizontal: 8, paddingVertical: 3, borderRadius: 6 },
  chipText: { fontSize: 11, fontWeight: '600' },
  chipActive: { backgroundColor: '#052e16' },
  chipTextActive: { color: '#22c55e' },
  chipPaused: { backgroundColor: '#2a2a1a' },
  chipTextPaused: { color: '#f59e0b' },
  chipRepealed: { backgroundColor: '#2d1515' },
  chipTextRepealed: { color: '#ef4444' },
  chipConstitutional: { backgroundColor: '#1e1040' },
  chipTextConstitutional: { color: '#a78bfa' },
  meta: { fontSize: 11, color: '#4b5563', marginLeft: 'auto' },
  title: { fontSize: 15, fontWeight: '600', color: '#ffffff', marginBottom: 8, lineHeight: 21 },
  hash: { fontSize: 11, color: '#374151', fontFamily: 'monospace' },
});
