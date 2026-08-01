//! Tests for the identity-anchor derivation primitives.
//!
//! Run with `nargo test` from `circuits/oprf-identity-anchor/`.
//!
//! These use ZKPassport's own `SAMPLE_DG1` / `ID_CARD_SAMPLE_DG1` fixtures (from
//! `utils::constants`) rather than hand-rolled byte arrays, so the MRZ offsets are being
//! checked against the same test vectors the upstream circuits are checked against.

use super::{
    ASCII_CODE_FOR_LESS_THAN, assert_dg1_clear_value, assert_personal_number_populated,
    combine_committee_anchors, derive_committee_anchor_term, derive_identity_input,
    get_personal_number_from_dg1, NUM_COMMITTEES, PASSPORT_MRZ_PERSONAL_NUMBER_LENGTH,
};
use utils::constants::{DG1_LENGTH, ID_CARD_SAMPLE_DG1, SAMPLE_DG1};
use utils::types::{DG1Data, SaltedValue};

/// `SAMPLE_DG1`'s MRZ line 2 is:
///   `PA1234567_AUS881112_M300101_<CYBERCITY<<<<<<`
/// Character positions 29-42 of that line are `<CYBERCITY<<<<` — the 14 bytes immediately
/// after the expiry check digit, before the personal-number check digit and the composite
/// check digit. This is the concrete offset check: if `PASSPORT_MRZ_PERSONAL_NUMBER_INDEX`
/// were wrong by even one byte this test fails.
#[test]
fn extracts_td3_personal_number_at_positions_29_to_42() {
    let personal_number = get_personal_number_from_dg1(SAMPLE_DG1);
    let expected: [u8; 14] = [
        60, // '<'
        67, 89, 66, 69, 82, 67, 73, 84, 89, // 'CYBERCITY'
        60, 60, 60, 60, // '<<<<'
    ];
    assert_eq(personal_number, expected, "Personal number slice mismatch");
}

#[test]
fn accepts_a_populated_personal_number() {
    assert_personal_number_populated(get_personal_number_from_dg1(SAMPLE_DG1));
}

/// A country that does not populate the optional-data slot emits all-filler. Every one of its
/// citizens would otherwise derive an identical anchor.
#[test(should_fail_with = "MRZ personal-number field is empty")]
fn rejects_an_all_filler_personal_number() {
    let mut dg1: DG1Data = SAMPLE_DG1;
    // DG1 index = 5 (DG1_TO_MRZ_OFFSET) + 72 (flat MRZ index) = 77.
    for i in 0..PASSPORT_MRZ_PERSONAL_NUMBER_LENGTH {
        dg1[77 + i] = ASCII_CODE_FOR_LESS_THAN;
    }
    assert_personal_number_populated(get_personal_number_from_dg1(dg1));
}

/// Same failure mode, via zero bytes rather than ICAO filler.
#[test(should_fail_with = "MRZ personal-number field is empty")]
fn rejects_a_zeroed_personal_number() {
    let mut dg1: DG1Data = SAMPLE_DG1;
    for i in 0..PASSPORT_MRZ_PERSONAL_NUMBER_LENGTH {
        dg1[77 + i] = 0;
    }
    assert_personal_number_populated(get_personal_number_from_dg1(dg1));
}

/// TD1 ID cards have a different MRZ layout; reading TD3 offsets out of one would yield a
/// well-formed but meaningless anchor.
#[test(should_fail_with = "TD1 ID cards are not supported")]
fn rejects_id_cards() {
    let _ = get_personal_number_from_dg1(ID_CARD_SAMPLE_DG1);
}

/// The whole point of the design: the anchor must survive passport renewal. Renewal changes
/// the document number and both dates but not the personal number, so mutating exactly those
/// fields must leave the identity input untouched.
#[test]
fn identity_input_is_stable_across_a_simulated_renewal() {
    let before = derive_identity_input(SAMPLE_DG1);

    let mut renewed: DG1Data = SAMPLE_DG1;
    // Document number: flat MRZ 44-52 -> DG1 49-57. New document, new number.
    let new_document_number: [u8; 9] = [80, 66, 57, 56, 55, 54, 53, 52, 51]; // "PB9876543"
    for i in 0..9 {
        renewed[49 + i] = new_document_number[i];
    }
    // Expiry date: flat MRZ 65-70 -> DG1 70-75. New document, new expiry.
    let new_expiry: [u8; 6] = [52, 48, 48, 49, 48, 49]; // "400101"
    for i in 0..6 {
        renewed[70 + i] = new_expiry[i];
    }

    assert(
        renewed != SAMPLE_DG1,
        "Renewal fixture must actually differ from the original DG1",
    );
    assert_eq(
        derive_identity_input(renewed),
        before,
        "Identity input must be unchanged by a passport renewal",
    );
}

