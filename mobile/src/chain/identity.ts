/**
 * Identity pallet integration (crate `pallet-identity-zk`, runtime module
 * name `Identity` at pallet index 8 — see runtime/src/lib.rs — so the
 * @polkadot/api section is `api.query.identity` / `api.tx.identity`).
 *
 * Passport NFC scan + on-device ZKPassport ZK proof generation is NOT
 * implemented yet (RegisterScreen.tsx still has the scan/liveness/proving
 * steps stubbed out with TODOs; see `zkProving.ts` for the proving seam that
 * exists in place of a real native prover). That's a separate gap from
 * *signing key custody*, which this module owns: `getSigningKeypair()` below
 * now sources its `KeyringPair` from `keystoreWallet.ts` (Android Keystore-
 * wrapped seed, see that module and `../native/keystoreSigner.ts` for the
 * honest accounting of what "hardware-backed" covers) whenever that's
 * available — real signing key custody, not the old dev mnemonic. The
 * `DEV_ONLY_MNEMONIC` path below still exists, but only as a `__DEV__`-gated
 * fallback for when the Keystore module isn't available (e.g. iOS, which has
 * no equivalent yet, or a JS-only tooling context with no native module
 * linked) — see `resolveSigningKeypair()`. It is unreachable whenever
 * `__DEV__` is false, i.e. in any release build, regardless of platform.
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
 *
 * # A 4th extrinsic: `submit_oprf_query`
 *
 * `submitOprfQuery` below wraps a 4th pallet-identity extrinsic, `submit_oprf_query`
 * (call index 15) — changelog entry 82's on-chain OPRF mailbox, replacing an earlier
 * relay-server design. It doesn't share the other three's `zk_proof`/`public_inputs`
 * shape (it takes a single `blinded_query: [u8; 64]`), but it's the same kind of thing:
 * a validated, direct pallet-identity extrinsic wrapper, so it lives here alongside them
 * rather than in a new file. `resolveCommitteeMemberIndex`, next to it, is not an
 * extrinsic at all — it's a read-only lookup against `CommitteeMembers` storage, needed
 * to translate an `AccountId` into the 1-based DKG roster position that round1/round2
 * OPRF committee responses are keyed by (see that function's doc comment).
 */
import { Keyring } from '@polkadot/keyring';
import { KeyringPair } from '@polkadot/keyring/types';
import { blake2AsHex, cryptoWaitReady } from '@polkadot/util-crypto';
import { getApi } from './api';
import { getOrCreateKeystoreKeypair } from './keystoreWallet';
import { isKeystoreSigningAvailable } from '../native/keystoreSigner';
import { assertValidPublicInputs } from './proofEncoding';
import { submitExtrinsic } from './submitExtrinsic';

// DEV-ONLY — this is the well-known Substrate test mnemonic. Every install
// that hits this path derives the SAME keypair from it. It exists purely as
// a `__DEV__`-gated fallback (see `resolveSigningKeypair` below) for when the
// real Android Keystore-backed path (`keystoreWallet.ts`) isn't available —
// e.g. iOS, which has no equivalent native module yet, or a JS-only tooling
// context. Never reachable when `__DEV__` is false: `resolveSigningKeypair`
// throws instead of falling back to this in that case. Never use this for
// anything beyond local development against a --dev chain.
const DEV_ONLY_MNEMONIC =
  'bottom drive obey lake curtain smoke basket hold race lonely fit walk';

let _devPairPromise: Promise<KeyringPair> | null = null;

function devKeyringPair(): Promise<KeyringPair> {
  if (!_devPairPromise) {
    _devPairPromise = (async () => {
      await cryptoWaitReady();
      const keyring = new Keyring({ type: 'sr25519', ss58Format: 42 });
      return keyring.addFromUri(DEV_ONLY_MNEMONIC, { name: 'agora-dev' });
    })();
  }
  return _devPairPromise;
}

/**
 * Picks the real signing keypair: the Android Keystore-backed one
 * (`keystoreWallet.ts`) whenever it's available — which takes priority even
 * in a `__DEV__` build, so dev/prod parity holds on any real Android device
 * — falling back to the `DEV_ONLY_MNEMONIC` keypair only when it isn't
 * *and* `__DEV__` is true. Outside of `__DEV__` (any release build, any
 * platform), an unavailable Keystore path is a hard error, never a silent
 * fallback to the dev mnemonic — this is the gate that keeps the dev
 * mnemonic from ever reaching a release build, following the same `__DEV__`
 * convention `api.ts`'s `getApi()` and `RegisterScreen.tsx`'s test-passport
 * button already use elsewhere in this app.
 */
