/**
 * Turns the Android Keystore-wrapped-secret primitive in
 * `../native/keystoreSigner.ts` into actual `KeyringPair`s this app's
 * chain-calling code can sign with — **two independent ones**, each its own
 * random seed under its own on-disk file:
 *
 *  - {@link getOrCreateKeystoreKeypair} — the citizen's main wallet, used by
 *    `identity.ts`, `voting.ts`, `governance.ts`, `constitution.ts`,
 *    `courts.ts` (via `identity.ts`'s `getSigningKeypair`).
 *  - {@link getOrCreateDelegatePersonaKeypair} — a second, independent
 *    keypair used only to sign `pallet-elections`' `register_as_delegate`
 *    (its `persona_account` argument must equal `ensure_signed(origin)`, so
 *    the delegate-persona proof and the account submitting it must match —
 *    see `pallets/pallet-elections/src/lib.rs`'s `PersonaAccountMismatch`
 *    check) and whatever else a delegate persona later needs to sign as
 *    itself, e.g. its own on-chain actions once seated.
 *
 * ## Why the delegate-persona seed is independently random, not derived
 *
 * It would be simpler to derive the persona seed from `identity_input` (the
 * same passport-derived secret `circuits/oprf-identity-anchor` builds
 * `anchor`/`backing_commitment` from) the way, say, a hardened HD wallet
 * derives child keys. This deliberately does not do that:
 * `identity_input` is **not rotatable** — it is a deterministic function of
 * the passport's own MRZ fields (see `derive_identity_input` in
 * `circuits/oprf-identity-anchor/lib/identity-anchor/src/lib.nr`), fixed for
 * the life of that passport. A persona key derived from it could never be
 * rotated if the derived key material were ever compromised (leaked device
 * backup, a flawed derivation, etc.) — there would be no way to "re-derive a
 * fresh one" without a new passport. A bare independent random seed carries
 * no such liability: if it's ever compromised, the citizen simply creates a
 * fresh delegate-persona proof (`../../circuits/oprf-identity-anchor/
 * delegate-persona`, see `zkProving.ts`'s `proveDelegatePersona`) binding a
 * newly generated persona account, and stops using the old one. Nothing is
 * lost by this independence: the one property that actually matters —
 * "each citizen can mint at most one delegate persona" — is already
 * enforced by the ZK nullifier scheme itself (`DelegatePersonaUsed` on
 * `pallet-elections`, gated by the delegate-persona circuit's own committee-
 * derived `delegate_persona_id`), not by anything about how the *signing*
 * key for that persona is generated.
 *
 * ## What's stored, and where
 *
 * Each keypair's random 32-byte sr25519 seed is generated on first use,
 * encrypted under the Android Keystore wrapping key
 * (`keystoreSigner.encryptSecret` — see that module and the native
 * `KeystoreSigningModule.kt` for the honest accounting of what "hardware-
 * backed" does and doesn't cover here), and persisted as a small JSON file
 * in this app's private sandboxed storage (`RNFS.DocumentDirectoryPath` —
 * not shared storage, not backed up to a public location) via
 * `react-native-fs`, already a dependency of this app. The plaintext seed
 * itself is never written to disk. The two keypairs live at two distinct
 * file paths ({@link WALLET_FILE_PATH} / {@link DELEGATE_PERSONA_WALLET_FILE_PATH})
 * so losing or regenerating one never touches the other.
 *
 * On every subsequent call, the same file is read back, decrypted, and used
 * to rebuild the same `KeyringPair` — so each on-chain address is stable
 * across app restarts, matching what `DEV_ONLY_MNEMONIC` gave for free (a
 * fixed keypair) but with the seed now random-per-install and encrypted at
 * rest instead of a public, hardcoded, identical-on-every-install value.
 *
 * Not a migration/recovery story on its own: if the citizen wallet's file is
 * lost (app data cleared, uninstall/reinstall, device reset), that seed is
 * unrecoverable and a fresh one is generated on next use, meaning a *new*
 * on-chain address — this module itself doesn't attempt account recovery,
 * it only provides *a* hardware-backed key. Chain-side recovery now exists
 * to make that new address usable again: `pallet-identity`'s
 * `recover_account` extrinsic (same passport re-scan / ZK proof flow as
 * original registration) rebinds the citizen's on-file identity (nullifier,
 * OPRF anchor, reverification deadline, jury-selection index, etc.) from
 * the old, now-unreachable `AccountId` onto whatever fresh one this module
 * generates next — see `pallets/pallet-identity/src/lib.rs`'s
 * `recover_account` doc comment and `docs/project/pallets/identity.md`. As
 * of this writing no mobile-side code calls it yet (this file, and the rest
 * of `mobile/src/chain/`, has no `recoverAccount` wrapper analogous to
 * `identity.ts`'s `registerCitizen`) — wiring the re-scan UI flow to that
 * call is separate, unstarted work; only the chain-side mechanism is done.
 * It also inherits `register_citizen`'s own standing limitation: it depends
 * on a real OPRF committee existing (`OprfCommitteeKeys`/`CommitteeMembers`
 * are still empty — see CLAUDE.md's "Remaining Work" #1), so it can't be
 * exercised end-to-end yet either. The delegate-persona wallet file has no
 * recovery story of its own yet either: losing it would strand whatever
 * `delegate_persona_id` was bound to it, with no `recover_account`-style
 * rebind extrinsic for `pallet-elections`' `DelegatePersonaIdOf` — a real,
 * currently-open gap, not addressed by this module.
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

/** Second, independent wallet file — see this module's doc comment for why it's a separate random seed, not derived from the citizen wallet's. */
const DELEGATE_PERSONA_WALLET_FILE_PATH = `${RNFS.DocumentDirectoryPath}/agora-delegate-persona-key.enc.json`;

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

