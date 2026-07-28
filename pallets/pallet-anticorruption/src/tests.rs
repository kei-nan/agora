use crate::{
    mock::*, AssetDisclosures, ConflictRegistry, ConflictType, Error, Event, Investigators,
    NextReportId, ReportNullifiers, ReportStatus, WhistleblowerReports,
};
use frame_support::{assert_noop, assert_ok, traits::ConstU32, BoundedVec};
use sp_runtime::DispatchError;

fn valid_proof() -> BoundedVec<u8, ConstU32<4096>> {
    BoundedVec::try_from(vec![VALID_PROOF_MARKER]).unwrap()
}

fn invalid_proof() -> BoundedVec<u8, ConstU32<4096>> {
    BoundedVec::try_from(vec![INVALID_PROOF_MARKER]).unwrap()
}

fn public_inputs(nullifier: [u8; 32]) -> BoundedVec<[u8; 32], ConstU32<16>> {
    BoundedVec::try_from(vec![nullifier]).unwrap()
}

fn empty_public_inputs() -> BoundedVec<[u8; 32], ConstU32<16>> {
    BoundedVec::try_from(Vec::new()).unwrap()
}

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
        // MissingNullifierInput check must run before the verifier is invoked, so this
        // must fail with MissingNullifierInput, not InvalidZkProof.
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

// ─── report workflow: clear_report ──────────────────────────────────────────

#[test]
fn clear_report_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        assert_ok!(AntiCorruption::flag_report(RuntimeOrigin::signed(9), 0));
        assert_ok!(AntiCorruption::open_investigation(RuntimeOrigin::signed(9), 0));

        assert_ok!(AntiCorruption::clear_report(RuntimeOrigin::signed(9), 0));

        assert_eq!(WhistleblowerReports::<Test>::get(0).unwrap().status, ReportStatus::Cleared);
        System::assert_last_event(Event::ReportCleared { report_id: 0, investigator: 9 }.into());
    });
}

#[test]
fn clear_report_fails_for_non_investigator() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        assert_ok!(AntiCorruption::flag_report(RuntimeOrigin::signed(9), 0));
        assert_ok!(AntiCorruption::open_investigation(RuntimeOrigin::signed(9), 0));

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

// ─── report workflow: refer_report_to_courts ────────────────────────────────

#[test]
fn refer_report_to_courts_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        assert_ok!(AntiCorruption::flag_report(RuntimeOrigin::signed(9), 0));
        assert_ok!(AntiCorruption::open_investigation(RuntimeOrigin::signed(9), 0));

        assert_ok!(AntiCorruption::refer_report_to_courts(RuntimeOrigin::signed(9), 0));

        assert_eq!(
            WhistleblowerReports::<Test>::get(0).unwrap().status,
            ReportStatus::ReferredToCourts
        );
        System::assert_last_event(
            Event::ReportReferredToCourts { report_id: 0, investigator: 9 }.into(),
        );
    });
}

#[test]
fn refer_report_to_courts_fails_for_non_investigator() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_investigator(9);
        submit_report(1, NULLIFIER_A, CONTENT_A);
        assert_ok!(AntiCorruption::flag_report(RuntimeOrigin::signed(9), 0));
        assert_ok!(AntiCorruption::open_investigation(RuntimeOrigin::signed(9), 0));

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
fn remove_investigator_is_noop_ok_for_non_investigator() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        // Removing an account that was never an investigator is not an error —
        // `retain` is a no-op — but it still emits the event.
        assert_ok!(AntiCorruption::remove_investigator(RuntimeOrigin::root(), 9));
        System::assert_last_event(Event::InvestigatorRemoved { who: 9 }.into());
    });
}
