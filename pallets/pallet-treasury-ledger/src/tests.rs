use crate::{
	mock::*, AuditFrozenDepartments, CourtFrozenDepartments, DepartmentBudgets,
	DepartmentExpenditures, DepartmentSpenders, DepartmentSpent, Error, Event, ExpenditureLog,
	NextExpenditureIndex,
};
use frame_support::{assert_noop, assert_ok};

const DEPT: u32 = 1;
const OTHER_DEPT: u32 = 2;
const SPENDER: u64 = 10;
const NOT_SPENDER: u64 = 11;
const HASH_A: [u8; 32] = [0xAA; 32];
const HASH_B: [u8; 32] = [0xBB; 32];
const COUNCIL_MEMBER: u64 = 20;

fn root() -> RuntimeOrigin {
	RuntimeOrigin::root()
}

fn signed(who: u64) -> RuntimeOrigin {
	RuntimeOrigin::signed(who)
}

/// Simulate the Oracle Council having approved `unfreeze_department(department_id)` for
/// `COUNCIL_MEMBER` to consume (standing in for `propose_admin_action`/`approve_admin_action`
/// reaching M-of-N in the real runtime — see `mock::MockCourtOrigin`).
fn approve_unfreeze(department_id: u32) {
	let call_hash = crate::pallet::legislature_call_hash(
		b"pallet-treasury-ledger::unfreeze_department",
		department_id,
	);
	approve_court_action(call_hash, COUNCIL_MEMBER);
}

// ---------------------------------------------------------------------
// allocate_budget
// ---------------------------------------------------------------------

#[test]
fn allocate_budget_works_via_legislature_origin() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_eq!(DepartmentBudgets::<Test>::get(DEPT), 1_000);
		System::assert_last_event(Event::BudgetAllocated { department_id: DEPT, amount: 1_000 }.into());
	});
}

#[test]
fn allocate_budget_fails_for_non_legislature_origin() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			TreasuryLedger::allocate_budget(signed(SPENDER), DEPT, 1_000),
			sp_runtime::DispatchError::BadOrigin
		);
		assert_eq!(DepartmentBudgets::<Test>::get(DEPT), 0);
	});
}

#[test]
fn allocate_budget_replaces_not_adds_prior_allocation() {
	// Per the doc comment on `allocate_budget`: calling it twice replaces the
	// total, it is not additive. A supplemental appropriation must pass the
	// new grand total.
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 500));
		assert_eq!(DepartmentBudgets::<Test>::get(DEPT), 500);
	});
}

#[test]
fn allocate_budget_does_not_reset_department_spent() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 300, HASH_A));
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 300);

		// Re-allocating a new budget must not wipe out the accumulated spend.
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 2_000));
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 300);
	});
}

// ---------------------------------------------------------------------
// reset_department_spent
// ---------------------------------------------------------------------

#[test]
fn reset_department_spent_works_via_legislature_origin() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 300, HASH_A));
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 300);

		assert_ok!(TreasuryLedger::reset_department_spent(root(), DEPT));
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 0);
	});
}

#[test]
fn reset_department_spent_fails_for_non_legislature_origin() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			TreasuryLedger::reset_department_spent(signed(SPENDER), DEPT),
			sp_runtime::DispatchError::BadOrigin
		);
	});
}

// ---------------------------------------------------------------------
// register_department_spender / remove_department_spender
// ---------------------------------------------------------------------

#[test]
fn register_department_spender_works_for_root() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_eq!(DepartmentSpenders::<Test>::get(DEPT), Some(SPENDER));
		System::assert_last_event(
			Event::SpenderRegistered { department_id: DEPT, spender: SPENDER }.into(),
		);
	});
}

#[test]
fn register_department_spender_fails_for_non_root() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			TreasuryLedger::register_department_spender(signed(SPENDER), DEPT, SPENDER),
			sp_runtime::DispatchError::BadOrigin
		);
		assert_eq!(DepartmentSpenders::<Test>::get(DEPT), None);
	});
}

