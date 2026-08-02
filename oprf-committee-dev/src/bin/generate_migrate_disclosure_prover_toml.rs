//! **DEV-ONLY.** Same dual-committee-generation flow as `generate_migrate_prover_toml.rs`,
//! printing a `Prover.toml` for `circuits/oprf-identity-anchor`'s `migrate-disclosure`
//! package instead — adds `current_date`/`service_scope`/`service_subscope` over `migrate`'s
//! fixture, exactly as `disclosure/Prover.toml` adds them over `anchor`'s.

use ark_bn254::Fr;
use ark_ff::PrimeField;
use num_bigint::BigUint;
use oprf_committee_dev::prover_toml::{
    generate_proof_set, render_migrate_disclosure_prover_toml, MigrateDisclosureFixture,
};
use rand::SeedableRng;

fn fe_hex(h: &str) -> Fr {
    Fr::from_be_bytes_mod_order(&hex::decode(h).unwrap())
}

fn main() {
    let identity_input =
        fe_hex("27e62fda546af6970d595963b00dbb8af2ae8fe08060b48e40c3e3d29cab6b46");
    let comm_in = "0x09b01eae21f4d04f3e2e513020415e549e5322003a7dd77e17e465dca7949699";

    let ds_dlog = Fr::from_be_bytes_mod_order(
        &BigUint::parse_bytes(b"1523098184080632582082867317389990410064981862", 10)
            .unwrap()
            .to_bytes_be(),
    );

    // Same recipe as generate_migrate_prover_toml.rs — see that file's module docs. Reusing
    // the identical old-side seed/beta means this circuit's `old_anchor` output should also
    // match anchor/Prover.toml's already-proven anchor.
    let old_beta = BigUint::parse_bytes(
        b"63865932500786004558985758765891911620034145599124743624338496685411118977",
        10,
    )
    .unwrap();
    let mut old_rng = rand::rngs::StdRng::seed_from_u64(0xA6012);
    let old_proofs = generate_proof_set(&mut old_rng, identity_input, &old_beta, ds_dlog);

    let new_beta = BigUint::parse_bytes(
        b"41182991887665293104223948844221039920178327766195412027819128736409813371",
        10,
    )
    .unwrap();
    let mut new_rng = rand::rngs::StdRng::seed_from_u64(0xA6099);
    let new_proofs = generate_proof_set(&mut new_rng, identity_input, &new_beta, ds_dlog);

    // Matches disclosure/Prover.toml's own current_date/service_scope/service_subscope
    // (2026-08-02, well inside the fixture's 2030-01-01 MRZ expiry) and the same arbitrary
    // service-scope pair, for continuity across this crate's generated fixtures.
    let fixture = MigrateDisclosureFixture {
        comm_in: comm_in.to_string(),
        old_scheme_version: "1".to_string(),
        new_scheme_version: "2".to_string(),
        current_date: "1785628800".to_string(),
        service_scope: "111".to_string(),
        service_subscope: "222".to_string(),
    };
    let toml = render_migrate_disclosure_prover_toml(&fixture, &old_proofs, &new_proofs);
    print!("{toml}");
}
