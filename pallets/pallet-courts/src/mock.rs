use crate as pallet_courts;
use crate::pallet::{CitizenChecker, CitizenSelector, CitizenSuspender, LawEnforcer, TreasuryEnforcer};
use frame_support::{derive_impl, traits::ConstU32};
use frame_system::EnsureRoot;
use sp_runtime::{BuildStorage, DispatchResult};
use std::cell::RefCell;

type Block = frame_system::mocking::MockBlock<Test>;
pub type AccountId = u64;
pub type BlockNumber = u64;

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
	pub type Courts = pallet_courts::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = Block;
	type AccountId = AccountId;
}

thread_local! {
	/// The citizen pool used by `CitizenSelector`. Index i -> account (i+1) by default,
	/// resized by tests via `set_citizen_count`.
	pub static CITIZEN_COUNT: RefCell<u32> = RefCell::new(30);
	/// Accounts considered suspended (i.e. NOT active citizens) by `CitizenChecker`.
	pub static SUSPENDED: RefCell<Vec<AccountId>> = RefCell::new(Vec::new());
	/// law_id -> number of times `invalidate_law` was called with it.
	pub static INVALIDATED_LAWS: RefCell<Vec<u32>> = RefCell::new(Vec::new());
	/// department_id -> number of times `freeze_department` was called with it.
	pub static FROZEN_DEPARTMENTS: RefCell<Vec<u32>> = RefCell::new(Vec::new());
	/// (nullifier, suspension_until) recorded by `suspend_citizen`.
	pub static SUSPENDED_CITIZENS: RefCell<Vec<([u8; 32], Option<BlockNumber>)>> = RefCell::new(Vec::new());
}

pub fn set_citizen_count(n: u32) {
	CITIZEN_COUNT.with(|c| *c.borrow_mut() = n);
}

pub fn set_suspended(who: AccountId) {
	SUSPENDED.with(|s| s.borrow_mut().push(who));
}

pub fn invalidated_laws() -> Vec<u32> {
	INVALIDATED_LAWS.with(|v| v.borrow().clone())
}

pub fn frozen_departments() -> Vec<u32> {
	FROZEN_DEPARTMENTS.with(|v| v.borrow().clone())
}

pub fn suspended_citizens() -> Vec<([u8; 32], Option<BlockNumber>)> {
	SUSPENDED_CITIZENS.with(|v| v.borrow().clone())
}

fn reset_mocks() {
	CITIZEN_COUNT.with(|c| *c.borrow_mut() = 30);
	SUSPENDED.with(|s| s.borrow_mut().clear());
	INVALIDATED_LAWS.with(|v| v.borrow_mut().clear());
	FROZEN_DEPARTMENTS.with(|v| v.borrow_mut().clear());
	SUSPENDED_CITIZENS.with(|v| v.borrow_mut().clear());
}

/// Citizens are accounts `1..=CITIZEN_COUNT`, one-indexed to match `citizen_at`'s 0-based index
/// (`citizen_at(0)` -> account 1, etc).
pub struct MockCitizenSelector;
impl CitizenSelector<AccountId> for MockCitizenSelector {
	fn citizen_at(index: u32) -> Option<AccountId> {
		let total = CITIZEN_COUNT.with(|c| *c.borrow());
		if index < total {
			Some((index as u64) + 1)
		} else {
			None
		}
	}

	fn total_citizens() -> u32 {
		CITIZEN_COUNT.with(|c| *c.borrow())
	}
}

pub struct MockCitizenChecker;
impl CitizenChecker<AccountId> for MockCitizenChecker {
	fn is_active_citizen(who: &AccountId) -> bool {
		!SUSPENDED.with(|s| s.borrow().contains(who))
	}
}

pub struct MockLawEnforcer;
impl LawEnforcer for MockLawEnforcer {
	fn invalidate_law(law_id: u32) -> DispatchResult {
		INVALIDATED_LAWS.with(|v| v.borrow_mut().push(law_id));
		Ok(())
	}
}

pub struct MockTreasuryEnforcer;
impl TreasuryEnforcer for MockTreasuryEnforcer {
	fn freeze_department(department_id: u32) -> DispatchResult {
		FROZEN_DEPARTMENTS.with(|v| v.borrow_mut().push(department_id));
		Ok(())
	}
}

pub struct MockCitizenSuspender;
impl CitizenSuspender<BlockNumber> for MockCitizenSuspender {
	fn suspend_citizen(nullifier: [u8; 32], suspension_until: Option<BlockNumber>) -> DispatchResult {
		SUSPENDED_CITIZENS.with(|v| v.borrow_mut().push((nullifier, suspension_until)));
		Ok(())
	}
}

frame_support::parameter_types! {
	pub const AutoChallengeAccountId: AccountId = 0;
}

impl pallet_courts::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type AppealWindowBlocks = ConstU32<100>;
	type CitizenSelector = MockCitizenSelector;
	type CitizenChecker = MockCitizenChecker;
	type LawEnforcer = MockLawEnforcer;
	type TreasuryEnforcer = MockTreasuryEnforcer;
	type OracleOrigin = EnsureRoot<AccountId>;
	type CitizenSuspender = MockCitizenSuspender;
	// Short delay so tests don't need to advance hundreds of blocks.
	type JurySeedDelayBlocks = ConstU32<3>;
	type AutoChallengeAccount = AutoChallengeAccountId;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
	let mut ext: sp_io::TestExternalities =
		frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into();
	ext.execute_with(|| {
		reset_mocks();
		System::set_block_number(1);
	});
	ext
}