#[test]
fn register_department_spender_replaces_existing_spender() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, NOT_SPENDER));
		assert_eq!(DepartmentSpenders::<Test>::get(DEPT), Some(NOT_SPENDER));
	});
}

#[test]
fn remove_department_spender_works_for_root() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_ok!(TreasuryLedger::remove_department_spender(root(), DEPT));
		assert_eq!(DepartmentSpenders::<Test>::get(DEPT), None);
		System::assert_last_event(Event::SpenderRemoved { department_id: DEPT }.into());
	});
}

#[test]
fn remove_department_spender_fails_for_non_root() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_noop!(
			TreasuryLedger::remove_department_spender(signed(SPENDER), DEPT),
			sp_runtime::DispatchError::BadOrigin
		);
		assert_eq!(DepartmentSpenders::<Test>::get(DEPT), Some(SPENDER));
	});
}

#[test]
fn remove_department_spender_fails_when_none_registered() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			TreasuryLedger::remove_department_spender(root(), DEPT),
			Error::<Test>::DepartmentHasNoSpender
		);
	});
}

// ---------------------------------------------------------------------
// record_expenditure — authorization
// ---------------------------------------------------------------------

#[test]
fn record_expenditure_fails_when_no_spender_registered() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_noop!(
			TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A),
			Error::<Test>::DepartmentHasNoSpender
		);
		// No partial mutation: nothing spent, no log entry, no audit call.
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 0);
		assert_eq!(NextExpenditureIndex::<Test>::get(), 0);
		assert!(audit_calls().is_empty());
	});
}

#[test]
fn record_expenditure_fails_for_unauthorized_spender() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));

		assert_noop!(
			TreasuryLedger::record_expenditure(signed(NOT_SPENDER), DEPT, 100, HASH_A),
			Error::<Test>::NotAuthorizedSpender
		);
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 0);
		assert_eq!(NextExpenditureIndex::<Test>::get(), 0);
		assert!(audit_calls().is_empty());
	});
}

#[test]
fn record_expenditure_is_scoped_per_department() {
	// A spender authorized for one department must not be able to spend
	// against a different department's budget.
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), OTHER_DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));

		assert_noop!(
			TreasuryLedger::record_expenditure(signed(SPENDER), OTHER_DEPT, 100, HASH_A),
			Error::<Test>::DepartmentHasNoSpender
		);
	});
}

#[test]
fn record_expenditure_fails_when_department_frozen() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));

		assert_noop!(
			TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A),
			Error::<Test>::DepartmentFrozen
		);
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 0);
		assert!(audit_calls().is_empty());
	});
}

// ---------------------------------------------------------------------
// record_expenditure — spend-cap accounting invariants
// ---------------------------------------------------------------------

#[test]
fn record_expenditure_happy_path_decrements_remaining_budget_and_fires_audit_hook() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));

		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 400, HASH_A));

		// Spend counter updated; budget itself is untouched (budget - spent = remaining).
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 400);
		assert_eq!(DepartmentBudgets::<Test>::get(DEPT), 1_000);

		// Expenditure log + index advanced.
		assert_eq!(ExpenditureLog::<Test>::get(0), Some((DEPT, 400u128, HASH_A)));
		assert_eq!(NextExpenditureIndex::<Test>::get(), 1);

		// Event emitted.
		System::assert_last_event(
			Event::FundsSpent { department_id: DEPT, amount: 400, metadata_hash: HASH_A }.into(),
		);

		// Audit hook fired exactly once with accurate data.
		let calls = audit_calls();
		assert_eq!(calls.len(), 1);
		assert_eq!(calls[0], (0u64, DEPT, 400u128, HASH_A));
	});
}

