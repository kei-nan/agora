/**
 * Pure logic for the QR-code alternate liveness challenge — an accessible
 * opt-in alongside `RegisterScreen.tsx`'s default blink/turn challenge (see
 * `runLivenessGate`/`challengePassed` there) for citizens who can't perform
 * facial articulation (paralysis, certain facial differences) or who are
 * simply having trouble with the camera-based blink/turn detection.
 *
 * Design (two-shot, revised — see "The flaw this replaced" below): instead
 * of blinking or turning their head, the citizen displays a QR code encoding
 * a random, short-lived nonce for the current registration attempt — on a
 * second device, or printed — and holds it up, along with their own face, to
 * the same front-camera capture step that would otherwise ask for a
 * blink/turn. `RegisterScreen.tsx` drives this as exactly two combined
 * captures (`../native/faceMatch.ts#captureFaceAndQr`, which runs ML Kit
 * face detection *and* ML Kit Barcode Scanning against the same frame — see
 * that function's doc comment), each gated by {@link combinedCapturePassed}
 * below against its own freshly-issued session from
 * {@link createQrChallengeSession}: substep one, then substep two with a
 * brand-new nonce. Only the *second* capture's photo is the one handed to
 * the passport face-match — see `RegisterScreen.tsx`'s `qrCapture2` substep.
 *
 * A nonce that's unique per attempt and expires quickly is what does the
 * actual liveness-equivalent work here: it proves the capture happened
 * *now*, during this specific registration attempt, which a pre-recorded
 * photo/video from an earlier session cannot satisfy. Requiring a face to be
 * present *in the same frame* as that nonce, twice, with the nonce
 * refreshed in between, is what closes the original flaw below.
 *
 * ## The flaw this replaced
 * The single-shot predecessor of this design decoded a QR nonce with *no*
 * face check at all, reusing an earlier, separately-captured "baseline"
 * face photo (itself only checked once, with no freshness requirement tied
 * to that specific check) for the eventual passport face-match. Concretely:
 * an attacker holding only a static photo of the citizen — enough to pass
 * that one-off baseline face check — could then complete the *entire*
 * liveness+face-match pipeline by simply showing the QR code to the camera
 * afterward, with no live person or facial movement required at any point
 * during the QR substep itself.
 *
 * ## Residual risk (still open, deliberately scoped, not a claim of full defense)
 * This still does not defend against a sufficiently prepared attacker who
 * can present the *same* static photo of the citizen's face at both
 * combined-capture moments while *also* correctly relaying each freshly-
 * issued QR code in real time (e.g. a confederate reading the on-screen
 * nonce back to them, or an automated relay) — two-shot with a fresh nonce
 * each time raises the bar (a single photo shown once, disconnected from any
 * challenge, is no longer sufficient) but does not require the *face itself*
 * to move or react between the two shots, only that a face and a correct
 * nonce coincide in each frame. This is the same honesty standard, and the
 * same category of gap, this codebase already documents for the default
 * blink/turn challenge against a sufficiently prepared video-capable
 * attacker (see `RegisterScreen.tsx`'s `EYES_OPEN_THRESHOLD` doc comment,
 * `docs/project/changelog/087.md`) — not a claim that either challenge
 * defeats a sophisticated, resourced attacker, only that it closes the
 * much weaker "no live presentation required at all" gap this file's
 * original single-shot design had.
 *
 * RN-free by design (no `react-native` import), same reasoning as
 * `faceMatchGating.ts`'s doc comment: this repo's jest environment can't load
 * the real `react-native` package, so anything meant to be unit-tested
 * directly needs to avoid it. `../native/qrChallenge.ts` and
 * `../native/faceMatch.ts` (the native bridges that actually capture a frame
 * and run ML Kit against it) are deliberately kept separate from this file
 * for the same reason.
 */
import { randomAsU8a } from '@polkadot/util-crypto';

/** Bytes of entropy in a freshly generated session nonce (128 bits). */
export const QR_CHALLENGE_NONCE_BYTES = 16;

/**
 * How long a generated QR challenge stays valid. Short enough that a citizen
 * scanning it in a single sitting is unaffected, but short enough that a
 * captured photo of the code can't be reused across a meaningfully different
 * time window. Unvalidated placeholder, same honesty standard as
 * `RegisterScreen.tsx`'s `EYES_OPEN_THRESHOLD` etc. — no real-world usability
 * data has informed this number yet.
 */
