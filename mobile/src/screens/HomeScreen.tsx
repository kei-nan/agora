import React, { useState, useCallback } from 'react';
import {
  ActivityIndicator,
  Alert,
  StyleSheet,
  Text,
  TouchableOpacity,
  View,
} from 'react-native';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { useFocusEffect } from '@react-navigation/native';
import { RootStackParamList } from '../App';
import { getRegistered, setRegistered } from '../chain/citizenState';
// Chain imports stubbed until @polkadot/api polyfills are set up for React Native
// import { isCitizen } from '../chain/identity';
// import { getApi } from '../chain/api';

type Props = NativeStackScreenProps<RootStackParamList, 'Main'>;

interface ChainStats {
  blockNumber: number;
  totalCitizens: number;
  activeProposals: number;
}

export default function HomeScreen({ navigation }: Props) {
  const [citizenStatus, setCitizenStatus] = useState<'checking' | 'registered' | 'unregistered'>('checking');
  const [stats, setStats] = useState<ChainStats | null>(null);

  useFocusEffect(useCallback(() => {
    setCitizenStatus(getRegistered() ? 'registered' : 'unregistered');
  }, []));

  return (
    <View style={s.container}>
      <Text style={s.headline}>Agora</Text>
      <Text style={s.tagline}>Distributed democracy on-chain</Text>

      <TouchableOpacity
        style={s.statusCard}
        activeOpacity={1}
        onLongPress={() => {
          if (citizenStatus !== 'registered') return;
          Alert.alert('Reset registration?', 'This will clear your citizen status for this session.', [
            { text: 'Cancel', style: 'cancel' },
            { text: 'Reset', style: 'destructive', onPress: () => { setRegistered(false); setCitizenStatus('unregistered'); } },
          ]);
        }}
      >
        {citizenStatus === 'checking' ? (
          <ActivityIndicator color="#6C63FF" />
        ) : citizenStatus === 'registered' ? (
          <>
            <Text style={s.statusBadge}>✓ Registered citizen</Text>
            <Text style={s.statusSub}>Your passport identity is active on-chain</Text>
          </>
        ) : (
          <>
            <Text style={[s.statusBadge, s.notRegistered]}>Not registered</Text>
            <Text style={s.statusSub}>Register your passport to participate</Text>
            <TouchableOpacity
              style={s.registerBtn}
              onPress={() => navigation.navigate('Register')}
            >
              <Text style={s.registerBtnText}>Register now</Text>
            </TouchableOpacity>
          </>
        )}
      </TouchableOpacity>

      {stats && (
        <View style={s.statsRow}>
          <Stat label="Block" value={`#${stats.blockNumber.toLocaleString()}`} />
          <Stat label="Citizens" value={stats.totalCitizens.toString()} />
          <Stat label="Proposals" value={stats.activeProposals.toString()} />
        </View>
      )}

      <Text style={s.sectionTitle}>Quick access</Text>
      <View style={s.quickGrid}>
        <QuickCard emoji="🗳" label="Vote on proposals" onPress={() => (navigation as any).navigate('Proposals')} disabled={citizenStatus !== 'registered'} />
        <QuickCard emoji="📝" label="Sign petitions" onPress={() => (navigation as any).navigate('Petitions')} disabled={citizenStatus !== 'registered'} />
        <QuickCard emoji="🤝" label="Manage delegation" onPress={() => (navigation as any).navigate('Delegate')} disabled={citizenStatus !== 'registered'} />
        <QuickCard emoji="💻" label="Sign in to desktop" onPress={() => navigation.navigate('Auth', {})} disabled={citizenStatus !== 'registered'} />
      </View>
    </View>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <View style={s.stat}>
      <Text style={s.statValue}>{value}</Text>
      <Text style={s.statLabel}>{label}</Text>
    </View>
  );
}

function QuickCard({ emoji, label, onPress, disabled }: { emoji: string; label: string; onPress: () => void; disabled?: boolean }) {
  return (
    <TouchableOpacity
      style={[s.card, disabled && s.cardDisabled]}
      onPress={disabled ? undefined : onPress}
      activeOpacity={disabled ? 1 : 0.7}
    >
      <Text style={[s.cardEmoji, disabled && s.cardEmojiDisabled]}>{emoji}</Text>
      <Text style={[s.cardLabel, disabled && s.cardLabelDisabled]}>{label}</Text>
      {disabled && <Text style={s.cardLock}>🔒</Text>}
    </TouchableOpacity>
  );
}

const s = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#0f1117', padding: 20 },
  headline: { fontSize: 32, fontWeight: '800', color: '#ffffff', marginTop: 12 },
  tagline: { fontSize: 14, color: '#6b7280', marginBottom: 24 },
  statusCard: {
    backgroundColor: '#161b27',
    borderRadius: 16,
    padding: 20,
    marginBottom: 20,
    alignItems: 'flex-start',
    borderWidth: 1,
    borderColor: '#1f2937',
  },
  statusBadge: { fontSize: 16, fontWeight: '700', color: '#22c55e', marginBottom: 4 },
  notRegistered: { color: '#f59e0b' },
  statusSub: { fontSize: 13, color: '#6b7280' },
  registerBtn: {
    marginTop: 14,
    backgroundColor: '#6C63FF',
    paddingVertical: 10,
    paddingHorizontal: 24,
    borderRadius: 10,
  },
  registerBtnText: { color: '#ffffff', fontWeight: '600', fontSize: 14 },
  statsRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    marginBottom: 24,
  },
  stat: { alignItems: 'center', flex: 1 },
  statValue: { fontSize: 18, fontWeight: '700', color: '#ffffff' },
  statLabel: { fontSize: 11, color: '#6b7280', marginTop: 2 },
  sectionTitle: { fontSize: 14, fontWeight: '600', color: '#9ca3af', marginBottom: 12, textTransform: 'uppercase', letterSpacing: 0.8 },
  quickGrid: { flexDirection: 'row', flexWrap: 'wrap', gap: 12 },
  card: {
    width: '47%',
    backgroundColor: '#161b27',
    borderRadius: 14,
    padding: 18,
    borderWidth: 1,
    borderColor: '#1f2937',
  },
  cardEmoji: { fontSize: 28, marginBottom: 8 },
  cardLabel: { fontSize: 13, fontWeight: '600', color: '#d1d5db' },
  cardDisabled: { opacity: 0.4 },
  cardEmojiDisabled: { opacity: 0.6 },
  cardLabelDisabled: { color: '#6b7280' },
  cardLock: { fontSize: 10, position: 'absolute', top: 8, right: 8 },
});
