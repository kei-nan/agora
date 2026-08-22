//! Agora **delegate-persona** OPRF query circuit — round 1 of the delegate-persona two-round
//! OPRF flow, exactly as `../query` is round 1 of the citizen-registration flow.
//!
//! Structurally identical to `../query`, with exactly one change: it blinds
//! `derive_delegate_identity_input(dg1)` instead of `derive_identity_input(dg1)`. This is a
//! **separate query, sent to the same 5 committees, over a distinctly-scoped client input** —
//! not a reuse of the registration query. See
//! `identity_anchor::derive_delegate_identity_input`'s doc comment for the full reasoning: a
//! citizen's delegate persona must be derivable from an independent OPRF evaluation, not merely
//! a differently-tagged hash of the same evaluation already made at registration, so that
//! unlinkability between a citizen's anchor and their delegate persona rests on the OPRF's own
//! PRF security rather than solely on Poseidon2 domain separation between two hashes that would
//! otherwise share a secret preimage.
//!
//! This proof is verified by the OPRF committee nodes, not on-chain — same role `../query`
//! plays for registration: it proves the blinded point was built from a personal-number field
//! belonging to a genuine, government-signed passport, without revealing which one, so the
//! committee cannot be used as a blind oracle for arbitrary inputs.

use common::hash_salt_dg1_dg2_hash_private_nullifier;
use identity_anchor::{assert_dg1_clear_value, derive_delegate_identity_input};
use oprf::blinded_query::blinded_query;
use utils::types::{DG1Data, MRZExpiryDate, SaltedValue};

/// Public inputs:  `comm_in`
/// Public outputs: `(blinded_query_x, blinded_query_y)`
///
/// Same shape and same rationale as `../query::main` — see that file for the full parameter
/// documentation, which applies here unchanged.
fn main(
    comm_in: pub Field,
    salted_dg1: SaltedValue<DG1Data>,
    salted_expiry_date: SaltedValue<MRZExpiryDate>,
    salted_dg2_hash: SaltedValue<Field>,
    salted_dg2_hash_type: SaltedValue<u32>,
    salted_private_nullifier: SaltedValue<Field>,
    beta: Field,
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
        "Commitment mismatch in oprf_delegate_persona_query",
    );
    assert_dg1_clear_value(salted_dg1);

    // The only behavioural difference from `../query`: blind the delegate-scoped client input.
    let blinded = blinded_query(beta, derive_delegate_identity_input(salted_dg1.value));
    (blinded.x, blinded.y)
}

mod tests {
    use super::main;
    use common::hash_salt_dg1_dg2_hash_private_nullifier;
    use identity_anchor::derive_identity_input;
    use oprf::blinded_query::blinded_query;
    use utils::constants::{SAMPLE_DG1, SHA256_IDENTIFIER};
    use utils::types::{DG1Data, MRZExpiryDate, SaltedValue};

    /// Blinding randomness. Must be a valid, non-zero BabyJubJub scalar; reused from
    /// `oprf-nr`'s own `oprf_example/Prover.toml` so it is known-good — same value `../query`
    /// uses, since `beta` is per-query client randomness with no relationship to which client
    /// input it blinds.
    global TEST_BETA: Field =
        63865932500786004558985758765891911620034145599124743624338496685411118977;

    pub fn fixture() -> (SaltedValue<DG1Data>, SaltedValue<MRZExpiryDate>, SaltedValue<Field>, SaltedValue<u32>, SaltedValue<Field>) {
        let salted_dg1: SaltedValue<DG1Data> =
            SaltedValue { salt: 1111, value: SAMPLE_DG1, hash: 0 };
        let salted_expiry_date: SaltedValue<MRZExpiryDate> =
            SaltedValue { salt: 2222, value: [51, 48, 48, 49, 48, 49], hash: 0 };
        let salted_dg2_hash: SaltedValue<Field> =
            SaltedValue { salt: 3333, value: 4444, hash: 0 };
        let salted_dg2_hash_type: SaltedValue<u32> =
            SaltedValue { salt: 5555, value: SHA256_IDENTIFIER, hash: 0 };
        let salted_private_nullifier: SaltedValue<Field> =
            SaltedValue { salt: 0, value: 0, hash: 6666 };
        (
            salted_dg1, salted_expiry_date, salted_dg2_hash, salted_dg2_hash_type,
            salted_private_nullifier,
        )
    }

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

    /// The delegate-persona query must actually blind a different point than the registration
    /// query would over the same passport — otherwise this circuit would be pointless and the
    /// committee interaction could just be skipped in favour of reusing `../query`'s output.
    #[test]
    fn blinds_a_different_point_than_the_registration_query_would() {
        let (dg1, expiry, dg2_hash, dg2_hash_type, private_nullifier) = fixture();
        let comm_in = hash_salt_dg1_dg2_hash_private_nullifier(
            dg1,
            expiry,
            dg2_hash,
            dg2_hash_type,
            private_nullifier,
        );
        let (delegate_x, delegate_y) =
            main(comm_in, dg1, expiry, dg2_hash, dg2_hash_type, private_nullifier, TEST_BETA);

        // `../query`'s own blinded-query derivation, inlined: same `beta`, same passport, but
        // the registration-scoped client input.
        let registration_input = derive_identity_input(dg1.value);
        let registration_blinded = blinded_query(TEST_BETA, registration_input);

        assert(
            (delegate_x != registration_blinded.x) | (delegate_y != registration_blinded.y),
            "Delegate-persona blinded query must differ from the registration blinded query",
        );
    }

    #[test(should_fail_with = "Commitment mismatch in oprf_delegate_persona_query")]
    fn rejects_a_mismatched_commitment() {
        let (dg1, expiry, dg2_hash, dg2_hash_type, private_nullifier) = fixture();
        let _ = main(1, dg1, expiry, dg2_hash, dg2_hash_type, private_nullifier, TEST_BETA);
    }

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
