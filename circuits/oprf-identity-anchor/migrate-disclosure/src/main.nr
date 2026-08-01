//! Agora identity-anchor **migration disclosure-subproof** circuit — the pipeline-integrated
//! form of `../migrate`, exactly as `../disclosure` is the pipeline-integrated form of
//! `../anchor`.
//!
//! # Why this exists
//!
//! `../migrate` proves "`old_anchor` and `new_anchor` were both derived from the personal
//! number of the DG1 committed at `comm_in`, under the old and new OPRF committee
//! generations respectively" — but `comm_in` there is the same kind of *private* witness
//! problem `../disclosure`'s own header already documents for `../anchor`: it never appears
//! among an outer ZKPassport proof's public inputs, so a standalone `../migrate` proof cannot
//! be bound on-chain to a genuine, currently-valid passport. Nothing would stop a prover
//! pairing a real (but unrelated) outer proof with a `../migrate` proof over a `comm_in` of
//! their own choosing — which would let `migrate_oprf_scheme` reassign a citizen's anchor to
//! an arbitrary, uncommitted-to value with no real OPRF evaluation behind it at all, defeating
//! both the "same human" continuity `../migrate` exists to prove and the freshness guarantee
//! (`check_expiry_from_date` below) that the citizen still holds a currently valid passport at
//! migration time — see HANDOFF changelog entry 76 for the full trail (this circuit is the fix
//! for the gap entry 75 flagged but did not close).
//!
//! This variant closes it exactly the way `../disclosure` does: matching the disclosure-subproof
//! interface the outer circuit already recursively verifies (`prepare_disclosure_inputs` in
//! `src/noir/lib/outer/src/lib.nr`), so both migrated anchors ride out inside the outer proof's
//! own `param_commitments` array. One proof, one `comm_in`, authenticated end-to-end back to
//! the certificate registry root.
//!
//! # Status
//!
//! Compiles and produces a verification key, but — like `../disclosure` and `../migrate` before
//! it — has never been run inside an actual outer proof or against a live OPRF committee. Treat
//! the interface match as carefully derived, not empirically confirmed. See the README.

use common::hash_salt_dg1_dg2_hash_private_nullifier;
use data_check_expiry::check_expiry_from_date;
use identity_anchor::{
    assert_dg1_clear_value, combine_committee_anchors, derive_committee_anchor_term,
    derive_identity_input, DS_ANCHOR_OUT, DS_DLOG, NUM_COMMITTEES,
};
use oprf::oprf_output::verified_oprf;
use poseidon::poseidon2::Poseidon2;
use utils::types::{DG1Data, MRZExpiryDate, OPRFProof, SaltedValue};

/// Agora-specific proof type tag for the migration parameter commitment. Deliberately distinct
/// from `disclosure`'s `PROOF_TYPE_AGORA_IDENTITY_ANCHOR` (200) so a migration commitment can
/// never be confused with (or substituted for) a registration/reverification one — the two
/// have different arities and different security obligations on the Rust-verifier side.
global PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE: Field = 201;

/// `param_commitment = Poseidon2(PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE, old_anchor,
/// new_anchor, old_scheme_version, new_scheme_version, old_oprf_pk_hashes[0..5],
/// new_oprf_pk_hashes[0..5])` — 14 elements. Mirrors `disclosure::calculate_param_commitment`'s
/// shape (tag, then every disclosed parameter in argument order).
fn calculate_param_commitment(
    old_anchor: Field,
    new_anchor: Field,
    old_scheme_version: Field,
    new_scheme_version: Field,
    old_oprf_pk_hashes: [Field; NUM_COMMITTEES],
    new_oprf_pk_hashes: [Field; NUM_COMMITTEES],
) -> Field {
    Poseidon2::hash(
        [
            PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE,
            old_anchor,
            new_anchor,
            old_scheme_version,
            new_scheme_version,
            old_oprf_pk_hashes[0],
            old_oprf_pk_hashes[1],
            old_oprf_pk_hashes[2],
            old_oprf_pk_hashes[3],
            old_oprf_pk_hashes[4],
            new_oprf_pk_hashes[0],
            new_oprf_pk_hashes[1],
            new_oprf_pk_hashes[2],
            new_oprf_pk_hashes[3],
            new_oprf_pk_hashes[4],
        ],
        15,
    )
}

/// Public inputs:  `comm_in`, `current_date`, `service_scope`, `service_subscope`
/// Public outputs: `(param_commitment, nullifier_type, scoped_nullifier, oprf_pk_hash)`
///
/// The last three outputs are always 0, same non-participating convention as `../disclosure`.
fn main(
    comm_in: pub Field,
    current_date: pub u64,
    old_scheme_version: Field,
    new_scheme_version: Field,
    salted_dg1: SaltedValue<DG1Data>,
    salted_expiry_date: SaltedValue<MRZExpiryDate>,
    salted_dg2_hash: SaltedValue<Field>,
    salted_dg2_hash_type: SaltedValue<u32>,
    salted_private_nullifier: SaltedValue<Field>,
    old_oprf_proofs: [OPRFProof; NUM_COMMITTEES],
    new_oprf_proofs: [OPRFProof; NUM_COMMITTEES],
    service_scope: pub Field,
    service_subscope: pub Field,
) -> pub (Field, Field, Field, Field) {
    assert(
        comm_in
            == hash_salt_dg1_dg2_hash_private_nullifier(
                salted_dg1,
                salted_expiry_date,
                salted_dg2_hash,
                salted_dg2_hash_type,
                salted_private_nullifier,
            ),
        "Commitment mismatch in oprf_identity_anchor_migrate_disclosure",
    );
    assert_dg1_clear_value(salted_dg1);
    assert(old_scheme_version != 0, "Old OPRF scheme version must be non-zero");
    assert(
        new_scheme_version != old_scheme_version,
        "Migration must move between two different OPRF scheme versions",
    );

    // Same "valid at registration AND at vote time" requirement disclosure enforces — a
    // citizen migrating must currently hold an unexpired passport, not just have held one at
    // original registration.
    check_expiry_from_date(salted_expiry_date.value, current_date);

    // Single binding, consumed by all 10 evaluations below — exactly `../migrate`'s
    // consistency argument: there is no way for the two combined anchors to have come from
    // different inputs.
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

    let old_anchor = combine_committee_anchors(old_terms);
    let new_anchor = combine_committee_anchors(new_terms);
    let param_commitment = calculate_param_commitment(
        old_anchor,
        new_anchor,
        old_scheme_version,
        new_scheme_version,
        old_oprf_pk_hashes,
        new_oprf_pk_hashes,
    );

    // Silence unused-parameter warnings without changing the constrained system: these two
    // exist purely to occupy their slots in the outer circuit's fixed public-input layout.
    let _ = service_scope;
    let _ = service_subscope;

    (param_commitment, 0, 0, 0)
}
