use crate::{
    mock::*, AssetDisclosures, ConflictRegistry, ConflictType, Error, Event, Investigators,
    NextReportId, PendingReportAction, ReportAction, ReportNullifiers, ReportStatus,
    WhistleblowerReports, WHISTLEBLOWER_REPORT_SERVICE_SCOPE, WHISTLEBLOWER_REPORT_SERVICE_SUBSCOPE,
};
use frame_support::{assert_noop, assert_ok, traits::ConstU32, BoundedVec};
use sp_runtime::DispatchError;

fn valid_proof() -> BoundedVec<u8, ConstU32<4096>> {
    BoundedVec::try_from(vec![VALID_PROOF_MARKER]).unwrap()
}

fn invalid_proof() -> BoundedVec<u8, ConstU32<4096>> {
    BoundedVec::try_from(vec![INVALID_PROOF_MARKER]).unwrap()
}

/// A minimal, structurally valid ZKPassport `count_4` outer-circuit public-input array
/// (`D = 1` disclosure subproof, so `len == 9`), matching the layout documented in
/// `runtime/src/verifier.rs`'s module doc and mirrored in
/// `pallets/pallet-anticorruption/src/lib.rs`'s "ZKPassport public-input layout" section:
/// `[certificate_registry_root, circuit_registry_root, current_date, service_scope,
///   service_subscope, param_commitments[0], nullifier_type, scoped_nullifier, oprf_pk_hash]`.
/// Every field is caller-controlled so tests can independently vary the shared registry root
/// (index 0, NOT per-citizen) versus the real per-citizen `scoped_nullifier` (index `len - 2`
/// = 7), and the scope/subscope fields the domain-separation check reads (indices 3/4).
fn public_inputs_with(
    registry_root: [u8; 32],
    scope: [u8; 32],
    subscope: [u8; 32],
    nullifier: [u8; 32],
) -> BoundedVec<[u8; 32], ConstU32<16>> {
    BoundedVec::try_from(vec![
        registry_root,       // 0: certificate_registry_root
        [2u8; 32],           // 1: circuit_registry_root
        [0u8; 32],           // 2: current_date (not checked by this pallet)
        scope,               // 3: service_scope
        subscope,            // 4: service_subscope
        [3u8; 32],           // 5: param_commitments[0]
        [4u8; 32],           // 6: nullifier_type
        nullifier,           // 7 = len - 2: scoped_nullifier
        [6u8; 32],           // 8 = len - 1: oprf_pk_hash
    ])
    .unwrap()
}

/// Convenience wrapper for the common case: correct domain-separation scope/subscope, a
/// caller-chosen registry root and per-citizen nullifier.
fn public_inputs_for(registry_root: [u8; 32], nullifier: [u8; 32]) -> BoundedVec<[u8; 32], ConstU32<16>> {
    public_inputs_with(
        registry_root,
        WHISTLEBLOWER_REPORT_SERVICE_SCOPE,
        WHISTLEBLOWER_REPORT_SERVICE_SUBSCOPE,
        nullifier,
    )
}

/// Further convenience wrapper matching most existing tests' shape: a fixed default registry
/// root, correct scope/subscope, caller-chosen nullifier.
fn public_inputs(nullifier: [u8; 32]) -> BoundedVec<[u8; 32], ConstU32<16>> {
    public_inputs_for(DEFAULT_REGISTRY_ROOT, nullifier)
}

fn empty_public_inputs() -> BoundedVec<[u8; 32], ConstU32<16>> {
    BoundedVec::try_from(Vec::new()).unwrap()
}

/// Structurally too-short (fewer than the real layout's 9-element floor) but non-empty, so
/// tests can distinguish "empty" from "short but nonzero" — both must be rejected the same
/// way, before the array is ever indexed.
fn too_short_public_inputs() -> BoundedVec<[u8; 32], ConstU32<16>> {
    BoundedVec::try_from(vec![[1u8; 32]; 5]).unwrap()
}

