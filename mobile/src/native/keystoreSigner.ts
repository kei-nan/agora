/**
 * Thin JS/TS bridge to the Android-only native `KeystoreSigningModule`
 * (`mobile/android/app/src/main/java/com/agora/wallet/KeystoreSigningModule.kt`),
 * which wraps (encrypts) and unwraps (decrypts) an arbitrary secret using a
 * 256-bit AES-GCM key held in the Android Keystore — non-exportable, and
 * best-effort StrongBox-backed where the device has that secure element.
 *
 * See the native module's doc comment for the full honest accounting of what
 * "hardware-backed" means here: the AES *wrapping* key never leaves secure
 * hardware, but the thing it wraps (this app's sr25519 wallet seed, see
 * `../chain/keystoreWallet.ts`) is still decrypted into JS memory to sign,
 * because Android Keystore cannot generate or sign with an Sr25519 key
 * itself (only NIST curves/RSA) and this chain verifies Sr25519 signatures.
 * This module is strictly the encrypt/decrypt primitive; `keystoreWallet.ts`
 * is what turns it into an actual signing keypair.
 *
 * Mirrors `./nfcPassportReader.ts`'s bridging pattern: base64 in/out across
 * the RN bridge (it can't carry raw binary cleanly), a `NativeModules` lookup
 * that's `undefined` if the module isn't linked (e.g. iOS, or a JS-only test
 * environment), and an `isAvailable`-style guard callers check before use.
 *
 * **Android only.** iOS needs an equivalent Swift module wrapping the Secure
 * Enclave (`SecKeyCreateRandomKey` with `kSecAttrTokenIDSecureEnclave`) —
 * not built here, since no `ios/` native project exists yet in this repo
 * (see CLAUDE.md's Mobile App section and `docs/project/apps/mobile.md`).
 *
 * Not runtime-tested: no Android SDK, emulator, or physical device is
 * available in this development environment. Only `tsc --noEmit` and the
 * unit tests in `keystoreSigner.test.ts` (which mock the native bridge, the
 * normal RN testing pattern — they cannot exercise the real Keystore) have
 * verified this file.
 */
import { NativeModules, Platform } from 'react-native';
import { Buffer } from 'buffer';

/** The native module's actual bridge shape: base64-encoded strings in and out. */
interface KeystoreSigningModuleNative {
  encryptSecret(plaintextB64: string): Promise<{ ciphertext: string; iv: string }>;
  decryptSecret(ciphertextB64: string, ivB64: string): Promise<string>;
  isHardwareBacked(): Promise<boolean>;
}

const KeystoreSigningModule = NativeModules.KeystoreSigningModule as
  | KeystoreSigningModuleNative
  | undefined;

/**
 * `true` only on Android with the native module actually linked. Every other
 * export below throws if called while this is `false` — callers (in
 * practice just `../chain/keystoreWallet.ts`) must check this first and fall
 * back to whatever this platform/build's non-hardware-backed path is, if any
 * (see `../chain/identity.ts`'s `resolveSigningKeypair`, which is the only
 * place that fallback decision is made).
 */
export function isKeystoreSigningAvailable(): boolean {
  return Platform.OS === 'android' && KeystoreSigningModule != null;
}

function requireModule(): KeystoreSigningModuleNative {
  if (!isKeystoreSigningAvailable()) {
    throw new Error(
      `keystoreSigner: not available on this platform/build (Platform.OS=${Platform.OS}, ` +
        `native module linked=${KeystoreSigningModule != null}). Call isKeystoreSigningAvailable() first.`,
    );
  }
  return KeystoreSigningModule!;
}

/** Encrypts `plaintext` under the Keystore wrapping key. A fresh IV is generated natively per call. */
export async function encryptSecret(
  plaintext: Uint8Array,
): Promise<{ ciphertext: Uint8Array; iv: Uint8Array }> {
  const { ciphertext, iv } = await requireModule().encryptSecret(Buffer.from(plaintext).toString('base64'));
  return {
    ciphertext: new Uint8Array(Buffer.from(ciphertext, 'base64')),
    iv: new Uint8Array(Buffer.from(iv, 'base64')),
  };
}

/**
 * Decrypts a `{ ciphertext, iv }` pair previously produced by
 * {@link encryptSecret}. Rejects (does not return garbage) if either was
 * tampered with or doesn't match — GCM's authentication tag guarantees this.
 */
export async function decryptSecret(ciphertext: Uint8Array, iv: Uint8Array): Promise<Uint8Array> {
  const plaintextB64 = await requireModule().decryptSecret(
    Buffer.from(ciphertext).toString('base64'),
    Buffer.from(iv).toString('base64'),
  );
  return new Uint8Array(Buffer.from(plaintextB64, 'base64'));
}

/**
 * Whether the wrapping key currently reports being inside secure hardware
 * (TEE, or StrongBox where available). Purely informational/diagnostic.
 * Returns `false` (rather than throwing) if the native module isn't
 * available at all, since "no" is the honest answer in that case too.
 */
export async function isHardwareBacked(): Promise<boolean> {
  if (!isKeystoreSigningAvailable()) return false;
  return KeystoreSigningModule!.isHardwareBacked();
}
