import React, { useCallback, useState } from 'react';
import {
  FlatList, StyleSheet, Text,
  TextInput, TouchableOpacity, View,
} from 'react-native';
import { useFocusEffect, useNavigation } from '@react-navigation/native';
import { NativeStackNavigationProp } from '@react-navigation/native-stack';
import { RootStackParamList } from '../App';
import { fetchDelegateRegistry, DelegateProfile } from '../chain/governance';
import { getAllDelegations, getRegistered, DelegationEntry } from '../chain/citizenState';
import { useAppModal } from '../components/AppModal';
import { colors } from '../theme';

const TOPICS = ['General', 'Budget', 'Constitutional', 'Foreign Affairs', 'Public Safety'];
type StatusFilter = 'All' | 'Active' | 'Pending' | 'OnBreak';
type Nav = NativeStackNavigationProp<RootStackParamList>;

const HELP = {
  myDelegations:
    'When you delegate a topic, your vote on that subject is transferred to a representative of your choice. They vote on your behalf until the delegation expires or you revoke it. You can delegate each topic to a different person.',
  registry:
    'Delegates are citizens who have publicly registered with their verified passport name. They represent other citizens on specific topics. You choose which topics — and for how long — you trust each delegate.',
  backing:
    'Backing endorses a delegate as trustworthy. A delegate needs at least 50 backers to become Active and eligible to receive vote delegations. Backing is separate from delegation — you can back without delegating. Backing is proven with a zero-knowledge proof that reveals only "some citizen in good standing backs this delegate" — not which citizen. Nobody, including the delegate, can query which citizens back them. This protects the content of your backing, not the fact that your account submitted some transaction at some time — that is still ordinary, publicly analyzable chain data, the same residual gap every identity-gated action in this app has.',
  termLimit:
    'To prevent permanent concentration of power, delegates serve a maximum number of consecutive terms before a mandatory break. The progress bar shows how far through their current term they are.',
  becomeDelegate:
    'Registering as a delegate now uses a genuinely separate on-chain identity — a second account, proven by a dedicated zero-knowledge circuit to belong to a citizen in good standing, without revealing which one. Your delegate persona\'s on-chain activity is not linkable back to your personal citizen account by the cryptography itself. This does not hide your chosen display name (whatever you type is public), and it does not prevent ordinary chain-analysis clues — e.g. funding the new persona account from your known citizen account. You start as Pending and need 50 backers to become Active.',
};

function HelpIcon({ title, message }: { title: string; message: string }) {
  const { showInfo } = useAppModal();
  return (
    <TouchableOpacity
      onPress={() => showInfo(title, message)}
      hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
      style={s.helpBtn}
      accessibilityRole="button"
      accessibilityLabel={`Help: ${title}`}
    >
      <Text style={s.helpIcon}>?</Text>
    </TouchableOpacity>
  );
}

