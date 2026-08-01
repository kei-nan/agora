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

/// Agora's proof-type tag for the registration/reverification parameter commitment — must
/// match `circuits/oprf-identity-anchor/disclosure/src/main.nr`'s
/// `PROOF_TYPE_AGORA_IDENTITY_ANCHOR`.
const PROOF_TYPE_AGORA_IDENTITY_ANCHOR: u8 = 200;

/// Agora's proof-type tag for the migration parameter commitment — must match
/// `circuits/oprf-identity-anchor/migrate-disclosure/src/main.nr`'s
/// `PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE`. Deliberately distinct from the registration
/// tag above so a migration commitment (15 elements) can never be confused with a
/// registration one (8 elements).
const PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE: u8 = 201;

/// Number of OPRF committees — must match
/// `circuits/oprf-identity-anchor/lib/identity-anchor`'s `NUM_COMMITTEES` (changelog entry
/// 73's 5-independent-committee design).
pub const NUM_COMMITTEES: usize = 5;

fn u32_to_field_bytes(value: u32) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[28..32].copy_from_slice(&value.to_be_bytes());
    bytes
}

/// `Poseidon2(PROOF_TYPE_AGORA_IDENTITY_ANCHOR, anchor, scheme_version, oprf_pk_hashes[0],
/// .., oprf_pk_hashes[4])` — an 8-element hash, matching
/// `disclosure::calculate_param_commitment` field-for-field and argument-for-argument.
pub fn calculate_param_commitment(
    anchor: [u8; 32],
    scheme_version: u32,
    oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
) -> [u8; 32] {
    let mut tag = [0u8; 32];
    tag[31] = PROOF_TYPE_AGORA_IDENTITY_ANCHOR;

    let mut input: Vec<[u8; 32]> = Vec::with_capacity(3 + NUM_COMMITTEES);
    input.push(tag);
    input.push(anchor);
    input.push(u32_to_field_bytes(scheme_version));
    input.extend_from_slice(&oprf_pk_hashes);

    poseidon2_bn254::hash_bytes(&input)
}

/// The pure, storage-free half of the registration-anchor check — see the module docs for
/// what this does and does not cover. `outer_public_inputs` is the same `public_inputs`
/// array `register_citizen` already validated via `T::ZkVerifier::verify`.
pub fn check_registration_anchor(
    outer_public_inputs: &[[u8; 32]],
    anchor: [u8; 32],
    scheme_version: u32,
    oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
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

    // param_commitments occupy indices 5..len-3 — see crate::verifier's module docs for the
    // full outer-circuit public-input table this mirrors.
    let param_commitments = &outer_public_inputs[5..outer_public_inputs.len() - 3];

    let recomputed = calculate_param_commitment(anchor, scheme_version, oprf_pk_hashes);
    param_commitments.iter().any(|commitment| *commitment == recomputed)
}