const DEFAULT_REGISTRY_ROOT: [u8; 32] = [9u8; 32];
const NULLIFIER_A: [u8; 32] = [1u8; 32];
const CONTENT_A: [u8; 32] = [10u8; 32];
const CONTENT_B: [u8; 32] = [20u8; 32];

fn submit_report(who: u64, nullifier: [u8; 32], content_hash: [u8; 32]) {
    assert_ok!(AntiCorruption::submit_whistleblower_report(
        RuntimeOrigin::signed(who),
        content_hash,
        valid_proof(),
        public_inputs(nullifier),
    ));
}

fn add_investigator(who: u64) {
    assert_ok!(AntiCorruption::add_investigator(RuntimeOrigin::root(), who));
}

// ─── submit_asset_disclosure ────────────────────────────────────────────────

#[test]
fn submit_asset_disclosure_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let ipfs_hash = [5u8; 32];

        assert_ok!(AntiCorruption::submit_asset_disclosure(RuntimeOrigin::signed(1), ipfs_hash));

        let entry = AssetDisclosures::<Test>::get(1).unwrap();
        assert_eq!(entry.ipfs_hash, ipfs_hash);
        assert_eq!(entry.disclosed_at, 1);
        assert_eq!(entry.update_due_at, 1 + RENEWAL_BLOCKS as u64);
        System::assert_last_event(
            Event::AssetDisclosed { who: 1, ipfs_hash, update_due_at: 1 + RENEWAL_BLOCKS as u64 }
                .into(),
        );
    });
}

#[test]
fn submit_asset_disclosure_upserts_on_second_submission() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let first_hash = [5u8; 32];
        let second_hash = [6u8; 32];

        assert_ok!(AntiCorruption::submit_asset_disclosure(RuntimeOrigin::signed(1), first_hash));

        System::set_block_number(10);
        assert_ok!(AntiCorruption::submit_asset_disclosure(RuntimeOrigin::signed(1), second_hash));

        let entry = AssetDisclosures::<Test>::get(1).unwrap();
        assert_eq!(entry.ipfs_hash, second_hash);
        assert_eq!(entry.disclosed_at, 10);
        assert_eq!(entry.update_due_at, 10 + RENEWAL_BLOCKS as u64);
        System::assert_last_event(
            Event::AssetDisclosed {
                who: 1,
                ipfs_hash: second_hash,
                update_due_at: 10 + RENEWAL_BLOCKS as u64,
            }
            .into(),
        );
    });
}

#[test]
fn has_current_disclosure_false_when_none_on_file() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert!(!AntiCorruption::has_current_disclosure(&1));
    });
}

#[test]
fn has_current_disclosure_true_before_due_date() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(AntiCorruption::submit_asset_disclosure(RuntimeOrigin::signed(1), [5u8; 32]));

        // update_due_at = 1 + RENEWAL_BLOCKS; still current right up to and including that block.
        System::set_block_number(1 + RENEWAL_BLOCKS as u64);
        assert!(AntiCorruption::has_current_disclosure(&1));
    });
}

#[test]
fn has_current_disclosure_false_once_past_due_date() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(AntiCorruption::submit_asset_disclosure(RuntimeOrigin::signed(1), [5u8; 32]));

        // One block past update_due_at: the disclosure has lapsed.
        System::set_block_number(1 + RENEWAL_BLOCKS as u64 + 1);
        assert!(!AntiCorruption::has_current_disclosure(&1));
    });
}

// ─── DisclosureChecker (pallet-elections seating gate) ──────────────────────
//
// Exercises the trait impl itself (`pallet_elections::DisclosureChecker::has_current_disclosure`
// on `Pallet<T>`), not just the underlying inherent function it wraps -- confirms the impl is
// actually wired to the same logic and reachable through the trait object pallet-elections uses.

#[test]
fn disclosure_checker_trait_impl_matches_inherent_function() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert!(!<AntiCorruption as pallet_elections::DisclosureChecker<u64>>::has_current_disclosure(&1));

        assert_ok!(AntiCorruption::submit_asset_disclosure(RuntimeOrigin::signed(1), [5u8; 32]));
        assert!(<AntiCorruption as pallet_elections::DisclosureChecker<u64>>::has_current_disclosure(&1));

        System::set_block_number(1 + RENEWAL_BLOCKS as u64 + 1);
        assert!(!<AntiCorruption as pallet_elections::DisclosureChecker<u64>>::has_current_disclosure(&1));
    });
}

