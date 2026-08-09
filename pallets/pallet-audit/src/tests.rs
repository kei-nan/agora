use crate::{mock::*, AuditLog, AuditStatus, Auditors, Error, Event, OpenFlags};
use frame_support::{assert_noop, assert_ok};
use pallet_treasury_ledger::AuditHook;
use sp_runtime::DispatchError;

const DEPT: u32 = 7;
const AMOUNT: u128 = 1_000;
const IPFS_HASH: [u8; 32] = [9u8; 32];
const REASON_HASH: [u8; 32] = [4u8; 32];
const PERIOD_HASH: [u8; 32] = [5u8; 32];

fn add_auditor(who: u64) {
    assert_ok!(Audit::add_auditor(RuntimeOrigin::root(), who));
}

/// Drives an entry into existence via the `AuditHook` the same way
/// `pallet-treasury-ledger` would on a real expenditure.
fn record_expenditure(index: u64) {
    <crate::Pallet<Test> as AuditHook>::on_expenditure(index, DEPT, AMOUNT, IPFS_HASH);
}

fn pending_entry(index: u64) {
    record_expenditure(index);
}

fn flagged_entry(auditor: u64, index: u64) {
    record_expenditure(index);
    assert_ok!(Audit::flag_entry(RuntimeOrigin::signed(auditor), index, REASON_HASH));
}

fn disputed_entry(auditor: u64, index: u64) {
    flagged_entry(auditor, index);
    assert_ok!(Audit::dispute_entry(RuntimeOrigin::signed(auditor), index));
}

fn cleared_entry(auditor: u64, index: u64) {
    record_expenditure(index);
    assert_ok!(Audit::clear_entry(RuntimeOrigin::signed(auditor), index));
}

// ─── AuditHook::on_expenditure ──────────────────────────────────────────────

#[test]
fn on_expenditure_creates_pending_entry() {
    new_test_ext().execute_with(|| {
        record_expenditure(0);

        let entry = AuditLog::<Test>::get(0).expect("entry should exist");
        assert_eq!(entry.dept_id, DEPT);
        assert_eq!(entry.amount, AMOUNT);
        assert_eq!(entry.ipfs_hash, IPFS_HASH);
        assert!(entry.status == AuditStatus::Pending);
        assert_eq!(entry.flag_reason, None);
        assert_eq!(entry.flagged_by, None);
    });
}

#[test]
fn on_expenditure_overwrites_existing_index() {
    new_test_ext().execute_with(|| {
        record_expenditure(0);
        // A second hook call at the same index (shouldn't normally happen, but the hook
        // itself has no guard) simply overwrites the prior entry with fresh Pending data.
        <crate::Pallet<Test> as AuditHook>::on_expenditure(0, 99, 42, [1u8; 32]);

        let entry = AuditLog::<Test>::get(0).unwrap();
        assert_eq!(entry.dept_id, 99);
        assert_eq!(entry.amount, 42);
        assert!(entry.status == AuditStatus::Pending);
    });
}

// ─── add_auditor ─────────────────────────────────────────────────────────────

#[test]
fn add_auditor_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Audit::add_auditor(RuntimeOrigin::root(), 1));

        assert!(Auditors::<Test>::get().contains(&1));
        System::assert_last_event(Event::AuditorAdded { who: 1 }.into());
    });
}

#[test]
fn add_auditor_fails_for_non_root_origin() {
    new_test_ext().execute_with(|| {
        assert_noop!(
            Audit::add_auditor(RuntimeOrigin::signed(1), 2),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn add_auditor_fails_on_duplicate() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        assert_noop!(
            Audit::add_auditor(RuntimeOrigin::root(), 1),
            Error::<Test>::AlreadyAuditor
        );
    });
}

#[test]
fn add_auditor_fails_when_at_max_auditors_cap() {
    new_test_ext().execute_with(|| {
        // MaxAuditors is configured to 4 in the mock runtime.
        add_auditor(1);
        add_auditor(2);
        add_auditor(3);
        add_auditor(4);

        assert_noop!(
            Audit::add_auditor(RuntimeOrigin::root(), 5),
            Error::<Test>::TooManyAuditors
        );
        assert_eq!(Auditors::<Test>::get().len(), 4);
    });
}

// ─── remove_auditor ──────────────────────────────────────────────────────────

#[test]
fn remove_auditor_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_auditor(1);

        assert_ok!(Audit::remove_auditor(RuntimeOrigin::root(), 1));

        assert!(!Auditors::<Test>::get().contains(&1));
        System::assert_last_event(Event::AuditorRemoved { who: 1 }.into());
    });
}