/// The converse: two different people must not collide. Changing only the personal number
/// must change the identity input.
#[test]
fn identity_input_changes_with_the_personal_number() {
    let before = derive_identity_input(SAMPLE_DG1);
    let mut other: DG1Data = SAMPLE_DG1;
    other[77] = 88; // 'X' in place of the leading '<'
    assert(
        derive_identity_input(other) != before,
        "Identity input must depend on the personal-number field",
    );
}

/// And the issuing country is genuinely bound in, not decorative.
#[test]
fn identity_input_changes_with_the_issuing_country() {
    let before = derive_identity_input(SAMPLE_DG1);
    let mut other: DG1Data = SAMPLE_DG1;
    // Issuing country: flat MRZ 2-4 -> DG1 7-9. "AUS" -> "NZL".
    other[7] = 78;
    other[8] = 90;
    other[9] = 76;
    assert(
        derive_identity_input(other) != before,
        "Identity input must depend on the issuing country",
    );
}

/// A hash-only `SaltedValue<DG1Data>` leaves `value` zeroed while still satisfying `comm_in`.
/// If that were allowed, every prover would derive the same identity input.
#[test(should_fail_with = "DG1 must be provided as a clear value")]
fn rejects_hash_only_dg1() {
    let hash_only: SaltedValue<DG1Data> =
        SaltedValue { salt: 0, value: [0; DG1_LENGTH], hash: 12345 };
    assert_dg1_clear_value(hash_only);
}

#[test(should_fail_with = "DG1 salt must be non-zero")]
fn rejects_unsalted_clear_dg1() {
    let unsalted: SaltedValue<DG1Data> = SaltedValue { salt: 0, value: SAMPLE_DG1, hash: 0 };
    assert_dg1_clear_value(unsalted);
}

#[test]
fn accepts_salted_clear_dg1() {
    let salted: SaltedValue<DG1Data> = SaltedValue { salt: 7, value: SAMPLE_DG1, hash: 0 };
    assert_dg1_clear_value(salted);
}

/// Scheme version must actually separate anchor generations, or the 4-year OPRF rotation in
/// changelog entry 67 would be a no-op.
#[test]
fn committee_term_is_separated_by_scheme_version() {
    let oprf_output = 424242;
    assert(
        derive_committee_anchor_term(0, oprf_output, 1)
            != derive_committee_anchor_term(0, oprf_output, 2),
        "Committee terms under different scheme versions must differ",
    );
}

/// The whole point of hashing before summing (changelog entry 73): two committees producing
/// the numerically identical OPRF output must still contribute different terms, or the
/// combiner would leak equality between committees' secrets.
#[test]
fn committee_term_is_separated_by_committee_index() {
    let oprf_output = 424242;
    let scheme_version = 1;
    assert(
        derive_committee_anchor_term(0, oprf_output, scheme_version)
            != derive_committee_anchor_term(1, oprf_output, scheme_version),
        "Committee terms must differ by committee index alone",
    );
}

/// The combined anchor must actually depend on every one of the 5 terms — a combiner that
/// silently dropped a committee would defeat entry 73's "unpredictable if even one of 5 is
/// honest" property.
#[test]
fn combined_anchor_depends_on_every_committee_term() {
    let terms: [Field; NUM_COMMITTEES] = [1, 2, 3, 4, 5];
    let before = combine_committee_anchors(terms);
    for i in 0..NUM_COMMITTEES {
        let mut mutated = terms;
        mutated[i] += 1;
        assert(
            combine_committee_anchors(mutated) != before,
            "Combined anchor must change when any single committee's term changes",
        );
    }
}

/// Combination is plain field addition, checked explicitly rather than only indirectly via
/// the "depends on every term" test above.
#[test]
fn combined_anchor_is_the_field_sum_of_terms() {
    let terms: [Field; NUM_COMMITTEES] = [10, 20, 30, 40, 50];
    assert_eq(combine_committee_anchors(terms), 150, "Combination must be plain field addition");
}