async function resolveSigningKeypair(): Promise<KeyringPair> {
  if (isKeystoreSigningAvailable()) {
    return getOrCreateKeystoreKeypair();
  }
  if (__DEV__) {
    return devKeyringPair();
  }
  throw new Error(
    'resolveSigningKeypair: no hardware-backed signing key is available on this platform/build ' +
      '(Android Keystore module not linked), and the DEV_ONLY_MNEMONIC fallback is disabled ' +
      'outside __DEV__. Refusing to sign.',
  );
}

/**
 * Returns a real, working KeyringPair — Android Keystore-backed when
 * available, a `__DEV__`-only fallback otherwise (see `resolveSigningKeypair`
 * above) — plus a nullifier hash.
 *
 * The returned `nullifierHash` is a PLACEHOLDER, not a real ZKPassport
 * scoped nullifier. The real one is `scoped_nullifier`, a public input the
 * outer ZKPassport proof itself exposes (see `proofEncoding.ts`'s
 * `ZkPassportOuterPublicInputs.scopedNullifier`), computed on-device during
 * proving — that flow doesn't exist yet (see RegisterScreen.tsx TODOs). The
 * value below is just blake2(keypair.publicKey), so it's deterministic per
 * keypair for testing flows like AuthScreen's QR POST body, but it will NOT
 * match whatever CitizenNullifier actually stores on-chain for this address,
 * and must not be treated as a real identity proof.
 */
