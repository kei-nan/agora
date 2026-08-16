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
import React, { useCallback, useRef, useState } from 'react';
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
import { fetchPendingDuties, PendingDuty, submitRound1 } from '../chain/oprfCommittee';
import { freshOprfSeed } from '../crypto/wasmCommitteeCrypto';
import { devOprfSecretShare, devSigningKeypair } from '../storage/keyStorage';

/** `${queryId}:${committeeSlot}` -> the round-1 seed used for that pair, held only for
 * the lifetime of this screen (matches `committee-node/src/main.rs`'s own in-memory,
 * lost-on-restart `QueryProgress` — see that file's doc comment for the accepted
 * limitation this mirrors: a query already past round 1 simply cannot be resumed here
 * after a restart until it expires and a fresh one is posted). */
function dutyKey(duty: Pick<PendingDuty, 'queryId' | 'committeeSlot'>): string {
  return `${duty.queryId}:${duty.committeeSlot}`;
}

export default function CommitteeDutyScreen() {
  const [duties, setDuties] = useState<PendingDuty[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [fulfillingId, setFulfillingId] = useState<number | null>(null);
  const round1Seeds = useRef<Map<string, Uint8Array>>(new Map());

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
        const [pair, secretShareBytes] = await Promise.all([devSigningKeypair(), devOprfSecretShare()]);

        if (duty.phase === 'round1') {
          const seed = freshOprfSeed();
          await submitRound1({ duty, secretShareBytes, pair, seed });
          round1Seeds.current.set(dutyKey(duty), seed);
          Alert.alert(
            'Round 1 submitted',
            `Submitted round 1 for query #${duty.queryId} on committee slot ${duty.committeeSlot}. ` +
              'Round 2 opens once enough other members have also submitted round 1 — refresh later to check.',
          );
        } else {
          // phase === 'round2'. Round 2 needs this member's binding factor, Lagrange
          // coefficient, and the shared challenge — public values computed from the
          // locked round-1 set (see `chain/oprfCommittee.ts::submitRound2`'s doc
          // comment). This app has no JS/TS port of that aggregation math yet (it's
          // native-Rust-only today, in `oprf-committee-dev::threshold`), so round 2
          // cannot actually be submitted from here — surfacing that honestly rather
          // than fabricating placeholder values and submitting garbage on-chain.
          const seed = round1Seeds.current.get(dutyKey(duty));
          if (!seed) {
            throw new Error(
              'This screen only tracks a round-1 seed for the lifetime of the app session — ' +
                'restart or a missed round 1 means round 2 cannot be completed for this query ' +
                '(same limitation committee-node documents for its own in-memory query progress).',
            );
          }
          throw new Error(
            'Round 2 submission is not implemented yet: it needs this member\'s binding factor, ' +
              "Lagrange coefficient, and the shared challenge, computed from the locked round-1 " +
              'set. No JS/TS port of that math exists in this app (see oprfCommittee.ts::submitRound2).',
          );
        }
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
              Query #{item.queryId} · committee slot {item.committeeSlot} · {item.phase === 'round1' ? 'round 1' : 'round 2'}
            </Text>
            <Text style={styles.rowSubText}>from {item.submitter.slice(0, 10)}… · posted at block {item.postedAt}</Text>
            <Button
              title={fulfillingId === item.queryId ? 'Fulfilling…' : item.phase === 'round1' ? 'Submit round 1' : 'Submit round 2'}
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