export const QR_CHALLENGE_VALIDITY_MS = 2 * 60 * 1000;

/**
 * Every QR payload this app generates is prefixed with this literal so a
 * decoded barcode that happens to be some unrelated QR code (a poster, a
 * URL, anything) is never mistaken for a liveness challenge match.
 */
export const QR_CHALLENGE_PAYLOAD_PREFIX = 'agora-liveness-v1:';

export interface QrChallengeSession {
  /** Lowercase hex, `QR_CHALLENGE_NONCE_BYTES` bytes. */
  nonce: string;
  issuedAtMs: number;
  expiresAtMs: number;
}

function toLowerHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}

/**
 * Starts a new QR challenge session: a fresh random nonce plus its validity
 * window from `now`. Callers should generate a new session every time the
 * user (re)enters the QR-challenge substep — including on retry after an
 * expired or mismatched scan — never reuse an old one, since reuse is
 * exactly what would let a pre-recorded capture pass.
 *
 * Uses `@polkadot/util-crypto`'s `randomAsU8a`, the same RNG this codebase
 * already relies on for other security-relevant random material (see
 * `../chain/keystoreWallet.ts`'s seed generation) rather than introducing a
 * second source of randomness.
 */
export function createQrChallengeSession(now: number = Date.now()): QrChallengeSession {
  const nonce = toLowerHex(randomAsU8a(QR_CHALLENGE_NONCE_BYTES));
  return { nonce, issuedAtMs: now, expiresAtMs: now + QR_CHALLENGE_VALIDITY_MS };
}

/** The literal string encoded into the on-screen QR code for `session`. */
export function encodeQrPayload(session: Pick<QrChallengeSession, 'nonce'>): string {
  return `${QR_CHALLENGE_PAYLOAD_PREFIX}${session.nonce}`;
}

/**
 * Inverse of {@link encodeQrPayload}: extracts the nonce from a decoded
 * barcode string, or `null` if it isn't a well-formed Agora liveness-challenge
 * payload at all (wrong prefix, wrong nonce length/charset — e.g. an
 * unrelated QR code the camera happened to pick up).
 */
export function decodeQrPayload(text: string): string | null {
  if (!text.startsWith(QR_CHALLENGE_PAYLOAD_PREFIX)) return null;
  const nonce = text.slice(QR_CHALLENGE_PAYLOAD_PREFIX.length);
  if (nonce.length !== QR_CHALLENGE_NONCE_BYTES * 2) return null;
  if (!/^[0-9a-f]+$/i.test(nonce)) return null;
  return nonce.toLowerCase();
}

/**
 * Whether a barcode decoded from the capture step satisfies `session` right
 * now: it must decode to a well-formed Agora liveness payload, its nonce must
 * match this exact session (not some other/earlier one), and the session
 * must not have expired. `decodedText` is `null` when the native side found
 * no barcode in the frame at all (see `../native/qrChallenge.ts`).
 */
export function isQrChallengeValid(
  session: QrChallengeSession,
  decodedText: string | null,
  now: number = Date.now(),
): boolean {
  if (decodedText == null) return false;
  const nonce = decodeQrPayload(decodedText);
  if (nonce == null) return false;
  if (nonce !== session.nonce.toLowerCase()) return false;
  return now <= session.expiresAtMs;
}

/**
 * Whether one combined face+QR capture (`../native/faceMatch.ts#captureFaceAndQr`)
 * satisfies both halves of the two-shot QR-liveness-challenge design — see
 * this module's doc comment. `RegisterScreen.tsx` calls this once per
 * combined-capture substep (`qrCapture1`, then `qrCapture2` against a fresh
 * `session`), passing `facePassed` as whatever its own `baselinePassed()`
 * check (eyes-open/frontal-angle thresholds against the same capture's
 * signals) already returned — deliberately not duplicated here, since
 * `baselinePassed` operates on `CapturedPhoto`-shaped data this file has no
 * business importing a runtime dependency on (see the RN-free note above);
 * this function only combines that already-computed boolean with the QR
 * check this file *does* own.
 *
 * Both halves are required: a face with no valid/fresh code, or a valid code
 * with no face in frame, must fail — either one alone is exactly the
 * disconnected, independently-satisfiable check this design replaces.
 */
export function combinedCapturePassed(
  session: QrChallengeSession,
  decodedText: string | null,
  facePassed: boolean,
  now: number = Date.now(),
): boolean {
  return facePassed && isQrChallengeValid(session, decodedText, now);
}
