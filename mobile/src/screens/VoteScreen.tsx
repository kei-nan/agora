/**
 * Voting screen — shows active proposals and lets citizens vote or delegate.
 */
import React, { useEffect, useState } from 'react';
import {
  Button,
  FlatList,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { claimFiscalYearTokens, allocateBudget } from '../chain/voting';

interface Proposal {
  id: number;
  endsAt: number;
}

const BUDGET_CATEGORIES = [
  { id: 0, name: 'Healthcare' },
  { id: 1, name: 'Infrastructure' },
  { id: 2, name: 'Education' },
];

export default function VoteScreen() {
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [voteCounts, setVoteCounts] = useState<Record<number, string>>({
    0: '',
    1: '',
    2: '',
  });

  useEffect(() => {
    // TODO: query api.query.voting.proposals.entries() and populate list
  }, []);

  const handleClaim = () => {
    // TODO: pass the active KeyringPair from auth context
    // claimFiscalYearTokens(pair)
  };

  const handleAllocate = (categoryId: number) => {
    const count = parseInt(voteCounts[categoryId] ?? '0', 10);
    if (isNaN(count) || count <= 0) return;
    // TODO: pass the active KeyringPair from auth context
    // allocateBudget(pair, categoryId, count)
  };

  return (
    <ScrollView style={styles.container}>
      <Text style={styles.title}>Active Proposals</Text>
      <FlatList
        data={proposals}
        keyExtractor={(item) => String(item.id)}
        scrollEnabled={false}
        renderItem={({ item }) => (
          <View style={styles.card}>
            <Text style={styles.cardTitle}>Proposal #{item.id}</Text>
            <Text>Ends at block {item.endsAt}</Text>
            <Button title="Vote" onPress={() => { /* TODO */ }} />
            <Button title="Delegate" onPress={() => { /* TODO */ }} />
          </View>
        )}
        ListEmptyComponent={<Text>No active proposals</Text>}
      />

      <Text style={[styles.title, styles.sectionGap]}>Budget Allocation</Text>
      <View style={styles.card}>
        <Text style={styles.cardTitle}>Fiscal Year Tokens</Text>
        <Button title="Claim Budget Tokens" onPress={handleClaim} />
      </View>

      {BUDGET_CATEGORIES.map((cat) => (
        <View key={cat.id} style={styles.card}>
          <Text style={styles.cardTitle}>{cat.name}</Text>
          <TextInput
            style={styles.input}
            placeholder="Vote count"
            keyboardType="numeric"
            value={voteCounts[cat.id]}
            onChangeText={(text) =>
              setVoteCounts((prev) => ({ ...prev, [cat.id]: text }))
            }
          />
          <Button title="Allocate" onPress={() => handleAllocate(cat.id)} />
        </View>
      ))}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, padding: 16 },
  title: { fontSize: 22, fontWeight: '700', marginBottom: 12 },
  sectionGap: { marginTop: 24 },
  card: { borderWidth: 1, borderColor: '#ccc', borderRadius: 8, padding: 12, marginBottom: 12 },
  cardTitle: { fontWeight: '600', marginBottom: 4 },
  input: {
    borderWidth: 1,
    borderColor: '#aaa',
    borderRadius: 6,
    padding: 8,
    marginBottom: 8,
  },
});
