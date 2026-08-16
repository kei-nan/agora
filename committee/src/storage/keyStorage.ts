/**
 * Secure key storage for the committee-duty app.
 *
 * Two distinct credentials, doing two different jobs (changelog 082, "Two distinct
 * credentials per member device"):
 *
 *  1. The member's chain account signing key — signs `submit_oprf_round1`/
 *     `submit_oprf_round2` (see `chain/oprfCommittee.ts`) and is checked against the
 *     on-chain committee roster (`committeeMembers`). This is the *same kind* of key
 *     `mobile/src/chain/identity.ts`'s `getSigningKeypair()` already manages for the
 *     citizen-facing app (a `KeyringPair`), so `devSigningKeypair()` below mirrors that
 *     function's approach exactly, rather than inventing a new storage pattern, per
 *     this task's brief.
 *  2. The OPRF secret key share itself — raw bytes, never a `KeyringPair`, used only as
 *     input to `CommitteeCrypto.round1`/`round2Response`. Nothing analogous exists elsewhere in
 *     this codebase yet (grepped: no Keychain/Keystore/SecureEnclave integration exists
 *     anywhere under `mobile/` either — `identity.ts`'s own doc comment says real key
 *     custody "must live in iOS Secure Enclave / Android Keystore" but that isn't built
 *     yet, only the dev-mnemonic stand-in is), so `devOprfSecretShare()` below is a new
 *     but *parallel* dev-only stand-in, built the same way, for the same reason.
 *
 * # DEV-ONLY — matches `identity.ts`'s own disclosure verbatim
 *
 * Both functions below are placeholders that exist purely so the rest of this app
 * (`chain/oprfCommittee.ts`, the duty screen) has real values to exercise end-to-end
 * before hardware-backed key custody exists. **Real key custody for both credentials
 * must move to iOS Secure Enclave / Android Keystore** (CLAUDE.md's Identity System
 * section) before this app handles a real committee member's real secret share.
 * Concretely, that means:
 *  - the signing key: generate/import into the platform keystore as a non-extractable
 *    key, sign challenges via the platform's signing API instead of holding a
 *    `Uint8Array` seed in JS memory at all;
 *  - the OPRF secret share: import as a non-extractable secret at DKG-ceremony time
 *    (out of scope here, see changelog 082 "Still open") and expose it to
 *    `CommitteeCrypto.evaluateQuery` only via whatever handle the real Wasm host
 *    binding ends up requiring (ideally never as a plain JS `Uint8Array` at all, if the
 *    platform API allows operating on it without exporting it).
 *
 * Never use either function below for anything beyond a local `--dev` chain.
 */
import { Keyring } from '@polkadot/keyring';
import { KeyringPair } from '@polkadot/keyring/types';
import { blake2AsU8a, cryptoWaitReady } from '@polkadot/util-crypto';

// DEV-ONLY — the well-known Substrate test mnemonic, same one `identity.ts` uses.
// Every install of this build derives the SAME keypair from it.
const DEV_ONLY_SIGNING_MNEMONIC =
  'bottom drive obey lake curtain smoke basket hold race lonely fit walk';

// DEV-ONLY — a fixed, publicly-known seed string. devOprfSecretShare() below hashes
// this to a 32-byte value so callers have a stable, deterministic "secret" to exercise
// evaluateQuery's call boundary against. This is NOT a real OPRF secret share (no DKG
// ceremony produced it) and must never be treated as one.
const DEV_ONLY_OPRF_SEED = 'agora-committee-dev-only-oprf-secret-share-seed';

let _signingPairPromise: Promise<KeyringPair> | null = null;

/**
 * DEV-ONLY. Returns the same well-known dev `KeyringPair` every time, derived from
 * {@link DEV_ONLY_SIGNING_MNEMONIC}. Mirrors `mobile/src/chain/identity.ts`'s
 * `devKeyringPair()` precisely — see that file's module doc for the fuller rationale,
 * which applies unchanged here.
 */
export function devSigningKeypair(): Promise<KeyringPair> {
  if (!_signingPairPromise) {
    _signingPairPromise = (async () => {
      await cryptoWaitReady();
      const keyring = new Keyring({ type: 'sr25519', ss58Format: 42 });
      return keyring.addFromUri(DEV_ONLY_SIGNING_MNEMONIC, { name: 'agora-committee-dev' });
    })();
  }
  return _signingPairPromise;
}

let _oprfSecretShare: Uint8Array | null = null;

/**
 * DEV-ONLY. Returns a fixed 32-byte placeholder "OPRF secret share" — deterministic
 * across calls and across installs of this build (it's a hash of a hardcoded string),
 * so it is safe to exercise `CommitteeCrypto.evaluateQuery` against but carries no
 * cryptographic meaning. A real value only exists after a founding-phase DKG ceremony
 * (changelog 082, "Still open" — out of scope for this task) and must be imported into
 * platform-secure storage at that time, never generated or held here.
 */
export async function devOprfSecretShare(): Promise<Uint8Array> {
  if (!_oprfSecretShare) {
    await cryptoWaitReady();
    const encoder = new TextEncoder();
    _oprfSecretShare = blake2AsU8a(encoder.encode(DEV_ONLY_OPRF_SEED), 256);
  }
  return _oprfSecretShare;
}
