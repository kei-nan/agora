//! **DEV-ONLY.** Runs the simulated dual-committee-generation OPRF flow (changelog entry 78's
//! follow-on: "run the same flow twice, under two independent committee key generations")
//! and prints a `Prover.toml` for `circuits/oprf-identity-anchor`'s `migrate` package.
//!
//! `old_oprf_proofs` deliberately reuses the *exact* recipe
//! `generate_anchor_prover_toml.rs` used (same RNG seed `0xA6012`, same `identity_input`,
//! same `TEST_BETA`) — that recipe is deterministic, so it regenerates byte-identical
//! committee keys and byte-identical proofs, standing in for "the committee generation the
//! citizen originally registered under." `new_oprf_proofs` is an independently-generated
//! committee set (different seed, different beta) standing in for the post-rotation scheme.
//! Because both sides are driven from the same `identity_input`, `old_anchor` in this
//! circuit's output should equal the `anchor` value `anchor/Prover.toml`'s own proof already
//! produced (changelog entry 78: `0x0beff326e082ed177b5ad64c97336db7af826a90a072cdb18f65de2ac6d5326f`)
//! — a real cross-circuit consistency check, not just "it compiles."

use ark_bn254::Fr;
use ark_ff::PrimeField;
use num_bigint::BigUint;
use oprf_committee_dev::prover_toml::{generate_proof_set, render_migrate_prover_toml, MigrateFixture};
use rand::SeedableRng;

fn fe_hex(h: &str) -> Fr {
    Fr::from_be_bytes_mod_order(&hex::decode(h).unwrap())
}

fn main() {
    // `identity_anchor::derive_identity_input(SAMPLE_DG1)` — same fixture every Prover.toml
    // in this crate uses, see `generate_anchor_prover_toml.rs`'s module docs for provenance.
    let identity_input =
        fe_hex("27e62fda546af6970d595963b00dbb8af2ae8fe08060b48e40c3e3d29cab6b46");
    let comm_in = "0x09b01eae21f4d04f3e2e513020415e549e5322003a7dd77e17e465dca7949699";

    // `identity_anchor::DS_DLOG`, same value every circuit in this workspace uses.
    let ds_dlog = Fr::from_be_bytes_mod_order(
        &BigUint::parse_bytes(b"1523098184080632582082867317389990410064981862", 10)
            .unwrap()
            .to_bytes_be(),
    );

    // Old side: byte-identical to `generate_anchor_prover_toml.rs` — same seed, same beta —
    // so `old_anchor` here should match `anchor/Prover.toml`'s already-proven `anchor` output.
    let old_beta = BigUint::parse_bytes(
        b"63865932500786004558985758765891911620034145599124743624338496685411118977",
        10,
    )
    .unwrap();
    let mut old_rng = rand::rngs::StdRng::seed_from_u64(0xA6012);
    let old_proofs = generate_proof_set(&mut old_rng, identity_input, &old_beta, ds_dlog);

    // New side: an independently-generated committee set (different seed) and a fresh beta —
    // the citizen re-querying the same identity_input against the post-rotation committees.
    let new_beta = BigUint::parse_bytes(
        b"41182991887665293104223948844221039920178327766195412027819128736409813371",
        10,
    )
    .unwrap();
    let mut new_rng = rand::rngs::StdRng::seed_from_u64(0xA6099);
    let new_proofs = generate_proof_set(&mut new_rng, identity_input, &new_beta, ds_dlog);

    let fixture = MigrateFixture {
        comm_in: comm_in.to_string(),
        old_scheme_version: "1".to_string(),
        new_scheme_version: "2".to_string(),
    };
    let toml = render_migrate_prover_toml(&fixture, &old_proofs, &new_proofs);
    print!("{toml}");
}
