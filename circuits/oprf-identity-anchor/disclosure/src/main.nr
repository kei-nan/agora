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
//! # Binding `bound_account` (anti-front-running)
//!
//! `bound_account` — the `AccountId` this proof authorizes to call `register_citizen`/
//! `reverify_citizen`/`recover_account`/`migrate_oprf_scheme` with it — is folded into
//! `param_commitment` for exactly the reason `../delegate-persona` folds in `persona_account`
//! (see that circuit's own doc comment for the full rationale): without it, all proof material
//! is plaintext in a pending signed extrinsic, so anyone who observes a victim's pending
//! `register_citizen`/`recover_account` call could copy it verbatim into their own signed call
//! and get it mined first — stealing a citizenship slot, or (for `recover_account`, which has
//! no dispute window by design) permanently and irreversibly hijacking the victim's identity.
//! Folding `bound_account` into the cryptographically-verified `param_commitment` closes this:
//! a copied proof resubmitted under a different signer no longer recomputes to any
//! `param_commitments[i]` the outer proof exposes, so the Rust-side recomputation check
//! (`runtime/src/anchor_verifier.rs`) fails even though the outer proof's own pairing check
//! still passes. Packed via the same `utils::pack_be_bytes_into_fields::<32, 2, 31>` scheme
//! `../delegate-persona` uses, in the same `[lo, hi]` order — see that circuit's doc comment
//! for the exact byte-accumulation details the Rust verifier must reproduce.
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
    assert_dg1_clear_value, combine_committee_anchors, derive_backing_commitment,
    derive_backing_root_term, derive_committee_anchor_term, derive_identity_input, DS_ANCHOR_OUT,
    DS_DLOG, NUM_COMMITTEES,
};
use oprf::oprf_output::verified_oprf;
use poseidon::poseidon2::Poseidon2;
use utils::pack_be_bytes_into_fields;
use utils::types::{DG1Data, MRZExpiryDate, OPRFProof, SaltedValue};

/// ZKPassport's own `PROOF_TYPE_*` constants (`utils::constants`) currently occupy 0-10. Agora
/// tags live at 200+, deliberately far outside that range so an Agora extension can never be
/// confused with an upstream proof type, even if upstream adds more.
///
/// Proof-type tag 200 (`PROOF_TYPE_AGORA_IDENTITY_ANCHOR`, the original unbound 9-element
/// commitment shape) is retired — superseded outright by
/// [`PROOF_TYPE_AGORA_IDENTITY_ANCHOR_BOUND`] below, not run in parallel with it, since no real
/// citizen has ever registered on this chain (see CLAUDE.md) and there is no migration to
/// preserve. No `global` constant is declared for it (an unused one would just be dead code);
/// 200 must never be reused for anything else regardless.
///
/// Agora's proof-type tag for the **account-bound** parameter commitment — closes the
/// front-running/identity-hijack gap described above. Deliberately distinct from the retired
/// 200 above, from `../migrate-disclosure`'s
/// `PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE_BOUND` (204), and from `../delegate-persona`'s
/// `PROOF_TYPE_AGORA_DELEGATE_PERSONA` (202, already account-bound from the start) — every
/// Agora proof-type tag identifies exactly one commitment arity and binding shape.
global PROOF_TYPE_AGORA_IDENTITY_ANCHOR_BOUND: Field = 203;

/// `param_commitment = Poseidon2(PROOF_TYPE_AGORA_IDENTITY_ANCHOR_BOUND, anchor, scheme_version,
/// oprf_pk_hashes[0], ..., oprf_pk_hashes[4], backing_commitment, bound_account_lo,
/// bound_account_hi)`
///
/// Follows the same shape as `bind::calculate_param_commitment` and
/// `compare_age::calculate_param_commitment`: a proof-type tag followed by the disclosed
/// parameters. The on-chain verifier recomputes this from the (anchor, scheme_version,
/// 5×oprf_pk_hash, backing_commitment, bound_account) tuple submitted alongside the extrinsic
/// and checks it against the outer proof's `param_commitments[i]`. There is no room to carry
/// the 5 committee-key hashes (or `backing_commitment`/`bound_account`) as separate
/// disclosure-subproof outputs (the fixed 8-field outer layout only has one `oprf_pk_hash`
/// slot, and that slot belongs to a different OPRF entirely — see below), so all of them are
/// folded into this single field instead. `backing_commitment` rides the same tag as
/// `bound_account` (no separate tag of its own) — unlike `bound_account`, it binds to no
/// specific target/delegate and carries no front-running risk; it simply widens alongside
/// `bound_account` here because both live in the same preimage.
///
/// The anchor committees' `oprf_pk_hashes` are carried *here* rather than in the subproof's
/// dedicated `oprf_pk_hash` output because that output belongs to ZKPassport's own
/// salted-*nullifier* OPRF, which is a different key for a different purpose. This circuit
/// emits 0 there — the "non-participating" mode the outer circuit already supports for
/// facematch subproofs.
fn calculate_param_commitment(
    anchor: Field,
    scheme_version: Field,
    oprf_pk_hashes: [Field; NUM_COMMITTEES],
    backing_commitment: Field,
    bound_account_lo: Field,
    bound_account_hi: Field,
) -> Field {
    Poseidon2::hash(
        [
            PROOF_TYPE_AGORA_IDENTITY_ANCHOR_BOUND,
            anchor,
            scheme_version,
            oprf_pk_hashes[0],
            oprf_pk_hashes[1],
            oprf_pk_hashes[2],
            oprf_pk_hashes[3],
            oprf_pk_hashes[4],
            backing_commitment,
            bound_account_lo,
            bound_account_hi,
        ],
        11,
    )
}