#[test]
fn remove_auditor_fails_for_non_root_origin() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        assert_noop!(
            Audit::remove_auditor(RuntimeOrigin::signed(1), 1),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn remove_auditor_fails_when_not_an_auditor() {
    new_test_ext().execute_with(|| {
        assert_noop!(
            Audit::remove_auditor(RuntimeOrigin::root(), 1),
            Error::<Test>::NotAuditor
        );
    });
}

// ─── clear_entry ─────────────────────────────────────────────────────────────

#[test]
fn clear_entry_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_auditor(1);
        pending_entry(0);

        assert_ok!(Audit::clear_entry(RuntimeOrigin::signed(1), 0));

        let entry = AuditLog::<Test>::get(0).unwrap();
        assert!(entry.status == AuditStatus::Cleared);
        System::assert_last_event(Event::EntryCleared { index: 0 }.into());
    });
}

#[test]
fn clear_entry_fails_for_non_auditor() {
    new_test_ext().execute_with(|| {
        pending_entry(0);
        assert_noop!(
            Audit::clear_entry(RuntimeOrigin::signed(1), 0),
            Error::<Test>::NotAuditor
        );
    });
}

#[test]
fn clear_entry_fails_when_entry_not_found() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        assert_noop!(
            Audit::clear_entry(RuntimeOrigin::signed(1), 0),
            Error::<Test>::EntryNotFound
        );
    });
}

#[test]
fn clear_entry_fails_when_already_cleared() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        cleared_entry(1, 0);

        assert_noop!(
            Audit::clear_entry(RuntimeOrigin::signed(1), 0),
            Error::<Test>::AlreadyCertified
        );
    });
}

#[test]
fn clear_entry_fails_when_flagged() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        flagged_entry(1, 0);

        assert_noop!(
            Audit::clear_entry(RuntimeOrigin::signed(1), 0),
            Error::<Test>::AlreadyCertified
        );
    });
}

#[test]
fn clear_entry_fails_when_disputed() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        disputed_entry(1, 0);

        assert_noop!(
            Audit::clear_entry(RuntimeOrigin::signed(1), 0),
            Error::<Test>::AlreadyCertified
        );
    });
}

// ─── flag_entry ──────────────────────────────────────────────────────────────

#[test]
fn flag_entry_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_auditor(1);
        pending_entry(0);

        assert_ok!(Audit::flag_entry(RuntimeOrigin::signed(1), 0, REASON_HASH));

        let entry = AuditLog::<Test>::get(0).unwrap();
        assert!(entry.status == AuditStatus::Flagged);
        assert_eq!(entry.flag_reason, Some(REASON_HASH));
        assert_eq!(entry.flagged_by, Some(1));
        System::assert_last_event(
            Event::EntryFlagged { index: 0, by: 1, reason: REASON_HASH }.into(),
        );
    });
}

#[test]
fn flag_entry_fails_for_non_auditor() {
    new_test_ext().execute_with(|| {
        pending_entry(0);
        assert_noop!(
            Audit::flag_entry(RuntimeOrigin::signed(1), 0, REASON_HASH),
            Error::<Test>::NotAuditor
        );
    });
}

#[test]
fn flag_entry_fails_when_entry_not_found() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        assert_noop!(
            Audit::flag_entry(RuntimeOrigin::signed(1), 0, REASON_HASH),
            Error::<Test>::EntryNotFound
        );
    });
}

