/**
 * Identity pallet integration (crate `pallet-identity-zk`, runtime module
 * name `Identity` at pallet index 8 — see runtime/src/lib.rs — so the
 * @polkadot/api section is `api.query.identity` / `api.tx.identity`).
 *
 * Passport NFC scan + on-device ZKPassport ZK proof generation is NOT
 * implemented yet (RegisterScreen.tsx still has the scan/liveness/proving
 * steps stubbed out with TODOs; see `zkProving.ts` for the proving seam that
 * exists in place of a real native prover). Consequently getSigningKeypair()
 * below derives a DEV-ONLY keypair from a fixed, publicly-known mnemonic
 * instead of a hardware-backed key — this is not fit for anything but local
 * development against a --dev chain, matching how the rest of this codebase
 * marks dev/passthrough pieces (e.g. PassthroughMACIVerifier in
 * runtime/src/configs/mod.rs). Real key custody must live in iOS Secure
 * Enclave / Android Keystore per CLAUDE.md's Identity System section.
 *
 * # The three identity extrinsics, and why their shapes are all here
 *
 * `register_citizen`, `reverify_citizen`, and `migrate_oprf_scheme`
 * (`pallets/pallet-identity/src/lib.rs`) were restructured across HANDOFF
 * logs #75/#76 to all take the same kind of thing: a fresh outer ZKPassport
 * proof (`zk_proof`/`public_inputs`, verified via `T::ZkVerifier::verify`),
 * plus the OPRF identity-anchor material the proof's `disclosure`/
 * `migrate-disclosure` subproof authenticates (`anchor`/`oprf_pk_hashes`, or
 * `new_anchor`/`old_oprf_pk_hashes`/`new_oprf_pk_hashes` for migration). None
 * of the three take a bare, standalone anchor-proof blob any more — log #76's
 * finding was that doing so left `comm_in` an unauthenticated private
 * witness, the same flaw log #75 had already fixed for registration.
 *
 * This file previously (pre-log-#76, and in fact pre-log-#75 too) modeled
 * `register_citizen` on Rarimo's old 5-signal Groth16 layout
 * (`dg15PubKeyHash`, `passportHash`, `dg1Commitment`, `pkIdentityHash`,
 * `slaveMerkleRoot`) and took only 2 arguments — stale against both
 * restructurings. Nothing else in this codebase imported
 * `ZkRegistration`/the old `registerCitizen` shape (checked before
 * rewriting — grep across `mobile/src` turns up no other reference), so
 * there was no caller to preserve compatibility with.
 *
 * Producing a real `anchor`/`oprf_pk_hashes` value requires a live OPRF
 * committee (see HANDOFF logs #73-77) — no such service exists anywhere in
 * this project yet, so callers of the functions below must supply that
 * material from wherever that eventually lives; this module only validates,
 * encodes, and submits it.
 */
import { Keyring } from '@polkadot/keyring';
import { KeyringPair } from '@polkadot/keyring/types';
import { blake2AsHex, cryptoWaitReady } from '@polkadot/util-crypto';
import { getApi } from './api';
import { assertValidPublicInputs } from './proofEncoding';
import { submitExtrinsic } from './submitExtrinsic';

// DEV-ONLY — this is the well-known Substrate test mnemonic. Every install of
// this build derives the SAME keypair from it. It exists purely so the
// already-wired chain-calling code (voting.ts, constitution.ts, courts.ts,
// governance.ts) has a real KeyringPair with a working .sign() to exercise
// end-to-end before hardware-backed key custody exists. Never use this for
// anything beyond a local --dev chain.
const DEV_ONLY_MNEMONIC =
  'bottom drive obey lake curtain smoke basket hold race lonely fit walk';

let _pairPromise: Promise<KeyringPair> | null = null;

function devKeyringPair(): Promise<KeyringPair> {
  if (!_pairPromise) {
    _pairPromise = (async () => {
      await cryptoWaitReady();
      const keyring = new Keyring({ type: 'sr25519', ss58Format: 42 });
      return keyring.addFromUri(DEV_ONLY_MNEMONIC, { name: 'agora-dev' });
    })();
  }
  return _pairPromise;
}

