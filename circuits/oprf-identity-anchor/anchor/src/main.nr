//! Agora identity-**anchor** circuit.
//!
//! Second half of the two-round OPRF flow. `query` blinds the identity input and sends it to
//! the committee; the committee evaluates it under its secret key and returns a blinded
//! response plus a Chaum-Pedersen DLog-equality proof; this circuit checks that response and
//! turns it into the anchor value published on-chain.
//!
//! Structurally this mirrors `nullify` in
//! `src/noir/lib/commitment/scoped-nullifier/src/lib.nr` of zkpassport/circuits @
//! d3a75acb8529e82c61be136a402553daec259257 — same `verified_oprf` call, same commitment
//! check — but it emits an anchor over the renewal-stable identity input instead of a scoped
//! nullifier over the document-bound private nullifier, and it deliberately does *not* take
//! `service_scope`/`service_subscope`: the anchor is scope-free by design, checked once at
//! registration and never used for voting.
//!
//! This proof is what `pallet-identity`'s `AnchorProofVerifier::verify_registration_anchor`
//! (and `verify_reverification`, which has an identical layout) must ultimately verify.

use common::hash_salt_dg1_dg2_hash_private_nullifier;
use identity_anchor::{
    assert_dg1_clear_value, derive_anchor, derive_identity_input, DS_ANCHOR_OUT, DS_DLOG,
};
use oprf::oprf_output::verified_oprf;
use poseidon::poseidon2::Poseidon2;
use utils::types::{DG1Data, MRZExpiryDate, OPRFProof, SaltedValue};

/// Public inputs:  `comm_in`, `scheme_version`
/// Public outputs: `(anchor, oprf_pk_hash)`
///
/// `oprf_pk_hash = Poseidon2(oprf_pk.x, oprf_pk.y)` is emitted rather than taken as an input
/// so the verifier learns *which* committee key produced the evaluation. The circuit cannot
/// itself decide whether that key is the legitimate one for `scheme_version` — that binding
/// lives on-chain and must be enforced by the Rust verifier. See the README.
fn main(
    comm_in: pub Field,
    scheme_version: pub Field,
    salted_dg1: SaltedValue<DG1Data>,
    salted_expiry_date: SaltedValue<MRZExpiryDate>,
    salted_dg2_hash: SaltedValue<Field>,
    salted_dg2_hash_type: SaltedValue<u32>,
    salted_private_nullifier: SaltedValue<Field>,
    oprf_proof: OPRFProof,
) -> pub (Field, Field) {
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

    let identity_input = derive_identity_input(salted_dg1.value);

    // Recomputes the blinded query from `beta` and `identity_input`, checks the committee's
    // DLog-equality proof against it, checks the client unblinded correctly, and derives the
    // final OPRF output. Any mismatch — including a committee response for a *different*
    // input than the one this circuit derived — fails here.
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

    let anchor = derive_anchor(oprf_output, scheme_version);
    let oprf_pk_hash = Poseidon2::hash([oprf_proof.pk.x, oprf_proof.pk.y], 2);

    (anchor, oprf_pk_hash)
}
