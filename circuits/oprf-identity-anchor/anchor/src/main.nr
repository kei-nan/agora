//! Agora identity-**anchor** circuit.
//!
//! Second half of the two-round OPRF flow. `query` blinds the identity input and sends the
//! *same* blinded query symmetrically to all `NUM_COMMITTEES` (5) independent committees
//! (changelog entry 73 — not routed by hash to one committee, which would leak a low-entropy
//! per-citizen signal). Each committee evaluates it under its own secret key and returns a
//! blinded response plus a Chaum-Pedersen DLog-equality proof; this circuit checks all 5
//! responses and combines them into the single anchor value published on-chain.
//!
//! Structurally each per-committee check mirrors `nullify` in
//! `src/noir/lib/commitment/scoped-nullifier/src/lib.nr` of zkpassport/circuits @
//! d3a75acb8529e82c61be136a402553daec259257 — same `verified_oprf` call, same commitment
//! check — but it emits an anchor over the renewal-stable identity input instead of a scoped
//! nullifier over the document-bound private nullifier, and it deliberately does *not* take
//! `service_scope`/`service_subscope`: the anchor is scope-free by design, checked once at
//! registration and never used for voting. The 5-committee combination is entry 73's design,
//! not upstream's: see `identity_anchor::derive_committee_anchor_term` and
//! `combine_committee_anchors` for the hashed-summation combiner and why it must hash each
//! committee's term before adding.
//!
//! This proof is what `pallet-identity`'s `AnchorProofVerifier::verify_registration_anchor`
//! (and `verify_reverification`, which has an identical layout) must ultimately verify.

use common::hash_salt_dg1_dg2_hash_private_nullifier;
use identity_anchor::{
    assert_dg1_clear_value, combine_committee_anchors, derive_committee_anchor_term,
    derive_identity_input, DS_ANCHOR_OUT, DS_DLOG, NUM_COMMITTEES,
};
use oprf::oprf_output::verified_oprf;
use poseidon::poseidon2::Poseidon2;
use utils::types::{DG1Data, MRZExpiryDate, OPRFProof, SaltedValue};

/// Public inputs:  `comm_in`, `scheme_version`
/// Public outputs: `(anchor, oprf_pk_hashes)` — `oprf_pk_hashes[i] = Poseidon2(pk_i.x, pk_i.y)`
///
/// Each `oprf_pk_hashes[i]` is emitted rather than taken as an input so the verifier learns
/// *which* committee key produced committee `i`'s evaluation. The circuit cannot itself decide
/// whether that key is the legitimate one for `scheme_version`'s committee slot `i` — that
/// binding lives on-chain and must be enforced by the Rust verifier, once per committee. See
/// the README.
fn main(
    comm_in: pub Field,
    scheme_version: pub Field,
    salted_dg1: SaltedValue<DG1Data>,
    salted_expiry_date: SaltedValue<MRZExpiryDate>,
    salted_dg2_hash: SaltedValue<Field>,
    salted_dg2_hash_type: SaltedValue<u32>,
    salted_private_nullifier: SaltedValue<Field>,
    oprf_proofs: [OPRFProof; NUM_COMMITTEES],
) -> pub (Field, [Field; NUM_COMMITTEES]) {
    assert(
        comm_in
            == hash_salt_dg1_dg2_hash_private_nullifier(
                salted_dg1,
                salted_expiry_date,
                salted_dg2_hash,
                salted_dg2_hash_type,
                salted_private_nullifier,
            ),
        "Commitment mismatch in oprf_identity_anchor",
    );
    assert_dg1_clear_value(salted_dg1);
    assert(scheme_version != 0, "OPRF scheme version must be non-zero");

    // Single binding, consumed by all 5 committee evaluations below — this is what makes the
    // 5 committees provably answer the *same* identity input without an explicit equality
    // constraint, the same pattern `migrate` already uses across its two evaluations.
    let identity_input = derive_identity_input(salted_dg1.value);

    let mut terms: [Field; NUM_COMMITTEES] = [0; NUM_COMMITTEES];
    let mut oprf_pk_hashes: [Field; NUM_COMMITTEES] = [0; NUM_COMMITTEES];
    for i in 0..NUM_COMMITTEES {
        let oprf_proof = oprf_proofs[i];

        // Recomputes the blinded query from `beta` and `identity_input`, checks committee
        // `i`'s DLog-equality proof against it, checks the client unblinded correctly, and
        // derives that committee's OPRF output. Any mismatch — including a committee response
        // for a *different* input than the one this circuit derived — fails here.
        let oprf_output = verified_oprf(
            oprf_proof.beta,
            oprf_proof.pk,
            oprf_proof.dlog_e,
            oprf_proof.dlog_s,
            oprf_proof.response_blinded,
            oprf_proof.response,
            identity_input,
            DS_DLOG,
            DS_ANCHOR_OUT,
        );

        terms[i] = derive_committee_anchor_term(i, oprf_output, scheme_version);
        oprf_pk_hashes[i] = Poseidon2::hash([oprf_proof.pk.x, oprf_proof.pk.y], 2);
    }

    let anchor = combine_committee_anchors(terms);

    (anchor, oprf_pk_hashes)
}