export default function DelegateScreen() {
  const navigation = useNavigation<Nav>();
  const [delegates, setDelegates] = useState<DelegateProfile[]>([]);
  const [delegations, setDelegations] = useState<Map<number, DelegationEntry>>(new Map());
  const [loading, setLoading] = useState(true);
  const [searchQuery, setSearchQuery] = useState('');
  const [statusFilter, setStatusFilter] = useState<StatusFilter>('All');
  const isRegistered = getRegistered();
  const { showError } = useAppModal();

  useFocusEffect(useCallback(() => {
    fetchDelegateRegistry()
      .then(d => setDelegates(d))
      .catch((e: any) => showError("Couldn't load delegates", e))
      .finally(() => setLoading(false));
    setDelegations(getAllDelegations());
  }, [showError]));

  const activeDelegations = Array.from(delegations.entries());

  const warningDelegates = delegates.filter(
    d => d.warningEmitted && activeDelegations.some(([, entry]) => entry.delegate === d.address),
  );

  const filteredDelegates = delegates.filter(d => {
    const matchesSearch = d.displayName.toLowerCase().includes(searchQuery.toLowerCase());
    const matchesStatus = statusFilter === 'All' || d.status === statusFilter;
    return matchesSearch && matchesStatus;
  });

  const STATUS_FILTERS: { label: string; value: StatusFilter }[] = [
    { label: 'All', value: 'All' },
    { label: 'Active', value: 'Active' },
    { label: 'Pending', value: 'Pending' },
    { label: 'On Break', value: 'OnBreak' },
  ];

  return (
    <FlatList
      style={s.list}
      data={filteredDelegates}
      keyExtractor={d => d.address}
      refreshing={loading}
      onRefresh={() => {
        setLoading(true);
        fetchDelegateRegistry()
          .then(d => setDelegates(d))
          .catch((e: any) => showError("Couldn't load delegates", e))
          .finally(() => setLoading(false));
      }}
      ListHeaderComponent={
        <View>
          {/* Term warning banner */}
          {warningDelegates.map(d => (
            <View key={d.address} style={s.warningBanner}>
              <Text style={s.warningText}>
                ⚠ {d.displayName}'s term is ending soon — consider re-delegating
              </Text>
            </View>
          ))}

          {/* My Delegations */}
          <View style={s.section}>
            <View style={s.sectionHeading}>
              <Text style={s.sectionLabel}>My Delegations</Text>
              <HelpIcon title="Vote Delegation" message={HELP.myDelegations} />
            </View>
            {activeDelegations.length === 0 ? (
              <View style={s.card}>
                <Text style={s.emptyText}>You are voting directly on all topics.</Text>
              </View>
            ) : (
              <View style={s.card}>
                {activeDelegations.map(([topicId, entry]) => {
                  const { delegate: addr, expiresAt } = entry;
                  const profile = delegates.find(d => d.address === addr);
                  const expiryStr = new Date(expiresAt).toLocaleDateString('en-US', {
                    month: 'short', day: 'numeric', year: 'numeric',
                  });
                  return (
                    <TouchableOpacity
                      key={topicId}
                      style={s.delegationRow}
                      onPress={() => navigation.navigate('DelegateDetail', { address: addr })}
                      accessibilityRole="button"
                      accessibilityLabel={`${TOPICS[topicId] ?? `Topic ${topicId}`} delegated to ${profile?.displayName ?? 'delegate'}, until ${expiryStr}`}
                    >
                      <View>
                        <Text style={s.delegationTopic}>{TOPICS[topicId] ?? `Topic ${topicId}`}</Text>
                        <Text style={s.delegationExpiry}>Until {expiryStr}</Text>
                      </View>
                      <View style={s.delegationRight}>
                        <Text style={s.delegationName}>
                          {profile?.displayName ?? addr.slice(0, 8) + '…'}
                        </Text>
                        {profile?.warningEmitted && <Text style={s.warningDot}>⚠</Text>}
                        <Text style={s.chevron}>›</Text>
                      </View>
                    </TouchableOpacity>
                  );
                })}
              </View>
            )}
          </View>

          {/* Become a Delegate */}
          {isRegistered && (
            <View style={s.becomeDelegateRow}>
              <TouchableOpacity
                style={s.becomeBtn}
                onPress={() => navigation.navigate('RegisterDelegate')}
                accessibilityRole="button"
                accessibilityLabel="Become a delegate"
              >
                <Text style={s.becomeBtnText}>+ Become a delegate</Text>
              </TouchableOpacity>
              <HelpIcon title="Becoming a Delegate" message={HELP.becomeDelegate} />
            </View>
          )}

          {/* Registry header with search */}
          <View style={s.sectionHeading}>
            <Text style={s.sectionLabel}>Delegate Registry</Text>
            <HelpIcon title="Delegate Registry" message={HELP.registry} />
          </View>

          <View style={s.searchRow}>
            <TextInput
              style={s.searchInput}
              value={searchQuery}
              onChangeText={setSearchQuery}
              placeholder="Search by name…"
              placeholderTextColor={colors.textDim}
              clearButtonMode="while-editing"
              accessibilityLabel="Search delegates by name"
            />
          </View>

          <View style={s.filterRow}>
            {STATUS_FILTERS.map(f => (
              <TouchableOpacity
                key={f.value}
                style={[s.filterChip, statusFilter === f.value && s.filterChipActive]}
                onPress={() => setStatusFilter(f.value)}
                accessibilityRole="button"
                accessibilityLabel={`Filter: ${f.label}`}
                accessibilityState={{ selected: statusFilter === f.value }}
              >
                <Text style={[s.filterChipText, statusFilter === f.value && s.filterChipTextActive]}>
                  {f.label}
                </Text>
              </TouchableOpacity>
            ))}
          </View>
        </View>
      }
      ListEmptyComponent={
        loading ? null :
        <Text style={s.emptyText}>
          {searchQuery || statusFilter !== 'All' ? 'No delegates match your search.' : 'No delegates registered yet.'}
        </Text>
      }
      renderItem={({ item }) => (
        <DelegateRow
          delegate={item}
          onPress={() => navigation.navigate('DelegateDetail', { address: item.address })}
        />
      )}
      contentContainerStyle={s.content}
    />
  );
}

