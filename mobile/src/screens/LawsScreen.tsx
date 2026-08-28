import React, { useCallback, useEffect, useState } from 'react';
import {
  ActivityIndicator,
  FlatList,
  RefreshControl,
  StyleSheet,
  Text,
  View,
} from 'react-native';
import { useTranslation } from 'react-i18next';
import { Law, fetchLaws } from '../chain/governance';
import { useAppModal } from '../components/AppModal';
import IpfsContentBox from '../components/IpfsContentBox';
import { colors } from '../theme';

export default function LawsScreen() {
  const { t } = useTranslation('laws');
  const [laws, setLaws] = useState<Law[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const { showError } = useAppModal();

  const load = useCallback(async () => {
    try {
      setLaws(await fetchLaws());
    } catch (e: any) {
      // Without this, a chain-unreachable error left `laws` at its previous
      // value (empty on first load) with no indication anything went wrong —
      // indistinguishable from "no laws enacted yet."
      showError(t('loadError'), e);
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, [showError, t]);

  useEffect(() => { load(); }, [load]);

  if (loading) {
    return <View style={s.center}><ActivityIndicator color={colors.accent} /></View>;
  }

  return (
    <FlatList
      style={s.list}
      data={laws}
      keyExtractor={(l) => String(l.id)}
      refreshControl={
        <RefreshControl
          refreshing={refreshing}
          onRefresh={() => { setRefreshing(true); load(); }}
          tintColor={colors.accent}
        />
      }
      ListEmptyComponent={<Text style={s.empty}>{t('empty')}</Text>}
      renderItem={({ item }) => (
        <View
          style={s.card}
          accessible
          accessibilityRole="summary"
          accessibilityLabel={
            t('accessibilityLabel', {
              id: item.id,
              version: item.version,
              status: item.status,
              constitutionalSuffix: item.tier === 'Constitutional' ? `, ${t('constitutionalChip')}` : '',
              title: item.title,
            })
          }
        >
          <View style={s.chips}>
            <View style={[s.chip,
              item.status === 'Active' ? s.chipActive
              : item.status === 'Paused' ? s.chipPaused
              : s.chipRepealed
            ]}>
              <Text style={[s.chipText,
                item.status === 'Active' ? s.chipTextActive
                : item.status === 'Paused' ? s.chipTextPaused
                : s.chipTextRepealed
              ]}>{item.status}</Text>
            </View>
            {item.tier === 'Constitutional' && (
              <View style={[s.chip, s.chipConstitutional]}>
                <Text style={[s.chipText, s.chipTextConstitutional]}>{t('constitutionalChip')}</Text>
              </View>
            )}
            <Text style={s.meta}>{t('meta', { id: item.id, version: item.version })}</Text>
          </View>
          <Text style={s.title}>{item.title}</Text>
          <IpfsContentBox hashHex={item.contentHash} />
        </View>
      )}
    />
  );
}

const s = StyleSheet.create({
  list: { flex: 1, backgroundColor: colors.bg, padding: 16 },
  center: { flex: 1, backgroundColor: colors.bg, alignItems: 'center', justifyContent: 'center' },
  empty: { color: colors.textMuted, textAlign: 'center', marginTop: 40 },
  card: {
    backgroundColor: colors.card,
    borderRadius: 14,
    padding: 16,
    marginBottom: 10,
    borderWidth: 1,
    borderColor: colors.border,
  },
  chips: { flexDirection: 'row', alignItems: 'center', gap: 6, marginBottom: 10, flexWrap: 'wrap' },
  chip: { paddingHorizontal: 8, paddingVertical: 3, borderRadius: 6 },
  chipText: { fontSize: 11, fontWeight: '600' },
  chipActive: { backgroundColor: colors.successBg },
  chipTextActive: { color: colors.success },
  chipPaused: { backgroundColor: '#2a2a1a' },
  chipTextPaused: { color: colors.warning },
  chipRepealed: { backgroundColor: '#2d1515' },
  chipTextRepealed: { color: colors.danger },
  chipConstitutional: { backgroundColor: '#1e1040' },
  chipTextConstitutional: { color: '#a78bfa' },
  meta: { fontSize: 11, color: colors.textDim, marginLeft: 'auto' },
  title: { fontSize: 15, fontWeight: '600', color: colors.textPrimary, marginBottom: 8, lineHeight: 21 },
});
