//! Real `AnchorProofVerifier` for `pallet_identity_zk`'s OPRF identity-anchor checks — see
//! `circuits/oprf-identity-anchor/README.md`'s "What the Rust verifier still has to enforce"
//! and `docs/project/changelog/074.md`/`075.md`/`076.md` for the design this implements.
//!
//! # What is real here
//!
//! All three of [`Poseidon2AnchorVerifier::verify_registration_anchor`],
//! [`Poseidon2AnchorVerifier::verify_reverification`] and
//! [`Poseidon2AnchorVerifier::verify_migration`] do genuine cryptographic work, as of
//! changelog entry 76. Entry 75 landed only the first of these; entries 76's contribution is
//! extending the same pattern to the other two, which needed real circuit engineering, not
//! just Rust plumbing — see below.
//!
//! `reverify_citizen`/`migrate_oprf_scheme` (`pallets/pallet-identity/src/lib.rs`) used to
//! take only a bare `proof_bytes: BoundedVec<u8, ConstU32<4096>>` and hand it straight to
//! `AnchorVerifier`, with **no** `T::ZkVerifier::verify` call and no outer proof
//! `public_inputs` in scope at all. That was fine under the pre-entry-74 mental model (a
//! standalone anchor SNARK proof), but the README's own "why `disclosure` exists" section
//! already ruled that model out for the *registration* path — `comm_in` is a private
//! witness of the outer circuit, so a standalone anchor proof can't be bound to a genuine
//! passport proof on-chain, and a prover could pair a real outer proof with a `comm_in` of
//! their own invention. The exact same argument applies to reverification and migration, and
//! entry 75 flagged it precisely rather than wiring a check that couldn't actually perform
//! anything real. Closing it needed two things, both delivered in entry 76: (1)
//! `circuits/oprf-identity-anchor/migrate-disclosure`, a new outer-embedded circuit mirroring
//! `disclosure`'s relationship to `anchor` but for the dual old/new committee evaluation
//! `migrate` performs (reverification reuses `disclosure` itself — no new circuit needed,
//! since reverification is exactly "recompute the anchor and check it's still the one on
//! file", the same shape as registration); (2) the extrinsic surgery below, restructuring
//! `reverify_citizen`/`migrate_oprf_scheme` to accept the outer `zk_proof`/`public_inputs`,
//! run `T::ZkVerifier::verify` first, then recompute against `param_commitments`.
//!
//! # What each function actually does
//!
//! `pallet_identity_zk::register_citizen`/`reverify_citizen`/`migrate_oprf_scheme` each run
//! `T::ZkVerifier::verify(zk_proof, public_inputs)` (a real bb 5.0.0 pairing check, see
//! `crate::verifier`) *before* calling into this module — so by the time these functions run,
//! the `disclosure`/`migrate-disclosure` subproof folded into that same outer proof (see
//! `circuits/oprf-identity-anchor/README.md`) has already had its constraints checked,
//! including all 5 (or 10, for migration) committees' `verified_oprf` calls and the anchor
//! combination(s). No second SNARK/pairing check is needed here; each function only has to:
//!
//! 1. Recompute the relevant `param_commitment` from the tuple submitted with the extrinsic
//!    ([`calculate_param_commitment`] for registration/reverification,
//!    [`calculate_migration_param_commitment`] for migration — both via the
//!    [`poseidon2_bn254`] crate; see that crate's module docs for how its Poseidon2 port was
//!    validated against real `nargo`-produced output).
//! 2. Check the recomputed value against *every* `param_commitments[i]` the already-verified
//!    outer proof exposes (there can be more than one disclosure subproof; matching any one
//!    is sufficient — see `circuits/oprf-identity-anchor/README.md`'s public-input-layout
//!    table).
//!
//! These functions deliberately do **not** check the `oprf_pk_hashes` against a
//! governance-approved committee key, or `current_date` freshness — those are
//! chain-storage-dependent checks that `pallet_identity_zk`'s extrinsics perform directly
//! (mirroring how they already check `AllowedMerkleRoots` themselves rather than delegating
//! that to `T::ZkVerifier`), using the `OprfCommitteeKeys` storage. Keeping these functions
//! pure (no storage reads) is also what makes them cleanly unit-testable below with plain
//! fixtures.
//!
//! # `backing_commitment`
//!
//! [`calculate_param_commitment`]/[`check_registration_anchor`] additionally recompute and
//! check a `backing_commitment` value, folded into the same `param_commitment` preimage
//! `anchor`/`scheme_version`/`oprf_pk_hashes`/`bound_account` occupy — no new proof-type tag of
//! its own, since (unlike the delegate-persona commitment, and unlike `bound_account` below) it
//! binds to no specific target/delegate and carries no front-running risk. It is the Poseidon2
//! hash of a private `backing_root_secret` the citizen's wallet derives via the same
//! committee-OPRF-blinding construction as `anchor` itself (see
//! `circuits/oprf-identity-anchor/lib/identity-anchor/src/lib.nr`'s `derive_backing_root_term`/
//! `derive_backing_commitment`) — safe to store on-chain per citizen
//! (`pallet_identity_zk::BackingCommitment`) as a future Merkle-tree leaf, because unlike a
//! bare hash of `identity_input` it does not admit the personal-number brute-force attack a
//! low-entropy preimage would.
//!
//! # `bound_account` — the identity-hijack fix
//!
//! **The problem this closes.** `register_citizen` and `recover_account` verify a ZK proof and
//! check `anchor`/`oprf_pk_hashes`/`backing_commitment` against the proof's `param_commitments`
//! — but until this fix, nothing constrained `who` (the transaction's signer). All proof
//! material is plaintext in a pending signed extrinsic, so an attacker could copy a victim's
//! pending extrinsic verbatim into their own signed call and get it mined first: for
//! `register_citizen` this stole the victim's citizenship slot; for `recover_account` (which
//! has no dispute window, by design — see that call's own doc comment) this would have
//! **permanently and irreversibly hijacked the victim's identity**.
//!
//! **The fix.** `disclosure`/`migrate-disclosure` (`circuits/oprf-identity-anchor/`) now fold a
//! `bound_account: [u8; 32]` private witness into their own `param_commitment`, exactly the way
//! `pallet_elections::register_as_delegate`'s `delegate-persona` circuit already folds in
//! `persona_account` (see that circuit's doc comment, and
//! [`calculate_delegate_param_commitment`]/[`check_delegate_persona`] below, for the pattern
//! this mirrors). Because `param_commitment` is itself one of the outer proof's
//! cryptographically-verified public inputs, a copied proof resubmitted against a different
//! `bound_account` no longer recomputes to any `param_commitments[i]` the outer proof exposes —
//! [`check_registration_anchor`]/[`check_migration_anchor`] below fail even though the outer
//! proof's own pairing check still passes. `pallets/pallet-identity/src/lib.rs`'s
//! `register_citizen`/`reverify_citizen`/`recover_account`/`migrate_oprf_scheme` each take a
//! `bound_account: T::AccountId` argument and assert `who == bound_account` before calling into
//! this module (a redundant-but-good outer check on top of the real circuit-level binding,
//! mirroring `register_as_delegate`'s `ensure!(who == persona_account, ...)`), and pass
//! `bound_account`'s raw bytes through to the widened `T::AnchorVerifier` calls below.
//!
//! **Exploitability, precisely.** `register_citizen` and `recover_account` were genuinely
//! exploitable this way: neither reads any caller-keyed on-chain state before accepting the
//! submitted `anchor`/`oprf_pk_hashes`/`backing_commitment` tuple, so a copied extrinsic
//! resubmitted under an attacker's account would have passed every check. `reverify_citizen`
//! and `migrate_oprf_scheme` were **not** independently exploitable the same way, even before
//! this fix: both read `old_anchor`/`scheme_version` from the **caller's own** on-chain
//! `CitizenAnchor` storage first (see `pallets/pallet-identity/src/lib.rs`), so a copied
//! extrinsic resubmitted under an attacker's account gets the attacker's own stored values
//! substituted in, which won't match the victim's embedded `param_commitment` unless the
//! attacker already owns the victim's exact anchor — a precondition that defeats the point of
//! attacking in the first place. The fix is applied to those two calls anyway, for consistency
//! (cheap once the circuit-level pattern exists for `register_citizen`/`recover_account`, and
//! `reverify_citizen` shares `disclosure` — the same circuit `register_citizen` uses — so it
//! needed the widened call shape regardless).
//!
//! Proof-type tags 200 (unbound registration/reverification, 9-element) and 201 (unbound
//! migration, 15-element) are retired outright in favor of 203/204 — not run in parallel with
//! them — since no real citizen has ever registered on this chain (no OPRF committee has ever
//! existed to produce a real proof; see CLAUDE.md), so there is no migration path to preserve.
//! See `circuits/oprf-identity-anchor/disclosure/src/main.nr` and
//! `.../migrate-disclosure/src/main.nr` for the circuit-side half of this.

