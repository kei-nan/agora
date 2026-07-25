import React, { useCallback, useState } from 'react';
import {
  ActivityIndicator, Alert, ScrollView, StyleSheet,
  Text, TouchableOpacity, View,
} from 'react-native';
import { useFocusEffect } from '@react-navigation/native';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { RootStackParamList } from '../App';
import {
  fetchDelegateProfile, DelegateProfile,
  backDelegate, removeBackingFromDelegate, isBackingDelegate,
  delegateVote, revokeDelegation, fetchDelegateRegistry,
} from '../chain/governance';
import { getAllDelegations } from '../chain/citizenState';
import { getSigningKeypair } from '../chain/identity';

type Props = NativeStackScreenProps<RootStackParamList, 'DelegateDetail'>;

const TOPICS = [
  { id: 0, label: 'General' },
  { id: 1, label: 'Budget' },
  { id: 2, label: 'Constitutional' },
  { id: 3, label: 'Foreign Affairs' },
  { id: 4, label: 'Public Safety' },
];

function fmtExpiry(ts: number): string {
  return new Date(ts).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
}

export default function DelegateDetailScreen({ route }: Props) {
  const { address } = route.params;
  const [profile, setProfile] = useState<DelegateProfile | null>(null);
  const [isBacking, setIsBacking] = useState(false);
  const [topicDelegations, setTopicDelegations] = useState<Record<number, boolean>>({});
  // address of whoever currently holds each topic delegation (may be a different delegate)
  const [topicCurrentDelegate, setTopicCurrentDelegate] = useState<Record<number, string | null>>({});
  const [topicExpiry, setTopicExpiry] = useState<Record<number, number | null>>({});
  const [delegateNames, setDelegateNames] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(true);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [durationDays, setDurationDays] = useState(90);

  const DURATIONS = [
    { label: '30d',  days: 30 },
    { label: '90d',  days: 90 },
    { label: '6mo',  days: 180 },
    { label: '1yr',  days: 365 },
    { label: '2yr',  days: 730 },
  ];

  useFocusEffect(useCallback(() => {
    async function load() {
      const allDelegations = getAllDelegations();
      const [p, backing, registry] = await Promise.all([
        fetchDelegateProfile(address),
        isBackingDelegate('', address),
        fetchDelegateRegistry(),
      ]);
      setProfile(p);
      setIsBacking(backing);

      const names: Record<string, string> = {};
      registry.forEach(d => { names[d.address] = d.displayName; });
      setDelegateNames(names);

      const delegatedToThis: Record<number, boolean> = {};
      const currentMap: Record<number, string | null> = {};
      const expiries: Record<number, number | null> = {};
      TOPICS.forEach(t => {
        const entry = allDelegations.get(t.id);
        const current = entry?.delegate ?? null;
        delegatedToThis[t.id] = current === address;
        currentMap[t.id] = current;
        expiries[t.id] = current === address ? (entry?.expiresAt ?? null) : null;
      });
      setTopicDelegations(delegatedToThis);
      setTopicCurrentDelegate(currentMap);
      setTopicExpiry(expiries);
      setLoading(false);
    }
    load();
  }, [address]));

  async function toggleBacking() {
    setActionLoading('back');
    try {
      const { keypair } = await getSigningKeypair();
      if (isBacking) {
        await removeBackingFromDelegate(keypair, address);
        setIsBacking(false);
        setProfile(p => p ? { ...p, backingCount: p.backingCount - 1 } : p);
      } else {
        await backDelegate(keypair, address);
        setIsBacking(true);
        setProfile(p => p ? { ...p, backingCount: p.backingCount + 1 } : p);
      }
    } catch (e: any) {
      Alert.alert('Error', e.message);
    } finally {
      setActionLoading(null);
    }
  }

  async function toggleDelegation(topicId: number) {
    if (topicDelegations[topicId]) {
      // Revoking from this delegate — always allowed, even before expiry
      setActionLoading(`topic-${topicId}`);
      try {
        const { keypair } = await getSigningKeypair();
        await revokeDelegation(keypair, topicId);
        setTopicDelegations(prev => ({ ...prev, [topicId]: false }));
        setTopicCurrentDelegate(prev => ({ ...prev, [topicId]: null }));
        setTopicExpiry(prev => ({ ...prev, [topicId]: null }));
      } catch (e: any) {
        Alert.alert('Error', e.message);
      } finally {
        setActionLoading(null);
      }
      return;
    }

    const existing = topicCurrentDelegate[topicId];
    const topic = TOPICS.find(t => t.id === topicId)?.label ?? `Topic ${topicId}`;

    if (existing && existing !== address) {
      const existingName = delegateNames[existing] ?? existing.slice(0, 8) + '…';
      Alert.alert(
        'Switch delegation?',
        `Your ${topic} vote is currently delegated to ${existingName}. Delegating to this person will revoke that.`,
        [
          { text: 'Cancel', style: 'cancel' },
          { text: 'Switch', style: 'destructive', onPress: () => performDelegate(topicId) },
        ],
      );
    } else {
      performDelegate(topicId);
    }
  }

  async function performDelegate(topicId: number) {
    setActionLoading(`topic-${topicId}`);
    try {
      const { keypair } = await getSigningKeypair();
      await delegateVote(keypair, address, topicId, durationDays);
      const expiresAt = Date.now() + durationDays * 86_400_000;
      setTopicDelegations(prev => ({ ...prev, [topicId]: true }));
      setTopicCurrentDelegate(prev => ({ ...prev, [topicId]: address }));
      setTopicExpiry(prev => ({ ...prev, [topicId]: expiresAt }));
    } catch (e: any) {
      Alert.alert('Error', e.message);
    } finally {
      setActionLoading(null);
    }
  }

  if (loading) {
    return <View style={s.center}><ActivityIndicator color="#6C63FF" /></View>;
  }
  if (!profile) {
    return <View style={s.center}><Text style={s.emptyText}>Delegate not found.</Text></View>;
  }

  const statusColor = profile.status === 'Active' ? '#22c55e'
    : profile.status === 'Pending' ? '#f59e0b' : '#6b7280';
  const isActive = profile.status === 'Active';

  return (
    <ScrollView style={s.container} contentContainerStyle={s.content}>

      {/* Header */}
      <View style={s.profileCard}>
        <View style={s.profileHeader}>
          <Text style={s.name}>{profile.displayName}</Text>
          <View style={[s.statusBadge, { backgroundColor: statusColor + '22' }]}>
            <Text style={[s.statusText, { color: statusColor }]}>{profile.status}</Text>
          </View>
        </View>
        <Text style={s.address} numberOfLines={1}>{profile.address}</Text>
        <Text style={s.backingCount}>{profile.backingCount} backers</Text>

        {isActive && (
          <>
            {profile.warningEmitted && (
              <View style={s.warningBox}>
                <Text style={s.warningText}>⚠ Final term — ending soon. Re-delegate before term expires.</Text>
              </View>
            )}
            <View style={s.termRow}>
              <Text style={s.termLabel}>
                Term {profile.consecutiveTerms} of {profile.maxConsecutiveTerms}
              </Text>
              <Text style={s.termPct}>{profile.termProgressPct}%</Text>
            </View>
            <View style={s.progressBg}>
              <View style={[s.progressFill, {
                width: `${profile.termProgressPct}%` as any,
                backgroundColor: profile.warningEmitted ? '#f59e0b' : '#6C63FF',
              }]} />
            </View>
          </>
        )}

        {profile.status === 'OnBreak' && profile.breakEndsInBlocks !== undefined && (
          <Text style={s.breakText}>
            On mandatory break · returns in ~{Math.round(profile.breakEndsInBlocks / 7200)} days
          </Text>
        )}
      </View>

      {/* Backing */}
      <Text style={s.sectionLabel}>Your Backing</Text>
      <View style={s.card}>
        <View style={s.backingRow}>
          <View>
            <Text style={s.backingRowTitle}>
              {isBacking ? '✓ You are backing this delegate' : 'You are not backing this delegate'}
            </Text>
            <Text style={s.backingRowSub}>
              {isBacking
                ? 'You are endorsing this delegate — counts towards their 50-backer threshold'
                : 'Endorse them to help reach the 50-backer activation threshold'}
            </Text>
          </View>
          <TouchableOpacity
            style={[s.backingBtn, isBacking ? s.backingBtnActive : s.backingBtnInactive]}
            onPress={toggleBacking}
            disabled={actionLoading === 'back' || profile.status === 'OnBreak'}
          >
            {actionLoading === 'back'
              ? <ActivityIndicator size="small" color="#fff" />
              : <Text style={s.backingBtnText}>{isBacking ? 'Remove' : 'Back'}</Text>}
          </TouchableOpacity>
        </View>
      </View>

      {/* Vote delegation per topic */}
      <Text style={s.sectionLabel}>Delegate Your Vote</Text>

      {isActive && (
        <View style={s.durationRow}>
          <Text style={s.durationLabel}>Duration</Text>
          <View style={s.durationPicker}>
            {DURATIONS.map(d => (
              <TouchableOpacity
                key={d.days}
                style={[s.durationChip, durationDays === d.days && s.durationChipActive]}
                onPress={() => setDurationDays(d.days)}
              >
                <Text style={[s.durationChipText, durationDays === d.days && s.durationChipTextActive]}>
                  {d.label}
                </Text>
              </TouchableOpacity>
            ))}
          </View>
        </View>
      )}

      <View style={s.card}>
        {TOPICS.map((t, i) => {
          const isDelegated = topicDelegations[t.id] ?? false;
          const current = topicCurrentDelegate[t.id];
          const delegatedElsewhere = !isDelegated && !!current && current !== address;
          const isLoading = actionLoading === `topic-${t.id}`;
          const disabled = !isActive || isLoading;
          return (
            <View key={t.id} style={[s.topicRow, i < TOPICS.length - 1 && s.topicBorder]}>
              <View style={s.topicLeft}>
                <Text style={s.topicLabel}>{t.label}</Text>
                {isDelegated && topicExpiry[t.id] && (
                  <Text style={s.topicExpiry}>Until {fmtExpiry(topicExpiry[t.id]!)}</Text>
                )}
                {delegatedElsewhere && (
                  <Text style={s.topicElsewhere}>
                    → {delegateNames[current!] ?? current!.slice(0, 8) + '…'}
                  </Text>
                )}
              </View>
              <TouchableOpacity
                style={[
                  s.topicBtn,
                  isDelegated ? s.topicBtnOn : delegatedElsewhere ? s.topicBtnSwitch : s.topicBtnOff,
                  disabled && s.topicBtnDisabled,
                ]}
                onPress={() => toggleDelegation(t.id)}
                disabled={disabled}
              >
                {isLoading
                  ? <ActivityIndicator size="small" color="#fff" />
                  : <Text style={s.topicBtnText}>
                      {isDelegated ? 'Revoke' : delegatedElsewhere ? 'Switch' : 'Delegate'}
                    </Text>}
              </TouchableOpacity>
            </View>
          );
        })}
        {!isActive && (
          <Text style={s.inactiveNote}>
            {profile.status === 'Pending'
              ? 'This delegate is not yet active — they need more backers.'
              : 'This delegate is on a mandatory break and cannot receive delegations.'}
          </Text>
        )}
      </View>

      <View style={s.infoBox}>
        <Text style={s.infoText}>
          Backing and delegation are independent. Backing counts towards a delegate's eligibility
          threshold. Delegation transfers your vote on a specific topic. You can do either or both.
          No single delegate may hold more than 33% of total votes.
        </Text>
      </View>
    </ScrollView>
  );
}

