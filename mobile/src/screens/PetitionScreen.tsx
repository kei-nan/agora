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
import { useNavigation } from '@react-navigation/native';
import { Petition, fetchPetitions, fetchReferendumIdForPetition, signPetition } from '../chain/governance';
import { getSigningKeypair } from '../chain/identity';
import { getRegistered } from '../chain/citizenState';
import { colors } from '../theme';

export default function PetitionScreen() {
  // PetitionScreen is registered as a Tab.Screen (see App.tsx's MainTabs),
  // not a Stack.Screen, so — matching HomeScreen's QuickCard navigation —
  // cross-tab navigation to Proposals goes through `navigation as any`
  // rather than a typed TabParamList prop.
  const navigation = useNavigation();
  const [petitions, setPetitions] = useState<Petition[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [signing, setSigning] = useState<number | null>(null);
  const [signed, setSigned] = useState<Set<number>>(new Set());
  // petition_id -> referendum_id, for petitions that have crossed their
  // signature threshold. See fetchReferendumIdForPetition's doc comment
  // (governance.ts) for why "reached threshold" and "referendum exists" are
  // effectively the same moment on-chain.
  const [referendumLinks, setReferendumLinks] = useState<Record<number, number>>({});

  const load = useCallback(async () => {
    let data: Petition[];
    try {
      data = await fetchPetitions();
      setPetitions(data);
    } catch (e: any) {
      Alert.alert('Error', e.message);
      setLoading(false);
      setRefreshing(false);
      return;
    }

    // Looking up referendum links is a best-effort enhancement, not the
    // primary load — if it fails, the petition list should still render
    // (just without the "now up for a vote" badge) rather than surfacing an
    // error unrelated to fetching petitions themselves.
    try {
      const reached = data.filter((p) => p.sigCount >= p.threshold);
      const results = await Promise.all(
        reached.map((p) => fetchReferendumIdForPetition(p.id)),
      );
      const links: Record<number, number> = {};
      reached.forEach((p, i) => {
        const refId = results[i];
        if (refId !== null) links[p.id] = refId;
      });
      setReferendumLinks(links);
    } catch {
      // ignore — see comment above
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
    return <View style={s.center}><ActivityIndicator color={colors.accent} /></View>;
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
          tintColor={colors.accent}
        />
      }
      ListEmptyComponent={<Text style={s.empty}>No active petitions.</Text>}
      renderItem={({ item }) => {
        const pct = Math.min(100, Math.round((item.sigCount / item.threshold) * 100));
        const reached = item.sigCount >= item.threshold;
        const hasSigned = signed.has(item.id);
        const referendumId = referendumLinks[item.id];
        return (
          <View style={s.card}>
            <View style={s.cardHeader}>
              <Text style={s.id}>Petition #{item.id}</Text>
              {reached && (
                referendumId !== undefined ? (
                  <TouchableOpacity
                    style={s.reachedBadge}
                    onPress={() => (navigation as any).navigate('Proposals')}
                    accessibilityRole="link"
                    accessibilityLabel={`Petition ${item.id} is now up for a vote as proposal ${referendumId}. View it in Proposals.`}
                  >
                    <Text style={s.reachedText}>Now up for a vote → #{referendumId}</Text>
                  </TouchableOpacity>
                ) : (
                  <View style={s.reachedBadge}>
                    <Text style={s.reachedText}>Referendum pending</Text>
                  </View>
                )
              )}
            </View>

            <View
              accessible
              accessibilityLabel={`${item.title}. ${item.description ? item.description + '. ' : ''}${item.sigCount.toLocaleString()} of ${item.threshold.toLocaleString()} signatures, ${pct}%.`}
            >
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
            </View>

            <TouchableOpacity
              style={[s.signBtn, (hasSigned || reached) && s.signBtnDone]}
              onPress={() => handleSign(item.id)}
              disabled={signing === item.id || hasSigned || reached}
              accessibilityRole="button"
              accessibilityLabel={
                hasSigned ? `Petition ${item.id} signed` : `Sign petition ${item.id}`
              }
              accessibilityState={{ disabled: signing === item.id || hasSigned || reached }}
            >
              {signing === item.id
                ? <ActivityIndicator color={colors.textPrimary} size="small" />
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
  cardHeader: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', marginBottom: 10 },
  id: { fontSize: 12, color: colors.textDim, fontWeight: '600' },
  reachedBadge: { backgroundColor: '#052e16', paddingHorizontal: 8, paddingVertical: 3, borderRadius: 6 },
  reachedText: { fontSize: 11, color: colors.success, fontWeight: '600' },
  title: { fontSize: 16, fontWeight: '700', color: colors.textPrimary, marginBottom: 8, lineHeight: 22 },
  description: { fontSize: 13, color: colors.textSecondary, lineHeight: 19, marginBottom: 14 },
  sigRow: { flexDirection: 'row', justifyContent: 'space-between', marginBottom: 6 },
  sigCount: { fontSize: 12, color: colors.textMuted },
  pct: { fontSize: 12, color: colors.textMuted },
  barBg: { height: 6, backgroundColor: colors.border, borderRadius: 3, marginBottom: 14 },
  barFill: { height: 6, backgroundColor: colors.accent, borderRadius: 3 },
  barFillReached: { backgroundColor: colors.success },
  signBtn: { backgroundColor: colors.accent, paddingVertical: 12, borderRadius: 10, alignItems: 'center' },
  signBtnDone: { backgroundColor: colors.border },
  signBtnText: { color: colors.textPrimary, fontWeight: '600', fontSize: 14 },
});