#![cfg(not(feature = "dev-mode"))]

extern crate alloc;

use alloc::vec::Vec;

/// BN254 scalar field modulus `r`, big-endian — duplicated from `crate::verifier` rather
/// than imported, since that constant is private to that module and this is a three-line
/// check not worth widening its visibility for.
const BN254_FR_MODULUS_BE: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

fn is_canonical_fr(value: &[u8; 32]) -> bool {
    *value < BN254_FR_MODULUS_BE
}

/// The number of outer-circuit public inputs that are not `param_commitments`:
/// `certificate_registry_root`, `circuit_registry_root`, `current_date`, `service_scope`,
/// `service_subscope`, `nullifier_type`, `scoped_nullifier`, `oprf_pk_hash`. Duplicated from
/// `crate::verifier::FIXED_PUBLIC_INPUT_COUNT` for the same reason as the modulus above.
const FIXED_PUBLIC_INPUT_COUNT: usize = 8;

/// Agora's proof-type tag for the **account-bound** registration/reverification parameter
/// commitment — must match `circuits/oprf-identity-anchor/disclosure/src/main.nr`'s
/// `PROOF_TYPE_AGORA_IDENTITY_ANCHOR_BOUND`. Supersedes the retired, unbound tag 200 outright
/// (see this module's top-of-file docs on `bound_account` for why there is no parallel unbound
/// path to maintain).
const PROOF_TYPE_AGORA_IDENTITY_ANCHOR_BOUND: u8 = 203;

/// Agora's proof-type tag for the **account-bound** migration parameter commitment — must
/// match `circuits/oprf-identity-anchor/migrate-disclosure/src/main.nr`'s
/// `PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE_BOUND`. Deliberately distinct from
/// [`PROOF_TYPE_AGORA_IDENTITY_ANCHOR_BOUND`] so a migration commitment (17 elements) can never
/// be confused with a registration one (11 elements). Supersedes the retired, unbound tag 201.
const PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE_BOUND: u8 = 204;

/// Number of OPRF committees — must match
/// `circuits/oprf-identity-anchor/lib/identity-anchor`'s `NUM_COMMITTEES` (changelog entry
/// 73's 5-independent-committee design).
pub const NUM_COMMITTEES: usize = 5;

fn u32_to_field_bytes(value: u32) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[28..32].copy_from_slice(&value.to_be_bytes());
    bytes
}

/// Splits a raw 32-byte value (an `AccountId`) into the same two BN254-field-safe limbs every
/// account-binding circuit below derives via `utils::pack_be_bytes_into_fields::<32, 2,
/// 31>(account)` — `(lo, hi)` where `hi` holds `account[0]` alone (so it is always canonical: a
/// single byte is always `< r`) and `lo` holds `account[1..32]` (31 bytes, also always
/// canonical: 31 bytes is 248 bits, strictly below BN254's ~254-bit modulus). An arbitrary
/// 32-byte value cannot always be represented as one canonical `Field` on its own — BN254's
/// modulus is a little under `2^254` — hence the split, matching how `SaltedValue<[u8;
/// N]>::get_hash` in ZKPassport's own `utils` crate already packs arbitrary byte arrays for
/// Poseidon2 hashing. Shared by [`calculate_param_commitment`],
/// [`calculate_migration_param_commitment`] and [`calculate_delegate_param_commitment`] — all
/// three account-binding circuits use the identical packing.
fn account_to_field_limbs(account: [u8; 32]) -> ([u8; 32], [u8; 32]) {
    let mut hi = [0u8; 32];
    hi[31] = account[0];
    let mut lo = [0u8; 32];
    lo[1..32].copy_from_slice(&account[1..32]);
    (lo, hi)
}

/// `Poseidon2(PROOF_TYPE_AGORA_IDENTITY_ANCHOR_BOUND, anchor, scheme_version, oprf_pk_hashes[0],
/// .., oprf_pk_hashes[4], backing_commitment, bound_account_lo, bound_account_hi)` — an
/// 11-element hash, matching `disclosure::calculate_param_commitment` field-for-field and
/// argument-for-argument (including the `[lo, hi]` limb order — see
/// [`account_to_field_limbs`]).
pub fn calculate_param_commitment(
    anchor: [u8; 32],
    scheme_version: u32,
    oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
    backing_commitment: [u8; 32],
    bound_account: [u8; 32],
) -> [u8; 32] {
    let mut tag = [0u8; 32];
    tag[31] = PROOF_TYPE_AGORA_IDENTITY_ANCHOR_BOUND;
    let (lo, hi) = account_to_field_limbs(bound_account);

    let mut input: Vec<[u8; 32]> = Vec::with_capacity(6 + NUM_COMMITTEES);
    input.push(tag);
    input.push(anchor);
    input.push(u32_to_field_bytes(scheme_version));
    input.extend_from_slice(&oprf_pk_hashes);
    input.push(backing_commitment);
    input.push(lo);
    input.push(hi);

    poseidon2_bn254::hash_bytes(&input)
}

