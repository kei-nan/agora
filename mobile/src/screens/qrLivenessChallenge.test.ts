/**
 * Covers the pure QR-liveness-challenge logic (`qrLivenessChallenge.ts`):
 * session creation, payload encode/decode, and the freshness/match check
 * `RegisterScreen.tsx`'s QR-challenge substep gates registration on. See
 * that file's doc comment for the overall design and
 * `faceMatchGating.test.ts` for the sibling pattern this follows.
 */
import {
  combinedCapturePassed,
  createQrChallengeSession,
  decodeQrPayload,
  encodeQrPayload,
  isQrChallengeValid,
  QR_CHALLENGE_NONCE_BYTES,
  QR_CHALLENGE_PAYLOAD_PREFIX,
  QR_CHALLENGE_VALIDITY_MS,
} from './qrLivenessChallenge';

describe('createQrChallengeSession', () => {
  it('generates a nonce of the expected hex length', () => {
    const session = createQrChallengeSession(1_000_000);
    expect(session.nonce).toMatch(/^[0-9a-f]+$/);
    expect(session.nonce).toHaveLength(QR_CHALLENGE_NONCE_BYTES * 2);
  });

  it('sets issuedAtMs/expiresAtMs relative to the given now', () => {
    const session = createQrChallengeSession(1_000_000);
    expect(session.issuedAtMs).toBe(1_000_000);
    expect(session.expiresAtMs).toBe(1_000_000 + QR_CHALLENGE_VALIDITY_MS);
  });

  it('generates different nonces across calls', () => {
    const a = createQrChallengeSession();
    const b = createQrChallengeSession();
    expect(a.nonce).not.toBe(b.nonce);
  });
});

describe('encodeQrPayload / decodeQrPayload', () => {
  it('round-trips a session nonce', () => {
    const session = createQrChallengeSession();
    const payload = encodeQrPayload(session);
    expect(payload).toBe(`${QR_CHALLENGE_PAYLOAD_PREFIX}${session.nonce}`);
    expect(decodeQrPayload(payload)).toBe(session.nonce);
  });

  it('rejects a payload with the wrong prefix', () => {
    expect(decodeQrPayload('https://example.com')).toBeNull();
  });

  it('rejects a payload with a truncated nonce', () => {
    expect(decodeQrPayload(`${QR_CHALLENGE_PAYLOAD_PREFIX}abcd`)).toBeNull();
  });

  it('rejects a payload with non-hex characters in the nonce', () => {
    const badNonce = 'g'.repeat(QR_CHALLENGE_NONCE_BYTES * 2);
    expect(decodeQrPayload(`${QR_CHALLENGE_PAYLOAD_PREFIX}${badNonce}`)).toBeNull();
  });

  it('lowercases a mixed-case nonce', () => {
    const session = createQrChallengeSession();
    const upper = `${QR_CHALLENGE_PAYLOAD_PREFIX}${session.nonce.toUpperCase()}`;
    expect(decodeQrPayload(upper)).toBe(session.nonce.toLowerCase());
  });
});

describe('isQrChallengeValid', () => {
  it('accepts a fresh, matching scan', () => {
    const session = createQrChallengeSession(1_000_000);
    const scanned = encodeQrPayload(session);
    expect(isQrChallengeValid(session, scanned, 1_000_000 + 1000)).toBe(true);
  });

  it('rejects no barcode found at all (null)', () => {
    const session = createQrChallengeSession(1_000_000);
    expect(isQrChallengeValid(session, null, 1_000_000)).toBe(false);
  });

  it('rejects an unrelated QR code', () => {
    const session = createQrChallengeSession(1_000_000);
    expect(isQrChallengeValid(session, 'https://example.com', 1_000_000)).toBe(false);
  });

  it('rejects a nonce from a different (e.g. earlier, expired) session', () => {
    const earlier = createQrChallengeSession(1_000_000);
    const current = createQrChallengeSession(1_000_000 + QR_CHALLENGE_VALIDITY_MS + 1);
    const scannedOldCode = encodeQrPayload(earlier);
    expect(isQrChallengeValid(current, scannedOldCode, current.issuedAtMs)).toBe(false);
  });

  it('rejects a match after the validity window has elapsed', () => {
    const session = createQrChallengeSession(1_000_000);
    const scanned = encodeQrPayload(session);
    const justExpired = session.expiresAtMs + 1;
    expect(isQrChallengeValid(session, scanned, justExpired)).toBe(false);
  });

  it('accepts exactly at the expiry boundary', () => {
    const session = createQrChallengeSession(1_000_000);
    const scanned = encodeQrPayload(session);
    expect(isQrChallengeValid(session, scanned, session.expiresAtMs)).toBe(true);
  });
});

describe('combinedCapturePassed', () => {
  it('accepts when both the face check and the QR check pass', () => {
    const session = createQrChallengeSession(1_000_000);
    const scanned = encodeQrPayload(session);
    expect(combinedCapturePassed(session, scanned, true, 1_000_000 + 1000)).toBe(true);
  });

  it('rejects when the face check fails, even with a valid matching code', () => {
    const session = createQrChallengeSession(1_000_000);
    const scanned = encodeQrPayload(session);
    expect(combinedCapturePassed(session, scanned, false, 1_000_000 + 1000)).toBe(false);
  });

  it('rejects when the face check passes but no code was found', () => {
    const session = createQrChallengeSession(1_000_000);
    expect(combinedCapturePassed(session, null, true, 1_000_000)).toBe(false);
  });

  it('rejects when the face check passes but the code is from a different (e.g. earlier) session', () => {
    const earlier = createQrChallengeSession(1_000_000);
    const current = createQrChallengeSession(1_000_000 + QR_CHALLENGE_VALIDITY_MS + 1);
    const scannedOldCode = encodeQrPayload(earlier);
    expect(combinedCapturePassed(current, scannedOldCode, true, current.issuedAtMs)).toBe(false);
  });

  it('rejects when the face check passes but the code has expired', () => {
    const session = createQrChallengeSession(1_000_000);
    const scanned = encodeQrPayload(session);
    expect(combinedCapturePassed(session, scanned, true, session.expiresAtMs + 1)).toBe(false);
  });

  it('rejects when both checks fail', () => {
    const session = createQrChallengeSession(1_000_000);
    expect(combinedCapturePassed(session, null, false, 1_000_000)).toBe(false);
  });
});