function DelegateRow({ delegate: d, onPress }: { delegate: DelegateProfile; onPress: () => void }) {
  const statusColor = d.status === 'Active' ? colors.success : d.status === 'Pending' ? colors.warning : colors.textMuted;
  return (
    <TouchableOpacity
      style={s.delegateCard}
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={`${d.displayName}, ${d.status}, ${d.backingCount} backers`}
    >
      <View style={s.delegateHeader}>
        <View style={s.delegateNameRow}>
          <Text style={s.delegateName}>{d.displayName}</Text>
          {d.warningEmitted && <Text style={s.warningDotSmall}>⚠</Text>}
        </View>
        <View style={[s.statusBadge, { backgroundColor: statusColor + '22' }]}>
          <Text style={[s.statusText, { color: statusColor }]}>{d.status === 'OnBreak' ? 'On Break' : d.status}</Text>
        </View>
      </View>

      <View style={s.delegateMeta}>
        <View style={s.metaItem}>
          <Text style={s.backingText}>{d.backingCount} backers</Text>
          <HelpIcon title="Backing" message={HELP.backing} />
        </View>
        {d.status === 'Active' && (
          <View style={s.metaItem}>
            <Text style={s.termText}>Term {d.consecutiveTerms}/{d.maxConsecutiveTerms}</Text>
            <HelpIcon title="Term Limits" message={HELP.termLimit} />
          </View>
        )}
        {d.status === 'OnBreak' && d.breakEndsInBlocks !== undefined && (
          <Text style={s.breakText}>Break: {Math.round(d.breakEndsInBlocks / 7200)}d remaining</Text>
        )}
      </View>

      {d.status === 'Active' && (
        <View style={s.progressBg}>
          <View style={[s.progressFill, { width: `${d.termProgressPct}%` as any,
            backgroundColor: d.warningEmitted ? colors.warning : colors.accent }]} />
        </View>
      )}

      {d.status === 'Pending' && (
        <Text style={s.pendingHint}>
          Needs {Math.max(0, 50 - d.backingCount)} more backers to activate
        </Text>
      )}

      <Text style={s.chevronRight}>›</Text>
    </TouchableOpacity>
  );
}