// ─── register_conflict / clear_conflict ─────────────────────────────────────

#[test]
fn register_conflict_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        assert_ok!(AntiCorruption::register_conflict(
            RuntimeOrigin::signed(1),
            42,
            ConflictType::FinancialInterest,
        ));

        let entry = ConflictRegistry::<Test>::get((1, 42)).unwrap();
        assert_eq!(entry.conflict_type, ConflictType::FinancialInterest);
        assert_eq!(entry.registered_at, 1);
        System::assert_last_event(
            Event::ConflictRegistered { who: 1, entity_id: 42, conflict_type: ConflictType::FinancialInterest }
                .into(),
        );
    });
}

#[test]
fn register_conflict_overwrites_on_reregister() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(AntiCorruption::register_conflict(
            RuntimeOrigin::signed(1),
            42,
            ConflictType::FinancialInterest,
        ));

        System::set_block_number(5);
        assert_ok!(AntiCorruption::register_conflict(
            RuntimeOrigin::signed(1),
            42,
            ConflictType::FamilyRelation,
        ));

        let entry = ConflictRegistry::<Test>::get((1, 42)).unwrap();
        assert_eq!(entry.conflict_type, ConflictType::FamilyRelation);
        assert_eq!(entry.registered_at, 5);
    });
}

#[test]
fn clear_conflict_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(AntiCorruption::register_conflict(
            RuntimeOrigin::signed(1),
            42,
            ConflictType::FinancialInterest,
        ));

        assert_ok!(AntiCorruption::clear_conflict(RuntimeOrigin::signed(1), 42));

        assert!(ConflictRegistry::<Test>::get((1, 42)).is_none());
        System::assert_last_event(Event::ConflictCleared { who: 1, entity_id: 42 }.into());
    });
}

#[test]
fn clear_conflict_fails_when_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        assert_noop!(
            AntiCorruption::clear_conflict(RuntimeOrigin::signed(1), 42),
            Error::<Test>::ConflictNotFound
        );
    });
}

// ─── submit_whistleblower_report ────────────────────────────────────────────

#[test]
fn submit_whistleblower_report_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        assert_ok!(AntiCorruption::submit_whistleblower_report(
            RuntimeOrigin::signed(1),
            CONTENT_A,
            valid_proof(),
            public_inputs(NULLIFIER_A),
        ));

        let report = WhistleblowerReports::<Test>::get(0).unwrap();
        assert_eq!(report.content_hash, CONTENT_A);
        assert_eq!(report.submitted_at, 1);
        assert_eq!(report.status, ReportStatus::Pending);
        assert_eq!(report.nullifier, NULLIFIER_A);
        assert!(ReportNullifiers::<Test>::get((NULLIFIER_A, CONTENT_A)));
        assert_eq!(NextReportId::<Test>::get(), 1);
        System::assert_last_event(
            Event::ReportSubmitted { report_id: 0, content_hash: CONTENT_A }.into(),
        );
    });
}

#[test]
fn submit_whistleblower_report_fails_with_empty_public_inputs_before_verifier_runs() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        // Valid proof (the mock verifier would accept it) but empty public_inputs: the
        // length check must run before the verifier is invoked, so this must fail with
        // MissingNullifierInput, not InvalidZkProof.
        assert_noop!(
            AntiCorruption::submit_whistleblower_report(
                RuntimeOrigin::signed(1),
                CONTENT_A,
                valid_proof(),
                empty_public_inputs(),
            ),
            Error::<Test>::MissingNullifierInput
        );
        assert!(WhistleblowerReports::<Test>::get(0).is_none());
    });
}

