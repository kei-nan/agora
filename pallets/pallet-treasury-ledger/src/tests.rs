use crate::{
	mock::*, DepartmentBudgets, DepartmentSpenders, DepartmentSpent, Error, Event,
	ExpenditureLog, FrozenDepartments, NextExpenditureIndex,
};
use frame_support::{assert_noop, assert_ok};

const DEPT: u32 = 1;
const OTHER_DEPT: u32 = 2;
const SPENDER: u64 = 10;
const NOT_SPENDER: u64 = 11;
const HASH_A: [u8; 32] = [0xAA; 32];
const HASH_B: [u8; 32] = [0xBB; 32];

fn root() -> RuntimeOrigin {
	RuntimeOrigin::root()
}

fn signed(who: u64) -> RuntimeOrigin {
	RuntimeOrigin::signed(who)
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
// freeze / unfreeze
// ---------------------------------------------------------------------

#[test]
fn freeze_department_internal_sets_frozen_and_emits_event() {
	new_test_ext().execute_with(|| {
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		assert!(FrozenDepartments::<Test>::get(DEPT));
		System::assert_last_event(Event::DepartmentFrozen { department_id: DEPT }.into());
	});
}

#[test]
fn unfreeze_department_works_for_root() {
	new_test_ext().execute_with(|| {
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		assert_ok!(TreasuryLedger::unfreeze_department(root(), DEPT));
		assert!(!FrozenDepartments::<Test>::get(DEPT));
		System::assert_last_event(Event::DepartmentUnfrozen { department_id: DEPT }.into());
	});
}

#[test]
fn unfreeze_department_fails_for_non_root() {
	new_test_ext().execute_with(|| {
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		assert_noop!(
			TreasuryLedger::unfreeze_department(signed(SPENDER), DEPT),
			sp_runtime::DispatchError::BadOrigin
		);
		assert!(FrozenDepartments::<Test>::get(DEPT));
	});
}

#[test]
fn unfreeze_department_fails_when_not_frozen() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			TreasuryLedger::unfreeze_department(root(), DEPT),
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
		assert_ok!(TreasuryLedger::unfreeze_department(root(), DEPT));

		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A));
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 100);
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
fn legislature_call_hash_differs_for_different_department_ids() {
	let hash_a =
		crate::pallet::legislature_call_hash(b"pallet-treasury-ledger::reset_department_spent", 1u32);
	let hash_b =
		crate::pallet::legislature_call_hash(b"pallet-treasury-ledger::reset_department_spent", 2u32);
	assert_ne!(hash_a, hash_b);
}

// ---------------------------------------------------------------------
// unfreeze_department_internal (used by pallet-audit's TreasuryFreezer wiring)
// ---------------------------------------------------------------------

#[test]
fn unfreeze_department_internal_clears_frozen_and_emits_event() {
	new_test_ext().execute_with(|| {
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		assert!(FrozenDepartments::<Test>::get(DEPT));

		assert_ok!(crate::Pallet::<Test>::unfreeze_department_internal(DEPT));

		assert!(!FrozenDepartments::<Test>::get(DEPT));
		System::assert_last_event(Event::DepartmentUnfrozen { department_id: DEPT }.into());
	});
}

#[test]
fn unfreeze_department_internal_is_idempotent_when_not_frozen() {
	new_test_ext().execute_with(|| {
		// No prior freeze — must not panic or emit a spurious event.
		System::set_block_number(1);
		assert_ok!(crate::Pallet::<Test>::unfreeze_department_internal(DEPT));
		assert!(!FrozenDepartments::<Test>::get(DEPT));
		assert!(System::events().is_empty());
	});
}

#[test]
fn record_expenditure_succeeds_again_after_unfreeze_department_internal() {
	new_test_ext().execute_with(|| {
		assert_ok!(TreasuryLedger::allocate_budget(root(), DEPT, 1_000));
		assert_ok!(TreasuryLedger::register_department_spender(root(), DEPT, SPENDER));
		assert_ok!(crate::Pallet::<Test>::freeze_department_internal(DEPT));
		assert_noop!(
			TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A),
			Error::<Test>::DepartmentFrozen
		);

		// This is the path pallet-audit's TreasuryFreezer wiring uses (as opposed to the
		// root-only `unfreeze_department` dispatchable).
		assert_ok!(crate::Pallet::<Test>::unfreeze_department_internal(DEPT));

		assert_ok!(TreasuryLedger::record_expenditure(signed(SPENDER), DEPT, 100, HASH_A));
		assert_eq!(DepartmentSpent::<Test>::get(DEPT), 100);
	});
}
