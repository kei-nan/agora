import React, { useCallback, useEffect, useState } from 'react';
import {
  ActivityIndicator,
  Alert,
  StyleSheet,
  Text,
  TextInput,
  TouchableOpacity,
  View,
  ScrollView,
} from 'react-native';
import { delegateVote, getDelegation, revokeDelegation } from '../chain/governance';
import { getSigningKeypair } from '../chain/identity';

const TOPICS = [
  { id: 0, label: 'General' },
  { id: 1, label: 'Budget' },
  { id: 2, label: 'Constitutional' },
  { id: 3, label: 'Foreign Affairs' },
  { id: 4, label: 'Public Safety' },
];

export default function DelegateScreen() {
  const [selectedTopic, setSelectedTopic] = useState(0);
  const [currentDelegate, setCurrentDelegate] = useState<string | null>(null);
  const [delegateInput, setDelegateInput] = useState('');
  const [loading, setLoading] = useState(false);
  const [checking, setChecking] = useState(true);
  const [myAddress, setMyAddress] = useState<string>('');

  const loadDelegation = useCallback(async (address: string, topicId: number) => {
    setChecking(true);
    try {
      const d = await getDelegation(address, topicId);
      setCurrentDelegate(d);
    } finally {
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    getSigningKeypair()
      .then(({ keypair }) => {
        const addr = keypair.address;
        setMyAddress(addr);
        loadDelegation(addr, selectedTopic);
      })
      .catch(() => setChecking(false));
  }, [loadDelegation, selectedTopic]);

  async function handleDelegate() {
    if (!delegateInput.trim()) {
      Alert.alert('Missing address', 'Enter the delegate address.');
      return;
    }
    setLoading(true);
    try {
      const { keypair } = await getSigningKeypair();
      await delegateVote(keypair, delegateInput.trim(), selectedTopic);
      Alert.alert('Delegated', `Votes for topic "${TOPICS[selectedTopic]?.label}" delegated.`);
      loadDelegation(keypair.address, selectedTopic);
      setDelegateInput('');
    } catch (e: any) {
      Alert.alert('Delegation failed', e.message);
    } finally {
      setLoading(false);
    }
  }

  async function handleRevoke() {
    setLoading(true);
    try {
      const { keypair } = await getSigningKeypair();
      await revokeDelegation(keypair, selectedTopic);
      Alert.alert('Revoked', 'Delegation revoked. Your votes are now direct.');
      setCurrentDelegate(null);
    } catch (e: any) {
      Alert.alert('Revoke failed', e.message);
    } finally {
      setLoading(false);
    }
  }

  return (
    <ScrollView style={s.container} contentContainerStyle={s.content}>
      <Text style={s.title}>Vote Delegation</Text>
      <Text style={s.sub}>
        Delegate your votes per topic to a trusted citizen. You can revoke at any time.
      </Text>

      <Text style={s.sectionLabel}>Topic</Text>
      <View style={s.topicRow}>
        {TOPICS.map((t) => (
          <TouchableOpacity
            key={t.id}
            style={[s.topicChip, selectedTopic === t.id && s.topicChipActive]}
            onPress={() => setSelectedTopic(t.id)}
          >
            <Text style={[s.topicChipText, selectedTopic === t.id && s.topicChipTextActive]}>
              {t.label}
            </Text>
          </TouchableOpacity>
        ))}
      </View>

      <View style={s.statusCard}>
        {checking ? (
          <ActivityIndicator color="#6C63FF" />
        ) : currentDelegate ? (
          <>
            <Text style={s.delegatedLabel}>Currently delegating to:</Text>
            <Text style={s.delegateAddress}>{currentDelegate}</Text>
            <TouchableOpacity style={s.revokeBtn} onPress={handleRevoke} disabled={loading}>
              {loading
                ? <ActivityIndicator color="#fff" size="small" />
                : <Text style={s.revokeBtnText}>Revoke delegation</Text>}
            </TouchableOpacity>
          </>
        ) : (
          <Text style={s.noDelegation}>Voting directly — no delegation set.</Text>
        )}
      </View>

      {!currentDelegate && (
        <>
          <Text style={s.sectionLabel}>Delegate to</Text>
          <TextInput
            style={s.input}
            value={delegateInput}
            onChangeText={setDelegateInput}
            placeholder="5G... (Substrate address)"
            placeholderTextColor="#4b5563"
            autoCapitalize="none"
            autoCorrect={false}
          />
          <TouchableOpacity style={s.delegateBtn} onPress={handleDelegate} disabled={loading}>
            {loading
              ? <ActivityIndicator color="#fff" />
              : <Text style={s.delegateBtnText}>Delegate votes</Text>}
          </TouchableOpacity>
        </>
      )}

      <View style={s.infoBox}>
        <Text style={s.infoText}>
          Delegation is transitive (your delegate may re-delegate) and revocable at any time.
          No single delegate may hold more than 33% of total votes.
        </Text>
      </View>
    </ScrollView>
  );
}

const s = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#0f1117' },
  content: { padding: 20 },
  title: { fontSize: 22, fontWeight: '700', color: '#ffffff', marginBottom: 6 },
  sub: { fontSize: 14, color: '#6b7280', marginBottom: 24, lineHeight: 20 },
  sectionLabel: { fontSize: 12, fontWeight: '600', color: '#9ca3af', textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 10 },
  topicRow: { flexDirection: 'row', flexWrap: 'wrap', gap: 8, marginBottom: 24 },
  topicChip: { paddingHorizontal: 14, paddingVertical: 8, borderRadius: 20, backgroundColor: '#161b27', borderWidth: 1, borderColor: '#1f2937' },
  topicChipActive: { backgroundColor: '#6C63FF', borderColor: '#6C63FF' },
  topicChipText: { fontSize: 13, color: '#9ca3af', fontWeight: '500' },
  topicChipTextActive: { color: '#ffffff', fontWeight: '700' },
  statusCard: { backgroundColor: '#161b27', borderRadius: 14, padding: 16, marginBottom: 24, borderWidth: 1, borderColor: '#1f2937', minHeight: 70, justifyContent: 'center' },
  delegatedLabel: { fontSize: 12, color: '#9ca3af', marginBottom: 4 },
  delegateAddress: { fontSize: 12, color: '#a78bfa', fontFamily: 'monospace', marginBottom: 14 },
  revokeBtn: { backgroundColor: '#7f1d1d', paddingVertical: 10, borderRadius: 8, alignItems: 'center' },
  revokeBtnText: { color: '#fca5a5', fontWeight: '600', fontSize: 13 },
  noDelegation: { fontSize: 14, color: '#6b7280' },
  input: { backgroundColor: '#161b27', borderWidth: 1, borderColor: '#1f2937', borderRadius: 12, padding: 14, color: '#ffffff', fontSize: 14, marginBottom: 14, fontFamily: 'monospace' },
  delegateBtn: { backgroundColor: '#6C63FF', paddingVertical: 14, borderRadius: 12, alignItems: 'center', marginBottom: 24 },
  delegateBtnText: { color: '#ffffff', fontWeight: '700', fontSize: 15 },
  infoBox: { backgroundColor: '#161b27', borderRadius: 12, padding: 14, borderWidth: 1, borderColor: '#1f2937' },
  infoText: { fontSize: 12, color: '#6b7280', lineHeight: 18 },
});
