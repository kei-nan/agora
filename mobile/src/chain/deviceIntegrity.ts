/**
 * Device/app-integrity attestation captured alongside registration, via the
 * Android Play Integrity API — a defense-in-depth signal against a modified
 * client (patched app, hooked native module) skipping the on-device
 * face-match/liveness checks (`../native/faceMatch.ts`, `../screens/
 * qrLivenessChallenge.ts`) that currently have nothing tying their result to
 * the submitted ZK proof.
 *
 * # What this module does, concretely
 *
 * `captureDeviceIntegritySignal` generates a fresh random nonce, asks the
 * native `PlayIntegrityModule` (`../native/playIntegrity.ts`) for a token
 * bound to it via the Play Integrity classic API's `setNonce(...)`, and hands
 * back the opaque token plus the nonce it was bound to. That's it — this
 * module does not, and structurally cannot, verify the token itself.
 *
 * # Why verification can't happen here (research finding, not an oversight)
 *
 * Every documented Play Integrity flow requires a server-side call to decode
 * the token: the client library returns an *encrypted* blob, and decrypting
 * it requires calling `playintegrity.googleapis.com` authenticated as a
 * Google Cloud service account tied to this app's Play listing — credentials
 * that must never ship inside the app itself. Google's own guidance is
 * explicit that verification must happen server-side, never client-side.
 * There is no self-verification path, full stop — confirmed by reading
 * Android's Play Integrity "classic request" and "returned verdict" docs
 * before writing this module (developer.android.com/google/play/integrity/
 * classic, .../verdicts).
 *
 * # Why this app can't just add that server call itself
 *
 * This app has no traditional backend server — registration and proving
 * genuinely happen device-to-chain (see CLAUDE.md's Desktop App section for
 * the same property on the desktop side, and `court-oracle/`/
 * `committee-node/` for this project's existing precedent of standalone
 * off-chain *services* rather than a shared backend). A Play Integrity
 * verifier is exactly that kind of standalone service, not a mobile-app
 * change: it would need to hold a Google Cloud service account credential,
 * expose some endpoint the mobile app or a future oracle-style component
 * could reach, and decide what to do with the verdict (flag the citizen
 * record on-chain? gate nothing and just log for later audit? — a real
 * product decision this task does not make). Building that is comparable in
 * scope to `court-oracle/` itself, not a few more lines in this file.
 *
 * # What a future verifier service would need to do
 *
 * Concretely, once someone builds it:
 *   1. Receive `{ token, nonceBase64 }` (see `DeviceIntegritySignal` below)
 *      from wherever this signal ends up landing (see "Where this signal
 *      lands today" below — nowhere on-chain yet).
 *   2. Call `playintegrity.googleapis.com/v1/PACKAGE_NAME:decodeIntegrityToken`
 *      (classic API) authenticated as the linked Cloud project's service
 *      account, passing `token`.
 *   3. Check the decoded verdict's `requestDetails.nonce` matches
 *      `nonceBase64` exactly (replay protection — this is the whole reason
 *      the nonce exists) and that `requestDetails.timestampMillis` is recent.
 *   4. Check `appIntegrity.packageName` is `com.agora` and
 *      `appIntegrity.certificateSha256Digest` matches this app's real
 *      release signing cert (not a debug key) — otherwise a *different*,
 *      attacker-controlled app could request its own valid token and forward
 *      it, defeating the whole point.
 *   5. Check `deviceIntegrity.deviceRecognitionVerdict` contains
 *      `MEETS_DEVICE_INTEGRITY` (rejects rooted/emulated/tampered devices)
 *      and `accountDetails.appLicensingVerdict` as appropriate for this
 *      app's distribution model.
 *   6. Decide what to do with a failing verdict — this is a policy decision
 *      with no obviously-correct default (reject the registration outright?
 *      flag it for review? weight it as one signal among several?) that
 *      whoever builds the verifier needs to make deliberately, matching how
 *      `court-oracle`'s ruling logic was a real design decision, not just
 *      plumbing.
 *
 * # Where this signal lands today — explicitly nowhere on-chain
 *
 * `pallet-identity`'s real `register_citizen` extrinsic
 * (`pallets/pallet-identity/src/lib.rs`) takes `zk_proof`, `public_inputs`,
 * `anchor`, `oprf_pk_hashes`, `backing_commitment` — no device-integrity
 * field exists there, and this module does not add one: doing so would need
 * a corresponding pallet change (a new bounded-bytes field, analogous to how
 * `backing_commitment` itself was added), which is out of scope here and
 * would be a lie to claim "wired" when the chain would simply reject an
 * extra argument. `../chain/proofEncoding.ts` documents the matching
 * sibling-field shape (`DeviceIntegritySubmissionExtras`) for when that
 * pallet change happens. Today, `captureDeviceIntegritySignal`'s result is
 * only ever persisted into `registrationState.ts`'s local pipeline record
 * (see `RegisterScreen.tsx`'s liveness step) — visible to the app itself,
 * not submitted anywhere.
 *
 * This is deliberately staged the same honest way this codebase already
 * stages other not-yet-exercisable-end-to-end mechanisms: `runtime/src/
 * verifier.rs` verifies real ZK proofs well ahead of any OPRF committee that
 * would let one actually be produced (see CLAUDE.md's Identity System
 * section) — the mechanism is built and real, the end-to-end path isn't.
 * Same shape here, smaller scope: the token-capture mechanism is real and
 * genuinely requests a real Google API; nothing downstream reads it yet.
 */
import { randomAsU8a } from '@polkadot/util-crypto';
import { isPlayIntegrityAvailable, requestIntegrityToken } from '../native/playIntegrity';

