use crate as pallet_accountability_council;
use crate::pallet::{ExecutiveChecker, LegislatureChecker};
use frame_support::{derive_impl, traits::ConstU32};
use sp_runtime::BuildStorage;
use std::cell::RefCell;

type Block = frame_system::mocking::MockBlock<Test>;
pub type AccountId = u64;

/// Maximum Council size used across tests. Deliberately larger than the 7-member bootstrapped
/// council most tests use, so add_member tests have room to grow the council.
pub const MAX_COUNCIL_SIZE: u32 = 10;
/// Supermajority fraction used across tests: 2/3.
pub const SUPERMAJORITY_NUMERATOR: u32 = 2;
pub const SUPERMAJORITY_DENOMINATOR: u32 = 3;
/// How many blocks an `ApprovedAction` token may sit unconsumed before it can be cleared.
pub const APPROVAL_EXPIRY: u32 = 20;

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
    pub type AccountabilityCouncil = pallet_accountability_council::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type Block = Block;
    type AccountId = AccountId;
}

thread_local! {
    /// Accounts the mock considers current pallet-legislature members.
    pub static LEGISLATURE_MEMBERS: RefCell<Vec<AccountId>> = RefCell::new(Vec::new());
    /// Accounts the mock considers current executive ministers/PM.
    pub static ACTIVE_MINISTERS: RefCell<Vec<AccountId>> = RefCell::new(Vec::new());
}

pub fn set_legislature_member(who: AccountId) {
    LEGISLATURE_MEMBERS.with(|v| v.borrow_mut().push(who));
}

pub fn set_active_minister(who: AccountId) {
    ACTIVE_MINISTERS.with(|v| v.borrow_mut().push(who));
}

fn reset_mocks() {
    LEGISLATURE_MEMBERS.with(|v| v.borrow_mut().clear());
    ACTIVE_MINISTERS.with(|v| v.borrow_mut().clear());
}

pub struct MockLegislatureChecker;
impl LegislatureChecker<AccountId> for MockLegislatureChecker {
    fn is_legislature_member(who: &AccountId) -> bool {
        LEGISLATURE_MEMBERS.with(|v| v.borrow().contains(who))
    }
}

pub struct MockExecutiveChecker;
impl ExecutiveChecker<AccountId> for MockExecutiveChecker {
    fn is_active_minister(who: &AccountId) -> bool {
        ACTIVE_MINISTERS.with(|v| v.borrow().contains(who))
    }
}

impl pallet_accountability_council::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type MaxCouncilSize = ConstU32<MAX_COUNCIL_SIZE>;
    type SupermajorityNumerator = ConstU32<SUPERMAJORITY_NUMERATOR>;
    type SupermajorityDenominator = ConstU32<SUPERMAJORITY_DENOMINATOR>;
    type ApprovalExpiryBlocks = ConstU32<APPROVAL_EXPIRY>;
    type LegislatureChecker = MockLegislatureChecker;
    type ExecutiveChecker = MockExecutiveChecker;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    let storage = frame_system::GenesisConfig::<Test>::default().build_storage().unwrap();
    let mut ext: sp_io::TestExternalities = storage.into();
    ext.execute_with(|| {
        reset_mocks();
        System::set_block_number(1);
    });
    ext
}