#[test]
fn submit_whistleblower_report_fails_with_too_short_public_inputs_before_verifier_runs() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        // Non-empty (5 elements) but still short of the real layout's 9-element floor
        // (count_4, D = 1). Must be rejected before indexing service_scope/service_subscope/
        // scoped_nullifier, and before the verifier runs — same MissingNullifierInput error
        // as the empty case, not a panic and not InvalidZkProof.
        assert_noop!(
            AntiCorruption::submit_whistleblower_report(
                RuntimeOrigin::signed(1),
                CONTENT_A,
                valid_proof(),
                too_short_public_inputs(),
            ),
            Error::<Test>::MissingNullifierInput
        );
        assert!(WhistleblowerReports::<Test>::get(0).is_none());
    });
}

#[test]
fn submit_whistleblower_report_fails_with_wrong_service_scope() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        // A structurally valid, otherwise-acceptable proof, but stamped with a scope that
        // isn't this call's domain-separation constant — e.g. as if replayed from a proof
        // generated for a different purpose such as pallet-identity::register_citizen. Must
        // be rejected with InvalidProofScope, before the verifier even runs (checked before
        // T::ZkVerifier::verify in the call body), and no report may be persisted.
        let wrong_scope = [0xABu8; 32];
        assert_noop!(
            AntiCorruption::submit_whistleblower_report(
                RuntimeOrigin::signed(1),
                CONTENT_A,
                valid_proof(),
                public_inputs_with(
                    DEFAULT_REGISTRY_ROOT,
                    wrong_scope,
                    WHISTLEBLOWER_REPORT_SERVICE_SUBSCOPE,
                    NULLIFIER_A,
                ),
            ),
            Error::<Test>::InvalidProofScope
        );
        assert!(WhistleblowerReports::<Test>::get(0).is_none());
    });
}

#[test]
fn submit_whistleblower_report_fails_with_wrong_service_subscope() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        // Correct service_scope but wrong service_subscope must also be rejected — the two
        // are checked independently, so a proof can't pass by matching only one of them.
        let wrong_subscope = [0xCDu8; 32];
        assert_noop!(
            AntiCorruption::submit_whistleblower_report(
                RuntimeOrigin::signed(1),
                CONTENT_A,
                valid_proof(),
                public_inputs_with(
                    DEFAULT_REGISTRY_ROOT,
                    WHISTLEBLOWER_REPORT_SERVICE_SCOPE,
                    wrong_subscope,
                    NULLIFIER_A,
                ),
            ),
            Error::<Test>::InvalidProofScope
        );
        assert!(WhistleblowerReports::<Test>::get(0).is_none());
    });
}

#[test]
fn submit_whistleblower_report_uses_real_scoped_nullifier_not_shared_registry_root() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        // Two different citizens (different scoped_nullifier — the real per-citizen value at
        // index len-2) but the *same* certificate_registry_root (index 0 — shared by every
        // citizen at a given registry state) and the same content_hash. Under the old,
        // buggy code, which stored public_inputs[0] (the shared registry root) as
        // "nullifier", the second submission below would collide with the first — the
        // (registry_root, CONTENT_A) pair would already be marked used — and fail with
        // DuplicateReport even though it's a genuinely different citizen filing a genuinely
        // different report. The fix keys dedup on the real scoped_nullifier, so both must
        // succeed.
        let shared_registry_root = [7u8; 32];
        let nullifier_citizen_a = [111u8; 32];
        let nullifier_citizen_b = [222u8; 32];
        assert_ne!(nullifier_citizen_a, shared_registry_root);
        assert_ne!(nullifier_citizen_b, shared_registry_root);

        assert_ok!(AntiCorruption::submit_whistleblower_report(
            RuntimeOrigin::signed(1),
            CONTENT_A,
            valid_proof(),
            public_inputs_for(shared_registry_root, nullifier_citizen_a),
        ));
        assert_ok!(AntiCorruption::submit_whistleblower_report(
            RuntimeOrigin::signed(2),
            CONTENT_A,
            valid_proof(),
            public_inputs_for(shared_registry_root, nullifier_citizen_b),
        ));

        let report_a = WhistleblowerReports::<Test>::get(0).unwrap();
        let report_b = WhistleblowerReports::<Test>::get(1).unwrap();
        assert_eq!(report_a.nullifier, nullifier_citizen_a);
        assert_eq!(report_b.nullifier, nullifier_citizen_b);
        assert_ne!(report_a.nullifier, report_b.nullifier);
        // Neither stored nullifier is the shared registry root — proves the pallet isn't
        // still reading public_inputs[0].
        assert_ne!(report_a.nullifier, shared_registry_root);
        assert_ne!(report_b.nullifier, shared_registry_root);
        assert!(ReportNullifiers::<Test>::get((nullifier_citizen_a, CONTENT_A)));
        assert!(ReportNullifiers::<Test>::get((nullifier_citizen_b, CONTENT_A)));
        // The old buggy key — (shared_registry_root, CONTENT_A) — was never written.
        assert!(!ReportNullifiers::<Test>::get((shared_registry_root, CONTENT_A)));

        // Genuine duplicate (same citizen, same content_hash) is still rejected.
        assert_noop!(
            AntiCorruption::submit_whistleblower_report(
                RuntimeOrigin::signed(1),
                CONTENT_A,
                valid_proof(),
                public_inputs_for(shared_registry_root, nullifier_citizen_a),
            ),
            Error::<Test>::DuplicateReport
        );
    });
}