#[test]
fn record_expenditure_multiple_calls_accumulate_spend_and_log_sequentially() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));

		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 300, HASH_A));
		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 200, HASH_B));

		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 500);
		assert_eq!(ExpenditureLog::<Test>::get(0), Some((DEPT, 300u128, HASH_A)));
		assert_eq!(ExpenditureLog::<Test>::get(1), Some((DEPT, 200u128, HASH_B)));
		assert_eq!(NextExpenditureIndex::<Test>::get(), 2);

		let calls = audit_calls();
		assert_eq!(calls.len(), 2);
		assert_eq!(calls[0], (0u64, DEPT, 300u128, HASH_A));
		assert_eq!(calls[1], (1u64, DEPT, 200u128, HASH_B));
	});
}

#[test]
fn record_expenditure_exact_remaining_budget_succeeds() {
	// Spending exactly the last unit of remaining budget must succeed
	// (new_spent <= budget, boundary case new_spent == budget).
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 999, HASH_A));

		// Only 1 unit of budget remains; spend it exactly.
		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 1, HASH_B));
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 1_000);
	});
}

#[test]
fn record_expenditure_one_over_remaining_budget_fails_cleanly() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 999, HASH_A));
		reset_audit_calls();

		// Only 1 unit remains; asking for 2 must fail without mutating state.
		assert_noop!(
			TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 2, HASH_B),
			Error::<Test>::InsufficientBudget
		);
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 999);
		assert_eq!(NextExpenditureIndex::<Test>::get(), 1);
		assert_eq!(ExpenditureLog::<Test>::get(1), None);
		assert!(audit_calls().is_empty());
	});
}

#[test]
fn record_expenditure_against_zero_budget_fails() {
	new_test_ext().execute_with(|| {
		// Department has an authorized spender but no budget allocation at all.
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_noop!(
			TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 1, HASH_A),
			Error::<Test>::InsufficientBudget
		);
	});
}

#[test]
fn record_expenditure_zero_amount_succeeds_and_still_logs_and_fires_audit_hook() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));

		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 0, HASH_A));
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 0);
		assert_eq!(NextExpenditureIndex::<Test>::get(), 1);

		let calls = audit_calls();
		assert_eq!(calls.len(), 1);
		assert_eq!(calls[0], (0u64, DEPT, 0u128, HASH_A));
	});
}

#[test]
fn record_expenditure_zero_amount_against_zero_budget_succeeds() {
	// Boundary: 0 <= 0 is a valid no-op expenditure (e.g. a zero-value
	// justification record) and must not be rejected as InsufficientBudget.
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 0, HASH_A));
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 0);
	});
}

#[test]
fn record_expenditure_overflow_is_rejected_not_wrapped() {
	// Regression test for the treasury accounting invariant: the spend-cap
	// check must be computed on a checked (non-wrapping) sum. If `spent +
	// amount` were allowed to wrap on overflow, a malicious/buggy spend could
	// wrap `DepartmentSpent` back down near zero and pass the `<= budget`
	// check, permanently corrupting the ledger and bypassing the cap.
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		DepartmentBudgets::<Test>::insert(DEPT, u128::MAX);
		DepartmentSpent::<Test>::insert(DEPT, u128::MAX);

		assert_noop!(
			TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 1, HASH_A),
			Error::<Test>::Overflow
		);
		// Spend counter must be untouched, not wrapped to a small number.
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), u128::MAX);
		assert!(audit_calls().is_empty());
	});
}

#[test]
fn record_expenditure_budget_check_uses_post_transaction_total_not_pre_transaction() {
	// Regression test for the specific accounting invariant fixed early in the
	// project (HANDOFF.md item 2, "Fix treasury accounting bug"): the cap
	// check must compare the department's total spend *after* this
	// expenditure (`new_spent`) against the budget — not the stale
	// pre-transaction `spent` value, which would let a single expenditure
	// blow through the remaining budget as long as spend-so-far was already
	// under cap.
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A));
		reset_audit_calls();

		// spent = 100, budget = 1000, remaining = 900. A 901-unit expenditure
		// must be rejected even though pre-transaction `spent` (100) is well
		// under budget.
		assert_noop!(
			TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 901, HASH_B),
			Error::<Test>::InsufficientBudget
		);
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 100);
		assert!(audit_calls().is_empty());
	});
}

