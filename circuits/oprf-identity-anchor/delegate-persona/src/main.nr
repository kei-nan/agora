//! Agora **delegate-persona** disclosure-subproof circuit — round 2 of the delegate-persona
//! two-round OPRF flow, and the pipeline-integrated (outer-circuit-riding) counterpart of
//! `../disclosure`, for delegate-persona creation instead of citizen registration.
//!
//! # What this is, and what it is not
//!
//! This runs **on demand**, whenever a registered citizen chooses to become a delegate — a
//! fresh, dedicated full proof event, entirely separate from `register_citizen`/
//! `reverify_citizen`. It is not folded into ordinary registration, and does not touch the
//! citizen's existing `anchor`.
//!
//! # Why it is disclosure-shaped, not standalone like `../anchor`
//!
//! Exactly the reasoning `../disclosure`'s own header gives for why a standalone `../anchor`
//! proof is unsafe on-chain applies here unchanged: `comm_in` is a *private* witness inside
//! ZKPassport's outer circuit, never among that proof's public inputs. A standalone
//! delegate-persona proof's `comm_in` would therefore be an unauthenticated value a prover
//! could pick freely, letting them pair a real outer passport proof with a delegate-persona
//! proof over a `comm_in` of their own invention — defeating the entire point of proving a
//! delegate persona was derived from a genuine, currently-valid passport. Riding inside the
//! outer proof as a disclosure subproof, with one shared `comm_in`, closes that the same way
//! `../disclosure` does for registration.
//!
//! # Binding `persona_account` (anti-front-running)
//!
//! `persona_account` — the AccountId the citizen is registering as their delegate persona — is
//! folded into `param_commitment` alongside `delegate_persona_id`/`scheme_version`/
//! `oprf_pk_hashes`, exactly the way `../migrate-disclosure` folds a wider payload than
//! `../disclosure` into its own single `param_commitment` slot when the fixed 8-field
//! outer-circuit layout has no room for extra public output slots (see that file's header).
//! Because `param_commitment` is itself one of the outer proof's cryptographically-verified
//! public inputs, an observer cannot take a valid proof and resubmit it against a *different*
//! `persona_account` than the one the prover committed to: the recomputed `param_commitment`
//! would no longer match any `param_commitments[i]` the outer proof exposes, so
//! `T::ZkVerifier::verify` (already run on the whole outer proof before a future caller checks
//! this commitment, exactly as `register_citizen` already does for `../disclosure`) would still
//! pass, but the Rust-side recomputation check below would fail. This is the same
//! recompute-and-check split `runtime/src/anchor_verifier.rs` already uses for registration —
//! see that module's docs for why the pairing check and the commitment recomputation are kept
//! separate rather than one function doing both.
//!
//! `persona_account` is a raw 32-byte value (an sr25519/ed25519 public key), not guaranteed to
//! be a canonical BN254 field element on its own, so it cannot be passed as a single `Field`
//! without risking silent truncation/reduction. It is packed into two `Field` limbs via
//! `utils::pack_be_bytes_into_fields::<32, 2, 31>` — the same helper
//! `SaltedValue<[u8; N]>::get_hash` (upstream `utils::types`) already uses to safely fold
//! arbitrary byte arrays into a Poseidon2 hash — before being folded into `param_commitment`.
//! `runtime/src/anchor_verifier.rs`'s Rust-side recomputation must reproduce this exact packing
//! byte-for-byte.
//!
//! # `delegate_persona_id`: why it is a genuinely separate OPRF evaluation
//!
//! See `identity_anchor::derive_delegate_identity_input`'s doc comment for the full reasoning.
//! In short: this circuit runs its own 5 `verified_oprf` calls against
//! `derive_delegate_identity_input(dg1)` (via `../delegate-query`'s round-1 blinded query, a
//! **separate** committee round-trip from the one `../disclosure` uses for `identity_input`),
//! not against the value `../disclosure` already evaluated. `delegate_persona_id` is therefore
//! an independent PRF output per committee, not merely a differently-tagged hash of the
//! citizen's existing `anchor` terms — deliberately avoiding a design that would rely solely on
//! Poseidon2 domain separation between two hashes sharing a secret preimage.
//!
//! # Status
//!
//! Compiles and produces a verification key, but — like `../disclosure` before it — has never
//! been run inside an actual outer proof or against a live OPRF committee. Treat the interface
//! match as carefully derived from `../disclosure`'s own upstream-matched shape, not as
//! empirically confirmed. See the README.

use common::hash_salt_dg1_dg2_hash_private_nullifier;
use data_check_expiry::check_expiry_from_date;
use identity_anchor::{
    assert_dg1_clear_value, combine_committee_anchors, derive_delegate_identity_input,
    derive_delegate_persona_term, DS_DELEGATE_OUT, DS_DLOG, NUM_COMMITTEES,
};
use oprf::oprf_output::verified_oprf;
use poseidon::poseidon2::Poseidon2;
use utils::pack_be_bytes_into_fields;
use utils::types::{DG1Data, MRZExpiryDate, OPRFProof, SaltedValue};

