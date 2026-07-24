import React, { useEffect, useState } from 'react';
import {
  ActivityIndicator,
  StyleSheet,
  Text,
  TouchableOpacity,
  View,
} from 'react-native';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { RootStackParamList } from '../App';
import { isCitizen } from '../chain/identity';
import { getApi } from '../chain/api';

type Props = NativeStackScreenProps<RootStackParamList, 'Main'>;

interface ChainStats {
  blockNumber: number;
  totalCitizens: number;
  activeProposals: number;
  activeLaws: number;
}

export default function HomeScreen({ navigation }: Props) {
  const [citizenStatus, setCitizenStatus] = useState<'checking' | 'registered' | 'unregistered'>('checking');
  const [stats, setStats] = useState<ChainStats | null>(null);

  useEffect(() => {
    // Check citizenship with a stub address — in production this reads from the persisted keypair
    isCitizen('5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY')
      .then((is) => setCitizenStatus(is ? 'registered' : 'unregistered'))
      .catch(() => setCitizenStatus('unregistered'));

    getApi().then(async (api) => {
      const [blockHash, citizenCount] = await Promise.all([
        api.rpc.chain.getHeader(),
        api.query.identity.totalCitizens(),
      ]);
      const proposals = await api.query.voting.referenda.entries();
      const laws = await api.query.constitution.laws.entries();
      setStats({
        blockNumber: (blockHash as any).number.toNumber(),
        totalCitizens: (citizenCount as any).toNumber(),
        activeProposals: proposals.length,
        activeLaws: laws.length,
      });
    }).catch(() => {});
  }, []);

  return (
    <View style={s.container}>
      <Text style={s.headline}>Agora</Text>
      <Text style={s.tagline}>Distributed democracy on-chain</Text>

      <View style={s.statusCard}>
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
      </View>

      {stats && (
        <View style={s.statsRow}>
          <Stat label="Block" value={`#${stats.blockNumber.toLocaleString()}`} />
          <Stat label="Citizens" value={stats.totalCitizens.toString()} />
          <Stat label="Proposals" value={stats.activeProposals.toString()} />
          <Stat label="Laws" value={stats.activeLaws.toString()} />
        </View>
      )}

      <Text style={s.sectionTitle}>Quick access</Text>
      <View style={s.quickGrid}>
        <QuickCard emoji="🗳" label="Vote on proposals" onPress={() => {}} />
        <QuickCard emoji="📜" label="Browse laws" onPress={() => {}} />
        <QuickCard emoji="✍" label="Sign petitions" onPress={() => {}} />
        <QuickCard emoji="💻" label="Sign in to desktop" onPress={() => navigation.navigate('Auth', {})} />
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

function QuickCard({ emoji, label, onPress }: { emoji: string; label: string; onPress: () => void }) {
  return (
    <TouchableOpacity style={s.card} onPress={onPress}>
      <Text style={s.cardEmoji}>{emoji}</Text>
      <Text style={s.cardLabel}>{label}</Text>
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
});