/// The pure, storage-free half of the registration-anchor check — see the module docs for
/// what this does and does not cover. `outer_public_inputs` is the same `public_inputs`
/// array `register_citizen` already validated via `T::ZkVerifier::verify`. `bound_account` is
/// the account this proof authorizes to submit it — see the module's `bound_account` docs.
pub fn check_registration_anchor(
    outer_public_inputs: &[[u8; 32]],
    anchor: [u8; 32],
    scheme_version: u32,
    oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
    backing_commitment: [u8; 32],
    bound_account: [u8; 32],
) -> bool {
    // A `count_N` outer proof always exposes at least one param_commitment (register_citizen
    // already enforces public_inputs.len() >= 9, but this function is also unit-tested
    // standalone, so it re-checks rather than assuming that invariant holds).
    if outer_public_inputs.len() <= FIXED_PUBLIC_INPUT_COUNT {
        return false;
    }

    if !is_canonical_fr(&anchor) {
        return false;
    }
    for pk_hash in oprf_pk_hashes.iter() {
        if !is_canonical_fr(pk_hash) {
            return false;
        }
    }
    if !is_canonical_fr(&backing_commitment) {
        return false;
    }
    // Defense-in-depth, mirroring check_delegate_persona's own zero-account rejection: the
    // circuit already asserts bound_account != 0, so a genuine proof can never carry a zero
    // value here, but rejecting it on the Rust side too costs nothing and removes any reliance
    // on that circuit-side assertion alone.
    if bound_account == [0u8; 32] {
        return false;
    }

    // param_commitments occupy indices 5..len-3 — see crate::verifier's module docs for the
    // full outer-circuit public-input table this mirrors.
    let param_commitments = &outer_public_inputs[5..outer_public_inputs.len() - 3];

    let recomputed = calculate_param_commitment(
        anchor,
        scheme_version,
        oprf_pk_hashes,
        backing_commitment,
        bound_account,
    );
    param_commitments.iter().any(|commitment| *commitment == recomputed)
}

/// `Poseidon2(PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE_BOUND, old_anchor, new_anchor,
/// old_scheme_version, new_scheme_version, old_oprf_pk_hashes[0..5], new_oprf_pk_hashes[0..5],
/// bound_account_lo, bound_account_hi)` — a 17-element hash, matching
/// `migrate_disclosure::calculate_param_commitment` field-for-field and argument-for-argument.
pub fn calculate_migration_param_commitment(
    old_anchor: [u8; 32],
    new_anchor: [u8; 32],
    old_scheme_version: u32,
    new_scheme_version: u32,
    old_oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
    new_oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
    bound_account: [u8; 32],
) -> [u8; 32] {
    let mut tag = [0u8; 32];
    tag[31] = PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE_BOUND;
    let (lo, hi) = account_to_field_limbs(bound_account);

    let mut input: Vec<[u8; 32]> = Vec::with_capacity(7 + 2 * NUM_COMMITTEES);
    input.push(tag);
    input.push(old_anchor);
    input.push(new_anchor);
    input.push(u32_to_field_bytes(old_scheme_version));
    input.push(u32_to_field_bytes(new_scheme_version));
    input.extend_from_slice(&old_oprf_pk_hashes);
    input.extend_from_slice(&new_oprf_pk_hashes);
    input.push(lo);
    input.push(hi);

    poseidon2_bn254::hash_bytes(&input)
}

/// The pure, storage-free half of the migration-anchor check — mirrors
/// [`check_registration_anchor`] but over `migrate-disclosure`'s wider commitment shape.
/// `outer_public_inputs` is the same `public_inputs` array `migrate_oprf_scheme` already
/// validated via `T::ZkVerifier::verify`.
pub fn check_migration_anchor(
    outer_public_inputs: &[[u8; 32]],
    old_anchor: [u8; 32],
    new_anchor: [u8; 32],
    old_scheme_version: u32,
    new_scheme_version: u32,
    old_oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
    new_oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
    bound_account: [u8; 32],
) -> bool {
    if outer_public_inputs.len() <= FIXED_PUBLIC_INPUT_COUNT {
        return false;
    }

    if !is_canonical_fr(&old_anchor) || !is_canonical_fr(&new_anchor) {
        return false;
    }
    for pk_hash in old_oprf_pk_hashes.iter().chain(new_oprf_pk_hashes.iter()) {
        if !is_canonical_fr(pk_hash) {
            return false;
        }
    }
    if bound_account == [0u8; 32] {
        return false;
    }

    let param_commitments = &outer_public_inputs[5..outer_public_inputs.len() - 3];

    let recomputed = calculate_migration_param_commitment(
        old_anchor,
        new_anchor,
        old_scheme_version,
        new_scheme_version,
        old_oprf_pk_hashes,
        new_oprf_pk_hashes,
        bound_account,
    );
    param_commitments.iter().any(|commitment| *commitment == recomputed)
}

/// Agora's proof-type tag for the delegate-persona parameter commitment — must match
/// `circuits/oprf-identity-anchor/delegate-persona/src/main.nr`'s
/// `PROOF_TYPE_AGORA_DELEGATE_PERSONA`. Deliberately distinct from both the registration tag
/// ([`PROOF_TYPE_AGORA_IDENTITY_ANCHOR_BOUND`], 203) and the migration tag
/// ([`PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE_BOUND`], 204) — see that circuit's module docs
/// for the full rationale and, more importantly,
/// `circuits/oprf-identity-anchor/lib/identity-anchor/src/lib.nr`'s
/// `derive_delegate_identity_input` doc comment for why delegate-persona creation is backed by
/// a genuinely separate OPRF query rather than a reuse of the registration anchor's evaluation.
const PROOF_TYPE_AGORA_DELEGATE_PERSONA: u8 = 202;

/// `Poseidon2(PROOF_TYPE_AGORA_DELEGATE_PERSONA, delegate_persona_id, persona_account_lo,
/// persona_account_hi, scheme_version, oprf_pk_hashes[0..5])` — a 10-element hash, matching
/// `delegate_persona::calculate_param_commitment` field-for-field and argument-for-argument
/// (including the `[lo, hi]` limb order — see [`account_to_field_limbs`]).
pub fn calculate_delegate_param_commitment(
    delegate_persona_id: [u8; 32],
    persona_account: [u8; 32],
    scheme_version: u32,
    oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
) -> [u8; 32] {
    let mut tag = [0u8; 32];
    tag[31] = PROOF_TYPE_AGORA_DELEGATE_PERSONA;
    let (lo, hi) = account_to_field_limbs(persona_account);

    let mut input: Vec<[u8; 32]> = Vec::with_capacity(5 + NUM_COMMITTEES);
    input.push(tag);
    input.push(delegate_persona_id);
    input.push(lo);
    input.push(hi);
    input.push(u32_to_field_bytes(scheme_version));
    input.extend_from_slice(&oprf_pk_hashes);

    poseidon2_bn254::hash_bytes(&input)
}