/**
 * Returns a real, working KeyringPair (DEV-ONLY — see module doc) plus a
 * nullifier hash.
 *
 * The returned `nullifierHash` is a PLACEHOLDER, not a real ZKPassport
 * scoped nullifier. The real one is `scoped_nullifier`, a public input the
 * outer ZKPassport proof itself exposes (see `proofEncoding.ts`'s
 * `ZkPassportOuterPublicInputs.scopedNullifier`), computed on-device during
 * proving — that flow doesn't exist yet (see RegisterScreen.tsx TODOs). The
 * value below is just blake2(keypair.publicKey), so it's deterministic per
 * dev keypair for testing flows like AuthScreen's QR POST body, but it will
 * NOT match whatever CitizenNullifier actually stores on-chain for this
 * address, and must not be treated as a real identity proof.
 */
export async function getSigningKeypair(): Promise<{ keypair: KeyringPair; nullifierHash: string }> {
  const keypair = await devKeyringPair();
  const nullifierHash = blake2AsHex(keypair.publicKey, 256);
  return { keypair, nullifierHash };
}

/** Real query against pallet-identity's CitizenNullifier storage map. */
export async function isCitizen(address: string): Promise<boolean> {
  const api = await getApi();
  const nullifier = await api.query.identity.citizenNullifier(address);
  return (nullifier as any).isSome;
}

/**
 * Number of independent OPRF committees. Mirrors
 * `pallet_identity_zk::NUM_COMMITTEES = 5` (changelog entry 73's
 * 5-committee topology) — must stay in lockstep with that constant, the same
 * way this file's other mirrored constants (in `proofEncoding.ts`) already
 * do for `verifier.rs`.
 */
export const NUM_OPRF_COMMITTEES = 5;

/**
 * Exactly `NUM_OPRF_COMMITTEES` 32-byte OPRF committee public-key hashes,
 * one per committee slot `0..NUM_OPRF_COMMITTEES`, in slot order. Checked
 * on-chain against the governance-approved `OprfCommitteeKeys` allowlist
 * (`check_committee_keys`) — see `pallets/pallet-identity/src/lib.rs`.
 */
export type OprfCommitteeKeyHashes = readonly [
  Uint8Array,
  Uint8Array,
  Uint8Array,
  Uint8Array,
  Uint8Array,
];

/**
 * A fresh outer ZKPassport proof, ready to submit to `register_citizen`,
 * `reverify_citizen`, or `migrate_oprf_scheme` — all three take this exact
 * `zk_proof`/`public_inputs` pair (HANDOFF log #76 gave `reverify_citizen`/
 * `migrate_oprf_scheme` the same shape `register_citizen` already had).
 * Matches what `zkProving.ts`'s `proveRegistration`/`proveReverification`/
 * `proveMigration` return, but is defined independently here so a caller can
 * construct one without depending on that module (e.g. in tests).
 */
export interface OuterProofPayload {
  /** `zk_proof` — the envelope `encodeUltraHonkProof` produced. */
  zkProof: Uint8Array;
  /** `public_inputs` — 32-byte field elements, in the circuit's own order. */
  publicInputs: Uint8Array[];
  /**
   * Which `main/outer/count_N` variant produced the proof. Used only to
   * validate `publicInputs`'s length/shape before submission (via
   * `assertValidPublicInputs`) — not itself sent on-chain.
   */
  outerCount: number;
}

function assertValidAnchor(anchor: Uint8Array, label: string): void {
  if (anchor.length !== 32) {
    throw new RangeError(`${label}: anchor is ${anchor.length} bytes, expected 32`);
  }
}

function assertValidOprfCommitteeKeyHashes(
  hashes: OprfCommitteeKeyHashes,
  label: string,
): void {
  if (hashes.length !== NUM_OPRF_COMMITTEES) {
    throw new RangeError(
      `${label}: expected ${NUM_OPRF_COMMITTEES} OPRF committee key hashes, got ${hashes.length}`,
    );
  }
  hashes.forEach((hash, index) => {
    if (hash.length !== 32) {
      throw new RangeError(
        `${label}: OPRF committee key hash ${index} is ${hash.length} bytes, expected 32`,
      );
    }
  });
}

/**
 * Parameters for `register_citizen` (call index 0, HANDOFF log #75): a
 * validated ZKPassport outer proof plus the mandatory OPRF identity anchor
 * and the 5 per-committee key hashes it was derived under. There is no
 * separate anchor SNARK proof parameter — the `disclosure` circuit rides
 * inside `zkProof` itself as a recursively-verified subproof.
 */
export interface RegisterCitizenParams extends OuterProofPayload {
  anchor: Uint8Array;
  oprfPkHashes: OprfCommitteeKeyHashes;
}

