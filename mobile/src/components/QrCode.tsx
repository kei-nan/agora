/**
 * Renders a QR code as a plain grid of `View`s, from the module matrix
 * `../screens/qrCodeMatrix.ts` computes via `qrcode-generator`.
 *
 * # Library choice
 *
 * `qrcode-generator` was picked over the more common RN QR packages
 * (`react-native-qrcode-svg`, `react-qr-code`) because those all render
 * through `react-native-svg`, which isn't a dependency of this app and would
 * pull in a native module of its own. `qrcode-generator` is pure JS with zero
 * dependencies — it only computes the dark/light module matrix — so this
 * component can render that matrix with nothing but `View`/`StyleSheet`,
 * consistent with this app's existing preference for narrow, purpose-built
 * native surface area over general-purpose native libraries (see
 * `FaceCameraView.tsx`'s doc comment on the same tradeoff for the camera
 * preview). ML Kit's own barcode module (`../native/qrChallenge.ts`) was
 * checked first for a *generation* API and, as expected, only does detection
 * — see that file's doc comment.
 *
 * # Colors are intentionally NOT theme tokens
 *
 * Every other screen in this app pulls its palette from `../theme.ts`, but a
 * scannable code needs strong, unambiguous contrast for a phone camera (and
 * needs to still work if printed on plain paper) — not this app's dark
 * theme. Fixed black-on-white with a white quiet-zone border, matching
 * standard QR-code rendering practice, on purpose.
 *
 * Not runtime-tested — same standing limitation as every other RN-rendering
 * component in this app (`FaceCameraView.tsx`, `AppModal.tsx`, etc.): no
 * Android SDK/emulator in this environment. `buildQrMatrix` itself (the
 * actual QR-encoding logic this component just renders) is unit-tested
 * directly in `../screens/qrCodeMatrix.test.ts`.
 */
import React from 'react';
import { View, StyleSheet } from 'react-native';
import { buildQrMatrix } from '../screens/qrCodeMatrix';

interface Props {
  /** The text to encode — see `../screens/qrLivenessChallenge.ts`'s `encodeQrPayload`. */
  value: string;
  /** Rendered width/height in px, including the quiet-zone border. Default 220. */
  size?: number;
}

const QUIET_ZONE_PX = 16;

export default function QrCode({ value, size = 220 }: Props) {
  const matrix = buildQrMatrix(value);
  const moduleCount = matrix.length;
  const innerSize = size - QUIET_ZONE_PX * 2;
  const cell = innerSize / moduleCount;

  return (
    <View style={[s.quietZone, { width: size, height: size }]} accessibilityRole="image" accessibilityLabel="QR code">
      <View style={{ width: innerSize, height: innerSize }}>
        {matrix.map((row, r) => (
          <View key={r} style={s.row}>
            {row.map((dark, c) => (
              <View key={c} style={{ width: cell, height: cell, backgroundColor: dark ? '#000000' : '#ffffff' }} />
            ))}
          </View>
        ))}
      </View>
    </View>
  );
}

const s = StyleSheet.create({
  quietZone: {
    backgroundColor: '#ffffff',
    padding: QUIET_ZONE_PX,
    borderRadius: 8,
    alignItems: 'center',
    justifyContent: 'center',
  },
  row: { flexDirection: 'row' },
});