// ---------------------------------------------------------------------
// DepartmentExpenditures — secondary index consistency with ExpenditureLog
//
// court-oracle's `fetch_expenditures_for_department` used to scan the entire ExpenditureLog
// and filter client-side (a real scaling problem for a long-lived chain, flagged in that
// crate's own doc comments). This index lets a caller scope a chain read to one department
// instead. These tests confirm it never drifts from the primary log it mirrors.
// ---------------------------------------------------------------------

#[test]
fn record_expenditure_populates_department_expenditures_index() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));

		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 400, HASH_A));

		assert!(DepartmentExpenditures::<Test>::contains_key(DEPT, 0));
		assert_eq!(DepartmentExpenditures::<Test>::get(DEPT, 0), Some(()));
	});
}

#[test]
fn department_expenditures_index_only_lists_that_departments_indices() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::allocate_budget(root(), OTHER_DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_ok!(TreasuryLedger::register_department_spender(root(), OTHER_DEPT, NOT_SPENDER));

		// Interleaved expenditures across two departments — indices 0, 2 belong to DEPT;
		// index 1 belongs to OTHER_DEPT.
		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A));
		assert_ok!(TreasuryLedger::record_expenditure(signed(NOT_SPENDER), OTHER_DEPT, 200, HASH_B));
		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 300, HASH_A));

		let dept_indices: Vec<u64> = DepartmentExpenditures::<Test>::iter_prefix(DEPT)
			.map(|(idx, _)| idx)
			.collect();
		let mut dept_indices = dept_indices;
		dept_indices.sort();
		assert_eq!(dept_indices, vec![0, 2]);

		let other_indices: Vec<u64> = DepartmentExpenditures::<Test>::iter_prefix(OTHER_DEPT)
			.map(|(idx, _)| idx)
			.collect();
		assert_eq!(other_indices, vec![1]);

		// Every index reachable through the secondary index must resolve to a primary
		// ExpenditureLog entry tagged with the same department — the invariant the index
		// exists to preserve.
		for idx in DepartmentExpenditures::<Test>::iter_prefix(DEPT).map(|(idx, _)| idx) {
			let (logged_dept, _, _) = ExpenditureLog::<Test>::get(idx).expect("log entry must exist");
			assert_eq!(logged_dept, DEPT);
		}
	});
}

#[test]
fn department_expenditures_index_stays_empty_when_no_expenditure_recorded() {
	new_test_ext().execute_with(|| {
		assert_eq!(DepartmentExpenditures::<Test>::iter_prefix(DEPT).count(), 0);
	});
}

#[test]
fn record_expenditure_index_grows_one_entry_per_call_never_a_scan_of_everything() {
	// Not a perf benchmark, but a structural check: the index for a department must have
	// exactly as many entries as expenditures recorded against it, proving a department-scoped
	// read (iter_prefix) returns precisely that department's rows rather than requiring the
	// caller to walk the whole ExpenditureLog and filter.
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 10_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		for _ in 0..5 {
			assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 1, HASH_A));
		}
		assert_eq!(DepartmentExpenditures::<Test>::iter_prefix(DEPT).count(), 5);
		assert_eq!(NextExpenditureIndex::<Test>::get(), 5);
	});
}

// ---------------------------------------------------------------------
// freeze / unfreeze
// ---------------------------------------------------------------------

#[test]
fn freeze_department_internal_sets_frozen_and_emits_event() {
	new_test_ext().execute_with(|| {
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		assert!(CourtFrozenDepartments::<Test>::get(DEPT));
		assert!(!AuditFrozenDepartments::<Test>::get(DEPT));
		System::assert_last_event(Event::DepartmentFrozen { department_id: DEPT }.into());
	});
}

