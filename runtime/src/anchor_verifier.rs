//! Real (partial) `AnchorProofVerifier` for `pallet_identity_zk`'s OPRF identity-anchor
//! checks — see `circuits/oprf-identity-anchor/README.md`'s "What the Rust verifier still
//! has to enforce" and `docs/project/changelog/074.md`/`075.md` for the design this
//! implements.
//!
//! # What is real here, and what is still `PassthroughAnchorVerifier`
//!
//! Only [`Poseidon2AnchorVerifier::verify_registration_anchor`] does genuine cryptographic
//! work. `verify_reverification` and `verify_migration` are **not** implemented here and
//! stay wired to `PassthroughAnchorVerifier` in `configs/mod.rs` — not because they were
//! forgotten, but because they hit a real, structural blocker distinct from (and larger
//! than) the Poseidon2 one this module solves:
//!
//! `reverify_citizen`/`migrate_oprf_scheme` (`pallets/pallet-identity/src/lib.rs`) take only
//! a bare `proof_bytes: BoundedVec<u8, ConstU32<4096>>` and hand it straight to
//! `AnchorVerifier`, with **no** `T::ZkVerifier::verify` call and no outer proof
//! `public_inputs` in scope at all. That was fine under the pre-log-#74 mental model (a
//! standalone anchor SNARK proof), but the README's own "why `disclosure` exists" section
//! already ruled that model out for the *registration* path — `comm_in` is a private
//! witness of the outer circuit, so a standalone anchor proof can't be bound to a genuine
//! passport proof on-chain, and a prover could pair a real outer proof with a `comm_in` of
//! their own invention. The exact same argument applies to reverification and migration:
//! neither extrinsic currently accepts or verifies an outer ZKPassport proof at all, so
//! there is no already-authenticated `param_commitments` array to recompute against for
//! either of them. Building a real verifier for those two needs the same kind of extrinsic
//! surgery `register_citizen` got in this session (accept the outer `zk_proof` +
//! `public_inputs`, run `T::ZkVerifier::verify` first, then recompute against its
//! `param_commitments`) — not attempted here; flagging it precisely is more honest than
//! wiring `Poseidon2AnchorVerifier` in for a check it cannot actually perform.
//!
//! # What `verify_registration_anchor` actually does
//!
//! `pallet_identity_zk::register_citizen` already runs `T::ZkVerifier::verify(zk_proof,
//! public_inputs)` (a real bb 5.0.0 pairing check, see `crate::verifier`) *before* calling
//! `T::AnchorVerifier::verify_registration_anchor` — so by the time this function runs, the
//! `disclosure` subproof folded into that same outer proof (see
//! `circuits/oprf-identity-anchor/README.md`) has already had its constraints checked,
//! including all 5 committees' `verified_oprf` calls and the anchor combination. No second
//! SNARK/pairing check is needed here; this function only has to:
//!
//! 1. Recompute `param_commitment = Poseidon2(200, anchor, scheme_version,
//!    oprf_pk_hashes[0..5])` from the tuple submitted with the extrinsic
//!    ([`calculate_param_commitment`], via the [`poseidon2_bn254`] crate — see that crate's
//!    module docs for how its Poseidon2 port was validated against real `nargo`-produced
//!    output).
//! 2. Check the recomputed value against *every* `param_commitments[i]` the already-verified
//!    outer proof exposes (there can be more than one disclosure subproof; matching any one
//!    is sufficient — see `circuits/oprf-identity-anchor/README.md`'s public-input-layout
//!    table).
//!
//! It deliberately does **not** check the 5 `oprf_pk_hashes` against a governance-approved
//! committee key, or `current_date` freshness — those are chain-storage-dependent checks
//! that `pallet_identity_zk::register_citizen` performs directly (mirroring how it already
//! checks `AllowedMerkleRoots` itself rather than delegating that to `T::ZkVerifier`), using
//! the new `OprfCommitteeKeys` storage. Keeping this function pure (no storage reads) is
//! also what makes it cleanly unit-testable below with plain fixtures.

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

/// Agora's proof-type tag for the parameter commitment — must match
/// `circuits/oprf-identity-anchor/disclosure/src/main.nr`'s
/// `PROOF_TYPE_AGORA_IDENTITY_ANCHOR`.
const PROOF_TYPE_AGORA_IDENTITY_ANCHOR: u8 = 200;

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

/// Real `AnchorProofVerifier` for the non-dev-mode runtime.
///
/// `verify_registration_anchor` is genuinely checked (see module docs). `verify_reverification`
/// and `verify_migration` are **not** — they return `true` unconditionally, exactly matching
/// `PassthroughAnchorVerifier`'s existing behavior, because there is currently no way to
/// implement them for real: `pallet_identity_zk::reverify_citizen`/`migrate_oprf_scheme`
/// don't accept an outer ZKPassport proof or run `T::ZkVerifier::verify` at all (see module
/// docs), so there is no already-authenticated `param_commitments` array for either call to
/// recompute against. Returning `true` here is not a security regression — both methods were
/// already `true` unconditionally under `PassthroughAnchorVerifier`, which is what this type
/// replaces only for `verify_registration_anchor` — but it is a deliberate choice not to
/// return `false` either: failing closed on a check this type cannot actually perform would
/// just permanently lock every citizen out of reverification/migration, which is an
/// availability regression with no corresponding security gain (the check still wouldn't be
/// validating anything real). Wiring one method real and two still-passthrough on the *same*
/// type, rather than only ever using this type for registration and leaving `configs/mod.rs`
/// to reference `PassthroughAnchorVerifier` directly for the other two, is a judgment call in
/// favor of a single, greppable "this is the anchor verifier" type — see
/// `runtime/src/configs/mod.rs` for how it's wired in.
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

    fn verify_reverification(_proof_bytes: &[u8], _anchor: [u8; 32]) -> bool {
        true
    }

    fn verify_migration(_proof_bytes: &[u8], _old_anchor: [u8; 32], _new_anchor: [u8; 32]) -> bool {
        true
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
}
