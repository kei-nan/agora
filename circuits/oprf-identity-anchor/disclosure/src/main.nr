//! Agora identity-anchor **disclosure-subproof** circuit — the pipeline-integrated form of
//! `../anchor`.
//!
//! # Why this exists
//!
//! `../anchor` proves "this anchor was derived from the personal-number field of the DG1
//! committed at `comm_in`". That is only half of what an on-chain verifier needs, because
//! `comm_in` is a *private* witness inside ZKPassport's outer circuit — it never appears in
//! that proof's public inputs (`certificate_registry_root`, `circuit_registry_root`,
//! `current_date`, `service_scope`, `service_subscope`, `param_commitments`,
//! `nullifier_type`, `scoped_nullifier`, `oprf_pk_hash`). So a standalone `../anchor` proof
//! and an outer passport proof cannot be linked to each other on-chain: nothing stops a
//! prover pairing a real outer proof with an anchor proof over a `comm_in` of their own
//! choosing, which would defeat the whole Sybil check.
//!
//! This variant closes that by matching the disclosure-subproof interface the outer circuit
//! already recursively verifies (`prepare_disclosure_inputs` in
//! `src/noir/lib/outer/src/lib.nr`), so the anchor rides out inside the outer proof's own
//! `param_commitments` array. One proof, one `comm_in`, authenticated end-to-end back to the
//! certificate registry root.
//!
//! Modelled on `src/noir/bin/bind/standard/src/main.nr` from zkpassport/circuits @
//! d3a75acb8529e82c61be136a402553daec259257 — same public-input ordering, same 4-tuple
//! return shape.
//!
//! # Status
//!
//! Compiles and produces a verification key, but has **never been run inside an actual outer
//! proof** — doing that needs the full ZKPassport subproof pipeline plus a live OPRF
//! committee, neither of which exists here yet. Treat the interface match as carefully
//! derived from the upstream source, not as empirically confirmed. See the README.

use common::hash_salt_dg1_dg2_hash_private_nullifier;
use data_check_expiry::check_expiry_from_date;
use identity_anchor::{
    assert_dg1_clear_value, derive_anchor, derive_identity_input, DS_ANCHOR_OUT, DS_DLOG,
};
use oprf::oprf_output::verified_oprf;
use poseidon::poseidon2::Poseidon2;
use utils::types::{DG1Data, MRZExpiryDate, OPRFProof, SaltedValue};

/// Agora-specific proof type tag for the parameter commitment.
///
/// ZKPassport's own `PROOF_TYPE_*` constants (`utils::constants`) currently occupy 0-10.
/// 200 is deliberately far outside that range so an Agora extension can never be confused
/// with an upstream proof type, even if upstream adds more.
global PROOF_TYPE_AGORA_IDENTITY_ANCHOR: Field = 200;

/// `param_commitment = Poseidon2(PROOF_TYPE_AGORA_IDENTITY_ANCHOR, anchor, scheme_version,
/// oprf_pk_hash)`
///
/// Follows the same shape as `bind::calculate_param_commitment` and
/// `compare_age::calculate_param_commitment`: a proof-type tag followed by the disclosed
/// parameters. The on-chain verifier recomputes this from the (anchor, scheme_version,
/// oprf_pk_hash) triple submitted alongside the extrinsic and checks it against the outer
/// proof's `param_commitments[i]`.
///
/// The anchor committee's `oprf_pk_hash` is carried *here* rather than in the subproof's
/// dedicated `oprf_pk_hash` output because that output belongs to ZKPassport's own
/// salted-*nullifier* OPRF, which is a different key for a different purpose. This circuit
/// emits 0 there — the "non-participating" mode the outer circuit already supports for
/// facematch subproofs.
fn calculate_param_commitment(anchor: Field, scheme_version: Field, oprf_pk_hash: Field) -> Field {
    Poseidon2::hash(
        [PROOF_TYPE_AGORA_IDENTITY_ANCHOR, anchor, scheme_version, oprf_pk_hash],
        4,
    )
}

/// Public inputs:  `comm_in`, `current_date`, `service_scope`, `service_subscope`
/// Public outputs: `(param_commitment, nullifier_type, scoped_nullifier, oprf_pk_hash)`
///
/// The last three are always 0 — see `calculate_param_commitment` above. `service_scope` and
/// `service_subscope` are unused by this circuit but must be declared, in this position, to
/// match the fixed 8-field layout the outer circuit feeds to `verify_proof_with_type`.
fn main(
    comm_in: pub Field,
    current_date: pub u64,
    scheme_version: Field,
    salted_dg1: SaltedValue<DG1Data>,
    salted_expiry_date: SaltedValue<MRZExpiryDate>,
    salted_dg2_hash: SaltedValue<Field>,
    salted_dg2_hash_type: SaltedValue<u32>,
    salted_private_nullifier: SaltedValue<Field>,
    oprf_proof: OPRFProof,
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
        "Commitment mismatch in oprf_identity_anchor_disclosure",
    );
    assert_dg1_clear_value(salted_dg1);
    assert(scheme_version != 0, "OPRF scheme version must be non-zero");

    // CLAUDE.md: "Passport must be valid at registration AND at vote time". This covers the
    // registration/re-verification half, and is what makes a single proof of this circuit
    // sufficient for `AnchorProofVerifier::verify_reverification`.
    check_expiry_from_date(salted_expiry_date.value, current_date);

    let identity_input = derive_identity_input(salted_dg1.value);
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
    let anchor_oprf_pk_hash = Poseidon2::hash([oprf_proof.pk.x, oprf_proof.pk.y], 2);
    let param_commitment =
        calculate_param_commitment(anchor, scheme_version, anchor_oprf_pk_hash);

    // Silence unused-parameter warnings without changing the constrained system: these two
    // exist purely to occupy their slots in the outer circuit's fixed public-input layout.
    let _ = service_scope;
    let _ = service_subscope;

    // (param_commitment, nullifier_type, scoped_nullifier, oprf_pk_hash)
    // The trailing three are 0: this subproof does not participate in scoped-nullifier
    // derivation, matching how the facematch circuits behave.
    (param_commitment, 0, 0, 0)
}