#[test]
fn unfreeze_department_succeeds_with_court_origin_approval() {
	// The fix for the finding below: `unfreeze_department` requires Oracle Council M-of-N
	// approval (`T::CourtOrigin`), not bare root.
	new_test_ext().execute_with(|| {
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		approve_unfreeze(DEPT);
		assert_ok!(TreasuryLedger::unfreeze_department(signed(COUNCIL_MEMBER), DEPT));
		assert!(!CourtFrozenDepartments::<Test>::get(DEPT));
		System::assert_last_event(Event::DepartmentUnfrozen { department_id: DEPT }.into());
	});
}

// ── CVE-class regression: court-ordered freeze reversible by a lone Root key ──────────────
//
// `freeze_department` (via `TreasuryEnforcer`, triggered only by a court ruling that has
// itself cleared pallet-courts' M-of-7 `EnsureOracleCouncilApproved` flow) used to be
// pairable with an `unfreeze_department` gated by bare `ensure_root` — letting a single
// Root/sudo key silently undo an already-adjudicated court ruling with no council or jury
// involvement. `unfreeze_department` is now gated the same way `freeze_department` effectively
// is: Oracle Council M-of-N approval via `T::CourtOrigin`.

#[test]
fn unfreeze_department_fails_for_lone_root() {
	new_test_ext().execute_with(|| {
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		assert_noop!(
			TreasuryLedger::unfreeze_department(root(), DEPT),
			sp_runtime::DispatchError::BadOrigin
		);
		assert!(CourtFrozenDepartments::<Test>::get(DEPT));
	});
}

#[test]
fn unfreeze_department_fails_for_unapproved_signed_origin() {
	new_test_ext().execute_with(|| {
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		assert_noop!(
			TreasuryLedger::unfreeze_department(signed(SPENDER), DEPT),
			sp_runtime::DispatchError::BadOrigin
		);
		assert!(CourtFrozenDepartments::<Test>::get(DEPT));
	});
}

#[test]
fn unfreeze_department_fails_when_not_frozen() {
	new_test_ext().execute_with(|| {
		approve_unfreeze(DEPT);
		assert_noop!(
			TreasuryLedger::unfreeze_department(signed(COUNCIL_MEMBER), DEPT),
			Error::<Test>::DepartmentNotFrozen
		);
	});
}

#[test]
fn record_expenditure_succeeds_again_after_unfreeze() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		approve_unfreeze(DEPT);
		assert_ok!(TreasuryLedger::unfreeze_department(signed(COUNCIL_MEMBER), DEPT));

		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A));
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 100);
	});
}

// ---------------------------------------------------------------------
// cross-authority independence (the bug this fix addresses)
//
// Regression coverage for the original bug: a single shared `FrozenDepartments` bool let
// pallet-audit's unfreeze path clear a still-open pallet-courts freeze (and vice versa)
// whenever the *other* authority's own state happened to clear. `CourtFrozenDepartments` and
// `AuditFrozenDepartments` are now independent axes; `is_frozen`/`record_expenditure` check
// the OR of both.
// ---------------------------------------------------------------------

#[test]
fn audit_unfreeze_does_not_lift_a_still_open_court_freeze() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));

		// pallet-courts freezes the department for an unresolved ruling.
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		// pallet-audit independently opens (and then fully resolves) its own flag against the
		// same department — simulating flag_entry followed by resolve_entry bringing
		// OpenFlags back to zero.
		assert_ok!(crate::Pallet::<Test>::audit_freeze_department_internal(DEPT));
		assert_ok!(crate::Pallet::<Test>::audit_unfreeze_department_internal(DEPT));

		// The court-ordered freeze must still be in effect: audit resolving its own last flag
		// must NOT silently lift it.
		assert!(crate::Pallet::<Test>::is_frozen(DEPT));
		assert!(CourtFrozenDepartments::<Test>::get(DEPT));
		assert!(!AuditFrozenDepartments::<Test>::get(DEPT));
		assert_noop!(
			TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A),
			Error::<Test>::DepartmentFrozen
		);

		// Only the Oracle-Council-approved manual override clears the remaining (court) axis.
		approve_unfreeze(DEPT);
		assert_ok!(TreasuryLedger::unfreeze_department(signed(COUNCIL_MEMBER), DEPT));
		assert!(!crate::Pallet::<Test>::is_frozen(DEPT));
		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A));
	});
}

