/**
 * The committee-duty app's single screen: "check for pending duties" + "fulfill duty".
 *
 * Polling only — no push notifications (changelog 082's deliberate baseline: a
 * committee member checks chain state, the same way a citizen checks
 * `pallet-courts`' jury pool rather than being notified). "Check for pending duties"
 * is a manual pull-to-refresh plus an on-mount load; there is no background poll
 * timer here, matching how the rest of this codebase's screens (e.g.
 * `mobile/src/screens/VoteScreen.tsx`) only reload on focus/pull-to-refresh, not on an
 * interval.
 *
 * Not unit-tested, same convention `mobile/`'s own screens follow — this repo's jest
 * config for both apps only matches `src/**\/*.test.ts` (not `.tsx`), so screens are
 * exercised manually/via a real app run, and the logic they call
 * (`chain/oprfCommittee.ts`) carries the real test coverage instead.
 */
import React, { useCallback, useState } from 'react';
import {
  ActivityIndicator,
  Alert,
  Button,
  FlatList,
  RefreshControl,
  SafeAreaView,
  StyleSheet,
  Text,
  View,
} from 'react-native';
import { fetchPendingDuties, fulfillDuty, PendingDuty } from '../chain/oprfCommittee';
import { devOprfSecretShare, devSigningKeypair } from '../storage/keyStorage';

export default function CommitteeDutyScreen() {
  const [duties, setDuties] = useState<PendingDuty[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [fulfillingId, setFulfillingId] = useState<number | null>(null);

  const load = useCallback(async () => {
    try {
      const { keypair } = { keypair: await devSigningKeypair() };
      const found = await fetchPendingDuties(keypair.address);
      setDuties(found);
    } catch (e) {
      Alert.alert('Failed to check for pending duties', String(e));
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, []);

  React.useEffect(() => {
    load();
  }, [load]);

  const onRefresh = useCallback(() => {
    setRefreshing(true);
    load();
  }, [load]);

  const onFulfill = useCallback(
    async (duty: PendingDuty) => {
      setFulfillingId(duty.queryId);
      try {
        const [pair, secretKeyBytes] = await Promise.all([devSigningKeypair(), devOprfSecretShare()]);
        await fulfillDuty({ duty, secretKeyBytes, pair });
        Alert.alert('Duty fulfilled', `Responded to query #${duty.queryId} on committee slot ${duty.committeeSlot}.`);
        setDuties((prev) => prev.filter((d) => !(d.queryId === duty.queryId && d.committeeSlot === duty.committeeSlot)));
      } catch (e) {
        Alert.alert('Failed to fulfill duty', String(e));
      } finally {
        setFulfillingId(null);
      }
    },
    [],
  );

  if (loading) {
    return (
      <SafeAreaView style={styles.container}>
        <ActivityIndicator />
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView style={styles.container}>
      <Text style={styles.title}>Committee Duties</Text>
      <FlatList
        data={duties}
        keyExtractor={(item) => `${item.queryId}:${item.committeeSlot}`}
        refreshControl={<RefreshControl refreshing={refreshing} onRefresh={onRefresh} />}
        ListEmptyComponent={<Text style={styles.empty}>No pending duties right now.</Text>}
        renderItem={({ item }) => (
          <View style={styles.row}>
            <Text style={styles.rowText}>
              Query #{item.queryId} · committee slot {item.committeeSlot}
            </Text>
            <Text style={styles.rowSubText}>from {item.submitter.slice(0, 10)}… · posted at block {item.postedAt}</Text>
            <Button
              title={fulfillingId === item.queryId ? 'Fulfilling…' : 'Fulfill duty'}
              onPress={() => onFulfill(item)}
              disabled={fulfillingId !== null}
            />
          </View>
        )}
      />
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, padding: 16 },
  title: { fontSize: 20, fontWeight: '600', marginBottom: 12 },
  empty: { color: '#666', marginTop: 24, textAlign: 'center' },
  row: { paddingVertical: 12, borderBottomWidth: StyleSheet.hairlineWidth, borderColor: '#ccc' },
  rowText: { fontSize: 16, fontWeight: '500' },
  rowSubText: { fontSize: 12, color: '#666', marginBottom: 8 },
});
