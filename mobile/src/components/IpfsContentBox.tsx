/**
 * Resolves a law/proposal/petition/ruling's on-chain content hash to its real IPFS text
 * and displays it, in place of showing the raw hex hash to a citizen — the mobile side of
 * the gap desktop already closed via `fetch_ipfs_content` (see `docs/project/changelog/092.md`'s
 * "Citizen/UX findings": "Mobile shows raw hex (topicHash) instead of proposal/law/petition
 * content — mobile never fetches IPFS content anywhere, unlike desktop").
 *
 * Content is fetched lazily (tap to load, mirroring how ProposalsScreen/PetitionScreen
 * already gate other actions behind an explicit tap) rather than eagerly for every row on
 * screen — a proposals/laws list can have many items, and eagerly firing a gateway fetch
 * per row on every render would be wasteful and would hammer the public gateway for
 * content the citizen may never actually read.
 *
 * On failure this shows an honest, visible error — never a silent fallback that makes
 * unverified or fabricated content look like the real thing. The raw hash stays visible
 * alongside the error so the citizen can still reference/verify it themselves.
 */
import React, { useState } from 'react';
import { ActivityIndicator, StyleSheet, Text, TouchableOpacity, View } from 'react-native';
import { fetchIpfsContent } from '../chain/ipfs';
import { colors } from '../theme';

type LoadState =
  | { status: 'collapsed' }
  | { status: 'loading' }
  | { status: 'loaded'; text: string }
  | { status: 'error'; message: string };

interface Props {
  /** On-chain content hash, hex-encoded (with or without `0x`) — e.g. `item.topicHash` / `item.contentHash`. */
  hashHex: string;
}

export default function IpfsContentBox({ hashHex }: Props) {
  const [state, setState] = useState<LoadState>({ status: 'collapsed' });

  async function load() {
    setState({ status: 'loading' });
    try {
      const text = await fetchIpfsContent(hashHex);
      setState({ status: 'loaded', text });
    } catch (e: any) {
      setState({ status: 'error', message: e?.message ?? String(e) });
    }
  }

  if (state.status === 'collapsed') {
    return (
      <TouchableOpacity
        onPress={load}
        accessibilityRole="button"
        accessibilityLabel="Load full content from IPFS"
      >
        <Text style={s.hash} numberOfLines={1}>{hashHex}</Text>
        <Text style={s.link}>Tap to load full content</Text>
      </TouchableOpacity>
    );
  }

  if (state.status === 'loading') {
    return (
      <View style={s.loadingRow} accessible accessibilityLabel="Loading content">
        <ActivityIndicator size="small" color={colors.accent} />
        <Text style={s.loadingText}>Loading content…</Text>
      </View>
    );
  }

  if (state.status === 'error') {
    return (
      <View>
        <Text style={s.error} accessibilityRole="alert">
          Couldn't load content: {state.message}
        </Text>
        <Text style={s.hash} numberOfLines={1}>{hashHex}</Text>
        <TouchableOpacity onPress={load} accessibilityRole="button" accessibilityLabel="Retry loading content">
          <Text style={s.link}>Retry</Text>
        </TouchableOpacity>
      </View>
    );
  }

  return <Text style={s.content}>{state.text}</Text>;
}

const s = StyleSheet.create({
  hash: { fontSize: 11, color: colors.textDim, fontFamily: 'monospace', marginBottom: 4 },
  link: { fontSize: 12, color: colors.accent, fontWeight: '600' },
  loadingRow: { flexDirection: 'row', alignItems: 'center', gap: 8 },
  loadingText: { fontSize: 12, color: colors.textMuted },
  error: { fontSize: 12, color: colors.danger, marginBottom: 4 },
  content: { fontSize: 13, color: colors.textBody, lineHeight: 19 },
});
