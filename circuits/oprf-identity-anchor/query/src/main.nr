//! Agora identity-anchor OPRF **query** circuit.
//!
//! Direct fork of `src/noir/bin/oprf-auth/src/main.nr` from
//! zkpassport/circuits @ d3a75acb8529e82c61be136a402553daec259257.
//!
//! The only behavioural change is *what value gets blinded*. Upstream blinds
//! `salted_private_nullifier.value`, i.e. `calculate_private_nullifier(dg1, e_content,
//! sod_signature)` — a document-bound value that changes on every passport renewal. This
//! fork blinds `derive_identity_input(dg1)` instead: the TD3 MRZ personal-number field
//! (ICAO Doc 9303 Part 4, line 2 positions 29-42) folded together with the issuing country.
//! That field is renewal-stable, which is the whole point (changelog entry 67).
//!
//! Both values are byte ranges of DG1, and DG1 is already bound into `comm_in` by exactly
//! the same commitment the upstream circuit checks. Nothing upstream in the commitment
//! chain changes — see the README for why that is a narrower fork than changelog entry 67
//! anticipated.
//!
//! This proof is verified by the OPRF committee nodes, not on-chain. Its purpose is to stop
//! the committee from being used as a blind oracle for arbitrary inputs: it proves the
//! blinded point was built from a personal-number field belonging to a genuine,
//! government-signed passport, without revealing which one.

use common::hash_salt_dg1_dg2_hash_private_nullifier;
use identity_anchor::{assert_dg1_clear_value, derive_identity_input};
use oprf::blinded_query::blinded_query;
use utils::types::{DG1Data, MRZExpiryDate, SaltedValue};

/// Public inputs:  `comm_in`
/// Public outputs: `(blinded_query_x, blinded_query_y)`
///
/// `comm_in` is the `comm_out` of the integrity-check subproof, unchanged from upstream —
/// the same value `oprf-auth` and the disclosure circuits consume.
///
/// `salted_private_nullifier` is still required in order to reconstruct `comm_in`, but its
/// clear value is deliberately *not* used here; it may (and in Agora's flow should) be
/// supplied in hash-only mode via `SaltedValue::from_hash`, keeping the document-bound
/// per-vote nullifier entirely out of the anchor derivation path. Changelog entry 67
/// requires exactly this separation: the anchor must never be correlatable with voting
/// activity.
fn main(
    comm_in: pub Field,
    salted_dg1: SaltedValue<DG1Data>,
    salted_expiry_date: SaltedValue<MRZExpiryDate>,
    salted_dg2_hash: SaltedValue<Field>,
    salted_dg2_hash_type: SaltedValue<u32>,
    salted_private_nullifier: SaltedValue<Field>,
    beta: Field,
) -> pub (Field, Field) {
    // OPRF servers will verify `comm_in` here is the same as the one in the facematch proof.
    // Verify the commitment matches the provided salted values.
    assert(
        comm_in
            == hash_salt_dg1_dg2_hash_private_nullifier(
                salted_dg1,
                salted_expiry_date,
                salted_dg2_hash,
                salted_dg2_hash_type,
                salted_private_nullifier,
            ),
        "Commitment mismatch in oprf_identity_anchor_query",
    );

    // Unlike upstream, this circuit reads DG1's *bytes*, so the hash-only form of
    // `SaltedValue` must be rejected — see `assert_dg1_clear_value`.
    assert_dg1_clear_value(salted_dg1);

    // Compute the blinded query from the renewal-stable identity input rather than from the
    // document-bound private nullifier.
    let blinded = blinded_query(beta, derive_identity_input(salted_dg1.value));
    (blinded.x, blinded.y)
}

mod tests {
    use super::main;
    use common::hash_salt_dg1_dg2_hash_private_nullifier;
    use utils::constants::{SAMPLE_DG1, SHA256_IDENTIFIER};
    use utils::types::{DG1Data, MRZExpiryDate, SaltedValue};