#[test]
fn submit_whistleblower_report_fails_with_invalid_proof() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        assert_noop!(
            AntiCorruption::submit_whistleblower_report(
                RuntimeOrigin::signed(1),
                CONTENT_A,
                invalid_proof(),
                public_inputs(NULLIFIER_A),
            ),
            Error::<Test>::InvalidZkProof
        );
    });
}

#[test]
fn submit_whistleblower_report_fails_on_duplicate_nullifier_and_content_hash() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        submit_report(1, NULLIFIER_A, CONTENT_A);

        assert_noop!(
            AntiCorruption::submit_whistleblower_report(
                RuntimeOrigin::signed(1),
                CONTENT_A,
                valid_proof(),
                public_inputs(NULLIFIER_A),
            ),
            Error::<Test>::DuplicateReport
        );
    });
}

#[test]
fn submit_whistleblower_report_allows_same_nullifier_different_content_hash() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        submit_report(1, NULLIFIER_A, CONTENT_A);

        assert_ok!(AntiCorruption::submit_whistleblower_report(
            RuntimeOrigin::signed(1),
            CONTENT_B,
            valid_proof(),
            public_inputs(NULLIFIER_A),
        ));

        assert!(WhistleblowerReports::<Test>::get(0).is_some());
        assert!(WhistleblowerReports::<Test>::get(1).is_some());
        assert_eq!(NextReportId::<Test>::get(), 2);
    });
}

// ─── report workflow: flag_report ───────────────────────────────────────────

#[test]
fn flag_report_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);

        assert_ok!(AntiCorruption::flag_report(RuntimeOrigin::signed(9), 0));

        assert_eq!(WhistleblowerReports::<Test>::get(0).unwrap().status, ReportStatus::Flagged);
        System::assert_last_event(Event::ReportFlagged { report_id: 0, investigator: 9 }.into());
    });
}

#[test]
fn flag_report_fails_for_non_investigator() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        submit_report(1, NULLIFIER_A, CONTENT_A);

        assert_noop!(
            AntiCorruption::flag_report(RuntimeOrigin::signed(9), 0),
            Error::<Test>::NotInvestigator
        );
    });
}

#[test]
fn flag_report_fails_when_report_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);

        assert_noop!(
            AntiCorruption::flag_report(RuntimeOrigin::signed(9), 0),
            Error::<Test>::ReportNotFound
        );
    });
}

#[test]
fn flag_report_fails_when_not_pending() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        assert_ok!(AntiCorruption::flag_report(RuntimeOrigin::signed(9), 0));

        // Already Flagged — flagging again must fail.
        assert_noop!(
            AntiCorruption::flag_report(RuntimeOrigin::signed(9), 0),
            Error::<Test>::InvalidReportState
        );
    });
}

// ─── report workflow: open_investigation ────────────────────────────────────