/// Public inputs:  `comm_in`, `current_date`, `service_scope`, `service_subscope`
/// Public outputs: `(param_commitment, nullifier_type, scoped_nullifier, oprf_pk_hash)`
///
/// The last three are always 0 — see `calculate_param_commitment` above. `service_scope` and
/// `service_subscope` are unused by this circuit but must be declared, in this position, to
/// match the fixed 8-field layout the outer circuit feeds to `verify_proof_with_type`.
/// `bound_account` is a private (non-`pub`) witness, same placement as
/// `../delegate-persona::main`'s `persona_account` — it does not need its own outer-proof-
/// visible slot, since it is folded into the publicly exposed `param_commitment` above.
fn main(
    comm_in: pub Field,
    current_date: pub u64,
    scheme_version: Field,
    bound_account: [u8; 32],
    salted_dg1: SaltedValue<DG1Data>,
    salted_expiry_date: SaltedValue<MRZExpiryDate>,
    salted_dg2_hash: SaltedValue<Field>,
    salted_dg2_hash_type: SaltedValue<u32>,
    salted_private_nullifier: SaltedValue<Field>,
    oprf_proofs: [OPRFProof; NUM_COMMITTEES],
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
    assert(bound_account != [0; 32], "bound_account must not be the zero AccountId");

    // CLAUDE.md: "Passport must be valid at registration AND at vote time". This covers the
    // registration/re-verification half, and is what makes a single proof of this circuit
    // sufficient for `AnchorProofVerifier::verify_reverification`.
    check_expiry_from_date(salted_expiry_date.value, current_date);

    // Single binding, consumed by all 5 committee evaluations — see `anchor/src/main.nr` for
    // why this is what proves the 5 committees answered the same identity input.
    let identity_input = derive_identity_input(salted_dg1.value);

    let mut terms: [Field; NUM_COMMITTEES] = [0; NUM_COMMITTEES];
    let mut backing_terms: [Field; NUM_COMMITTEES] = [0; NUM_COMMITTEES];
    let mut oprf_pk_hashes: [Field; NUM_COMMITTEES] = [0; NUM_COMMITTEES];
    for i in 0..NUM_COMMITTEES {
        let oprf_proof = oprf_proofs[i];
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
        // Same `oprf_output` witness as `terms[i]` above, a different domain-separated term —
        // see `derive_backing_root_term`'s doc comment for why this reuse is safe here (unlike
        // the delegate-persona circuits' deliberate fresh-query scoping).
        backing_terms[i] = derive_backing_root_term(i, oprf_output, scheme_version);
        oprf_pk_hashes[i] = Poseidon2::hash([oprf_proof.pk.x, oprf_proof.pk.y], 2);
    }

    let anchor = combine_committee_anchors(terms);
    let backing_root_secret = combine_committee_anchors(backing_terms);
    let backing_commitment = derive_backing_commitment(backing_root_secret);

    let bound_account_fields: [Field; 2] = pack_be_bytes_into_fields::<32, 2, 31>(bound_account);
    let param_commitment = calculate_param_commitment(
        anchor,
        scheme_version,
        oprf_pk_hashes,
        backing_commitment,
        bound_account_fields[0],
        bound_account_fields[1],
    );

    // Silence unused-parameter warnings without changing the constrained system: these two
    // exist purely to occupy their slots in the outer circuit's fixed public-input layout.
    let _ = service_scope;
    let _ = service_subscope;

    // (param_commitment, nullifier_type, scoped_nullifier, oprf_pk_hash)
    // The trailing three are 0: this subproof does not participate in scoped-nullifier
    // derivation, matching how the facematch circuits behave.
    (param_commitment, 0, 0, 0)
}
