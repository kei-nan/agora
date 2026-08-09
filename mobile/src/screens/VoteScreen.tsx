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
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  TouchableOpacity,
  View,
} from 'react-native';
import { useFocusEffect } from '@react-navigation/native';
import { claimFiscalYearTokens, allocateBudget } from '../chain/voting';
import { getSigningKeypair } from '../chain/identity';
import { getApi } from '../chain/api';
import { useAppModal } from '../components/AppModal';
import { colors } from '../theme';

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

/**
 * pallet-voting's `allocate_budget` (pallets/pallet-voting/src/lib.rs) charges
 * quadratically on the *total* votes set for a category, refunding/charging
 * only the delta against whatever was already allocated there this epoch —
 * not the full N² every time. Mirrors that math here so the live preview
 * (and the pre-submit balance check below) matches what the chain will
 * actually do, rather than a simplified "N² always debited" model that would
 * misfire for anyone adjusting an existing allocation.
 */
function computeAllocationCost(count: number, oldVotes: number): { cost: number; delta: number } {
  const oldCost = oldVotes * oldVotes;
  const cost = count * count;
  return { cost, delta: Math.max(0, cost - oldCost) };
}

export default function VoteScreen() {
  const [state, setState] = useState<BudgetState | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [claiming, setClaiming] = useState(false);
  const [allocating, setAllocating] = useState<number | null>(null);
  const [voteCounts, setVoteCounts] = useState<Record<number, string>>({ 0: '', 1: '', 2: '' });
  const { showInfo, showError } = useAppModal();

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
      showError('Failed to load budget', e);
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, [showError]);

  useFocusEffect(useCallback(() => { load(); }, [load]));

  async function handleClaim() {
    setClaiming(true);
    try {
      const { keypair } = await getSigningKeypair();
      await claimFiscalYearTokens(keypair);
      showInfo('Tokens claimed', 'Your fiscal year budget tokens have been claimed.');
      await load();
    } catch (e: any) {
      showError('Claim failed', e, 'Your budget tokens could not be claimed. Please try again.');
    } finally {
      setClaiming(false);
    }
  }

  async function handleAllocate(categoryId: number) {
    const count = parseInt(voteCounts[categoryId] ?? '0', 10);
    if (isNaN(count) || count < 0) {
      showInfo('Invalid amount', 'Enter a whole number of votes (0 or more).');
      return;
    }
    // Belt-and-suspenders: the Allocate button is already disabled once the
    // live preview below shows this would overspend, but check again here
    // too in case state has moved since the last render (e.g. balance
    // changed from another device) rather than relying solely on the UI
    // being in sync.
    const oldVotes = state?.allocations[categoryId] ?? 0;
    const { cost, delta } = computeAllocationCost(count, oldVotes);
    const balance = state?.balance ?? 0;
    if (delta > balance) {
      showInfo(
        'Not enough budget tokens',
        `Setting this category to ${count} votes costs ${cost} tokens total (${delta} more than your current allocation here) — your balance is only ${balance}.`,
      );
      return;
    }
    setAllocating(categoryId);
    try {
      const { keypair } = await getSigningKeypair();
      await allocateBudget(keypair, categoryId, count);
      showInfo('Allocated', `Set ${count} votes on this category.`);
      await load();
    } catch (e: any) {
      showError('Allocation failed', e, 'Your allocation could not be submitted. Please try again.');
    } finally {
      setAllocating(null);
    }
  }

  if (loading) {
    return (
      <View style={styles.center}>
        <ActivityIndicator color={colors.accent} />
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
          tintColor={colors.accent}
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
          <ActivityIndicator color={colors.accent} />
        ) : (
          <TouchableOpacity
            style={[styles.btn, (!state?.epoch || state?.claimed) && styles.btnDisabled]}
            onPress={handleClaim}
            disabled={!state?.epoch || state?.claimed}
            accessibilityRole="button"
            accessibilityLabel={state?.claimed ? 'Budget tokens already claimed' : `Claim budget tokens for fiscal year ${state?.epoch ?? 0}`}
            accessibilityState={{ disabled: !state?.epoch || state?.claimed }}
          >
            <Text style={styles.btnText}>{state?.claimed ? 'Already Claimed' : 'Claim Budget Tokens'}</Text>
          </TouchableOpacity>
        )}
      </View>

      {BUDGET_CATEGORIES.map((cat) => {
        const rawCount = voteCounts[cat.id] ?? '';
        const parsed = parseInt(rawCount, 10);
        const count = !isNaN(parsed) && parsed >= 0 ? parsed : 0;
        const oldVotes = state?.allocations[cat.id] ?? 0;
        const balance = state?.balance ?? 0;
        const { cost, delta } = computeAllocationCost(count, oldVotes);
        const overBudget = delta > balance;

        return (
          <View key={cat.id} style={styles.card}>
            <Text style={styles.cardTitle}>{cat.name}</Text>
            <Text style={styles.currentText}>Current: {oldVotes} votes</Text>
            <TextInput
              style={styles.input}
              placeholder="Vote count"
              placeholderTextColor={colors.textMuted}
              keyboardType="numeric"
              value={voteCounts[cat.id]}
              onChangeText={(text) => setVoteCounts((prev) => ({ ...prev, [cat.id]: text }))}
              accessibilityLabel={`Vote count for ${cat.name}`}
            />
            <Text
              style={[styles.costText, overBudget && styles.costTextWarning]}
              accessibilityLabel={
                `Allocating ${count} votes to ${cat.name} costs ${cost} tokens in total`
                + (delta > 0 ? `, ${delta} more than your current balance` : oldVotes > count ? `, refunding ${(oldVotes * oldVotes) - cost} tokens` : '')
              }
            >
              Cost: {count}² = {cost} tokens
              {delta > 0
                ? ` (${delta} more from your balance)`
                : oldVotes > count
                  ? ` (refunds ${(oldVotes * oldVotes) - cost})`
                  : ''}
            </Text>
            {overBudget && (
              <Text style={styles.warningText} accessibilityRole="alert">
                Exceeds your balance of {balance} tokens.
              </Text>
            )}
            {allocating === cat.id ? (
              <ActivityIndicator color={colors.accent} />
            ) : (
              <TouchableOpacity
                style={[styles.btn, overBudget && styles.btnDisabled]}
                onPress={() => handleAllocate(cat.id)}
                disabled={overBudget}
                accessibilityRole="button"
                accessibilityLabel={`Allocate ${count} votes to ${cat.name}${overBudget ? ', exceeds balance' : ''}`}
                accessibilityState={{ disabled: overBudget }}
              >
                <Text style={styles.btnText}>Allocate</Text>
              </TouchableOpacity>
            )}
          </View>
        );
      })}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: colors.bg, padding: 16 },
  center: { flex: 1, backgroundColor: colors.bg, alignItems: 'center', justifyContent: 'center' },
  title: { fontSize: 22, fontWeight: '700', color: colors.textPrimary, marginBottom: 4 },
  subtitle: { fontSize: 13, color: colors.textMuted, marginBottom: 16 },
  card: {
    backgroundColor: colors.card,
    borderRadius: 14,
    borderWidth: 1,
    borderColor: colors.border,
    padding: 16,
    marginBottom: 12,
    gap: 8,
  },
  cardTitle: { fontSize: 15, fontWeight: '600', color: colors.textPrimary },
  balanceText: { fontSize: 13, color: colors.textSecondary },
  currentText: { fontSize: 12, color: colors.textMuted },
  input: {
    borderWidth: 1,
    borderColor: colors.border,
    backgroundColor: colors.bg,
    borderRadius: 8,
    padding: 10,
    color: colors.textPrimary,
  },
  costText: { fontSize: 12, color: colors.textSecondary },
  costTextWarning: { color: colors.warning, fontWeight: '600' },
  warningText: { fontSize: 12, color: colors.danger, fontWeight: '600' },
  btn: {
    backgroundColor: colors.accent,
    paddingVertical: 12,
    borderRadius: 10,
    alignItems: 'center',
  },
  btnDisabled: { backgroundColor: colors.border },
  btnText: { color: colors.textPrimary, fontWeight: '600', fontSize: 14 },
});