    /// Blinding randomness. Must be a valid, non-zero BabyJubJub scalar; reused from
    /// `oprf-nr`'s own `oprf_example/Prover.toml` so it is known-good.
    global TEST_BETA: Field =
        63865932500786004558985758765891911620034145599124743624338496685411118977;

    /// Builds the same fixture `Prover.toml` holds, so the committed prover input and this
    /// test cannot drift apart.
    pub fn fixture() -> (SaltedValue<DG1Data>, SaltedValue<MRZExpiryDate>, SaltedValue<Field>, SaltedValue<u32>, SaltedValue<Field>) {
        let salted_dg1: SaltedValue<DG1Data> =
            SaltedValue { salt: 1111, value: SAMPLE_DG1, hash: 0 };
        // "300101", the expiry date in SAMPLE_DG1's MRZ.
        let salted_expiry_date: SaltedValue<MRZExpiryDate> =
            SaltedValue { salt: 2222, value: [51, 48, 48, 49, 48, 49], hash: 0 };
        let salted_dg2_hash: SaltedValue<Field> =
            SaltedValue { salt: 3333, value: 4444, hash: 0 };
        let salted_dg2_hash_type: SaltedValue<u32> =
            SaltedValue { salt: 5555, value: SHA256_IDENTIFIER, hash: 0 };
        // Hash-only: Agora deliberately keeps the document-bound private nullifier out of the
        // anchor path entirely (changelog entry 67's anchor/nullifier separation).
        let salted_private_nullifier: SaltedValue<Field> =
            SaltedValue { salt: 0, value: 0, hash: 6666 };
        (
            salted_dg1, salted_expiry_date, salted_dg2_hash, salted_dg2_hash_type,
            salted_private_nullifier,
        )
    }

    /// End-to-end execution of the forked circuit: builds a commitment exactly the way the
    /// upstream integrity-check subproof would, feeds it in, and checks a usable blinded
    /// query point comes out. Prints `comm_in` so `Prover.toml` can be regenerated.
    #[test]
    fn executes_end_to_end_over_the_sample_passport() {
        let (dg1, expiry, dg2_hash, dg2_hash_type, private_nullifier) = fixture();
        let comm_in = hash_salt_dg1_dg2_hash_private_nullifier(
            dg1,
            expiry,
            dg2_hash,
            dg2_hash_type,
            private_nullifier,
        );
        println(f"comm_in = {comm_in}");

        let (x, y) = main(comm_in, dg1, expiry, dg2_hash, dg2_hash_type, private_nullifier, TEST_BETA);
        println(f"blinded_query = ({x}, {y})");
        assert(x != 0, "Blinded query x must be non-zero");
        assert(y != 0, "Blinded query y must be non-zero");
    }

    /// The commitment check must actually bind: a wrong `comm_in` has to fail.
    #[test(should_fail_with = "Commitment mismatch in oprf_identity_anchor_query")]
    fn rejects_a_mismatched_commitment() {
        let (dg1, expiry, dg2_hash, dg2_hash_type, private_nullifier) = fixture();
        let _ = main(1, dg1, expiry, dg2_hash, dg2_hash_type, private_nullifier, TEST_BETA);
    }

    /// A hash-only DG1 satisfies `comm_in` but leaves the personal number zeroed — the
    /// failure mode `assert_dg1_clear_value` exists to stop.
    #[test(should_fail_with = "DG1 must be provided as a clear value")]
    fn rejects_hash_only_dg1_end_to_end() {
        let (_, expiry, dg2_hash, dg2_hash_type, private_nullifier) = fixture();
        let hidden_dg1: SaltedValue<DG1Data> =
            SaltedValue { salt: 0, value: [0; 95], hash: 7777 };
        let comm_in = hash_salt_dg1_dg2_hash_private_nullifier(
            hidden_dg1,
            expiry,
            dg2_hash,
            dg2_hash_type,
            private_nullifier,
        );
        let _ = main(
            comm_in, hidden_dg1, expiry, dg2_hash, dg2_hash_type, private_nullifier, TEST_BETA,
        );
    }
}