/// The pure, storage-free half of the delegate-persona check — mirrors
/// [`check_registration_anchor`], but recomputes `delegate-persona`'s wider (`persona_account`-
/// binding) commitment shape instead of `disclosure`'s. `outer_public_inputs` is expected to be
/// the same `public_inputs` array a future caller has already validated via
/// `T::ZkVerifier::verify` — this function performs no pairing check itself, deliberately
/// mirroring [`check_registration_anchor`]/[`check_migration_anchor`]'s split (see this module's
/// top-of-file docs for why the pairing check and the commitment recomputation are kept
/// separate). **Update, commit `786b792`**: `pallet_elections::register_as_delegate` now calls
/// this for real, via its `T::DelegatePersonaVerifier` config item (wired in
/// `runtime/src/configs/mod.rs` to [`Poseidon2AnchorVerifier`]'s `DelegatePersonaVerifier` impl
/// below) — `T::ZkVerifier::verify(zk_proof, public_inputs)` runs first, then this recomputation,
/// exactly as `register_citizen` already does for [`check_registration_anchor`], matching the
/// call shape `circuits/oprf-identity-anchor/delegate-persona/src/main.nr`'s module docs describe.
pub fn check_delegate_persona(
    outer_public_inputs: &[[u8; 32]],
    delegate_persona_id: [u8; 32],
    persona_account: [u8; 32],
    scheme_version: u32,
    oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
) -> bool {
    if outer_public_inputs.len() <= FIXED_PUBLIC_INPUT_COUNT {
        return false;
    }

    if !is_canonical_fr(&delegate_persona_id) {
        return false;
    }
    if persona_account == [0u8; 32] {
        return false;
    }
    for pk_hash in oprf_pk_hashes.iter() {
        if !is_canonical_fr(pk_hash) {
            return false;
        }
    }

    let param_commitments = &outer_public_inputs[5..outer_public_inputs.len() - 3];

    let recomputed = calculate_delegate_param_commitment(
        delegate_persona_id,
        persona_account,
        scheme_version,
        oprf_pk_hashes,
    );
    param_commitments.iter().any(|commitment| *commitment == recomputed)
}

/// Real `AnchorProofVerifier` for the non-dev-mode runtime. All three methods are genuinely
/// checked (see module docs). `verify_reverification` shares `verify_registration_anchor`'s
/// exact recomputation (`disclosure`'s `param_commitment` shape is the same for both — see
/// `circuits/oprf-identity-anchor/README.md`) since reverification is "recompute the anchor
/// and confirm it's still the one on file", not a structurally different check.
pub struct Poseidon2AnchorVerifier;

impl pallet_identity_zk::AnchorProofVerifier for Poseidon2AnchorVerifier {
    fn verify_registration_anchor(
        outer_public_inputs: &[[u8; 32]],
        anchor: [u8; 32],
        scheme_version: u32,
        oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
        backing_commitment: [u8; 32],
        bound_account: [u8; 32],
    ) -> bool {
        check_registration_anchor(
            outer_public_inputs,
            anchor,
            scheme_version,
            oprf_pk_hashes,
            backing_commitment,
            bound_account,
        )
    }

    fn verify_reverification(
        outer_public_inputs: &[[u8; 32]],
        anchor: [u8; 32],
        scheme_version: u32,
        oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
        backing_commitment: [u8; 32],
        bound_account: [u8; 32],
    ) -> bool {
        check_registration_anchor(
            outer_public_inputs,
            anchor,
            scheme_version,
            oprf_pk_hashes,
            backing_commitment,
            bound_account,
        )
    }

    fn verify_migration(
        outer_public_inputs: &[[u8; 32]],
        old_anchor: [u8; 32],
        new_anchor: [u8; 32],
        old_scheme_version: u32,
        new_scheme_version: u32,
        old_oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
        new_oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
        bound_account: [u8; 32],
    ) -> bool {
        check_migration_anchor(
            outer_public_inputs,
            old_anchor,
            new_anchor,
            old_scheme_version,
            new_scheme_version,
            old_oprf_pk_hashes,
            new_oprf_pk_hashes,
            bound_account,
        )
    }
}