/// `Poseidon2(PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE, old_anchor, new_anchor,
/// old_scheme_version, new_scheme_version, old_oprf_pk_hashes[0..5], new_oprf_pk_hashes[0..5])`
/// — a 15-element hash, matching
/// `migrate_disclosure::calculate_param_commitment` field-for-field and argument-for-argument.
pub fn calculate_migration_param_commitment(
    old_anchor: [u8; 32],
    new_anchor: [u8; 32],
    old_scheme_version: u32,
    new_scheme_version: u32,
    old_oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
    new_oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
) -> [u8; 32] {
    let mut tag = [0u8; 32];
    tag[31] = PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE;

    let mut input: Vec<[u8; 32]> = Vec::with_capacity(5 + 2 * NUM_COMMITTEES);
    input.push(tag);
    input.push(old_anchor);
    input.push(new_anchor);
    input.push(u32_to_field_bytes(old_scheme_version));
    input.push(u32_to_field_bytes(new_scheme_version));
    input.extend_from_slice(&old_oprf_pk_hashes);
    input.extend_from_slice(&new_oprf_pk_hashes);

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

    let param_commitments = &outer_public_inputs[5..outer_public_inputs.len() - 3];

    let recomputed = calculate_migration_param_commitment(
        old_anchor,
        new_anchor,
        old_scheme_version,
        new_scheme_version,
        old_oprf_pk_hashes,
        new_oprf_pk_hashes,
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
    ) -> bool {
        check_registration_anchor(outer_public_inputs, anchor, scheme_version, oprf_pk_hashes)
    }

    fn verify_reverification(
        outer_public_inputs: &[[u8; 32]],
        anchor: [u8; 32],
        scheme_version: u32,
        oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
    ) -> bool {
        check_registration_anchor(outer_public_inputs, anchor, scheme_version, oprf_pk_hashes)
    }

    fn verify_migration(
        outer_public_inputs: &[[u8; 32]],
        old_anchor: [u8; 32],
        new_anchor: [u8; 32],
        old_scheme_version: u32,
        new_scheme_version: u32,
        old_oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
        new_oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
    ) -> bool {
        check_migration_anchor(
            outer_public_inputs,
            old_anchor,
            new_anchor,
            old_scheme_version,
            new_scheme_version,
            old_oprf_pk_hashes,
            new_oprf_pk_hashes,
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

    #[test]
    fn accepts_a_correctly_recomputed_commitment() {
        let commitment = calculate_param_commitment(ANCHOR, SCHEME_VERSION, PK_HASHES);
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        assert!(check_registration_anchor(
            &public_inputs,
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES
        ));
    }

    #[test]
    fn accepts_when_the_matching_commitment_is_not_the_first_slot() {
        // Two disclosure subproofs (D = 2): param_commitments at indices 5 and 6. The
        // anchor's commitment is the *second* one — check_registration_anchor must not
        // assume it's always index 5.
        let commitment = calculate_param_commitment(ANCHOR, SCHEME_VERSION, PK_HASHES);
        let mut public_inputs = alloc::vec![[0u8; 32]; 10];
        public_inputs[5] = [9u8; 32]; // an unrelated disclosure subproof's commitment
        public_inputs[6] = commitment;
        assert!(check_registration_anchor(
            &public_inputs,
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES
        ));
    }

    #[test]
    fn rejects_wrong_anchor() {
        let commitment = calculate_param_commitment(ANCHOR, SCHEME_VERSION, PK_HASHES);
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        let mut wrong_anchor = ANCHOR;
        wrong_anchor[0] ^= 1;
        assert!(!check_registration_anchor(
            &public_inputs,
            wrong_anchor,
            SCHEME_VERSION,
            PK_HASHES
        ));
    }

    #[test]
    fn rejects_wrong_scheme_version() {
        let commitment = calculate_param_commitment(ANCHOR, SCHEME_VERSION, PK_HASHES);
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        assert!(!check_registration_anchor(
            &public_inputs,
            ANCHOR,
            SCHEME_VERSION + 1,
            PK_HASHES
        ));
    }

    #[test]
    fn rejects_a_single_mutated_pk_hash() {
        let commitment = calculate_param_commitment(ANCHOR, SCHEME_VERSION, PK_HASHES);
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        for i in 0..NUM_COMMITTEES {
            let mut mutated = PK_HASHES;
            mutated[i][0] ^= 1;
            assert!(
                !check_registration_anchor(&public_inputs, ANCHOR, SCHEME_VERSION, mutated),
                "mutating pk_hash slot {i} must be rejected",
            );
        }
    }

    #[test]
    fn rejects_when_no_param_commitment_matches() {
        let public_inputs = outer_public_inputs_with_commitment([7u8; 32]);
        assert!(!check_registration_anchor(
            &public_inputs,
            ANCHOR,
            SCHEME_VERSION,
            PK_HASHES
        ));
    }

    #[test]
    fn rejects_non_canonical_anchor() {
        let commitment = calculate_param_commitment(ANCHOR, SCHEME_VERSION, PK_HASHES);
        let public_inputs = outer_public_inputs_with_commitment(commitment);
        assert!(!check_registration_anchor(
            &public_inputs,
            BN254_FR_MODULUS_BE,
            SCHEME_VERSION,
            PK_HASHES
        ));
    }

    #[test]
    fn rejects_too_few_public_inputs() {
        assert!(!check_registration_anchor(&[[0u8; 32]; 8], ANCHOR, SCHEME_VERSION, PK_HASHES));
    }

    #[test]
    fn calculate_param_commitment_matches_the_real_nargo_vector() {
        // Same vector as poseidon2_bn254's own test, restated at this layer to confirm the
        // tag/field assembly (not just the underlying hash) matches
        // disclosure::calculate_param_commitment's exact call shape: tag=200, anchor=111,
        // scheme_version=1, pk_hashes=[222,333,444,555,666].
        let mut anchor = [0u8; 32];
        anchor[31] = 111;
        let mut pk_hashes = [[0u8; 32]; NUM_COMMITTEES];
        // 333/444/555/666 don't fit in a single byte, so these are big-endian u16s in the
        // low two bytes of each 32-byte field.
        for (i, v) in [222u16, 333, 444, 555, 666].iter().enumerate() {
            pk_hashes[i][30..32].copy_from_slice(&v.to_be_bytes());
        }
        let got = calculate_param_commitment(anchor, 1, pk_hashes);
        let expected_hex = "2bbdcc5187d2d2f63d3b906c678f5ef5af7e0e86984d60b7db38ee2c4731dc2f";
        let expected = {
            let mut out = [0u8; 32];
            let bytes = (0..32)
                .map(|i| u8::from_str_radix(&expected_hex[i * 2..i * 2 + 2], 16).unwrap())
                .collect::<Vec<u8>>();
            out.copy_from_slice(&bytes);
            out
        };
        assert_eq!(got, expected);
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
                ),
                "mutating new_pk_hash slot {i} must be rejected",
            );
        }
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
        ));
    }

    #[test]
    fn calculate_migration_param_commitment_matches_the_real_nargo_vector() {
        // Real `nargo test --show-output` vector against
        // `migrate-disclosure::calculate_param_commitment`'s exact shape: tag=201,
        // old_anchor=111, new_anchor=222, old_scheme_version=1, new_scheme_version=2,
        // old_oprf_pk_hashes=[10,20,30,40,50], new_oprf_pk_hashes=[60,70,80,90,100].
        let got = migration_commitment();
        let expected_hex = "1fc03013b1ebd0943d9fe6702ba50f7b7224d38ce69bdd3106f993d4f905ae88";
        let expected = {
            let mut out = [0u8; 32];
            let bytes = (0..32)
                .map(|i| u8::from_str_radix(&expected_hex[i * 2..i * 2 + 2], 16).unwrap())
                .collect::<Vec<u8>>();
            out.copy_from_slice(&bytes);
            out
        };
        assert_eq!(got, expected);
    }

    #[test]
    fn calculate_migration_param_commitment_matches_the_all_zero_pk_hashes_nargo_vector() {
        // Second real `nargo` vector, exercising all-zero pk-hash slots: tag=201, old_anchor=5,
        // new_anchor=6, old_scheme_version=3, new_scheme_version=4, both pk-hash arrays zero.
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
        );
        let expected_hex = "179f60a933aa9e2ea3ffcb201052443276bb8e1e16281ef0f0c9c816bad0d3f1";
        let expected = {
            let mut out = [0u8; 32];
            let bytes = (0..32)
                .map(|i| u8::from_str_radix(&expected_hex[i * 2..i * 2 + 2], 16).unwrap())
                .collect::<Vec<u8>>();
            out.copy_from_slice(&bytes);
            out
        };
        assert_eq!(got, expected);
    }
}
