use crate as pallet_treasury_ledger;
use frame_support::derive_impl;
use frame_system::EnsureRoot;
use sp_runtime::BuildStorage;
use std::cell::RefCell;

type Block = frame_system::mocking::MockBlock<Test>;
pub type AccountId = u64;

#[frame_support::runtime]
mod runtime {
	// The main runtime
	#[runtime::runtime]
	// Runtime Types to be generated
	#[runtime::derive(
		RuntimeCall,
		RuntimeEvent,
		RuntimeError,
		RuntimeOrigin,
		RuntimeFreezeReason,
		RuntimeHoldReason,
		RuntimeSlashReason,
		RuntimeLockId,
		RuntimeTask,
		RuntimeViewFunction
	)]
	pub struct Test;

	#[runtime::pallet_index(0)]
	pub type System = frame_system::Pallet<Test>;

	#[runtime::pallet_index(1)]
	pub type TreasuryLedger = pallet_treasury_ledger::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = Block;
	type AccountId = AccountId;
}

thread_local! {
	/// Every call recorded by `RecordingAuditHook::on_expenditure`, in call order.
	pub static AUDIT_CALLS: RefCell<Vec<(u64, u32, u128, [u8; 32])>> = RefCell::new(Vec::new());
}

/// Test-only `AuditHook` implementation that records every call so tests can assert
/// an expenditure triggered exactly one audit hook call with the right data.
pub struct RecordingAuditHook;
impl pallet_treasury_ledger::AuditHook for RecordingAuditHook {
	fn on_expenditure(index: u64, dept_id: u32, amount: u128, ipfs_hash: [u8; 32]) {
		AUDIT_CALLS.with(|calls| calls.borrow_mut().push((index, dept_id, amount, ipfs_hash)));
	}
}

/// Clear the recorded audit hook calls. Call at the start of tests that assert on it.
pub fn audit_calls() -> Vec<(u64, u32, u128, [u8; 32])> {
	AUDIT_CALLS.with(|calls| calls.borrow().clone())
}

pub fn reset_audit_calls() {
	AUDIT_CALLS.with(|calls| calls.borrow_mut().clear());
}

impl pallet_treasury_ledger::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type Balance = u128;
	type AuditHook = RecordingAuditHook;
	// Root-gated: `RuntimeOrigin::root()` is authorized, any signed origin is not.
	type LegislatureOrigin = EnsureRoot<AccountId>;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
	let mut ext: sp_io::TestExternalities =
		frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into();
	ext.execute_with(|| {
		reset_audit_calls();
		System::set_block_number(1);
	});
	ext
}