/**
 * Submits `register_citizen` (call index 0). See `RegisterCitizenParams` for
 * the shape and `pallets/pallet-identity/src/lib.rs` for the extrinsic
 * itself. Validates `publicInputs` against `outerCount` and the anchor
 * material's lengths before ever touching the network, so a malformed
 * payload fails with a specific message instead of the chain's bare
 * `InvalidZKProof`/`CommitteeKeyMismatch`.
 */
export async function registerCitizen(params: RegisterCitizenParams): Promise<void> {
  assertValidPublicInputs(params.outerCount, params.publicInputs);
  assertValidAnchor(params.anchor, 'registerCitizen');
  assertValidOprfCommitteeKeyHashes(params.oprfPkHashes, 'registerCitizen');

  const api = await getApi();
  const { keypair } = await getSigningKeypair();
  return submitExtrinsic(
    api.tx.identity.registerCitizen(params.zkProof, params.publicInputs, params.anchor, params.oprfPkHashes),
    keypair,
  );
}

/**
 * Parameters for `reverify_citizen` (call index 6, HANDOFF log #76):
 * identical shape to `RegisterCitizenParams` — `anchor` must equal the
 * citizen's on-file anchor (`AnchorMismatch` is checked on-chain via
 * `CitizenAnchor`, not re-checked here), and `oprfPkHashes` are the 5
 * committee key hashes the fresh reverification proof was derived under.
 */
export type ReverifyCitizenParams = RegisterCitizenParams;

/**
 * Submits `reverify_citizen` (call index 6): proves the caller still holds a
 * currently-valid passport that recomputes to the anchor already on file,
 * pushing `ReverificationDeadline` forward. See
 * `pallets/pallet-identity/src/lib.rs`'s doc comment on the extrinsic for
 * the full rationale (log #76 — a standalone reverification proof's
 * `comm_in` would be an unauthenticated private witness, so this rides
 * inside a fresh outer proof exactly like registration does).
 */
export async function reverifyCitizen(params: ReverifyCitizenParams): Promise<void> {
  assertValidPublicInputs(params.outerCount, params.publicInputs);
  assertValidAnchor(params.anchor, 'reverifyCitizen');
  assertValidOprfCommitteeKeyHashes(params.oprfPkHashes, 'reverifyCitizen');

  const api = await getApi();
  const { keypair } = await getSigningKeypair();
  return submitExtrinsic(
    api.tx.identity.reverifyCitizen(params.zkProof, params.publicInputs, params.anchor, params.oprfPkHashes),
    keypair,
  );
}

/**
 * Parameters for `migrate_oprf_scheme` (call index 7, HANDOFF log #76).
 * Note `oldAnchor` is deliberately absent: unlike the pre-log-#76 shape, the
 * old anchor is no longer caller-supplied — the pallet reads it directly
 * from the caller's own `CitizenAnchor` entry, which is also what stops a
 * citizen from "migrating" using someone else's anchor value.
 */
export interface MigrateOprfSchemeParams extends OuterProofPayload {
  newAnchor: Uint8Array;
  oldOprfPkHashes: OprfCommitteeKeyHashes;
  newOprfPkHashes: OprfCommitteeKeyHashes;
}

/**
 * Submits `migrate_oprf_scheme` (call index 7): moves the caller's identity
 * anchor from their current on-file OPRF scheme version to the next one,
 * given a proof (the `migrate-disclosure` subproof folded into `zkProof`,
 * see HANDOFF log #76) that `old_anchor` (read on-chain) and `newAnchor`
 * were both derived from the same underlying passport value.
 */
export async function migrateOprfScheme(params: MigrateOprfSchemeParams): Promise<void> {
  assertValidPublicInputs(params.outerCount, params.publicInputs);
  assertValidAnchor(params.newAnchor, 'migrateOprfScheme');
  assertValidOprfCommitteeKeyHashes(params.oldOprfPkHashes, 'migrateOprfScheme (old)');
  assertValidOprfCommitteeKeyHashes(params.newOprfPkHashes, 'migrateOprfScheme (new)');

  const api = await getApi();
  const { keypair } = await getSigningKeypair();
  return submitExtrinsic(
    api.tx.identity.migrateOprfScheme(
      params.zkProof,
      params.publicInputs,
      params.newAnchor,
      params.oldOprfPkHashes,
      params.newOprfPkHashes,
    ),
    keypair,
  );
}
