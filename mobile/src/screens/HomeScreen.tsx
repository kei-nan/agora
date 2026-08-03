import React, { useState, useCallback } from 'react';
import {
  ActivityIndicator,
  StyleSheet,
  Text,
  TouchableOpacity,
  View,
} from 'react-native';
import { NativeStackNavigationProp } from '@react-navigation/native-stack';
import { useFocusEffect, useNavigation } from '@react-navigation/native';
import { RootStackParamList } from '../App';
import { getRegistered, setRegistered } from '../chain/citizenState';
import { getSigningKeypair, isCitizen } from '../chain/identity';
import { useAppModal } from '../components/AppModal';
import { colors } from '../theme';

// HomeScreen is registered as a Tab.Screen (see App.tsx's MainTabs), not a
// Stack.Screen, so — matching the pattern already used in DelegateScreen —
// its navigation is obtained via useNavigation() rather than a typed
// `navigation` prop. It still needs RootStackParamList (not just
// TabParamList) because it navigates to stack-only routes like 'Register'
// and 'Auth'.
type Nav = NativeStackNavigationProp<RootStackParamList>;

interface ChainStats {
  blockNumber: number;
  totalCitizens: number;
  activeProposals: number;
}

export default function HomeScreen() {
  const navigation = useNavigation<Nav>();
  const [citizenStatus, setCitizenStatus] = useState<'checking' | 'registered' | 'unregistered'>('checking');
  const [stats, setStats] = useState<ChainStats | null>(null);
  const { showConfirm } = useAppModal();

  useFocusEffect(useCallback(() => {
    // citizenState.ts's cache is a UI-convenience mirror, not the source of
    // truth — it's an in-memory singleton with nothing to keep it in sync if
    // the JS engine gets reclaimed while backgrounded (common on Android
    // under memory pressure) or the app restarts. Show the cached value
    // immediately so the screen isn't blank while the chain query is in
    // flight, then reconcile against the real on-chain state and update the
    // cache to match. If the chain is unreachable, silently keep whatever
    // was already shown rather than flashing an error on every home-screen
    // visit — this mirrors the desktop app's "degrades gracefully offline"
    // behavior (CLAUDE.md).
    let cancelled = false;
    setCitizenStatus(getRegistered() ? 'registered' : 'unregistered');
    (async () => {
      try {
        const { keypair } = await getSigningKeypair();
        const registered = await isCitizen(keypair.address);
        if (cancelled) return;
        setRegistered(registered);
        setCitizenStatus(registered ? 'registered' : 'unregistered');
      } catch {
        // Chain unreachable or not yet connected — leave the cached value shown above.
      }
    })();
    return () => {
      cancelled = true;
    };
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
          showConfirm({
            title: 'Reset registration?',
            message: 'This will clear your citizen status for this session.',
            confirmLabel: 'Reset',
            destructive: true,
            onConfirm: () => { setRegistered(false); setCitizenStatus('unregistered'); },
          });
        }}
      >
        {citizenStatus === 'checking' ? (
          <ActivityIndicator color={colors.accent} />
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
              accessibilityRole="button"
              accessibilityLabel="Register now"
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
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled }}
    >
      <Text style={[s.cardEmoji, disabled && s.cardEmojiDisabled]}>{emoji}</Text>
      <Text style={[s.cardLabel, disabled && s.cardLabelDisabled]}>{label}</Text>
      {disabled && <Text style={s.cardLock}>🔒</Text>}
    </TouchableOpacity>
  );
}

const s = StyleSheet.create({
  container: { flex: 1, backgroundColor: colors.bg, padding: 20 },
  headline: { fontSize: 32, fontWeight: '800', color: colors.textPrimary, marginTop: 12 },
  tagline: { fontSize: 14, color: colors.textMuted, marginBottom: 24 },
  statusCard: {
    backgroundColor: colors.card,
    borderRadius: 16,
    padding: 20,
    marginBottom: 20,
    alignItems: 'flex-start',
    borderWidth: 1,
    borderColor: colors.border,
  },
  statusBadge: { fontSize: 16, fontWeight: '700', color: colors.success, marginBottom: 4 },
  notRegistered: { color: colors.warning },
  statusSub: { fontSize: 13, color: colors.textMuted },
  registerBtn: {
    marginTop: 14,
    backgroundColor: colors.accent,
    paddingVertical: 10,
    paddingHorizontal: 24,
    borderRadius: 10,
  },
  registerBtnText: { color: colors.textPrimary, fontWeight: '600', fontSize: 14 },
  statsRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    marginBottom: 24,
  },
  stat: { alignItems: 'center', flex: 1 },
  statValue: { fontSize: 18, fontWeight: '700', color: colors.textPrimary },
  statLabel: { fontSize: 11, color: colors.textMuted, marginTop: 2 },
  sectionTitle: { fontSize: 14, fontWeight: '600', color: colors.textSecondary, marginBottom: 12, textTransform: 'uppercase', letterSpacing: 0.8 },
  quickGrid: { flexDirection: 'row', flexWrap: 'wrap', gap: 12 },
  card: {
    width: '47%',
    backgroundColor: colors.card,
    borderRadius: 14,
    padding: 18,
    borderWidth: 1,
    borderColor: colors.border,
  },
  cardEmoji: { fontSize: 28, marginBottom: 8 },
  cardLabel: { fontSize: 13, fontWeight: '600', color: colors.textBody },
  cardDisabled: { opacity: 0.4 },
  cardEmojiDisabled: { opacity: 0.6 },
  cardLabelDisabled: { color: colors.textMuted },
  cardLock: { fontSize: 10, position: 'absolute', top: 8, right: 8 },
});
