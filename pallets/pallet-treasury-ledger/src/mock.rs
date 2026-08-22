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
	/// The single `(call_hash, account)` pair `MockCourtOrigin` currently accepts, standing in
	/// for an Oracle Council M-of-N approval already recorded for that exact call in the real
	/// `pallet_courts::EnsureOracleCouncilApproved`. Consumed (cleared) on use, single-shot
	/// just like the real thing.
	static APPROVED_COURT_ACTION: RefCell<Option<([u8; 32], AccountId)>> = const { RefCell::new(None) };
}

/// Test helper: simulate the Oracle Council having approved `call_hash` for `who` to consume —
/// stands in for `pallet_courts::propose_admin_action` + `approve_admin_action` reaching M-of-N
/// in the real runtime.
pub fn approve_court_action(call_hash: [u8; 32], who: AccountId) {
	APPROVED_COURT_ACTION.with(|c| *c.borrow_mut() = Some((call_hash, who)));
}

/// Test-only stand-in for `pallet_courts::EnsureOracleCouncilApproved`. Unlike this mock's
/// `LegislatureOrigin` below (which adapts bare `EnsureRoot`, deferring the call-hash binding
/// invariant itself to pallet-legislature's own suite), the security property under test for
/// `CourtOrigin` is specifically that bare Root must NOT be sufficient — so this mock models
/// the real origin's actual shape (a call-hash-gated *signed* account, never `Root`) instead of
/// trivially accepting Root, letting this pallet's own tests prove that property directly.
pub struct MockCourtOrigin;
impl frame_support::traits::EnsureOriginWithArg<RuntimeOrigin, [u8; 32]> for MockCourtOrigin {
	type Success = ();

	fn try_origin(o: RuntimeOrigin, call_hash: &[u8; 32]) -> Result<Self::Success, RuntimeOrigin> {
		use frame_system::RawOrigin;
		match o.clone().into() {
			Ok(RawOrigin::Signed(who)) => {
				let matches =
					APPROVED_COURT_ACTION.with(|c| *c.borrow() == Some((*call_hash, who)));
				if matches {
					APPROVED_COURT_ACTION.with(|c| *c.borrow_mut() = None);
					Ok(())
				} else {
					Err(o)
				}
			}
			_ => Err(o),
		}
	}

	#[cfg(feature = "runtime-benchmarks")]
	fn try_successful_origin(_call_hash: &[u8; 32]) -> Result<RuntimeOrigin, ()> {
		Err(())
	}
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
	// `AsEnsureOriginWithArg` ignores the call-hash argument `LegislatureOrigin` now
	// requires -- this pallet's own tests exercise this pallet's logic, not the call-hash
	// binding invariant (covered by pallet-legislature's own test suite).
	type LegislatureOrigin = frame_support::traits::AsEnsureOriginWithArg<EnsureRoot<AccountId>>;
	type CourtOrigin = MockCourtOrigin;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
	let mut ext: sp_io::TestExternalities =
		frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into();
	ext.execute_with(|| {
		reset_audit_calls();
		APPROVED_COURT_ACTION.with(|c| *c.borrow_mut() = None);
		System::set_block_number(1);
	});
	ext
}