#[test]
fn court_freeze_does_not_get_lifted_by_audit_unfreeze_when_audit_never_froze() {
	// A department frozen only by pallet-courts (audit axis never set) must stay frozen if
	// pallet-audit's unfreeze path is invoked for unrelated reasons (idempotent no-op on an
	// axis that was never set).
	new_test_ext().execute_with(|| {
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		assert_ok!(crate::Pallet::<Test>::audit_unfreeze_department_internal(DEPT));
		assert!(crate::Pallet::<Test>::is_frozen(DEPT));
		assert!(CourtFrozenDepartments::<Test>::get(DEPT));
	});
}

#[test]
fn court_freeze_never_auto_clears_from_audit_side_even_after_many_audit_cycles() {
	// The reverse direction proven directly: pallet-audit freezing and unfreezing its own
	// axis repeatedly must never touch CourtFrozenDepartments.
	new_test_ext().execute_with(|| {
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		for _ in 0..3 {
			assert_ok!(crate::Pallet::<Test>::audit_freeze_department_internal(DEPT));
			assert_ok!(crate::Pallet::<Test>::audit_unfreeze_department_internal(DEPT));
		}
		assert!(CourtFrozenDepartments::<Test>::get(DEPT));
		assert!(crate::Pallet::<Test>::is_frozen(DEPT));
	});
}

#[test]
fn audit_freeze_alone_blocks_expenditure_and_audit_unfreeze_alone_clears_it() {
	// Sanity check that the audit-only axis (no court freeze involved) still works exactly as
	// before the split: freeze blocks spending, unfreeze (last flag resolved) restores it.
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));

		assert_ok!(crate::Pallet::<Test>::audit_freeze_department_internal(DEPT));
		assert_noop!(
			TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A),
			Error::<Test>::DepartmentFrozen
		);

		assert_ok!(crate::Pallet::<Test>::audit_unfreeze_department_internal(DEPT));
		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A));
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 100);
	});
}

#[test]
fn unfreeze_department_dispatchable_clears_both_axes() {
	// The manual override is an explicit full clear (documented design choice on
	// `unfreeze_department`), not a per-axis clear.
	new_test_ext().execute_with(|| {
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		assert_ok!(crate::Pallet::<Test>::audit_freeze_department_internal(DEPT));
		assert!(CourtFrozenDepartments::<Test>::get(DEPT));
		assert!(AuditFrozenDepartments::<Test>::get(DEPT));

		approve_unfreeze(DEPT);
		assert_ok!(TreasuryLedger::unfreeze_department(signed(COUNCIL_MEMBER), DEPT));

		assert!(!CourtFrozenDepartments::<Test>::get(DEPT));
		assert!(!AuditFrozenDepartments::<Test>::get(DEPT));
		assert!(!crate::Pallet::<Test>::is_frozen(DEPT));
	});
}

// ── legislature_call_hash (HIGH-severity motion-hijack fix) ────────────────────
//
// See the equivalent block in pallet-constitution's tests for the full rationale. The
// binding invariant itself (a token approved for call A is rejected against call B's
// hash) is proven against the real `EnsureLegislatureMotion` origin in
// pallet-legislature's own suite; here we just confirm this pallet's two
// `LegislatureOrigin`-gated calls never hash to the same value for overlapping raw
// parameters, which is the property that invariant depends on.
#[test]
fn legislature_call_hash_differs_across_allocate_budget_and_reset_department_spent() {
	let allocate_hash = crate::pallet::legislature_call_hash(
		b"pallet-treasury-ledger::allocate_budget",
		(DEPT, 1_000u128),
	);
	let reset_hash =
		crate::pallet::legislature_call_hash(b"pallet-treasury-ledger::reset_department_spent", DEPT);
	assert_ne!(allocate_hash, reset_hash);
}