/**
 * Raw entropy bytes for the nonce bound into the integrity token. Play
 * Integrity's classic API requires the base64-encoded nonce to be
 * 16..500 chars; 32 raw bytes (256 bits) comfortably clears the minimum
 * once base64-encoded (~43 chars) with a wide safety margin, matching the
 * example in Android's own classic-API documentation.
 */
export const DEVICE_INTEGRITY_NONCE_BYTES = 32;

/**
 * Generates a fresh random nonce for one Play Integrity token request. Uses
 * `@polkadot/util-crypto`'s `randomAsU8a`, the same RNG this codebase already
 * relies on elsewhere for security-relevant random material (see
 * `../chain/keystoreWallet.ts`'s seed generation, `../screens/
 * qrLivenessChallenge.ts`'s QR nonce) rather than introducing a second
 * source of randomness.
 *
 * Deliberately a *separate* nonce from the QR-liveness-challenge nonce
 * (`../screens/qrLivenessChallenge.ts`), even though both exist to prove
 * "this happened now, this attempt": the QR nonce is only generated when the
 * user opts into that specific alternate challenge, but device-integrity
 * attestation is meant to run on every registration attempt regardless of
 * which liveness method was used, so it needs its own independent nonce
 * generated unconditionally.
 */
export function generateDeviceIntegrityNonce(): Uint8Array {
  return randomAsU8a(DEVICE_INTEGRITY_NONCE_BYTES);
}

/**
 * Base64, URL-safe, no padding — the exact encoding Play Integrity's
 * `IntegrityTokenRequest.Builder.setNonce(String)` requires (confirmed
 * against Android's classic-API docs). RN/Hermes has no `btoa`; this uses
 * the globally-polyfilled `Buffer` (`index.js`, already relied on throughout
 * `../chain/*`) instead of adding a dedicated base64 dependency.
 */
export function nonceToBase64Url(nonce: Uint8Array): string {
  return Buffer.from(nonce)
    .toString('base64')
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/, '');
}

/** A captured, not-yet-verified Play Integrity signal. See this module's doc comment for what "not-yet-verified" means concretely. */
export interface DeviceIntegritySignal {
  /** The opaque, encrypted token from Google's client library. Meaningless to this app — only a server-side decode call can interpret it. */
  token: string;
  /** The exact base64url nonce `token` was bound to — a future verifier must check the decoded verdict's nonce matches this. */
  nonceBase64: string;
  requestedAtMs: number;
}

export type DeviceIntegrityResult =
  | { captured: true; signal: DeviceIntegritySignal }
  | { captured: false; reason: string };

/**
 * Caps how long {@link captureDeviceIntegritySignal} will wait on the native
 * `requestIntegrityToken` call before giving up and treating this attempt as
 * `captured: false`. `RegisterScreen.tsx` no longer blocks the liveness
 * capture UI on this call (it kicks it off without awaiting and only awaits
 * the result later, at the one place it's actually consumed — the
 * `LivenessVerified` status write), but a citizen could plausibly finish the
 * entire liveness step well within this window, so an unbounded hang on a
 * bad network could still leave that later `await` — and therefore
 * registration's progress past the liveness step — stuck indefinitely
 * without this. 15s comfortably covers a slow-but-working request; same
 * "unvalidated placeholder, not measured against real traffic" honesty
 * standard as this module's other numbers.
 */
export const DEVICE_INTEGRITY_TIMEOUT_MS = 15_000;

/**
 * Best-effort capture of a device-integrity signal for the current
 * registration attempt. Never throws — every failure mode (native module
 * unavailable, no Play Services, no network, Google-side error, or simply
 * taking longer than {@link DEVICE_INTEGRITY_TIMEOUT_MS}) comes back as
 * `{ captured: false, reason }` instead, because this is a defense-in-depth
 * signal layered *alongside* registration, not a gate: a citizen on a device
 * without Play Services (e.g. a de-Googled ROM) must still be able to
 * register today, the same way `matchAgainstPassport`'s `skipped` case
 * (`../screens/faceMatchGating.ts`) never blocks registration on an
 * unrelated capability gap. The underlying native call is not cancelled when
 * the timeout wins the race — there's no cancellation hook on it — this just
 * stops *this function* from waiting on it any longer.
 */
export async function captureDeviceIntegritySignal(
  nonce: Uint8Array = generateDeviceIntegrityNonce(),
): Promise<DeviceIntegrityResult> {
  if (!isPlayIntegrityAvailable()) {
    return { captured: false, reason: 'Play Integrity is not available on this device/build.' };
  }
  const nonceBase64 = nonceToBase64Url(nonce);
  // `timeoutHandle` is cleared in `finally` regardless of which side of the race
  // settles first — otherwise the timer set up here outlives this function on the
  // (overwhelmingly common) path where `requestIntegrityToken` wins the race,
  // leaking a real pending timer for the rest of `DEVICE_INTEGRITY_TIMEOUT_MS`.
  let timeoutHandle: ReturnType<typeof setTimeout> | undefined;
  try {
    const token = await Promise.race([
      requestIntegrityToken(nonceBase64),
      new Promise<never>((_resolve, reject) => {
        timeoutHandle = setTimeout(() => reject(new Error('Play Integrity request timed out')), DEVICE_INTEGRITY_TIMEOUT_MS);
      }),
    ]);
    return { captured: true, signal: { token, nonceBase64, requestedAtMs: Date.now() } };
  } catch (e: any) {
    return { captured: false, reason: e?.message ?? String(e) };
  } finally {
    clearTimeout(timeoutHandle);
  }
}
