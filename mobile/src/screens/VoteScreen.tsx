/**
 * Budget screen — claim quadratic-voting budget tokens for the current
 * fiscal year and allocate them across spending categories
 * (claim_fiscal_year_tokens / allocate_budget in pallet-voting).
 *
 * This screen used to also list active proposals with Vote/Delegate buttons,
 * duplicating ProposalsScreen (referendum voting) and DelegateScreen (topic
 * delegation), which already cover those against the real chain. That
 * section has been removed — this screen's only remaining unique value is
 * the budget/QV flow below, so it's registered in App.tsx as the "Budget" tab.
 */
import React, { useCallback, useState } from 'react';
import {
  ActivityIndicator,
  Alert,
  Button,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { useFocusEffect } from '@react-navigation/native';
import { claimFiscalYearTokens, allocateBudget } from '../chain/voting';
import { getSigningKeypair } from '../chain/identity';
import { getApi } from '../chain/api';

// pallet-voting's BudgetCategoryCount constant currently allows up to 10
// category ids (see runtime/src/configs/mod.rs). Real category names/
// taxonomy are a product & governance decision the chain doesn't encode, so
// this UI only exposes a fixed first few as a starting point.
const BUDGET_CATEGORIES = [
  { id: 0, name: 'Healthcare' },
  { id: 1, name: 'Infrastructure' },
  { id: 2, name: 'Education' },
];

interface BudgetState {
  epoch: number;
  claimed: boolean;
  balance: number;
  allocations: Record<number, number>;
}

export default function VoteScreen() {
  const [state, setState] = useState<BudgetState | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [claiming, setClaiming] = useState(false);
  const [allocating, setAllocating] = useState<number | null>(null);
  const [voteCounts, setVoteCounts] = useState<Record<number, string>>({ 0: '', 1: '', 2: '' });

  const load = useCallback(async () => {
    try {
      const api = await getApi();
      const { keypair } = await getSigningKeypair();
      const address = keypair.address;

      const epoch = (await api.query.voting.fiscalYearEpoch() as any).toNumber();
      const claimedEpoch = await api.query.voting.citizenClaimedEpoch(address);
      const balance = await api.query.voting.budgetBalance(address);

      const allocations: Record<number, number> = {};
      await Promise.all(
        BUDGET_CATEGORIES.map(async (cat) => {
          const v = await api.query.voting.categoryVotes([address, epoch, cat.id]);
          allocations[cat.id] = (v as any).toNumber();
        }),
      );

      setState({
        epoch,
        claimed: epoch > 0 && (claimedEpoch as any).isSome && (claimedEpoch as any).unwrap().toNumber() >= epoch,
        balance: (balance as any).toNumber(),
        allocations,
      });
      setVoteCounts(
        Object.fromEntries(BUDGET_CATEGORIES.map((c) => [c.id, String(allocations[c.id] ?? 0)])),
      );
    } catch (e: any) {
      Alert.alert('Failed to load budget', e.message);
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, []);

  useFocusEffect(useCallback(() => { load(); }, [load]));

  async function handleClaim() {
    setClaiming(true);
    try {
      const { keypair } = await getSigningKeypair();
      await claimFiscalYearTokens(keypair);
      Alert.alert('Tokens claimed', 'Your fiscal year budget tokens have been claimed.');
      await load();
    } catch (e: any) {
      Alert.alert('Claim failed', e.message);
    } finally {
      setClaiming(false);
    }
  }

  async function handleAllocate(categoryId: number) {
    const count = parseInt(voteCounts[categoryId] ?? '0', 10);
    if (isNaN(count) || count < 0) {
      Alert.alert('Invalid amount', 'Enter a whole number of votes (0 or more).');
      return;
    }
    setAllocating(categoryId);
    try {
      const { keypair } = await getSigningKeypair();
      await allocateBudget(keypair, categoryId, count);
      Alert.alert('Allocated', `Set ${count} votes on this category.`);
      await load();
    } catch (e: any) {
      Alert.alert('Allocation failed', e.message);
    } finally {
      setAllocating(null);
    }
  }

  if (loading) {
    return (
      <View style={styles.center}>
        <ActivityIndicator color="#6C63FF" />
      </View>
    );
  }

  return (
    <ScrollView
      style={styles.container}
      refreshControl={
        <RefreshControl
          refreshing={refreshing}
          onRefresh={() => { setRefreshing(true); load(); }}
          tintColor="#6C63FF"
        />
      }
    >
      <Text style={styles.title}>Budget Allocation</Text>
      <Text style={styles.subtitle}>
        Quadratic voting — allocating N votes to a category costs N² tokens.
      </Text>

      <View style={styles.card}>
        <Text style={styles.cardTitle}>Fiscal Year {state?.epoch ?? 0}</Text>
        <Text style={styles.balanceText}>Balance: {state?.balance ?? 0} tokens</Text>
        {claiming ? (
          <ActivityIndicator color="#6C63FF" />
        ) : (
          <Button
            title={state?.claimed ? 'Already Claimed' : 'Claim Budget Tokens'}
            onPress={handleClaim}
            disabled={!state?.epoch || state?.claimed}
          />
        )}
      </View>

      {BUDGET_CATEGORIES.map((cat) => (
        <View key={cat.id} style={styles.card}>
          <Text style={styles.cardTitle}>{cat.name}</Text>
          <Text style={styles.currentText}>Current: {state?.allocations[cat.id] ?? 0} votes</Text>
          <TextInput
            style={styles.input}
            placeholder="Vote count"
            placeholderTextColor="#6b7280"
            keyboardType="numeric"
            value={voteCounts[cat.id]}
            onChangeText={(text) => setVoteCounts((prev) => ({ ...prev, [cat.id]: text }))}
          />
          {allocating === cat.id ? (
            <ActivityIndicator color="#6C63FF" />
          ) : (
            <Button title="Allocate" onPress={() => handleAllocate(cat.id)} />
          )}
        </View>
      ))}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#0f1117', padding: 16 },
  center: { flex: 1, backgroundColor: '#0f1117', alignItems: 'center', justifyContent: 'center' },
  title: { fontSize: 22, fontWeight: '700', color: '#ffffff', marginBottom: 4 },
  subtitle: { fontSize: 13, color: '#6b7280', marginBottom: 16 },
  card: {
    backgroundColor: '#161b27',
    borderRadius: 14,
    borderWidth: 1,
    borderColor: '#1f2937',
    padding: 16,
    marginBottom: 12,
    gap: 8,
  },
  cardTitle: { fontSize: 15, fontWeight: '600', color: '#ffffff' },
  balanceText: { fontSize: 13, color: '#9ca3af' },
  currentText: { fontSize: 12, color: '#6b7280' },
  input: {
    borderWidth: 1,
    borderColor: '#1f2937',
    backgroundColor: '#0f1117',
    borderRadius: 8,
    padding: 10,
    color: '#ffffff',
  },
});