#[test]
fn legislature_call_hash_differs_for_reset_department_spent_and_unfreeze_department() {
	// Same reasoning as above, now covering `T::CourtOrigin`-gated `unfreeze_department`
	// (both take a single `u32` department_id — close enough in shape to collide without
	// domain separation by call tag).
	let reset_hash =
		crate::pallet::legislature_call_hash(b"pallet-treasury-ledger::reset_department_spent", DEPT);
	let unfreeze_hash =
		crate::pallet::legislature_call_hash(b"pallet-treasury-ledger::unfreeze_department", DEPT);
	assert_ne!(reset_hash, unfreeze_hash);
}

#[test]
fn legislature_call_hash_differs_for_different_department_ids() {
	let hash_a =
		crate::pallet::legislature_call_hash(b"pallet-treasury-ledger::reset_department_spent", 1u32);
	let hash_b =
		crate::pallet::legislature_call_hash(b"pallet-treasury-ledger::reset_department_spent", 2u32);
	assert_ne!(hash_a, hash_b);
}

// ---------------------------------------------------------------------
// audit_freeze_department_internal / audit_unfreeze_department_internal
// (used by pallet-audit's TreasuryFreezer wiring)
// ---------------------------------------------------------------------

#[test]
fn audit_unfreeze_department_internal_clears_frozen_and_emits_event() {
	new_test_ext().execute_with(|| {
		assert_ok!(crate::Pallet::<Test>::audit_freeze_department_internal(DEPT));
		assert!(AuditFrozenDepartments::<Test>::get(DEPT));

		assert_ok!(crate::Pallet::<Test>::audit_unfreeze_department_internal(DEPT));

		assert!(!AuditFrozenDepartments::<Test>::get(DEPT));
		System::assert_last_event(Event::DepartmentUnfrozen { department_id: DEPT }.into());
	});
}

#[test]
fn audit_unfreeze_department_internal_is_idempotent_when_not_frozen() {
	new_test_ext().execute_with(|| {
		// No prior freeze — must not panic or emit a spurious event.
		System::set_block_number(1);
		assert_ok!(crate::Pallet::<Test>::audit_unfreeze_department_internal(DEPT));
		assert!(!AuditFrozenDepartments::<Test>::get(DEPT));
		assert!(System::events().is_empty());
	});
}

#[test]
fn audit_unfreeze_department_internal_emits_no_event_while_court_freeze_still_active() {
	// If the court axis is still set, the department is NOT actually unfrozen overall — the
	// event must not claim otherwise, even though the audit axis itself did clear.
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		assert_ok!(crate::Pallet::<Test>::audit_freeze_department_internal(DEPT));
		let events_before = System::events().len();

		assert_ok!(crate::Pallet::<Test>::audit_unfreeze_department_internal(DEPT));

		assert!(!AuditFrozenDepartments::<Test>::get(DEPT));
		assert!(CourtFrozenDepartments::<Test>::get(DEPT));
		// No new event: the department is still frozen overall (court axis), so no
		// `DepartmentUnfrozen` should be emitted.
		assert_eq!(System::events().len(), events_before);
	});
}

#[test]
fn record_expenditure_succeeds_again_after_audit_unfreeze_department_internal() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_ok!(crate::Pallet::<Test>::audit_freeze_department_internal(DEPT));
		assert_noop!(
			TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A),
			Error::<Test>::DepartmentFrozen
		);

		// This is the path pallet-audit's TreasuryFreezer wiring uses (as opposed to the
		// Oracle-Council-gated `unfreeze_department` dispatchable).
		assert_ok!(crate::Pallet::<Test>::audit_unfreeze_department_internal(DEPT));

		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A));
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 100);
	});
}