#[test]
fn flag_entry_fails_when_already_cleared() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        cleared_entry(1, 0);

        assert_noop!(
            Audit::flag_entry(RuntimeOrigin::signed(1), 0, REASON_HASH),
            Error::<Test>::AlreadyCertified
        );
    });
}

#[test]
fn flag_entry_fails_when_already_flagged() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        flagged_entry(1, 0);

        assert_noop!(
            Audit::flag_entry(RuntimeOrigin::signed(1), 0, REASON_HASH),
            Error::<Test>::EntryAlreadyFlagged
        );
    });
}

#[test]
fn flag_entry_fails_when_already_disputed() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        disputed_entry(1, 0);

        assert_noop!(
            Audit::flag_entry(RuntimeOrigin::signed(1), 0, REASON_HASH),
            Error::<Test>::EntryAlreadyDisputed
        );
    });
}

// ─── dispute_entry ───────────────────────────────────────────────────────────

#[test]
fn dispute_entry_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_auditor(1);
        flagged_entry(1, 0);

        assert_ok!(Audit::dispute_entry(RuntimeOrigin::signed(1), 0));

        let entry = AuditLog::<Test>::get(0).unwrap();
        assert!(entry.status == AuditStatus::Disputed);
        System::assert_last_event(Event::EntryDisputed { index: 0 }.into());
    });
}

#[test]
fn dispute_entry_fails_for_non_auditor() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        flagged_entry(1, 0);
        assert_noop!(
            Audit::dispute_entry(RuntimeOrigin::signed(2), 0),
            Error::<Test>::NotAuditor
        );
    });
}

#[test]
fn dispute_entry_fails_when_entry_not_found() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        assert_noop!(
            Audit::dispute_entry(RuntimeOrigin::signed(1), 0),
            Error::<Test>::EntryNotFound
        );
    });
}

#[test]
fn dispute_entry_fails_when_still_pending() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        pending_entry(0);

        // Cannot skip Pending -> Disputed; must go through Flagged first.
        assert_noop!(
            Audit::dispute_entry(RuntimeOrigin::signed(1), 0),
            Error::<Test>::MustBeFlaggedFirst
        );
    });
}

#[test]
fn dispute_entry_fails_when_already_cleared() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        cleared_entry(1, 0);

        assert_noop!(
            Audit::dispute_entry(RuntimeOrigin::signed(1), 0),
            Error::<Test>::AlreadyCertified
        );
    });
}

#[test]
fn dispute_entry_fails_when_already_disputed() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        disputed_entry(1, 0);

        // A Disputed entry cannot be re-disputed / downgraded.
        assert_noop!(
            Audit::dispute_entry(RuntimeOrigin::signed(1), 0),
            Error::<Test>::MustBeFlaggedFirst
        );
    });
}

// ─── resolve_entry ───────────────────────────────────────────────────────────

#[test]
fn resolve_entry_clears_a_flagged_entry() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_auditor(1);
        flagged_entry(1, 0);

        assert_ok!(Audit::resolve_entry(RuntimeOrigin::signed(1), 0));

        let entry = AuditLog::<Test>::get(0).unwrap();
        assert!(entry.status == AuditStatus::Cleared);
        System::assert_last_event(Event::EntryResolved { index: 0, by: 1 }.into());
    });
}

#[test]
fn resolve_entry_clears_a_disputed_entry() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        disputed_entry(1, 0);

        assert_ok!(Audit::resolve_entry(RuntimeOrigin::signed(1), 0));

        let entry = AuditLog::<Test>::get(0).unwrap();
        assert!(entry.status == AuditStatus::Cleared);
    });
}

#[test]
fn resolve_entry_fails_for_non_auditor() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        flagged_entry(1, 0);
        assert_noop!(
            Audit::resolve_entry(RuntimeOrigin::signed(2), 0),
            Error::<Test>::NotAuditor
        );
    });
}

#[test]
fn resolve_entry_fails_when_entry_not_found() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        assert_noop!(
            Audit::resolve_entry(RuntimeOrigin::signed(1), 0),
            Error::<Test>::EntryNotFound
        );
    });
}