#[test]
fn open_investigation_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        assert_ok!(AntiCorruption::flag_report(RuntimeOrigin::signed(9), 0));

        assert_ok!(AntiCorruption::open_investigation(RuntimeOrigin::signed(9), 0));

        assert_eq!(
            WhistleblowerReports::<Test>::get(0).unwrap().status,
            ReportStatus::UnderInvestigation
        );
        System::assert_last_event(
            Event::InvestigationOpened { report_id: 0, investigator: 9 }.into(),
        );
    });
}

#[test]
fn open_investigation_fails_for_non_investigator() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        assert_ok!(AntiCorruption::flag_report(RuntimeOrigin::signed(9), 0));

        assert_noop!(
            AntiCorruption::open_investigation(RuntimeOrigin::signed(1), 0),
            Error::<Test>::NotInvestigator
        );
    });
}

#[test]
fn open_investigation_fails_when_report_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);

        assert_noop!(
            AntiCorruption::open_investigation(RuntimeOrigin::signed(9), 0),
            Error::<Test>::ReportNotFound
        );
    });
}

#[test]
fn open_investigation_fails_when_not_flagged() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);

        // Still Pending — never flagged.
        assert_noop!(
            AntiCorruption::open_investigation(RuntimeOrigin::signed(9), 0),
            Error::<Test>::InvalidReportState
        );
    });
}

// ─── report workflow: clear_report / approve_report_action (2-of-N recusal) ─
//
// clear_report/refer_report_to_courts only *propose* a transition now — see the module doc
// comment's "Recusal" section. A structural 2-of-N safeguard against a single investigator
// (including one clearing/referring a report that happens to be about themselves — the chain
// cannot check that, since report content is encrypted to the investigator's key) unilaterally
// closing any report.

fn open_investigation_on(report_id: u32, investigator: u64) {
    assert_ok!(AntiCorruption::flag_report(RuntimeOrigin::signed(investigator), report_id));
    assert_ok!(AntiCorruption::open_investigation(RuntimeOrigin::signed(investigator), report_id));
}

#[test]
fn clear_report_by_single_investigator_does_not_clear_the_report() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        open_investigation_on(0, 9);

        // Propose only — a lone investigator's clear_report call must not itself clear the
        // report; it only records a pending action awaiting a second, different investigator.
        assert_ok!(AntiCorruption::clear_report(RuntimeOrigin::signed(9), 0));

        assert_eq!(
            WhistleblowerReports::<Test>::get(0).unwrap().status,
            ReportStatus::UnderInvestigation
        );
        assert_eq!(PendingReportAction::<Test>::get(0), Some((ReportAction::Clear, 9)));
        System::assert_last_event(
            Event::ReportActionProposed { report_id: 0, action: ReportAction::Clear, proposer: 9 }
                .into(),
        );
    });
}

#[test]
fn refer_report_to_courts_by_single_investigator_does_not_refer_the_report() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        open_investigation_on(0, 9);

        assert_ok!(AntiCorruption::refer_report_to_courts(RuntimeOrigin::signed(9), 0));

        assert_eq!(
            WhistleblowerReports::<Test>::get(0).unwrap().status,
            ReportStatus::UnderInvestigation
        );
        assert_eq!(PendingReportAction::<Test>::get(0), Some((ReportAction::ReferToCourts, 9)));
    });
}

#[test]
fn clear_report_two_different_investigators_succeeds() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        add_investigator(10);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        open_investigation_on(0, 9);

        assert_ok!(AntiCorruption::clear_report(RuntimeOrigin::signed(9), 0));
        assert_ok!(AntiCorruption::approve_report_action(RuntimeOrigin::signed(10), 0));

        assert_eq!(WhistleblowerReports::<Test>::get(0).unwrap().status, ReportStatus::Cleared);
        assert!(PendingReportAction::<Test>::get(0).is_none());
        System::assert_last_event(
            Event::ReportCleared { report_id: 0, proposer: 9, approver: 10 }.into(),
        );
    });
}