const s = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#0f1117' },
  content: { padding: 20, paddingBottom: 40 },
  center: { flex: 1, backgroundColor: '#0f1117', alignItems: 'center', justifyContent: 'center' },
  emptyText: { color: '#6b7280' },
  sectionLabel: {
    fontSize: 12, fontWeight: '600', color: '#9ca3af',
    textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 10, marginTop: 4,
  },
  profileCard: {
    backgroundColor: '#161b27', borderRadius: 16, padding: 20,
    borderWidth: 1, borderColor: '#1f2937', marginBottom: 20,
  },
  profileHeader: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 6 },
  name: { fontSize: 20, fontWeight: '700', color: '#ffffff', flex: 1, marginRight: 10 },
  statusBadge: { paddingHorizontal: 8, paddingVertical: 3, borderRadius: 6 },
  statusText: { fontSize: 11, fontWeight: '600' },
  address: { fontSize: 11, color: '#4b5563', fontFamily: 'monospace', marginBottom: 8 },
  backingCount: { fontSize: 13, color: '#6b7280', marginBottom: 12 },
  warningBox: {
    backgroundColor: '#451a03', borderRadius: 8, padding: 10,
    marginBottom: 12, borderWidth: 1, borderColor: '#92400e',
  },
  warningText: { color: '#fcd34d', fontSize: 12, lineHeight: 17 },
  termRow: { flexDirection: 'row', justifyContent: 'space-between', marginBottom: 6 },
  termLabel: { fontSize: 12, color: '#9ca3af' },
  termPct: { fontSize: 12, color: '#9ca3af' },
  progressBg: { height: 6, backgroundColor: '#1f2937', borderRadius: 3 },
  progressFill: { height: 6, borderRadius: 3 },
  breakText: { fontSize: 13, color: '#6b7280', marginTop: 4 },
  card: {
    backgroundColor: '#161b27', borderRadius: 14,
    borderWidth: 1, borderColor: '#1f2937', marginBottom: 20, overflow: 'hidden',
  },
  backingRow: {
    flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between',
    padding: 16, gap: 12,
  },
  backingRowTitle: { fontSize: 14, fontWeight: '600', color: '#ffffff', marginBottom: 3 },
  backingRowSub: { fontSize: 12, color: '#6b7280', maxWidth: 200 },
  backingBtn: { paddingVertical: 8, paddingHorizontal: 16, borderRadius: 8, minWidth: 70, alignItems: 'center' },
  backingBtnActive: { backgroundColor: '#7f1d1d' },
  backingBtnInactive: { backgroundColor: '#6C63FF' },
  backingBtnText: { color: '#fff', fontWeight: '600', fontSize: 13 },
  topicRow: {
    flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between',
    paddingHorizontal: 16, paddingVertical: 12,
  },
  topicBorder: { borderBottomWidth: 1, borderBottomColor: '#1f2937' },
  topicLeft: { flex: 1, marginRight: 10 },
  topicLabel: { fontSize: 14, color: '#d1d5db', fontWeight: '500' },
  topicExpiry: { fontSize: 11, color: '#9ca3af', marginTop: 2 },
  topicElsewhere: { fontSize: 11, color: '#6b7280', marginTop: 2 },
  topicBtn: { paddingVertical: 6, paddingHorizontal: 14, borderRadius: 8, minWidth: 80, alignItems: 'center' },
  topicBtnOn: { backgroundColor: '#7f1d1d' },
  topicBtnOff: { backgroundColor: '#6C63FF' },
  topicBtnSwitch: { backgroundColor: '#92400e' },
  topicBtnDisabled: { opacity: 0.4 },
  topicBtnText: { color: '#fff', fontWeight: '600', fontSize: 13 },
  inactiveNote: { fontSize: 12, color: '#6b7280', padding: 14, paddingTop: 0, lineHeight: 17 },
  durationRow: { marginBottom: 10 },
  durationLabel: { fontSize: 12, fontWeight: '600', color: '#9ca3af', textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 8 },
  durationPicker: { flexDirection: 'row', gap: 8 },
  durationChip: {
    paddingHorizontal: 12, paddingVertical: 7, borderRadius: 8,
    backgroundColor: '#161b27', borderWidth: 1, borderColor: '#1f2937',
  },
  durationChipActive: { backgroundColor: '#6C63FF', borderColor: '#6C63FF' },
  durationChipText: { fontSize: 13, color: '#9ca3af', fontWeight: '500' },
  durationChipTextActive: { color: '#ffffff', fontWeight: '700' },
  infoBox: {
    backgroundColor: '#161b27', borderRadius: 12, padding: 14,
    borderWidth: 1, borderColor: '#1f2937',
  },
  infoText: { fontSize: 12, color: '#6b7280', lineHeight: 18 },
});