async function readPersistedSeed(filePath: string): Promise<Uint8Array | null> {
  const exists = await RNFS.exists(filePath);
  if (!exists) return null;

  const raw = await RNFS.readFile(filePath, 'utf8');
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

async function persistNewSeed(filePath: string): Promise<Uint8Array> {
  const seed = randomAsU8a(SEED_BYTES);
  const { ciphertext, iv } = await encryptSecret(seed);
  const file: EncryptedWalletFile = {
    version: WALLET_FILE_VERSION,
    ciphertext: Buffer.from(ciphertext).toString('base64'),
    iv: Buffer.from(iv).toString('base64'),
  };
  await RNFS.writeFile(filePath, JSON.stringify(file), 'utf8');
  return seed;
}

async function loadOrCreateSeed(filePath: string): Promise<Uint8Array> {
  const existing = await readPersistedSeed(filePath);
  if (existing) return existing;
  return persistNewSeed(filePath);
}

/**
 * Builds an independent, cached "load or create a Keystore-backed keypair at this file path"
 * getter. Each of {@link getOrCreateKeystoreKeypair}/{@link getOrCreateDelegatePersonaKeypair}
 * is one call to this, closed over its own file path and its own in-memory promise cache — the
 * two never share state.
 */
function createKeystoreBackedKeypairGetter(filePath: string, keyringName: string) {
  let cached: Promise<KeyringPair> | null = null;

  async function getOrCreate(): Promise<KeyringPair> {
    if (!isKeystoreSigningAvailable()) {
      throw new Error(
        'getOrCreateKeystoreKeypair: Android Keystore signing module is not available on this platform/build.',
      );
    }
    if (!cached) {
      cached = (async () => {
        await cryptoWaitReady();
        const seed = await loadOrCreateSeed(filePath);
        const keyring = new Keyring({ type: 'sr25519', ss58Format: 42 });
        return keyring.addFromSeed(seed, { name: keyringName }, 'sr25519');
      })();
    }
    try {
      return await cached;
    } catch (e) {
      cached = null; // Don't cache a failed attempt — the next call should retry fresh.
      throw e;
    }
  }

  return { getOrCreate, reset: () => { cached = null; } };
}

const _citizenKeypair = createKeystoreBackedKeypairGetter(WALLET_FILE_PATH, 'agora-keystore');
const _delegatePersonaKeypair = createKeystoreBackedKeypairGetter(
  DELEGATE_PERSONA_WALLET_FILE_PATH,
  'agora-delegate-persona',
);

/** Test-only escape hatch — production code never needs to reset this within a process lifetime. */
export function _resetCachedKeystoreKeypairForTests(): void {
  _citizenKeypair.reset();
  _delegatePersonaKeypair.reset();
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
  return _citizenKeypair.getOrCreate();
}

/**
 * Returns the citizen's second, independent delegate-persona `KeyringPair` (generating one on
 * first call) — see this module's doc comment for why it is a separate random seed rather than
 * derived from the citizen wallet's. Same availability/caching contract as
 * {@link getOrCreateKeystoreKeypair}: throws if {@link isKeystoreSigningAvailable} is `false`,
 * and is cached for the life of the process.
 */
export async function getOrCreateDelegatePersonaKeypair(): Promise<KeyringPair> {
  return _delegatePersonaKeypair.getOrCreate();
}