#[test]
fn refer_report_to_courts_two_different_investigators_succeeds() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        add_investigator(10);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        open_investigation_on(0, 9);

        assert_ok!(AntiCorruption::refer_report_to_courts(RuntimeOrigin::signed(9), 0));
        assert_ok!(AntiCorruption::approve_report_action(RuntimeOrigin::signed(10), 0));

        assert_eq!(
            WhistleblowerReports::<Test>::get(0).unwrap().status,
            ReportStatus::ReferredToCourts
        );
        assert!(PendingReportAction::<Test>::get(0).is_none());
        System::assert_last_event(
            Event::ReportReferredToCourts { report_id: 0, proposer: 9, approver: 10 }.into(),
        );
    });
}

#[test]
fn approve_report_action_fails_for_same_investigator_who_proposed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        add_investigator(10);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        open_investigation_on(0, 9);
        assert_ok!(AntiCorruption::clear_report(RuntimeOrigin::signed(9), 0));

        // Same investigator (9) who proposed cannot also approve — even though 9 is a valid
        // investigator, this must not be treated as sufficient sign-off.
        assert_noop!(
            AntiCorruption::approve_report_action(RuntimeOrigin::signed(9), 0),
            Error::<Test>::SameInvestigator
        );

        // Still pending — the report was not cleared.
        assert_eq!(
            WhistleblowerReports::<Test>::get(0).unwrap().status,
            ReportStatus::UnderInvestigation
        );
        assert!(PendingReportAction::<Test>::get(0).is_some());

        // A genuinely different investigator can still approve afterward.
        assert_ok!(AntiCorruption::approve_report_action(RuntimeOrigin::signed(10), 0));
        assert_eq!(WhistleblowerReports::<Test>::get(0).unwrap().status, ReportStatus::Cleared);
    });
}

#[test]
fn approve_report_action_fails_for_non_investigator() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        open_investigation_on(0, 9);
        assert_ok!(AntiCorruption::clear_report(RuntimeOrigin::signed(9), 0));

        assert_noop!(
            AntiCorruption::approve_report_action(RuntimeOrigin::signed(1), 0),
            Error::<Test>::NotInvestigator
        );
    });
}

#[test]
fn approve_report_action_fails_when_no_pending_action() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        open_investigation_on(0, 9);

        assert_noop!(
            AntiCorruption::approve_report_action(RuntimeOrigin::signed(9), 0),
            Error::<Test>::NoPendingReportAction
        );
    });
}

#[test]
fn clear_report_fails_for_non_investigator() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        open_investigation_on(0, 9);

        assert_noop!(
            AntiCorruption::clear_report(RuntimeOrigin::signed(1), 0),
            Error::<Test>::NotInvestigator
        );
    });
}

#[test]
fn clear_report_fails_when_report_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);

        assert_noop!(
            AntiCorruption::clear_report(RuntimeOrigin::signed(9), 0),
            Error::<Test>::ReportNotFound
        );
    });
}

#[test]
fn clear_report_fails_when_not_under_investigation() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        assert_ok!(AntiCorruption::flag_report(RuntimeOrigin::signed(9), 0));

        // Only Flagged, not yet UnderInvestigation.
        assert_noop!(
            AntiCorruption::clear_report(RuntimeOrigin::signed(9), 0),
            Error::<Test>::InvalidReportState
        );
    });
}

#[test]
fn clear_report_fails_when_action_already_pending() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        add_investigator(10);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        open_investigation_on(0, 9);
        assert_ok!(AntiCorruption::clear_report(RuntimeOrigin::signed(9), 0));

        // A second proposal (even from a different investigator, and even for the other kind
        // of action) is rejected while one is already pending on this report.
        assert_noop!(
            AntiCorruption::clear_report(RuntimeOrigin::signed(10), 0),
            Error::<Test>::ReportActionAlreadyPending
        );
        assert_noop!(
            AntiCorruption::refer_report_to_courts(RuntimeOrigin::signed(10), 0),
            Error::<Test>::ReportActionAlreadyPending
        );
    });
}

// ─── report workflow: refer_report_to_courts ────────────────────────────────