/// Agora-specific proof-type tag for the delegate-persona parameter commitment. Deliberately
/// distinct from `../disclosure`'s `PROOF_TYPE_AGORA_IDENTITY_ANCHOR` (200) and
/// `../migrate-disclosure`'s `PROOF_TYPE_AGORA_IDENTITY_ANCHOR_MIGRATE` (201), so a
/// delegate-persona commitment can never be confused with (or substituted for) a
/// registration/reverification or migration one — all three have different arities and
/// different Rust-side check obligations.
global PROOF_TYPE_AGORA_DELEGATE_PERSONA: Field = 202;

/// `param_commitment = Poseidon2(PROOF_TYPE_AGORA_DELEGATE_PERSONA, delegate_persona_id,
/// persona_account_lo, persona_account_hi, scheme_version, oprf_pk_hashes[0..5])` — 10
/// elements. Mirrors `../disclosure::calculate_param_commitment`'s shape (tag, then every
/// disclosed parameter in argument order), widened by the two `persona_account` limbs.
///
/// `persona_account_lo`/`persona_account_hi` are `pack_be_bytes_into_fields::<32, 2,
/// 31>(persona_account)`'s two output limbs, in `[lo, hi]` order (i.e. `result[0]`, the
/// 31-byte limb built from `persona_account[1..32]`, then `result[1]`, the 1-byte limb built
/// from `persona_account[0]`) — see that function's own byte-accumulation order in
/// `zkpassport/circuits`' `utils` crate. The Rust verifier must reproduce this split and this
/// order exactly.
fn calculate_param_commitment(
    delegate_persona_id: Field,
    persona_account_lo: Field,
    persona_account_hi: Field,
    scheme_version: Field,
    oprf_pk_hashes: [Field; NUM_COMMITTEES],
) -> Field {
    Poseidon2::hash(
        [
            PROOF_TYPE_AGORA_DELEGATE_PERSONA,
            delegate_persona_id,
            persona_account_lo,
            persona_account_hi,
            scheme_version,
            oprf_pk_hashes[0],
            oprf_pk_hashes[1],
            oprf_pk_hashes[2],
            oprf_pk_hashes[3],
            oprf_pk_hashes[4],
        ],
        10,
    )
}

/// Public inputs:  `comm_in`, `current_date`, `service_scope`, `service_subscope`
/// Public outputs: `(param_commitment, nullifier_type, scoped_nullifier, oprf_pk_hash)`
///
/// Same fixed 8-field outer-circuit-facing shape as `../disclosure`/`../migrate-disclosure` —
/// see either for why this shape cannot grow. `persona_account` is a plain (non-`pub`) witness:
/// it does not need its own outer-proof-visible slot, since it is folded into the publicly
/// exposed `param_commitment` above, the same way `scheme_version`/`oprf_pk_hashes` already are
/// in `../disclosure` without being separate public outputs themselves.
fn main(
    comm_in: pub Field,
    current_date: pub u64,
    scheme_version: Field,
    persona_account: [u8; 32],
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
        "Commitment mismatch in oprf_delegate_persona",
    );
    assert_dg1_clear_value(salted_dg1);
    assert(scheme_version != 0, "OPRF scheme version must be non-zero");
    assert(persona_account != [0; 32], "persona_account must not be the zero AccountId");

    // "Passport must be valid at registration AND at vote time" (CLAUDE.md) — becoming a
    // delegate is gated on currently holding a valid, unexpired passport, same as
    // registration/reverification/migration.
    check_expiry_from_date(salted_expiry_date.value, current_date);

    // A *separate* client input from `../disclosure`'s `identity_input` — see
    // `identity_anchor::derive_delegate_identity_input`'s doc comment for why this must not be
    // the same value (or a cached evaluation of it) reused under a different outer tag.
    let delegate_identity_input = derive_delegate_identity_input(salted_dg1.value);

    let mut terms: [Field; NUM_COMMITTEES] = [0; NUM_COMMITTEES];
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
            delegate_identity_input,
            DS_DLOG,
            DS_DELEGATE_OUT,
        );
        terms[i] = derive_delegate_persona_term(i, oprf_output, scheme_version);
        oprf_pk_hashes[i] = Poseidon2::hash([oprf_proof.pk.x, oprf_proof.pk.y], 2);
    }

    let delegate_persona_id = combine_committee_anchors(terms);

    let persona_account_fields: [Field; 2] =
        pack_be_bytes_into_fields::<32, 2, 31>(persona_account);
    let param_commitment = calculate_param_commitment(
        delegate_persona_id,
        persona_account_fields[0],
        persona_account_fields[1],
        scheme_version,
        oprf_pk_hashes,
    );

    // Silence unused-parameter warnings without changing the constrained system: these two
    // exist purely to occupy their slots in the outer circuit's fixed public-input layout.
    let _ = service_scope;
    let _ = service_subscope;

    (param_commitment, 0, 0, 0)
}
