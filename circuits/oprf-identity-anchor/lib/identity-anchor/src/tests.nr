//! Tests for the identity-anchor derivation primitives.
//!
//! Run with `nargo test` from `circuits/oprf-identity-anchor/`.
//!
//! These use ZKPassport's own `SAMPLE_DG1` / `ID_CARD_SAMPLE_DG1` fixtures (from
//! `utils::constants`) rather than hand-rolled byte arrays, so the MRZ offsets are being
//! checked against the same test vectors the upstream circuits are checked against.

use super::{
    ASCII_CODE_FOR_LESS_THAN, assert_dg1_clear_value, assert_personal_number_populated,
    backing_tree_node_hash, combine_committee_anchors, derive_backing_commitment,
    derive_backing_nullifier, derive_backing_root_term, derive_committee_anchor_term,
    derive_delegate_identity_input, derive_delegate_persona_term, derive_identity_input,
    DS_ANCHOR, DS_BACKING_COMMITMENT, DS_BACKING_ROOT, DS_DELEGATE, DS_DELEGATE_IDENTITY_INPUT,
    DS_DELEGATE_OUT, DS_IDENTITY_INPUT, get_personal_number_from_dg1, NUM_COMMITTEES,
    PASSPORT_MRZ_PERSONAL_NUMBER_LENGTH,
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

// --- delegate-persona derivation ---

/// The two domain separators actually differ, or the "distinct client input" argument in
/// `derive_delegate_identity_input`'s doc comment would be vacuous.
#[test]
fn delegate_domain_separators_are_distinct_from_each_other_and_from_registration() {
    assert(DS_DELEGATE_IDENTITY_INPUT != DS_IDENTITY_INPUT, "must differ from DS_IDENTITY_INPUT");
    assert(DS_DELEGATE_IDENTITY_INPUT != DS_DELEGATE_OUT, "input/out separators must differ");
    assert(DS_DELEGATE_IDENTITY_INPUT != DS_DELEGATE, "input/combiner separators must differ");
    assert(DS_DELEGATE_OUT != DS_DELEGATE, "out/combiner separators must differ");
}

/// The whole point: a citizen's delegate-persona client input must differ from their ordinary
/// registration client input, over the *same* passport. If these ever collided, the two OPRF
/// evaluations would be the same PRF query and the "distinct query" design would collapse back
/// into the reuse design this module's doc comment explains and rejects.
#[test]
fn delegate_identity_input_differs_from_registration_identity_input() {
    assert(
        derive_delegate_identity_input(SAMPLE_DG1) != derive_identity_input(SAMPLE_DG1),
        "Delegate-persona client input must differ from the registration client input",
    );
}

/// Same renewal-stability property `identity_input_is_stable_across_a_simulated_renewal`
/// checks for registration must hold for the delegate-persona client input too — a delegate
/// who renews their passport must still be able to re-derive the same delegate persona.
#[test]
fn delegate_identity_input_is_stable_across_a_simulated_renewal() {
    let before = derive_delegate_identity_input(SAMPLE_DG1);

    let mut renewed: DG1Data = SAMPLE_DG1;
    let new_document_number: [u8; 9] = [80, 66, 57, 56, 55, 54, 53, 52, 51]; // "PB9876543"
    for i in 0..9 {
        renewed[49 + i] = new_document_number[i];
    }
    let new_expiry: [u8; 6] = [52, 48, 48, 49, 48, 49]; // "400101"
    for i in 0..6 {
        renewed[70 + i] = new_expiry[i];
    }

    assert_eq(
        derive_delegate_identity_input(renewed),
        before,
        "Delegate-persona client input must be unchanged by a passport renewal",
    );
}

/// Mirrors `committee_term_is_separated_by_scheme_version` for the delegate-persona combiner.
#[test]
fn delegate_term_is_separated_by_scheme_version() {
    let oprf_output = 424242;
    assert(
        derive_delegate_persona_term(0, oprf_output, 1)
            != derive_delegate_persona_term(0, oprf_output, 2),
        "Delegate terms under different scheme versions must differ",
    );
}

/// Mirrors `committee_term_is_separated_by_committee_index` for the delegate-persona combiner.
#[test]
fn delegate_term_is_separated_by_committee_index() {
    let oprf_output = 424242;
    let scheme_version = 1;
    assert(
        derive_delegate_persona_term(0, oprf_output, scheme_version)
            != derive_delegate_persona_term(1, oprf_output, scheme_version),
        "Delegate terms must differ by committee index alone",
    );
}

/// Even given the *same* `oprf_output`/`scheme_version`/`committee_index` a real anchor term
/// and delegate term would only share (which cannot happen in the real protocol, since the two
/// come from separately-scoped OPRF queries — see `derive_delegate_identity_input`), the two
/// combiners must still disagree: this is what `DS_ANCHOR != DS_DELEGATE` buys as
/// defense-in-depth on top of the query-level separation.
#[test]
fn delegate_term_and_anchor_term_differ_under_identical_inputs() {
    let oprf_output = 424242;
    let scheme_version = 1;
    assert(
        derive_delegate_persona_term(0, oprf_output, scheme_version)
            != derive_committee_anchor_term(0, oprf_output, scheme_version),
        "Delegate and anchor combiners must be separated even under a hypothetically shared input",
    );
}

// --- backing_root_secret / backing_commitment derivation ---

/// The two new domain separators actually differ from each other and from every existing one —
/// or the separation `derive_backing_root_term`/`derive_backing_commitment` rely on would be
/// vacuous.
#[test]
fn backing_domain_separators_are_distinct_from_each_other_and_from_anchor() {
    assert(DS_BACKING_ROOT != DS_BACKING_COMMITMENT, "root/commitment separators must differ");
    assert(DS_BACKING_ROOT != DS_ANCHOR, "backing root separator must differ from DS_ANCHOR");
    assert(
        DS_BACKING_COMMITMENT != DS_ANCHOR,
        "backing commitment separator must differ from DS_ANCHOR",
    );
}

/// Mirrors `committee_term_is_separated_by_scheme_version` for the backing-root combiner.
#[test]
fn backing_root_term_is_separated_by_scheme_version() {
    let oprf_output = 424242;
    assert(
        derive_backing_root_term(0, oprf_output, 1) != derive_backing_root_term(0, oprf_output, 2),
        "Backing-root terms under different scheme versions must differ",
    );
}

/// Mirrors `committee_term_is_separated_by_committee_index` for the backing-root combiner.
#[test]
fn backing_root_term_is_separated_by_committee_index() {
    let oprf_output = 424242;
    let scheme_version = 1;
    assert(
        derive_backing_root_term(0, oprf_output, scheme_version)
            != derive_backing_root_term(1, oprf_output, scheme_version),
        "Backing-root terms must differ by committee index alone",
    );
}

/// The whole point of `derive_backing_root_term`'s doc comment: even reusing the exact same
/// `oprf_output`/`scheme_version`/`committee_index` an anchor term and a backing-root term
/// would ever see (which the real protocol does — see the doc comment for why that's safe
/// here), the two terms must still differ.
#[test]
fn backing_root_term_differs_from_anchor_term_under_the_same_reused_oprf_output() {
    let oprf_output = 424242;
    let scheme_version = 1;
    assert(
        derive_backing_root_term(0, oprf_output, scheme_version)
            != derive_committee_anchor_term(0, oprf_output, scheme_version),
        "Backing-root and anchor terms must be separated even when built from the same oprf_output",
    );
}

/// `combine_committee_anchors` is reused unchanged for the backing-root combination (same
/// "depends on every committee" property `combined_anchor_depends_on_every_committee_term`
/// already checks for the anchor case) — this checks `derive_backing_commitment` on top of it:
/// the published commitment must change whenever the private root secret does.
#[test]
fn backing_commitment_changes_with_backing_root_secret() {
    assert(
        derive_backing_commitment(111) != derive_backing_commitment(222),
        "Backing commitment must depend on backing_root_secret",
    );
}

/// A citizen's `backing_commitment` must never collide with their own `anchor` even if (by
/// some coincidence, which the protocol never actually produces since the two are built from
/// differently-tagged terms) the combined secrets happened to match — defense in depth beyond
/// the per-term separation above.
#[test]
fn backing_commitment_differs_from_a_same_valued_anchor() {
    let shared_combined_secret = 999;
    assert(
        derive_backing_commitment(shared_combined_secret) != shared_combined_secret,
        "Backing commitment must not be the identity function",
    );
}

// --- backing_tree_node_hash / derive_backing_nullifier (../backing-nullifier) ---

#[test]
fn backing_tree_node_hash_is_order_sensitive() {
    assert(
        backing_tree_node_hash(1, 2) != backing_tree_node_hash(2, 1),
        "Swapping left/right must change the node hash, or a forged path could reorder siblings",
    );
}

#[test]
fn backing_nullifier_changes_with_secret_or_slot() {
    let secret = 424242;
    assert(
        derive_backing_nullifier(secret, 0) != derive_backing_nullifier(secret, 1),
        "Two different slots for the same citizen must produce different nullifiers",
    );
    assert(
        derive_backing_nullifier(111, 0) != derive_backing_nullifier(222, 0),
        "Two different citizens' secrets must produce different nullifiers for the same slot",
    );
}

/// A backing-commitment leaf must never collide, under Poseidon2 domain separation, with an
/// internal node built from the same field values — the standard second-preimage defense a
/// Merkle tree's own hash function needs (RFC 6962-style leaf/node separation), here resting
/// on `DS_BACKING_COMMITMENT` (a leaf-domain tag) and `DS_BACKING_TREE_NODE` (a node-domain
/// tag, `210`) being different Poseidon2 preimages.
#[test]
fn backing_commitment_leaf_does_not_collide_with_a_node_hash_of_the_same_secret() {
    let secret = 424242;
    let leaf = derive_backing_commitment(secret);
    assert(
        leaf != backing_tree_node_hash(secret, secret),
        "Leaf domain and node domain must stay separated",
    );
}

/// Real cross-check vector: `poseidon2_bn254::hash_bytes` (the Rust port
/// `pallets/pallet-identity`/`runtime` both build on) computing
/// `Poseidon2(DS_BACKING_COMMITMENT, 424242)` and `Poseidon2(DS_BACKING_NULLIFIER, 424242, 2)`
/// — read via a temporary `#[test]` added to and then removed from
/// `pallets/pallet-identity/src/tests.rs` (`cargo test -p pallet-identity-zk`), not fabricated.
/// Confirms this crate's `Poseidon2::hash` and the Rust-side `poseidon2_bn254` crate agree on
/// these two new preimages, the same way `anchor_verifier.rs`'s existing
/// `calculate_param_commitment_matches_the_real_nargo_and_bb_vector` confirms it for the
/// preexisting ones.
#[test]
fn backing_commitment_and_nullifier_match_the_real_rust_side_vector() {
    let expected_commitment =
        0x1f071df975e292fcbe1b3abc7042d88b0e063cd17af79a73ec1c47c7f0a84610;
    assert_eq(derive_backing_commitment(424242), expected_commitment);

    let expected_nullifier =
        0x22b020294a8f8b82c34471b0c189da23fdacbbeaa2fec1427c327efa2b4d56e4;
    assert_eq(derive_backing_nullifier(424242, 2), expected_nullifier);
}

/// Real cross-check of the full 32-level tree walk against `pallets/pallet-identity`'s own
/// `recompute_backing_tree_path` for `leaf_index = 0` in an otherwise-empty tree (every
/// sibling on the path is that pallet's `backing_tree_zero_hash(level)`, and the leaf is
/// always the left child, since index stays even at every level of a `0` leaf index) — read
/// via the same temporary pallet-side test as the vector above. This is the exact computation
/// `../backing-nullifier` performs, so agreement here is what confirms this crate's tree
/// hashing genuinely matches the tree the pallet publishes on-chain, not just a
/// same-shaped-but-different one.
#[test]
fn backing_tree_root_matches_the_real_rust_side_vector_for_an_empty_tree_first_leaf() {
    let leaf = derive_backing_commitment(424242);
    let siblings: [Field; 32] = [
        0x16a6d47edce7b0993b10816d6d4460cc0f9c01845de1d399e3b0f8aaadef0b6b,
        0x0ff1c447d1ef5a528694a4136c2fb5a73c39955431759aa4933b26c82804d869,
        0x127a2e0a0fabf20e79b74f0b1dc142ef34b1a2b96897949ca8eda9c44cc15cec,
        0x24f38748daca7d742ff1e89fd2e392bc1b6f1350f2c6b535510bec20072653ee,
        0x17cc1dad0ceb51eac2e61ae17ce872aa685f220975666a8219c0c48acd07a3d8,
        0x2338faeb8f671731e719db24bc3120cd8fdca925f14222339790fb30096e4485,
        0x2fd1363825730a9c256b4065cb46a7e077e3a7dd18d63a7fe43e9590a4b6491a,
        0x0a074ddb0ca408f791508933f10d1d366b66aa53eeb1f57ecefd940eaba47d68,
        0x2d393d8911c5db59aa9abfcc0e48a751bb1b0cbaad41dd2e60de454540dd65dc,
        0x02b94f4247a758aac8c3142a10174941a1fc077bf9cbaad6ce5e3677d0e3d0ff,
        0x1275a9f2fd5bebca3e76d36a0cea99034f873e8c91765de4b2bbe641a094b4d3,
        0x0f937d9589069a406c8603b2ef608cf6213db4fc77fb036260a1fa5e41bafe49,
        0x2888aff9c0711a4e9df287fb43f8db4dfe5d6cb6d2b44fce6c25fb657255e0be,
        0x068d823f95c20eff7403f57dd08391ecddfb573af764bda91e4a7439c31cea44,
        0x25c4e39a8e9fb74b45aa01654c45c8f35888c57a7073fe01d4db2dec3df3abf3,
        0x1195bbd616681e2decf0aa8b5b1095e3418f8716eb12b2b852b772730d486ab4,
        0x277e87d20bcad693093566d14a8f52ec11ba52e4a7f2bb4817dd24c2a382bb0f,
        0x2ff5ff52088def59592b99e9ca9cfc806a11eb6529dc7a1d8804aff43ab919d7,
        0x18f2680546dd4484f95f7e8a77881634cdf0053917e7142cce299f888ba35ebd,
        0x2fdfd0706606aa1bdebaabb1a9252c40abacd3ee03006803e08ec7a6cc5a8dbd,
        0x2c4f3fa50306c38d44f2c971a74767fddc6f34e094f06b01c19a25e6cbe80316,
        0x16b241d5acc77ee8ff7c79930ccc934764c73a76e79049d4a1ca01ed347bc6ae,
        0x2eae37a1dbdacf6b9ce433f16bd0103b7f5a52f923ba70e84a501de61a9da063,
        0x2fab8211545d236550cbbab4fc8e8a37b68c639fadd87439f54d471427b0f747,
        0x1a28702748167233ae3d78271b4ca3c924a4d20ceb3ed5e17ea44447b18e28ad,
        0x2ea9b8751a33a12b4ff276ef8765edfbbde123e7744c1dd0666b3576deedd911,
        0x2483d2673ca9c6860686a40d7273071a50e016d302d4b2b496983bb1595d54a7,
        0x028e020c54878eb003e669b88af37968fc6c264937fece4ab11272dbf799f531,
        0x0eb394a8caf49da0ec70349d38670a0624afa9240454be94242b63419f33ad0b,
        0x19efdb04021cfb721d0f072340c01a4ab6de724381f9a3275152a40b0b6713ce,
        0x15f5e50cac7670592a741d1151f2f0a3de253681590dc38b37a6f69e1a0ed54a,
        0x1d74c0426d92db677c2ed90ca6d5ada72afc805a44c6bfa0e84bd81a90ce04da,
    ];
    let mut current = leaf;
    for level in 0..32 {
        // leaf_index = 0 stays even (left child) at every level.
        current = backing_tree_node_hash(current, siblings[level]);
    }
    let expected_root = 0x09efbc7edc789c162ded75ed12f2d9e84fb8822535552d9d2f80ce535762f46e;
    assert_eq(current, expected_root);
}
