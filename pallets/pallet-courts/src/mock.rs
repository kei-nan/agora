use crate as pallet_courts;
use crate::pallet::{CitizenChecker, CitizenSelector, CitizenSuspender, LawEnforcer, TreasuryEnforcer};
use frame_support::{derive_impl, traits::{ConstU32, ConstU64}};
use frame_system::EnsureRoot;
use sp_runtime::{BuildStorage, DispatchResult};
use std::cell::RefCell;

type Block = frame_system::mocking::MockBlock<Test>;
pub type AccountId = u64;
pub type BlockNumber = u64;
pub type Balance = u64;

/// Small, deterministic bond amount — mirrors pallet-elections' `CANDIDATE_DEPOSIT` mock
/// constant (see that pallet's mock.rs).
pub const CASE_FILING_BOND: Balance = 100;

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
	pub type Balances = pallet_balances::Pallet<Test>;

	#[runtime::pallet_index(2)]
	pub type Courts = pallet_courts::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = Block;
	type AccountId = AccountId;
	type AccountData = pallet_balances::AccountData<Balance>;
}

#[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
impl pallet_balances::Config for Test {
	type AccountStore = System;
	type Balance = Balance;
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
	/// (nullifier, suspension_until, jury_reviewed) recorded by `suspend_citizen`.
	pub static SUSPENDED_CITIZENS: RefCell<Vec<([u8; 32], Option<BlockNumber>, bool)>> = RefCell::new(Vec::new());
	/// account -> registered identity nullifier, used by `MockCitizenChecker::citizen_nullifier`
	/// (`appeal_ruling`'s ruled-against-party check).
	pub static CITIZEN_NULLIFIERS: RefCell<Vec<(AccountId, [u8; 32])>> = RefCell::new(Vec::new());
}

pub fn set_citizen_count(n: u32) {
	CITIZEN_COUNT.with(|c| *c.borrow_mut() = n);
}

pub fn set_suspended(who: AccountId) {
	SUSPENDED.with(|s| s.borrow_mut().push(who));
}

/// Register `who` as the citizen holding `nullifier`, for `MockCitizenChecker::citizen_nullifier`.
pub fn set_citizen_nullifier(who: AccountId, nullifier: [u8; 32]) {
	CITIZEN_NULLIFIERS.with(|v| v.borrow_mut().push((who, nullifier)));
}

pub fn invalidated_laws() -> Vec<u32> {
	INVALIDATED_LAWS.with(|v| v.borrow().clone())
}

pub fn frozen_departments() -> Vec<u32> {
	FROZEN_DEPARTMENTS.with(|v| v.borrow().clone())
}

pub fn suspended_citizens() -> Vec<([u8; 32], Option<BlockNumber>, bool)> {
	SUSPENDED_CITIZENS.with(|v| v.borrow().clone())
}

fn reset_mocks() {
	CITIZEN_COUNT.with(|c| *c.borrow_mut() = 30);
	SUSPENDED.with(|s| s.borrow_mut().clear());
	INVALIDATED_LAWS.with(|v| v.borrow_mut().clear());
	FROZEN_DEPARTMENTS.with(|v| v.borrow_mut().clear());
	SUSPENDED_CITIZENS.with(|v| v.borrow_mut().clear());
	CITIZEN_NULLIFIERS.with(|v| v.borrow_mut().clear());
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

	fn citizen_nullifier(who: &AccountId) -> Option<[u8; 32]> {
		CITIZEN_NULLIFIERS.with(|v| v.borrow().iter().find(|(a, _)| a == who).map(|(_, n)| *n))
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
	fn suspend_citizen(
		nullifier: [u8; 32],
		suspension_until: Option<BlockNumber>,
		jury_reviewed: bool,
	) -> DispatchResult {
		SUSPENDED_CITIZENS.with(|v| v.borrow_mut().push((nullifier, suspension_until, jury_reviewed)));
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
	type MaxCasesPerBlock = ConstU32<16>;
	type AutoChallengeAccount = AutoChallengeAccountId;
	type Currency = Balances;
	type CaseFilingBond = ConstU64<CASE_FILING_BOND>;
	type MaxAIGovernanceCouncilSize = ConstU32<10>;
	// 2/3 supermajority, matching pallet-executive's cabinet threshold in the runtime.
	type AIModelSupermajorityNumerator = ConstU32<2>;
	type AIModelSupermajorityDenominator = ConstU32<3>;
}

// Build genesis storage according to the mock runtime. Accounts 1..=30 start with a balance
// comfortably above `CASE_FILING_BOND` (matches the default citizen pool size, see
// `CITIZEN_COUNT`), so filing/deposit tests don't need per-test funding boilerplate; account
// 999 is left unfunded for the insufficient-balance case.
pub fn new_test_ext() -> sp_io::TestExternalities {
	let mut storage = frame_system::GenesisConfig::<Test>::default().build_storage().unwrap();
	pallet_balances::GenesisConfig::<Test> {
		balances: (1..=30u64).map(|who| (who, 10_000u64)).collect(),
		..Default::default()
	}
	.assimilate_storage(&mut storage)
	.unwrap();
	let mut ext: sp_io::TestExternalities = storage.into();
	ext.execute_with(|| {
		reset_mocks();
		System::set_block_number(1);
	});
	ext
}
