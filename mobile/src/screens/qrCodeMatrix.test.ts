/**
 * Covers `buildQrMatrix` — the pure QR-encoding step behind
 * `../components/QrCode.tsx`. See that component and `qrLivenessChallenge.ts`
 * for how this fits into the alternate liveness-challenge flow.
 */
import { buildQrMatrix } from './qrCodeMatrix';

describe('buildQrMatrix', () => {
  it('returns a square matrix', () => {
    const matrix = buildQrMatrix('agora-liveness-v1:0123456789abcdef0123456789abcdef');
    expect(matrix.length).toBeGreaterThan(0);
    matrix.forEach((row) => expect(row).toHaveLength(matrix.length));
  });

  it('encodes different text into different matrices', () => {
    const a = buildQrMatrix('agora-liveness-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
    const b = buildQrMatrix('agora-liveness-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb');
    expect(a).not.toEqual(b);
  });

  it('is deterministic for the same input', () => {
    const text = 'agora-liveness-v1:0123456789abcdef0123456789abcdef';
    expect(buildQrMatrix(text)).toEqual(buildQrMatrix(text));
  });

  it('produces at least one dark and one light module (not blank/solid)', () => {
    const matrix = buildQrMatrix('agora-liveness-v1:0123456789abcdef0123456789abcdef');
    const flat = matrix.flat();
    expect(flat.some((cell) => cell)).toBe(true);
    expect(flat.some((cell) => !cell)).toBe(true);
  });
});