const s = StyleSheet.create({
  list: { flex: 1, backgroundColor: colors.bg },
  content: { padding: 16, paddingBottom: 32 },

  sectionHeading: { flexDirection: 'row', alignItems: 'center', gap: 6, marginBottom: 10, marginTop: 4 },
  sectionLabel: {
    fontSize: 12, fontWeight: '600', color: colors.textSecondary,
    textTransform: 'uppercase', letterSpacing: 0.8,
  },
  helpBtn: {
    width: 20, height: 20, borderRadius: 10,
    backgroundColor: '#1e1b4b', borderWidth: 1, borderColor: colors.accent,
    alignItems: 'center', justifyContent: 'center',
  },
  helpIcon: { fontSize: 12, color: '#a5b4fc', fontWeight: '700', lineHeight: 14 },

  section: { marginBottom: 16 },
  card: {
    backgroundColor: colors.card, borderRadius: 14,
    borderWidth: 1, borderColor: colors.border, overflow: 'hidden',
  },
  emptyText: { color: colors.textMuted, padding: 16, textAlign: 'center' },

  delegationRow: {
    flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center',
    paddingHorizontal: 16, paddingVertical: 12,
    borderBottomWidth: 1, borderBottomColor: colors.border,
  },
  delegationTopic: { fontSize: 14, color: colors.textSecondary, fontWeight: '500' },
  delegationExpiry: { fontSize: 11, color: colors.textDim, marginTop: 2 },
  delegationRight: { flexDirection: 'row', alignItems: 'center', gap: 6 },
  delegationName: { fontSize: 14, fontWeight: '600', color: colors.textPrimary },
  warningDot: { fontSize: 14, color: colors.warning },
  warningDotSmall: { fontSize: 12, color: colors.warning },
  chevron: { fontSize: 18, color: colors.textMuted, marginLeft: 4 },

  warningBanner: {
    backgroundColor: colors.warningBg, borderRadius: 10, padding: 12,
    marginBottom: 12, borderWidth: 1, borderColor: colors.warningBorder,
  },
  warningText: { color: colors.warningTextStrong, fontSize: 13, lineHeight: 18 },

  becomeDelegateRow: { flexDirection: 'row', alignItems: 'center', gap: 10, marginBottom: 20 },
  becomeBtn: {
    flex: 1, borderWidth: 1, borderColor: colors.accent, borderRadius: 12,
    paddingVertical: 12, alignItems: 'center',
  },
  becomeBtnText: { color: colors.accent, fontWeight: '600', fontSize: 14 },

  searchRow: { marginBottom: 10 },
  searchInput: {
    backgroundColor: colors.card, borderWidth: 1, borderColor: colors.border,
    borderRadius: 12, paddingHorizontal: 14, paddingVertical: 10,
    color: colors.textPrimary, fontSize: 14,
  },
  filterRow: { flexDirection: 'row', gap: 8, marginBottom: 14, flexWrap: 'wrap' },
  filterChip: {
    paddingHorizontal: 12, paddingVertical: 6, borderRadius: 8,
    backgroundColor: colors.card, borderWidth: 1, borderColor: colors.border,
  },
  filterChipActive: { backgroundColor: colors.accent, borderColor: colors.accent },
  filterChipText: { fontSize: 13, color: colors.textMuted, fontWeight: '500' },
  filterChipTextActive: { color: colors.textPrimary, fontWeight: '600' },

  delegateCard: {
    backgroundColor: colors.card, borderRadius: 14, padding: 16,
    marginBottom: 10, borderWidth: 1, borderColor: colors.border,
  },
  delegateHeader: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', marginBottom: 6 },
  delegateNameRow: { flexDirection: 'row', alignItems: 'center', gap: 6 },
  delegateName: { fontSize: 15, fontWeight: '700', color: colors.textPrimary },
  statusBadge: { paddingHorizontal: 8, paddingVertical: 3, borderRadius: 6 },
  statusText: { fontSize: 11, fontWeight: '600' },
  delegateMeta: { flexDirection: 'row', gap: 16, marginBottom: 8, flexWrap: 'wrap' },
  metaItem: { flexDirection: 'row', alignItems: 'center', gap: 4 },
  backingText: { fontSize: 12, color: colors.textMuted },
  termText: { fontSize: 12, color: colors.textMuted },
  breakText: { fontSize: 12, color: colors.textMuted },
  pendingHint: { fontSize: 12, color: colors.textMuted, marginTop: 2 },
  progressBg: { height: 4, backgroundColor: colors.border, borderRadius: 2, marginBottom: 4 },
  progressFill: { height: 4, borderRadius: 2 },
  chevronRight: { position: 'absolute', right: 16, top: '50%', fontSize: 20, color: colors.textDim },
});