#[test]
fn resolve_entry_fails_when_still_pending() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        pending_entry(0);

        assert_noop!(
            Audit::resolve_entry(RuntimeOrigin::signed(1), 0),
            Error::<Test>::EntryNotOpen
        );
    });
}

#[test]
fn resolve_entry_fails_when_already_cleared() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        cleared_entry(1, 0);

        assert_noop!(
            Audit::resolve_entry(RuntimeOrigin::signed(1), 0),
            Error::<Test>::EntryNotOpen
        );
    });
}

// ─── TreasuryFreezer wiring (flag/resolve <-> freeze/unfreeze) ──────────────

#[test]
fn flag_entry_freezes_the_department() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        flagged_entry(1, 0);

        assert_eq!(frozen_calls(), vec![DEPT]);
        assert!(unfrozen_calls().is_empty());
        assert_eq!(OpenFlags::<Test>::get(DEPT), 1);
    });
}

#[test]
fn resolving_the_only_open_flag_unfreezes_the_department() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        flagged_entry(1, 0);
        assert_eq!(frozen_calls(), vec![DEPT]);

        assert_ok!(Audit::resolve_entry(RuntimeOrigin::signed(1), 0));

        assert_eq!(unfrozen_calls(), vec![DEPT]);
        assert_eq!(OpenFlags::<Test>::get(DEPT), 0);
    });
}

#[test]
fn resolving_one_of_two_open_flags_does_not_unfreeze() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        // Two separate expenditures, same department, both flagged.
        flagged_entry(1, 0);
        record_expenditure(1);
        assert_ok!(Audit::flag_entry(RuntimeOrigin::signed(1), 1, REASON_HASH));
        assert_eq!(OpenFlags::<Test>::get(DEPT), 2);
        // Only one `freeze_department` call — the second flag on an already-frozen
        // department doesn't re-trigger it.
        assert_eq!(frozen_calls(), vec![DEPT]);

        // Resolve just the first entry.
        assert_ok!(Audit::resolve_entry(RuntimeOrigin::signed(1), 0));

        // Department still has one open flag (index 1), so it must stay frozen.
        assert_eq!(OpenFlags::<Test>::get(DEPT), 1);
        assert!(unfrozen_calls().is_empty());

        // Resolving the second (last) open flag finally unfreezes it.
        assert_ok!(Audit::resolve_entry(RuntimeOrigin::signed(1), 1));
        assert_eq!(OpenFlags::<Test>::get(DEPT), 0);
        assert_eq!(unfrozen_calls(), vec![DEPT]);
    });
}

#[test]
fn disputing_a_flagged_entry_does_not_change_open_count_or_refreeze() {
    new_test_ext().execute_with(|| {
        add_auditor(1);
        disputed_entry(1, 0);

        // dispute_entry escalates an already-open flag; it must not double-count or
        // issue a second freeze call.
        assert_eq!(OpenFlags::<Test>::get(DEPT), 1);
        assert_eq!(frozen_calls(), vec![DEPT]);
    });
}

// ─── submit_audit_report ─────────────────────────────────────────────────────

#[test]
fn submit_audit_report_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_auditor(1);

        assert_ok!(Audit::submit_audit_report(RuntimeOrigin::signed(1), PERIOD_HASH));

        System::assert_last_event(Event::AuditReportSubmitted { period_hash: PERIOD_HASH }.into());
    });
}

#[test]
fn submit_audit_report_fails_for_non_auditor() {
    new_test_ext().execute_with(|| {
        assert_noop!(
            Audit::submit_audit_report(RuntimeOrigin::signed(1), PERIOD_HASH),
            Error::<Test>::NotAuditor
        );
    });
}

// Note: `submit_audit_report` records a period-level report hash only — it does not
// reference or mutate any specific `AuditLog` entry (no `expenditure_index` parameter
// exists on the call), so there is no "nonexistent entry index" rejection path to test
// for this call.
