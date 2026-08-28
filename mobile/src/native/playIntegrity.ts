/**
 * Thin JS/TS bridge to the Android-only `PlayIntegrityModule` native module
 * (`mobile/android/app/src/main/java/com/agora/integrity/PlayIntegrityModule.kt`)
 * — requests a Google Play Integrity API token bound to a nonce, as a
 * defense-in-depth signal alongside registration that a modified/hooked
 * client can't produce for a genuine, unmodified install. See
 * `../chain/deviceIntegrity.ts` for the session-nonce plumbing and the full
 * design note on what a *future* server-side verifier would need to do with
 * the token this returns — **nothing in this codebase verifies it yet**.
 *
 * Mirrors this app's other native-module bridges (`./faceMatch.ts`,
 * `./qrChallenge.ts`): an `isAvailable`-style guard plus throwing wrappers.
 *
 * **Android only.** iOS has no equivalent (Apple's closest analog, App
 * Attest/DeviceCheck, is a wholly separate API this codebase has not looked
 * at) — consistent with every other native module in this app being
 * Android-only today (`ios/` doesn't exist, see CLAUDE.md).
 *
 * Not runtime-tested: no Android SDK, emulator, or physical device is
 * available in this development environment, and Play Integrity specifically
 * also requires a real Google Play Services install and a real app listing
 * (package name + signing cert registered with Play) to return a genuine
 * verdict at all — even a real device in this environment couldn't exercise
 * this end-to-end without that.
 */
import { NativeModules, Platform } from 'react-native';

interface PlayIntegrityModuleNative {
  /**
   * Requests a fresh integrity token bound to `nonceBase64` (must already be
   * base64, URL-safe, no padding — see `../chain/deviceIntegrity.ts`'s
   * `nonceToBase64Url`). Resolves with the raw, opaque, encrypted token
   * string Google's client library returns — this app has no way to
   * interpret it; only Google's server-side decode endpoint can. Rejects if
   * Play Services/Play Integrity is unavailable (e.g. no Play Services on
   * this device, no network) or the request otherwise fails.
   */
  requestIntegrityToken(nonceBase64: string): Promise<string>;
}

const PlayIntegrityModule = NativeModules.PlayIntegrityModule as PlayIntegrityModuleNative | undefined;

/** `true` only on Android with the native module actually linked. Does NOT guarantee Play Services/a real verdict is available — see `requestIntegrityToken`. */
export function isPlayIntegrityAvailable(): boolean {
  return Platform.OS === 'android' && PlayIntegrityModule != null;
}

/**
 * Requests a Play Integrity token bound to `nonceBase64`. Throws if called
 * before checking {@link isPlayIntegrityAvailable}. Callers should treat any
 * rejection as "no signal captured this attempt" and proceed without
 * blocking registration — see `../chain/deviceIntegrity.ts`'s doc comment
 * for why this is defense-in-depth, not a hard gate.
 */
export async function requestIntegrityToken(nonceBase64: string): Promise<string> {
  if (!isPlayIntegrityAvailable()) {
    throw new Error(
      `playIntegrity: not available on this platform/build (Platform.OS=${Platform.OS}, ` +
        `PlayIntegrityModule linked=${PlayIntegrityModule != null}). Call isPlayIntegrityAvailable() first.`,
    );
  }
  return PlayIntegrityModule!.requestIntegrityToken(nonceBase64);
}
