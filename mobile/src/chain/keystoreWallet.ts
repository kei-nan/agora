/**
 * Turns the Android Keystore-wrapped-secret primitive in
 * `../native/keystoreSigner.ts` into an actual `KeyringPair` this app's
 * chain-calling code (`identity.ts`, `voting.ts`, `governance.ts`,
 * `constitution.ts`, `courts.ts` — via `identity.ts`'s `getSigningKeypair`)
 * can sign with.
 *
 * ## What's stored, and where
 *
 * A random 32-byte sr25519 seed is generated on first use, encrypted under
 * the Android Keystore wrapping key (`keystoreSigner.encryptSecret` — see
 * that module and the native `KeystoreSigningModule.kt` for the honest
 * accounting of what "hardware-backed" does and doesn't cover here), and
 * persisted as a small JSON file in this app's private sandboxed storage
 * (`RNFS.DocumentDirectoryPath` — not shared storage, not backed up to a
 * public location) via `react-native-fs`, already a dependency of this app.
 * The plaintext seed itself is never written to disk.
 *
 * On every subsequent call, the same file is read back, decrypted, and used
 * to rebuild the same `KeyringPair` — so the citizen's on-chain address is
 * stable across app restarts, matching what `DEV_ONLY_MNEMONIC` gave for
 * free (a fixed keypair) but with the seed now random-per-install and
 * encrypted at rest instead of a public, hardcoded, identical-on-every-
 * install value.
 *
 * Not a migration/recovery story: if this file is lost (app data cleared,
 * uninstall/reinstall, device reset) the seed is unrecoverable and a fresh
 * one is generated, meaning a *new* on-chain address — this module doesn't
 * attempt account recovery. CLAUDE.md's Identity System section says
 * "Recovery = re-scan valid passport", which implies recovery should key off
 * the passport-derived OPRF identity anchor (`getSigningKeypair`'s
 * `nullifierHash`/pallet-identity's `CitizenAnchor`), not this file — wiring
 * that up is separate, unstarted work; this module only provides *a*
 * hardware-backed key, not identity continuity across reinstalls.
 */
import RNFS from 'react-native-fs';
import { Keyring } from '@polkadot/keyring';
import { KeyringPair } from '@polkadot/keyring/types';
import { cryptoWaitReady, randomAsU8a } from '@polkadot/util-crypto';
import { Buffer } from 'buffer';
import { decryptSecret, encryptSecret, isKeystoreSigningAvailable } from '../native/keystoreSigner';

/** 32 bytes — the seed length `Keyring.addFromSeed` expects for both sr25519 and ed25519. */
const SEED_BYTES = 32;

/** Bumped if the on-disk shape ever changes; unrecognized versions are treated as "no wallet on file". */
const WALLET_FILE_VERSION = 1;

const WALLET_FILE_PATH = `${RNFS.DocumentDirectoryPath}/agora-wallet-key.enc.json`;

interface EncryptedWalletFile {
  version: number;
  /** Base64 AES-GCM ciphertext of the sr25519 seed. */
  ciphertext: string;
  /** Base64 GCM IV used for that ciphertext. */
  iv: string;
}

function isEncryptedWalletFile(value: unknown): value is EncryptedWalletFile {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as any).ciphertext === 'string' &&
    typeof (value as any).iv === 'string'
  );
}

async function readPersistedSeed(): Promise<Uint8Array | null> {
  const exists = await RNFS.exists(WALLET_FILE_PATH);
  if (!exists) return null;

  const raw = await RNFS.readFile(WALLET_FILE_PATH, 'utf8');
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null; // Corrupt file — treated the same as "none on file" below.
  }
  if (!isEncryptedWalletFile(parsed) || parsed.version !== WALLET_FILE_VERSION) {
    return null;
  }

  return decryptSecret(
    new Uint8Array(Buffer.from(parsed.ciphertext, 'base64')),
    new Uint8Array(Buffer.from(parsed.iv, 'base64')),
  );
}

async function persistNewSeed(): Promise<Uint8Array> {
  const seed = randomAsU8a(SEED_BYTES);
  const { ciphertext, iv } = await encryptSecret(seed);
  const file: EncryptedWalletFile = {
    version: WALLET_FILE_VERSION,
    ciphertext: Buffer.from(ciphertext).toString('base64'),
    iv: Buffer.from(iv).toString('base64'),
  };
  await RNFS.writeFile(WALLET_FILE_PATH, JSON.stringify(file), 'utf8');
  return seed;
}

async function loadOrCreateSeed(): Promise<Uint8Array> {
  const existing = await readPersistedSeed();
  if (existing) return existing;
  return persistNewSeed();
}

let _keypairPromise: Promise<KeyringPair> | null = null;

/** Test-only escape hatch — production code never needs to reset this within a process lifetime. */
export function _resetCachedKeystoreKeypairForTests(): void {
  _keypairPromise = null;
}

/**
 * Returns a real `KeyringPair` backed by the Android Keystore-wrapped seed
 * (generating one on first call). Throws if
 * {@link isKeystoreSigningAvailable} is `false` — callers must check that
 * first; see `identity.ts`'s `resolveSigningKeypair` for the only place in
 * this app that decides what to do when it isn't (currently: a `__DEV__`-
 * gated fallback to the legacy dev mnemonic, or a hard error).
 *
 * Cached for the life of the process, same as the old `devKeyringPair()` —
 * repeated calls don't re-hit disk/Keystore.
 */
export async function getOrCreateKeystoreKeypair(): Promise<KeyringPair> {
  if (!isKeystoreSigningAvailable()) {
    throw new Error(
      'getOrCreateKeystoreKeypair: Android Keystore signing module is not available on this platform/build.',
    );
  }
  if (!_keypairPromise) {
    _keypairPromise = (async () => {
      await cryptoWaitReady();
      const seed = await loadOrCreateSeed();
      const keyring = new Keyring({ type: 'sr25519', ss58Format: 42 });
      return keyring.addFromSeed(seed, { name: 'agora-keystore' }, 'sr25519');
    })();
  }
  try {
    return await _keypairPromise;
  } catch (e) {
    _keypairPromise = null; // Don't cache a failed attempt — the next call should retry fresh.
    throw e;
  }
}
