use crate as pallet_legislature;
use frame_support::derive_impl;
use sp_runtime::BuildStorage;
use std::cell::RefCell;
use std::collections::BTreeSet;

type Block = frame_system::mocking::MockBlock<Test>;

thread_local! {
    /// Settable set of accounts currently treated as active executive ministers.
    /// `thread_local!` keeps this isolated per test thread since `cargo test` runs
    /// tests in parallel.
    static MINISTERS: RefCell<BTreeSet<u64>> = RefCell::new(BTreeSet::new());
}

/// Test double for `MinisterChecker`: consults the thread-local `MINISTERS` set.
pub struct TestMinisterChecker;

impl pallet_legislature::MinisterChecker<u64> for TestMinisterChecker {
    fn is_active_minister(who: &u64) -> bool {
        MINISTERS.with(|m| m.borrow().contains(who))
    }
}

/// Marks `who` as an active executive minister for the remainder of the test.
pub fn set_minister(who: u64) {
    MINISTERS.with(|m| m.borrow_mut().insert(who));
}

/// Clears minister status for `who`.
#[allow(dead_code)]
pub fn unset_minister(who: u64) {
    MINISTERS.with(|m| m.borrow_mut().remove(&who));
}

pub const MAX_MEMBERS: u32 = 5;
pub const MOTION_DURATION: u32 = 10;
pub const PASSAGE_THRESHOLD: u8 = 50;

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
    pub type Legislature = pallet_legislature::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type Block = Block;
}

impl pallet_legislature::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type MaxMembers = frame_support::traits::ConstU32<MAX_MEMBERS>;
    type MotionDurationBlocks = frame_support::traits::ConstU32<MOTION_DURATION>;
    type PassageThreshold = frame_support::traits::ConstU8<PASSAGE_THRESHOLD>;
    type MinisterChecker = TestMinisterChecker;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    MINISTERS.with(|m| m.borrow_mut().clear());
    frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}