#[test]
fn refer_report_to_courts_fails_for_non_investigator() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        open_investigation_on(0, 9);

        assert_noop!(
            AntiCorruption::refer_report_to_courts(RuntimeOrigin::signed(1), 0),
            Error::<Test>::NotInvestigator
        );
    });
}

#[test]
fn refer_report_to_courts_fails_when_report_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);

        assert_noop!(
            AntiCorruption::refer_report_to_courts(RuntimeOrigin::signed(9), 0),
            Error::<Test>::ReportNotFound
        );
    });
}

#[test]
fn refer_report_to_courts_fails_when_not_under_investigation() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);

        // Still Pending — never flagged or opened.
        assert_noop!(
            AntiCorruption::refer_report_to_courts(RuntimeOrigin::signed(9), 0),
            Error::<Test>::InvalidReportState
        );
    });
}

// ─── add_investigator / remove_investigator ─────────────────────────────────

#[test]
fn add_investigator_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        assert_ok!(AntiCorruption::add_investigator(RuntimeOrigin::root(), 9));

        assert!(Investigators::<Test>::get().contains(&9));
        System::assert_last_event(Event::InvestigatorAdded { who: 9 }.into());
    });
}

#[test]
fn add_investigator_fails_for_non_root() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        assert_noop!(
            AntiCorruption::add_investigator(RuntimeOrigin::signed(1), 9),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn add_investigator_requires_appointment_origin() {
    // `Config::AppointmentOrigin` (in production,
    // `pallet_accountability_council::EnsureAccountabilityCouncilApproved` — a genuine 2/3
    // Council supermajority for this exact call) rejects a lone signed account outright, even
    // one signed by the very account being appointed (no self-appointment). This mock's
    // permissive `AsEnsureOriginWithArg<EnsureRoot<u64>>` (see mock.rs) stands in for a
    // successful Council approval with bare `Root` — the real call-hash-binding/supermajority
    // invariant is covered by `pallet-accountability-council`'s own test suite.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            AntiCorruption::add_investigator(RuntimeOrigin::signed(9), 9),
            DispatchError::BadOrigin
        );
        assert_ok!(AntiCorruption::add_investigator(RuntimeOrigin::root(), 9));
        assert!(Investigators::<Test>::get().contains(&9));
    });
}

#[test]
fn add_investigator_fails_when_already_investigator() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);

        assert_noop!(
            AntiCorruption::add_investigator(RuntimeOrigin::root(), 9),
            Error::<Test>::AlreadyInvestigator
        );
    });
}

#[test]
fn add_investigator_fails_when_at_capacity() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        for i in 0..MAX_INVESTIGATORS as u64 {
            add_investigator(i);
        }

        assert_noop!(
            AntiCorruption::add_investigator(RuntimeOrigin::root(), MAX_INVESTIGATORS as u64),
            Error::<Test>::TooManyInvestigators
        );
    });
}

#[test]
fn remove_investigator_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);

        assert_ok!(AntiCorruption::remove_investigator(RuntimeOrigin::root(), 9));

        assert!(!Investigators::<Test>::get().contains(&9));
        System::assert_last_event(Event::InvestigatorRemoved { who: 9 }.into());
    });
}

#[test]
fn remove_investigator_fails_for_non_root() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);

        assert_noop!(
            AntiCorruption::remove_investigator(RuntimeOrigin::signed(1), 9),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn remove_investigator_requires_appointment_origin() {
    // Same property as `add_investigator_requires_appointment_origin` above, for
    // `remove_investigator`.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        assert_noop!(
            AntiCorruption::remove_investigator(RuntimeOrigin::signed(9), 9),
            DispatchError::BadOrigin
        );
        assert_ok!(AntiCorruption::remove_investigator(RuntimeOrigin::root(), 9));
        assert!(!Investigators::<Test>::get().contains(&9));
    });
}

#[test]
fn remove_investigator_is_noop_ok_for_non_investigator() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        // Removing an account that was never an investigator is not an error —
        // `retain` is a no-op — but it still emits the event.
        assert_ok!(AntiCorruption::remove_investigator(RuntimeOrigin::root(), 9));
        System::assert_last_event(Event::InvestigatorRemoved { who: 9 }.into());
    });
}
