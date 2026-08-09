use crate as pallet_audit;
use crate::pallet::TreasuryFreezer;
use frame_support::derive_impl;
use sp_runtime::{BuildStorage, DispatchResult};
use std::cell::RefCell;

type Block = frame_system::mocking::MockBlock<Test>;

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
    pub type Audit = pallet_audit::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type Block = Block;
}

thread_local! {
    /// department_id -> number of times `freeze_department` was called with it, in order.
    pub static FROZEN_CALLS: RefCell<Vec<u32>> = RefCell::new(Vec::new());
    /// department_id -> number of times `unfreeze_department` was called with it, in order.
    pub static UNFROZEN_CALLS: RefCell<Vec<u32>> = RefCell::new(Vec::new());
}

pub fn frozen_calls() -> Vec<u32> {
    FROZEN_CALLS.with(|v| v.borrow().clone())
}

pub fn unfrozen_calls() -> Vec<u32> {
    UNFROZEN_CALLS.with(|v| v.borrow().clone())
}

fn reset_mocks() {
    FROZEN_CALLS.with(|v| v.borrow_mut().clear());
    UNFROZEN_CALLS.with(|v| v.borrow_mut().clear());
}

/// Test-only `TreasuryFreezer` that just records calls, so tests can assert exactly when
/// (and how many times) pallet-audit asks the treasury pallet to freeze/unfreeze a
/// department, without needing pallet-treasury-ledger wired into this mock runtime.
pub struct RecordingTreasuryFreezer;
impl TreasuryFreezer for RecordingTreasuryFreezer {
    fn freeze_department(department_id: u32) -> DispatchResult {
        FROZEN_CALLS.with(|v| v.borrow_mut().push(department_id));
        Ok(())
    }
    fn unfreeze_department(department_id: u32) -> DispatchResult {
        UNFROZEN_CALLS.with(|v| v.borrow_mut().push(department_id));
        Ok(())
    }
}

impl pallet_audit::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type MaxAuditors = frame_support::traits::ConstU32<4>;
    type TreasuryFreezer = RecordingTreasuryFreezer;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    let mut ext: sp_io::TestExternalities =
        frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into();
    ext.execute_with(reset_mocks);
    ext
}