export async function getSigningKeypair(): Promise<{ keypair: KeyringPair; nullifierHash: string }> {
  const keypair = await resolveSigningKeypair();
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

/**
 * Parameters for `recover_account` (call index 20, `pallets/pallet-identity/src/lib.rs`):
 * same shape as `ReverifyCitizenParams` (a fresh outer proof re-establishing the caller
 * still holds the passport behind an existing registration) plus `backingCommitment`,
 * which must equal the *old* account's on-file `BackingCommitment` — mirrors
 * `reverify_citizen`'s own `backing_commitment` parameter, see that extrinsic's doc
 * comment for why it rides alongside `anchor` rather than being re-derived on-chain.
 *
 * Unlike `reverify_citizen`, the caller signing this extrinsic is a *different* account
 * than the one the registration currently lives under — that old account is looked up
 * on-chain via the proof's own nullifier, never supplied here directly.
 */
export interface RecoverAccountParams extends OuterProofPayload {
  anchor: Uint8Array;
  oprfPkHashes: OprfCommitteeKeyHashes;
  backingCommitment: Uint8Array;
}

/**
 * Submits `recover_account` (call index 20): rebinds an existing citizen's
 * pallet-identity registration from its old, presumably-lost `AccountId` onto the
 * caller's new one, given a proof that the caller holds the same passport the old
 * registration was built from. See `pallets/pallet-identity/src/lib.rs`'s doc comment on
 * the extrinsic for the exact storage items rebound and the (deliberate) absence of a
 * dispute window.
 *
 * IMPORTANT — known gap this wrapper does NOT close (see `docs/project/pallets/identity.md`
 * and `docs/project/next-steps.md`): `recover_account` only rebinds pallet-identity's own
 * per-account storage. It does not move the old account's AGR balance, pallet-voting
 * budget/delegations, pallet-elections delegate registration/backing, a legislature seat,
 * a cabinet role, or pallet-anticorruption disclosures — all of that silently stays on the
 * old, now-unusable account. Any UI calling this must disclose that plainly before the
 * citizen confirms, not just document it here — see `RecoverAccountScreen.tsx`.
 */
export async function recoverAccount(params: RecoverAccountParams): Promise<void> {
  assertValidPublicInputs(params.outerCount, params.publicInputs);
  assertValidAnchor(params.anchor, 'recoverAccount');
  assertValidOprfCommitteeKeyHashes(params.oprfPkHashes, 'recoverAccount');
  assertValidAnchor(params.backingCommitment, 'recoverAccount (backingCommitment)');

  const api = await getApi();
  const { keypair } = await getSigningKeypair();
  return submitExtrinsic(
    api.tx.identity.recoverAccount(
      params.zkProof,
      params.publicInputs,
      params.anchor,
      params.oprfPkHashes,
      params.backingCommitment,
    ),
    keypair,
  );
}

/**
 * Submits `submit_oprf_query(origin, blinded_query: [u8; 64])` (call index 15,
 * `pallets/pallet-identity/src/lib.rs`) — posts a citizen's blinded OPRF query to the
 * on-chain mailbox (changelog entry 82), which the query's target committee slot
 * (derived off-chain via `committee_slot_for`) polls and answers via
 * `submit_oprf_round1`/`submit_oprf_round2`.
 *
 * Validates `blindedQuery`'s length locally (mirrors `assertValidAnchor`/
 * `assertValidOprfCommitteeKeyHashes` above — fail before ever touching the network)
 * rather than letting a malformed call hit the chain's own decode failure.
 *
 * The pallet assigns and emits the query's id via `Event::OprfQuerySubmitted { query_id,
 * submitter }` — it is not derivable locally, so this reads it back out of the finalized
 * extrinsic's own events, the same way `voting.ts`'s `submitProposal` reads back
 * `ProposalCreated.id`. Returned as a decimal string, not a `u64`/`BN`/`number` — the
 * same JSON-safe convention `registrationState.ts`'s `queryId` fields already use, since
 * this value is meant to be persisted there.
 */
export async function submitOprfQuery(blindedQuery: Uint8Array): Promise<{ queryId: string }> {
  if (blindedQuery.length !== 64) {
    throw new RangeError(`submitOprfQuery: blindedQuery is ${blindedQuery.length} bytes, expected 64`);
  }

  const api = await getApi();
  const { keypair } = await getSigningKeypair();

  let queryId: string | null = null;
  await submitExtrinsic(api.tx.identity.submitOprfQuery(blindedQuery), keypair, {
    onEvents: (events) => {
      for (const { event } of events) {
        if (api.events.identity.OprfQuerySubmitted.is(event)) {
          queryId = (event.data as any).queryId.toString();
        }
      }
    },
  });

  if (queryId === null) {
    throw new Error(
      'submitOprfQuery: extrinsic finalized without ever emitting identity.OprfQuerySubmitted — ' +
        'cannot report a query id',
    );
  }
  return { queryId };
}

/**
 * Resolves `member`'s (an SS58 address) 1-based position in `CommitteeMembers[committeeSlot]`
 * — pallet-identity's `StorageMap<u8, BoundedVec<AccountId, MaxCommitteeSize>>` roster of
 * which accounts belong to which OPRF committee slot (`pallets/pallet-identity/src/lib.rs`,
 * "OPRF committee on-chain mailbox" section).
 *
 * This is the DKG party index a round1/round2 OPRF response is keyed by, per
 * `committee-node/README.md`'s own documented convention: "`MEMBER_INDEX` (this node's
 * 1-based `CommitteeMembers[slot]` roster position — the DKG party index; getting it wrong
 * fails silently downstream, see its doc comment)". 1-based (not 0-based) because that's
 * what the threshold-crypto Lagrange-interpolation math the DKG party index eventually
 * feeds requires — party index 0 is not a valid participant.
 *
 * Pure read-only data lookup, no cryptography — implemented for real, not a stub. Throws if
 * `member` is not found in the roster: an out-of-band round1/round2 response for a non-member
 * indicates corrupted chain state or a bug elsewhere in the caller, not a condition to handle
 * silently by e.g. returning `-1` or `0`.
 */
export async function resolveCommitteeMemberIndex(committeeSlot: number, member: string): Promise<number> {
  const api = await getApi();
  const roster = await api.query.identity.committeeMembers(committeeSlot);
  const addresses = Array.from(roster as unknown as Iterable<{ toString(): string }>).map((entry) =>
    entry.toString(),
  );
  const position = addresses.indexOf(member);
  if (position === -1) {
    throw new Error(
      `resolveCommitteeMemberIndex: ${member} is not a member of CommitteeMembers[${committeeSlot}] ` +
        `(roster has ${addresses.length} member(s)) — this indicates corrupted chain state or a bug ` +
        'elsewhere, not a case to handle silently',
    );
  }
  return position + 1;
}
