//! Agora identity-anchor **migration** (dual-evaluation) circuit.
//!
//! Implements changelog entry 67's scheduled OPRF-scheme rotation, over changelog entry 73's
//! 5-committee topology: the citizen evaluates the *same* personal-number-derived identity
//! input under both the 5 outgoing committees and the 5 incoming committees, and proves both
//! combined anchors came from that one input without revealing it.
//! `pallet-identity::migrate_oprf_scheme` then matches `old_anchor` against the citizen's
//! existing `CitizenAnchor` entry and claims `new_anchor` under the next scheme version.
//!
//! Consistency is enforced structurally rather than by an extra constraint: there is exactly
//! one `identity_input` binding in this circuit and all 10 `verified_oprf` calls consume it,
//! so there is no way for the two combined anchors to have come from different inputs.
//!
//! This is the proof `AnchorProofVerifier::verify_migration` must ultimately verify.
//!
//! Caveat, stated plainly: this circuit is only sound as a rotation mechanism while the
//! *outgoing* committee is still honest, because a leaked outgoing key lets an attacker
//! fabricate an `old_anchor` for an input they do not hold. That is exactly why changelog
//! entry 67 routes a suspected break through `pallet-emergency-council`'s emergency rotation
//! and its degraded fallback rather than through this path.

use common::hash_salt_dg1_dg2_hash_private_nullifier;
use identity_anchor::{
    assert_dg1_clear_value, combine_committee_anchors, derive_committee_anchor_term,
    derive_identity_input, DS_ANCHOR_OUT, DS_DLOG, NUM_COMMITTEES,
};
use oprf::oprf_output::verified_oprf;
use poseidon::poseidon2::Poseidon2;
use utils::types::{DG1Data, MRZExpiryDate, OPRFProof, SaltedValue};

/// Public inputs:  `comm_in`, `old_scheme_version`, `new_scheme_version`
/// Public outputs: `(old_anchor, new_anchor, old_oprf_pk_hashes, new_oprf_pk_hashes)`
///
/// Evaluates the same identity input under both the 5 outgoing committees and the 5 incoming
/// committees (changelog entry 73's topology applies identically on both sides of a rotation).
fn main(
    comm_in: pub Field,
    old_scheme_version: pub Field,
    new_scheme_version: pub Field,
    salted_dg1: SaltedValue<DG1Data>,
    salted_expiry_date: SaltedValue<MRZExpiryDate>,
    salted_dg2_hash: SaltedValue<Field>,
    salted_dg2_hash_type: SaltedValue<u32>,
    salted_private_nullifier: SaltedValue<Field>,
    old_oprf_proofs: [OPRFProof; NUM_COMMITTEES],
    new_oprf_proofs: [OPRFProof; NUM_COMMITTEES],
) -> pub (Field, Field, [Field; NUM_COMMITTEES], [Field; NUM_COMMITTEES]) {
    assert(
        comm_in
            == hash_salt_dg1_dg2_hash_private_nullifier(
                salted_dg1,
                salted_expiry_date,
                salted_dg2_hash,
                salted_dg2_hash_type,
                salted_private_nullifier,
            ),
        "Commitment mismatch in oprf_identity_anchor_migrate",
    );
    assert_dg1_clear_value(salted_dg1);
    assert(old_scheme_version != 0, "Old OPRF scheme version must be non-zero");
    assert(
        new_scheme_version != old_scheme_version,
        "Migration must move between two different OPRF scheme versions",
    );

    // Single binding, consumed by all 10 evaluations below — this is what makes the old and
    // new anchors provably same-input without an explicit equality constraint.
    let identity_input = derive_identity_input(salted_dg1.value);

    let mut old_terms: [Field; NUM_COMMITTEES] = [0; NUM_COMMITTEES];
    let mut old_oprf_pk_hashes: [Field; NUM_COMMITTEES] = [0; NUM_COMMITTEES];
    for i in 0..NUM_COMMITTEES {
        let old_oprf_proof = old_oprf_proofs[i];
        let old_oprf_output = verified_oprf(
            old_oprf_proof.beta,
            old_oprf_proof.pk,
            old_oprf_proof.dlog_e,
            old_oprf_proof.dlog_s,
            old_oprf_proof.response_blinded,
            old_oprf_proof.response,
            identity_input,
            DS_DLOG,
            DS_ANCHOR_OUT,
        );
        old_terms[i] = derive_committee_anchor_term(i, old_oprf_output, old_scheme_version);
        old_oprf_pk_hashes[i] = Poseidon2::hash([old_oprf_proof.pk.x, old_oprf_proof.pk.y], 2);
    }

    let mut new_terms: [Field; NUM_COMMITTEES] = [0; NUM_COMMITTEES];
    let mut new_oprf_pk_hashes: [Field; NUM_COMMITTEES] = [0; NUM_COMMITTEES];
    for i in 0..NUM_COMMITTEES {
        let new_oprf_proof = new_oprf_proofs[i];
        let new_oprf_output = verified_oprf(
            new_oprf_proof.beta,
            new_oprf_proof.pk,
            new_oprf_proof.dlog_e,
            new_oprf_proof.dlog_s,
            new_oprf_proof.response_blinded,
            new_oprf_proof.response,
            identity_input,
            DS_DLOG,
            DS_ANCHOR_OUT,
        );
        new_terms[i] = derive_committee_anchor_term(i, new_oprf_output, new_scheme_version);
        new_oprf_pk_hashes[i] = Poseidon2::hash([new_oprf_proof.pk.x, new_oprf_proof.pk.y], 2);
    }

    (
        combine_committee_anchors(old_terms),
        combine_committee_anchors(new_terms),
        old_oprf_pk_hashes,
        new_oprf_pk_hashes,
    )
}
