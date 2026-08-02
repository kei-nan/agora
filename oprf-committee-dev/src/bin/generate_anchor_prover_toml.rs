//! **DEV-ONLY.** Runs the simulated 5-committee OPRF flow for one fixed identity input and
//! prints a `Prover.toml` for `circuits/oprf-identity-anchor`'s `anchor` package.
//!
//! `identity_input` below is not invented: it is
//! `identity_anchor::derive_identity_input(utils::constants::SAMPLE_DG1)`'s real output,
//! obtained by running an actual `nargo test --show-output` against a throwaway package
//! depending on the real `identity_anchor` lib (see this crate's README / the changelog
//! entry it ships with for the exact commands) — the same fixture
//! `circuits/oprf-identity-anchor/query/Prover.toml` and `query/src/main.nr`'s
//! `tests::fixture()` already commit to (`SAMPLE_DG1`, salt `1111`, `comm_in =
//! 0x09b01eae21f4d04f3e2e513020415e549e5322003a7dd77e17e465dca7949699`). `beta` is that same
//! file's `TEST_BETA`. Using the query circuit's own already-documented fixture (rather than
//! inventing a new one) means this binary's `blinded_query` output can be, and was, checked
//! against a real `nargo execute --package oprf_identity_anchor_query` run of the actual
//! circuit before being trusted (see `oprf-committee-dev`'s tests).

use ark_bn254::Fr;
use ark_ff::PrimeField;
use num_bigint::BigUint;
use oprf_committee_dev::babyjubjub::Point;
use oprf_committee_dev::committee::DevCommitteeSet;
use oprf_committee_dev::oprf;
use oprf_committee_dev::prover_toml::{render_anchor_prover_toml, AnchorFixture, OprfProofWitness};
use rand::SeedableRng;

fn fe_hex(h: &str) -> Fr {
    Fr::from_be_bytes_mod_order(&hex::decode(h).unwrap())
}

fn main() {
    // `identity_anchor::derive_identity_input(SAMPLE_DG1)`, measured via real `nargo test
    // --show-output` in this session (see module docs).
    let identity_input =
        fe_hex("27e62fda546af6970d595963b00dbb8af2ae8fe08060b48e40c3e3d29cab6b46");

    // `query/src/main.nr::tests::TEST_BETA`, and `query/Prover.toml`'s `comm_in`.
    let beta = BigUint::parse_bytes(
        b"63865932500786004558985758765891911620034145599124743624338496685411118977",
        10,
    )
    .unwrap();
    let comm_in = "0x09b01eae21f4d04f3e2e513020415e549e5322003a7dd77e17e465dca7949699";

    // `identity_anchor::DS_DLOG` — ASCII "DLOG Equality Proof", byte-identical to
    // ZKPassport's own `DS_DLOG` per the README (the value a real committee service is
    // configured with).
    let ds_dlog = Fr::from_be_bytes_mod_order(
        &BigUint::parse_bytes(b"1523098184080632582082867317389990410064981862", 10)
            .unwrap()
            .to_bytes_be(),
    );

    let mut rng = rand::rngs::StdRng::seed_from_u64(0xA6012);

    let b_q: Point = oprf::blinded_query(&beta, identity_input);
    eprintln!(
        "blinded_query = ({}, {})",
        oprf_committee_dev::scalar::from_field(&b_q.x),
        oprf_committee_dev::scalar::from_field(&b_q.y),
    );

    let committees = DevCommitteeSet::generate(&mut rng);
    let evaluations = committees.evaluate_all(&mut rng, &b_q, ds_dlog);

    let mut proofs: Vec<OprfProofWitness> = Vec::with_capacity(5);
    for eval in evaluations.iter() {
        let response = oprf::unblind(&eval.response_blinded, &beta);
        proofs.push(OprfProofWitness::from_evaluation(eval, response, beta.clone()));
    }
    let proofs: [OprfProofWitness; 5] = proofs.try_into().unwrap_or_else(|_| panic!("exactly 5"));

    let fixture = AnchorFixture {
        comm_in: comm_in.to_string(),
        scheme_version: "1".to_string(),
    };
    let toml = render_anchor_prover_toml(&fixture, &proofs);
    print!("{toml}");
}
