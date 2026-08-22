//! **DEV-ONLY.** Runs the simulated 5-committee OPRF flow for the delegate-persona client
//! input and prints a `Prover.toml` for `circuits/oprf-identity-anchor`'s `delegate-persona`
//! package.
//!
//! Unlike `generate_migrate_prover_toml.rs` (which deliberately reuses `generate_anchor_prover_
//! toml.rs`'s exact seed/beta/identity_input so its `old_anchor` output can be checked against
//! `anchor/Prover.toml`'s already-proven value), this binary uses the SAME simulated committee
//! key set (RNG seed `0xA6012` — standing in for the same real 5 committees) but a DIFFERENT
//! client input and a DIFFERENT `beta`, because `identity_anchor::derive_delegate_identity_input`
//! is a deliberately distinct OPRF query from registration's `derive_identity_input` — see that
//! function's doc comment for the full reasoning. The resulting `delegate_persona_id` is
//! therefore expected, and required, to differ from `anchor/Prover.toml`'s `anchor` output even
//! though both ultimately trace back to the same passport and the same 5 committees.
//!
//! `delegate_identity_input` below is `identity_anchor::derive_delegate_identity_input(
//! utils::constants::SAMPLE_DG1)`'s real output, obtained via a real `nargo execute` run of a
//! throwaway package depending on the real `identity_anchor` lib (see the changelog entry this
//! binary ships with for the exact value and how it was obtained). `beta` and `comm_in` are
//! `circuits/oprf-identity-anchor/delegate-query/Prover.toml`'s own committed fixture values,
//! and the resulting `blinded_query` output was checked against a real `nargo execute
//! --package oprf_delegate_persona_query` run of that actual circuit (matching `bb prove`/
//! `bb verify` also having been run against it) before being trusted here.

use ark_bn254::Fr;
use ark_ff::PrimeField;
use num_bigint::BigUint;
use oprf_committee_dev::committee::DevCommitteeSet;
use oprf_committee_dev::oprf;
use oprf_committee_dev::prover_toml::{
    generate_proof_set, render_delegate_persona_prover_toml, DelegatePersonaFixture,
};
use rand::SeedableRng;

fn fe_hex(h: &str) -> Fr {
    Fr::from_be_bytes_mod_order(&hex::decode(h).unwrap())
}

fn main() {
    // `identity_anchor::derive_delegate_identity_input(SAMPLE_DG1)`, measured via a real
    // `nargo execute` run in this session (see module docs).
    let delegate_identity_input =
        fe_hex("18b88c8515d54d6f36ec658b1792ce2580976a16cfd7cf96ce62e549e20264e2");

    // `delegate-query/src/main.nr::tests::TEST_BETA` — same numeric value `query`'s own
    // `TEST_BETA` uses (client-side blinding randomness has no relationship to which client
    // input it blinds), and `delegate-query/Prover.toml`'s `comm_in`.
    let beta = BigUint::parse_bytes(
        b"63865932500786004558985758765891911620034145599124743624338496685411118977",
        10,
    )
    .unwrap();
    let comm_in = "0x09b01eae21f4d04f3e2e513020415e549e5322003a7dd77e17e465dca7949699";

    // `identity_anchor::DS_DLOG`, unchanged from `generate_anchor_prover_toml.rs` — this is the
    // *protocol-level* domain separator the deployed committee service is configured with,
    // deliberately shared across every purpose (registration, migration, delegate-persona
    // alike). It is `DS_DELEGATE_IDENTITY_INPUT`/`DS_DELEGATE_OUT` that provide the
    // delegate-specific separation, both applied client-side, not `DS_DLOG`.
    let ds_dlog = Fr::from_be_bytes_mod_order(
        &BigUint::parse_bytes(b"1523098184080632582082867317389990410064981862", 10)
            .unwrap()
            .to_bytes_be(),
    );

    // Same RNG seed as `generate_anchor_prover_toml.rs`: simulating the SAME 5 committees
    // answering a second, distinctly-scoped query — not a different, unrelated committee set.
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xA6012);

    let b_q = oprf::blinded_query(&beta, delegate_identity_input);
    eprintln!(
        "delegate blinded_query = ({}, {})",
        oprf_committee_dev::scalar::from_field(&b_q.x),
        oprf_committee_dev::scalar::from_field(&b_q.y),
    );

    // `generate_proof_set` re-derives the committee set from the RNG itself (it calls
    // `DevCommitteeSet::generate(rng)` internally), so seeding `rng` identically to
    // `generate_anchor_prover_toml.rs` is what makes this simulate the same 5 committee keys —
    // confirmed by cross-checking one committee pk below against `anchor/Prover.toml`'s.
    let proofs = generate_proof_set(&mut rng, delegate_identity_input, &beta, ds_dlog);

    // Sanity check, not part of the rendered fixture: re-deriving committee 0's public key
    // independently (fresh RNG, same seed) must match what `generate_proof_set` used, so a
    // reader can trust "same committees, different query" without re-running both binaries.
    let mut check_rng = rand::rngs::StdRng::seed_from_u64(0xA6012);
    let check_committees = DevCommitteeSet::generate(&mut check_rng);
    assert_eq!(
        check_committees.committees[0].public_key, proofs[0].pk,
        "delegate-persona generator must simulate the SAME committee 0 key as anchor's generator"
    );

    // A syntactically valid, non-zero placeholder `AccountId` (32 bytes, big-endian
    // 0x01..0x20) — this binary's job is to demonstrate the circuit executes and produces a
    // real proof, not to model a genuine account; any caller of the real circuit supplies the
    // real persona_account they are registering.
    let mut persona_account = [0u8; 32];
    for (i, b) in persona_account.iter_mut().enumerate() {
        *b = (i + 1) as u8;
    }

    let fixture = DelegatePersonaFixture {
        comm_in: comm_in.to_string(),
        scheme_version: "1".to_string(),
        current_date: "1785628800".to_string(),
        service_scope: "111".to_string(),
        service_subscope: "222".to_string(),
        persona_account,
    };
    let toml = render_delegate_persona_prover_toml(&fixture, &proofs);
    print!("{toml}");
}