/// Real `pallet_elections::DelegatePersonaVerifier` for the non-dev-mode runtime — the same
/// [`Poseidon2AnchorVerifier`] marker struct, extended with a fourth trait impl rather than a
/// new type, since it is genuinely the same recompute-and-check idiom over the same module's
/// [`check_delegate_persona`]. See `pallet_elections::register_as_delegate`'s doc comment for
/// how this is used: `T::ZkVerifier::verify` (the outer proof's pairing check) must already
/// have passed before this is called, exactly as `pallet_identity_zk::register_citizen` already
/// does for [`Poseidon2AnchorVerifier`]'s other three methods.
impl pallet_elections::DelegatePersonaVerifier for Poseidon2AnchorVerifier {
    fn check_delegate_persona(
        outer_public_inputs: &[[u8; 32]],
        delegate_persona_id: [u8; 32],
        persona_account: [u8; 32],
        scheme_version: u32,
        oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
    ) -> bool {
        check_delegate_persona(
            outer_public_inputs,
            delegate_persona_id,
            persona_account,
            scheme_version,
            oprf_pk_hashes,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal, well-shaped `count_4`-style public-input array: 8 fixed fields, 1
    /// `param_commitment` slot, matching `crate::verifier`'s documented layout.
    fn outer_public_inputs_with_commitment(commitment: [u8; 32]) -> Vec<[u8; 32]> {
        let mut v = alloc::vec![[0u8; 32]; 9];
        v[5] = commitment;
        // scoped_nullifier (index 6+D = 7 here) must be non-zero for a genuine proof, but
        // this function doesn't check it — only register_citizen does, elsewhere — so it's
        // left zero here to keep the fixture minimal.
        v
    }

    fn hex32(h: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        let bytes = (0..32)
            .map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap())
            .collect::<Vec<u8>>();
        out.copy_from_slice(&bytes);
        out
    }

    const ANCHOR: [u8; 32] = {
        let mut b = [0u8; 32];
        b[31] = 42;
        b
    };
    const SCHEME_VERSION: u32 = 1;
    const PK_HASHES: [[u8; 32]; NUM_COMMITTEES] = {
        let mut hashes = [[0u8; 32]; NUM_COMMITTEES];
        hashes[0][31] = 1;
        hashes[1][31] = 2;
        hashes[2][31] = 3;
        hashes[3][31] = 4;
        hashes[4][31] = 5;
        hashes
    };
    const BACKING_COMMITMENT: [u8; 32] = {
        let mut b = [0u8; 32];
        b[31] = 77;
        b
    };
    /// Arbitrary, non-zero fixture account for the small-int mutation-style tests below —
    /// deliberately distinct byte pattern from `delegate_persona_account()`'s `[1, 2, .., 32]`
    /// (used by the real-vector tests further down) so a test bug that accidentally swaps the
    /// two fixtures would be caught rather than silently passing.
    const BOUND_ACCOUNT: [u8; 32] = {
        let mut b = [0u8; 32];
        b[0] = 9;
        b[31] = 1;
        b
    };

    #[test]
    fn accepts_a_correctly_recomputed_commitment() {
        let commitment = calculate_param_commitment(
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        );
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        assert!(check_registration_anchor(
            &public_inputs,
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        ));
    }

    #[test]
    fn accepts_when_the_matching_commitment_is_not_the_first_slot() {
        // Two disclosure subproofs (D = 2): param_commitments at indices 5 and 6. The
        // anchor's commitment is the *second* one — check_registration_anchor must not
        // assume it's always index 5.
        let commitment = calculate_param_commitment(
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        );
        let mut public_inputs = alloc::vec![[0u8; 32]; 10];
        public_inputs[5] = [9u8; 32]; // an unrelated disclosure subproof's commitment
        public_inputs[6] = commitment;
        assert!(check_registration_anchor(
            &public_inputs,
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        ));
    }

    #[test]
    fn rejects_wrong_anchor() {
        let commitment = calculate_param_commitment(
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        );
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        let mut wrong_anchor = ANCHOR;
        wrong_anchor[0] ^= 1;
        assert!(!check_registration_anchor(
            &public_inputs,
            wrong_anchor,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        ));
    }

    #[test]
    fn rejects_wrong_scheme_version() {
        let commitment = calculate_param_commitment(
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        );
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        assert!(!check_registration_anchor(
            &public_inputs,
            ANCHOR,
            SCHEME_VERSION + 1,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        ));
    }

    #[test]
    fn rejects_a_single_mutated_pk_hash() {
        let commitment = calculate_param_commitment(
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        );
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        for i in 0..NUM_COMMITTEES {
            let mut mutated = PK_HASHES;
            mutated[i][0] ^= 1;
            assert!(
                !check_registration_anchor(
                    &public_inputs,
                    ANCHOR,
                    SCHEME_VERSION,
                    mutated,
                    BACKING_COMMITMENT,
                    BOUND_ACCOUNT,
                ),
                "mutating pk_hash slot {i} must be rejected",
            );
        }
    }

    #[test]
    fn rejects_wrong_backing_commitment() {
        // The whole point of folding backing_commitment into param_commitment: a caller cannot
        // resubmit a valid proof/public_inputs paired with a different claimed
        // backing_commitment and have it silently accepted.
        let commitment = calculate_param_commitment(
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        );
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        let mut wrong = BACKING_COMMITMENT;
        wrong[0] ^= 1;
        assert!(!check_registration_anchor(
            &public_inputs,
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            wrong,
            BOUND_ACCOUNT,
        ));
    }

    /// The whole point of folding `bound_account` into `param_commitment` (the identity-hijack
    /// fix this module's top-of-file docs describe): copying a victim's valid proof and
    /// resubmitting it against a *different* claimed `bound_account` must fail, not silently
    /// bind the proof to the new account. Mirrors
    /// `rejects_delegate_persona_with_a_swapped_persona_account` below.
    #[test]
    fn rejects_registration_with_a_swapped_bound_account() {
        let commitment = calculate_param_commitment(
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        );
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        let mut other_account = BOUND_ACCOUNT;
        other_account[1] ^= 1;
        assert!(!check_registration_anchor(
            &public_inputs,
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            other_account,
        ));
    }

    #[test]
    fn rejects_registration_with_zero_bound_account() {
        let commitment = calculate_param_commitment(
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        );
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        assert!(!check_registration_anchor(
            &public_inputs,
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            [0u8; 32],
        ));
    }

    #[test]
    fn rejects_when_no_param_commitment_matches() {
        let public_inputs = outer_public_inputs_with_commitment([7u8; 32]);
        assert!(!check_registration_anchor(
            &public_inputs,
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        ));
    }

    #[test]
    fn rejects_non_canonical_anchor() {
        let commitment = calculate_param_commitment(
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        );
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        assert!(!check_registration_anchor(
            &public_inputs,
            BN254_FR_MODULUS_BE,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        ));
    }

    #[test]
    fn rejects_non_canonical_backing_commitment() {
        let commitment = calculate_param_commitment(
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        );
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        assert!(!check_registration_anchor(
            &public_inputs,
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BN254_FR_MODULUS_BE,
            BOUND_ACCOUNT,
        ));
    }

    #[test]
    fn rejects_too_few_public_inputs() {
        assert!(!check_registration_anchor(
            &[[0u8; 32]; 8],
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES,
            BACKING_COMMITMENT,
            BOUND_ACCOUNT,
        ));
    }

    /// Real end-to-end vector, generated fresh for the `bound_account` widening: `nargo execute
    /// --package oprf_identity_anchor_disclosure` against `disclosure/Prover.toml` (a
    /// DEV-ONLY committee-simulator-backed fixture, with `bound_account = [1, 2, .., 32]`
    /// added), followed by a real `bb write_vk`/`bb prove`/`bb verify` round-trip that accepted
    /// the resulting proof (`target/bb-disclosure/public_inputs` in
    /// `circuits/oprf-identity-anchor/`).
    ///
    /// `anchor` and `backing_commitment` are unchanged from the pre-widening vector (confirmed
    /// byte-identical — `bound_account` does not affect their derivation, only
    /// `param_commitment`'s own preimage), read off `oprf_identity_anchor_migrate`'s own
    /// `old_anchor` output and cross-checked identical to what `disclosure`'s witness solves
    /// internally, since both share the same identity_input/oprf_proofs. `oprf_pk_hashes` are
    /// likewise unchanged. `param_commitment` is the real, freshly-regenerated `bb`-produced
    /// public output for the widened (tag-203, 11-element) shape — not the old tag-200 value.
    fn real_disclosure_anchor() -> [u8; 32] {
        hex32("0beff326e082ed177b5ad64c97336db7af826a90a072cdb18f65de2ac6d5326f")
    }

    fn real_disclosure_backing_commitment() -> [u8; 32] {
        hex32("221383d2793ff2aece98eeb39646dc8350d535ff862ffbb17ffa7eb137990571")
    }

    fn real_disclosure_param_commitment() -> [u8; 32] {
        hex32("2237299ee0cd8718848e31f976cf471aca278622d4043c38b39a1f022d778298")
    }

    #[test]
    fn calculate_param_commitment_matches_the_real_nargo_and_bb_vector() {
        let got = calculate_param_commitment(
            real_disclosure_anchor(),
            1,
            delegate_oprf_pk_hashes(),
            real_disclosure_backing_commitment(),
            delegate_persona_account(),
        );
        assert_eq!(got, real_disclosure_param_commitment());
    }

    #[test]
    fn accepts_the_real_nargo_and_bb_vector_via_check_registration_anchor() {
        // Same real vector as above, exercised through the full `check_registration_anchor`
        // path (canonicality checks + param_commitments scan), not just the raw hash.
        let public_inputs =
            outer_public_inputs_with_commitment(real_disclosure_param_commitment());
        assert!(check_registration_anchor(
            &public_inputs,
            real_disclosure_anchor(),
            1,
            delegate_oprf_pk_hashes(),
            real_disclosure_backing_commitment(),
            delegate_persona_account(),
        ));
    }

    // --- verify_migration / check_migration_anchor ---

    const OLD_ANCHOR: [u8; 32] = {
        let mut b = [0u8; 32];
        b[31] = 111;
        b
    };
    const NEW_ANCHOR: [u8; 32] = {
        let mut b = [0u8; 32];
        b[31] = 222;
        b
    };
    const OLD_SCHEME_VERSION: u32 = 1;
    const NEW_SCHEME_VERSION: u32 = 2;
    const OLD_PK_HASHES: [[u8; 32]; NUM_COMMITTEES] = {
        let mut hashes = [[0u8; 32]; NUM_COMMITTEES];
        hashes[0][31] = 10;
        hashes[1][31] = 20;
        hashes[2][31] = 30;
        hashes[3][31] = 40;
        hashes[4][31] = 50;
        hashes
    };
    const NEW_PK_HASHES: [[u8; 32]; NUM_COMMITTEES] = {
        let mut hashes = [[0u8; 32]; NUM_COMMITTEES];
        hashes[0][31] = 60;
        hashes[1][31] = 70;
        hashes[2][31] = 80;
        hashes[3][31] = 90;
        hashes[4][31] = 100;
        hashes
    };

    fn migration_commitment() -> [u8; 32] {
        calculate_migration_param_commitment(
            OLD_ANCHOR,
            NEW_ANCHOR,
            OLD_SCHEME_VERSION,
            NEW_SCHEME_VERSION,
            OLD_PK_HASHES,
            NEW_PK_HASHES,
            BOUND_ACCOUNT,
        )
    }

    #[test]
    fn accepts_a_correctly_recomputed_migration_commitment() {
        let commitment = migration_commitment();
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        assert!(check_migration_anchor(
            &public_inputs,
            OLD_ANCHOR,
            NEW_ANCHOR,
            OLD_SCHEME_VERSION,
            NEW_SCHEME_VERSION,
            OLD_PK_HASHES,
            NEW_PK_HASHES,
            BOUND_ACCOUNT,
        ));
    }

    #[test]
    fn rejects_wrong_old_anchor_in_migration() {
        let commitment = migration_commitment();
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        let mut wrong = OLD_ANCHOR;
        wrong[0] ^= 1;
        assert!(!check_migration_anchor(
            &public_inputs,
            wrong,
            NEW_ANCHOR,
            OLD_SCHEME_VERSION,
            NEW_SCHEME_VERSION,
            OLD_PK_HASHES,
            NEW_PK_HASHES,
            BOUND_ACCOUNT,
        ));
    }

    #[test]
    fn rejects_wrong_new_anchor_in_migration() {
        let commitment = migration_commitment();
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        let mut wrong = NEW_ANCHOR;
        wrong[0] ^= 1;
        assert!(!check_migration_anchor(
            &public_inputs,
            OLD_ANCHOR,
            wrong,
            OLD_SCHEME_VERSION,
            NEW_SCHEME_VERSION,
            OLD_PK_HASHES,
            NEW_PK_HASHES,
            BOUND_ACCOUNT,
        ));
    }

    #[test]
    fn rejects_swapped_scheme_versions_in_migration() {
        // old/new scheme_version are not interchangeable — swapping them must not verify.
        let commitment = migration_commitment();
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        assert!(!check_migration_anchor(
            &public_inputs,
            OLD_ANCHOR,
            NEW_ANCHOR,
            NEW_SCHEME_VERSION,
            OLD_SCHEME_VERSION,
            OLD_PK_HASHES,
            NEW_PK_HASHES,
            BOUND_ACCOUNT,
        ));
    }

    #[test]
    fn rejects_a_single_mutated_old_pk_hash_in_migration() {
        let commitment = migration_commitment();
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        for i in 0..NUM_COMMITTEES {
            let mut mutated = OLD_PK_HASHES;
            mutated[i][0] ^= 1;
            assert!(
                !check_migration_anchor(
                    &public_inputs,
                    OLD_ANCHOR,
                    NEW_ANCHOR,
                    OLD_SCHEME_VERSION,
                    NEW_SCHEME_VERSION,
                    mutated,
                    NEW_PK_HASHES,
                    BOUND_ACCOUNT,
                ),
                "mutating old_pk_hash slot {i} must be rejected",
            );
        }
    }

    #[test]
    fn rejects_a_single_mutated_new_pk_hash_in_migration() {
        let commitment = migration_commitment();
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        for i in 0..NUM_COMMITTEES {
            let mut mutated = NEW_PK_HASHES;
            mutated[i][0] ^= 1;
            assert!(
                !check_migration_anchor(
                    &public_inputs,
                    OLD_ANCHOR,
                    NEW_ANCHOR,
                    OLD_SCHEME_VERSION,
                    NEW_SCHEME_VERSION,
                    OLD_PK_HASHES,
                    mutated,
                    BOUND_ACCOUNT,
                ),
                "mutating new_pk_hash slot {i} must be rejected",
            );
        }
    }

    /// Mirrors `rejects_registration_with_a_swapped_bound_account` — the identity-hijack fix
    /// applies to migration too (see this module's top-of-file docs on why `migrate_oprf_scheme`
    /// is fixed for consistency even though it was not independently exploitable).
    #[test]
    fn rejects_migration_with_a_swapped_bound_account() {
        let commitment = migration_commitment();
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        let mut other_account = BOUND_ACCOUNT;
        other_account[1] ^= 1;
        assert!(!check_migration_anchor(
            &public_inputs,
            OLD_ANCHOR,
            NEW_ANCHOR,
            OLD_SCHEME_VERSION,
            NEW_SCHEME_VERSION,
            OLD_PK_HASHES,
            NEW_PK_HASHES,
            other_account,
        ));
    }

    #[test]
    fn rejects_migration_with_zero_bound_account() {
        let commitment = migration_commitment();
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        assert!(!check_migration_anchor(
            &public_inputs,
            OLD_ANCHOR,
            NEW_ANCHOR,
            OLD_SCHEME_VERSION,
            NEW_SCHEME_VERSION,
            OLD_PK_HASHES,
            NEW_PK_HASHES,
            [0u8; 32],
        ));
    }

    #[test]
    fn rejects_non_canonical_anchor_in_migration() {
        let commitment = migration_commitment();
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        assert!(!check_migration_anchor(
            &public_inputs,
            BN254_FR_MODULUS_BE,
            NEW_ANCHOR,
            OLD_SCHEME_VERSION,
            NEW_SCHEME_VERSION,
            OLD_PK_HASHES,
            NEW_PK_HASHES,
            BOUND_ACCOUNT,
        ));
    }

    #[test]
    fn rejects_when_no_migration_commitment_matches() {
        let public_inputs = outer_public_inputs_with_commitment([7u8; 32]);
        assert!(!check_migration_anchor(
            &public_inputs,
            OLD_ANCHOR,
            NEW_ANCHOR,
            OLD_SCHEME_VERSION,
            NEW_SCHEME_VERSION,
            OLD_PK_HASHES,
            NEW_PK_HASHES,
            BOUND_ACCOUNT,
        ));
    }

    #[test]
    fn accepts_migration_commitment_when_not_the_first_slot() {
        let commitment = migration_commitment();
        let mut public_inputs = alloc::vec![[0u8; 32]; 10];
        public_inputs[5] = [9u8; 32];
        public_inputs[6] = commitment;
        assert!(check_migration_anchor(
            &public_inputs,
            OLD_ANCHOR,
            NEW_ANCHOR,
            OLD_SCHEME_VERSION,
            NEW_SCHEME_VERSION,
            OLD_PK_HASHES,
            NEW_PK_HASHES,
            BOUND_ACCOUNT,
        ));
    }

    /// Small-int edge-case vector (all-zero `oprf_pk_hashes`), generated the same way the
    /// pre-widening test suite's own all-zero vector was: a temporary `#[test]` calling
    /// `calculate_param_commitment` directly with literal inputs, added to
    /// `migrate-disclosure/src/main.nr`, run via `nargo test --show-output`, the printed
    /// `Field` captured here, then removed — not a fabricated value. `old_anchor = 5,
    /// new_anchor = 6, old_scheme_version = 3, new_scheme_version = 4, both pk-hash arrays
    /// zero, bound_account = [1, 2, .., 32]` (same fixture bytes as `delegate_persona_account`).
    #[test]
    fn calculate_migration_param_commitment_matches_the_all_zero_pk_hashes_nargo_vector() {
        let mut old_anchor = [0u8; 32];
        old_anchor[31] = 5;
        let mut new_anchor = [0u8; 32];
        new_anchor[31] = 6;
        let got = calculate_migration_param_commitment(
            old_anchor,
            new_anchor,
            3,
            4,
            [[0u8; 32]; NUM_COMMITTEES],
            [[0u8; 32]; NUM_COMMITTEES],
            delegate_persona_account(),
        );
        let expected =
            hex32("08620dcacbc18baebdfad57a23e6beb7f9e85681e55f42e0061d5d8788df0d7d");
        assert_eq!(got, expected);
    }

    /// Real end-to-end vector, generated fresh for the `bound_account` widening:
    /// `nargo execute --package oprf_identity_anchor_migrate_disclosure` against
    /// `migrate-disclosure/Prover.toml` (with `bound_account` added), followed by a real
    /// `bb write_vk`/`bb prove`/`bb verify` round-trip that accepted the resulting proof
    /// (`target/bb-migrate-disclosure/public_inputs` in `circuits/oprf-identity-anchor/`).
    /// `old_anchor`/`new_anchor`/`old_oprf_pk_hashes`/`new_oprf_pk_hashes` were read off a
    /// companion real `nargo execute --package oprf_identity_anchor_migrate` +
    /// `bb write_vk`/`bb prove`/`bb verify` run against `migrate/Prover.toml` (whose header
    /// comment already documents it shares the SAME old/new committee proofs as
    /// `migrate-disclosure/Prover.toml`) — that standalone circuit exposes them directly as
    /// public outputs, confirming `old_anchor` matches `real_disclosure_anchor()` above
    /// byte-for-byte (both driven from the same committee generation and identity input,
    /// consistent with `README.md`'s existing cross-check for the pre-widening vectors).
    #[test]
    fn calculate_migration_param_commitment_matches_the_real_nargo_and_bb_vector() {
        let old_anchor = real_disclosure_anchor();
        let new_anchor =
            hex32("1c873a5f7ea81a5701ce2e4fd740ae56435687c92a0e2176954c9acd80690342");
        let old_oprf_pk_hashes = delegate_oprf_pk_hashes();
        let new_oprf_pk_hashes = [
            hex32("1330ef4ceb04fac773b175235420e6426d0832a985c2b6298147fe7f5a403676"),
            hex32("27e673e96f75ddff060f0591e56f953f97209a500e666a7a1aca5f9099bf2248"),
            hex32("15f0fea0a2155d5f6f104b16ff3bd6ec85cae7c2280ebb50ea024374f8312919"),
            hex32("023491a0450e421fb30f4bacc2c08ed2936d6c8562b22671acb894cecbbdee5a"),
            hex32("094160020b1e6c5ea518dbc98505f150eb8bbf3d43fd22672d53ebcd066d9e62"),
        ];
        let got = calculate_migration_param_commitment(
            old_anchor,
            new_anchor,
            1,
            2,
            old_oprf_pk_hashes,
            new_oprf_pk_hashes,
            delegate_persona_account(),
        );
        let expected =
            hex32("1ac43a326cdeec097f8ec5ae560f87d7f279c7dd8913460acbd22db709da5a5f");
        assert_eq!(got, expected);
    }

    // --- check_delegate_persona / calculate_delegate_param_commitment ---

    /// A real end-to-end vector: `nargo execute --package oprf_delegate_persona` against a
    /// real committee-simulator-backed `Prover.toml` (`oprf-committee-dev`'s
    /// `generate_delegate_persona_prover_toml` binary, same DEV-ONLY simulator entries 078/081
    /// used for `disclosure`/`migrate-disclosure`), followed by a real `bb prove`/`bb verify`
    /// round-trip that accepted the resulting proof — see
    /// `circuits/oprf-identity-anchor/delegate-persona/Prover.toml` for the exact fixture.
    /// `delegate_persona_id` and `oprf_pk_hashes` were read off the circuit's own internal
    /// values (temporarily printed via `println` during verification, then removed — the
    /// circuit itself does not expose them, only `param_commitment` folding them is public);
    /// `param_commitment` is the real `bb`-produced public output,
    /// `target/bb-delegate-persona/public_inputs` index 4.
    fn delegate_persona_id() -> [u8; 32] {
        hex32("217bd4a5a3e32f12ff876b4a5e1eb1bfa0fe91ee41861e2d45a665bd6fd961f2")
    }

    fn delegate_persona_account() -> [u8; 32] {
        core::array::from_fn(|i| (i + 1) as u8)
    }

    fn delegate_oprf_pk_hashes() -> [[u8; 32]; NUM_COMMITTEES] {
        [
            hex32("2c69df03538975e4e40ee58aae4448a0dc83cc22e65fe7de7127cda6ca13d69d"),
            hex32("0876cf59468731e62c4c2d229a0f17cd15912eab3d7d053d979d391007ef4894"),
            hex32("0e4a6f87caa020eeef7e18f2e09c640153f95e91c94178e3aa5cf7a96492f7ad"),
            hex32("181ef299c0ec64567ddb4f1d10fc650a0196ca4362886d004ee701f3968ed9aa"),
            hex32("0568c0f7896cad03a95f5cd24c79c328ba2afabd2cb21c2014e7adc29ec001fb"),
        ]
    }

    fn delegate_param_commitment() -> [u8; 32] {
        hex32("2b10a3eed7b4c85e442a495fdf8dc845c959785a4ba0fed206f45ae125941461")
    }

    #[test]
    fn calculate_delegate_param_commitment_matches_the_real_nargo_and_bb_vector() {
        let got = calculate_delegate_param_commitment(
            delegate_persona_id(),
            delegate_persona_account(),
            1,
            delegate_oprf_pk_hashes(),
        );
        assert_eq!(got, delegate_param_commitment());
    }

    #[test]
    fn account_to_field_limbs_matches_the_circuits_pack_be_bytes_into_fields_split() {
        // `account = [1, 2, .., 32]` (big-endian) — `hi` must hold just `account[0] = 1`, `lo`
        // must hold `account[1..32] = [2, .., 32]`. Cross-checked structurally here (not just
        // via the end-to-end vectors above) so a future refactor of the limb split alone, with
        // an unchanged Poseidon2 hash, still gets caught.
        let account: [u8; 32] = core::array::from_fn(|i| (i + 1) as u8);
        let (lo, hi) = account_to_field_limbs(account);
        let mut expected_hi = [0u8; 32];
        expected_hi[31] = 1;
        let mut expected_lo = [0u8; 32];
        expected_lo[1..32].copy_from_slice(&(2..=32u8).collect::<Vec<u8>>());
        assert_eq!(hi, expected_hi);
        assert_eq!(lo, expected_lo);
    }

    #[test]
    fn accepts_a_correctly_recomputed_delegate_persona_commitment() {
        let public_inputs = outer_public_inputs_with_commitment(delegate_param_commitment());
        assert!(check_delegate_persona(
            &public_inputs,
            delegate_persona_id(),
            delegate_persona_account(),
            1,
            delegate_oprf_pk_hashes(),
        ));
    }

    #[test]
    fn rejects_delegate_persona_with_a_swapped_persona_account() {
        // The whole point of folding `persona_account` into `param_commitment`: an observer
        // resubmitting the same proof/public_inputs against a *different* claimed
        // persona_account must fail, not silently accept a front-run substitution.
        let public_inputs = outer_public_inputs_with_commitment(delegate_param_commitment());
        let mut other_account = delegate_persona_account();
        other_account[0] ^= 1;
        assert!(!check_delegate_persona(
            &public_inputs,
            delegate_persona_id(),
            other_account,
            1,
            delegate_oprf_pk_hashes(),
        ));
    }

    #[test]
    fn rejects_delegate_persona_with_zero_persona_account() {
        let public_inputs = outer_public_inputs_with_commitment(delegate_param_commitment());
        assert!(!check_delegate_persona(
            &public_inputs,
            delegate_persona_id(),
            [0u8; 32],
            1,
            delegate_oprf_pk_hashes(),
        ));
    }

    #[test]
    fn rejects_delegate_persona_with_wrong_delegate_persona_id() {
        let public_inputs = outer_public_inputs_with_commitment(delegate_param_commitment());
        let mut wrong_id = delegate_persona_id();
        wrong_id[0] ^= 1;
        assert!(!check_delegate_persona(
            &public_inputs,
            wrong_id,
            delegate_persona_account(),
            1,
            delegate_oprf_pk_hashes(),
        ));
    }

    #[test]
    fn rejects_delegate_persona_with_wrong_scheme_version() {
        let public_inputs = outer_public_inputs_with_commitment(delegate_param_commitment());
        assert!(!check_delegate_persona(
            &public_inputs,
            delegate_persona_id(),
            delegate_persona_account(),
            2,
            delegate_oprf_pk_hashes(),
        ));
    }

    #[test]
    fn rejects_delegate_persona_with_a_single_mutated_pk_hash() {
        let public_inputs = outer_public_inputs_with_commitment(delegate_param_commitment());
        for i in 0..NUM_COMMITTEES {
            let mut mutated = delegate_oprf_pk_hashes();
            mutated[i][0] ^= 1;
            assert!(
                !check_delegate_persona(
                    &public_inputs,
                    delegate_persona_id(),
                    delegate_persona_account(),
                    1,
                    mutated,
                ),
                "mutating pk_hash slot {i} must be rejected",
            );
        }
    }

    #[test]
    fn rejects_delegate_persona_when_no_param_commitment_matches() {
        let public_inputs = outer_public_inputs_with_commitment([7u8; 32]);
        assert!(!check_delegate_persona(
            &public_inputs,
            delegate_persona_id(),
            delegate_persona_account(),
            1,
            delegate_oprf_pk_hashes(),
        ));
    }

    #[test]
    fn rejects_delegate_persona_with_non_canonical_id() {
        let public_inputs = outer_public_inputs_with_commitment(delegate_param_commitment());
        assert!(!check_delegate_persona(
            &public_inputs,
            BN254_FR_MODULUS_BE,
            delegate_persona_account(),
            1,
            delegate_oprf_pk_hashes(),
        ));
    }

    #[test]
    fn rejects_delegate_persona_too_few_public_inputs() {
        assert!(!check_delegate_persona(
            &[[0u8; 32]; 8],
            delegate_persona_id(),
            delegate_persona_account(),
            1,
            delegate_oprf_pk_hashes(),
        ));
    }

    /// A delegate-persona commitment must never collide with a registration/reverification or
    /// migration commitment built from a superficially similar tuple — the distinct proof-type
    /// tags (202/203/204) are what make substituting one for another fail even if a caller
    /// mixed up which check to call.
    #[test]
    fn delegate_persona_commitment_differs_from_registration_commitment_under_similar_inputs() {
        let registration_style = calculate_param_commitment(
            delegate_persona_id(),
            1,
            delegate_oprf_pk_hashes(),
            delegate_persona_account(), // any similarly-shaped [u8; 32] stand-in
            BOUND_ACCOUNT,
        );
        assert_ne!(registration_style, delegate_param_commitment());
    }
}
