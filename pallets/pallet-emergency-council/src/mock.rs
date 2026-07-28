use crate as pallet_emergency_council;
use frame_support::{derive_impl, traits::ConstU32};
use sp_runtime::BuildStorage;

type Block = frame_system::mocking::MockBlock<Test>;

/// Constitutional ceiling on emergency duration used across tests: 100 blocks.
pub const MAX_EMERGENCY_BLOCKS: u32 = 100;
/// Maximum council size used across tests.
pub const MAX_COUNCIL_SIZE: u32 = 10;
/// Supermajority fraction used across tests: 2/3.
pub const SUPERMAJORITY_NUMERATOR: u32 = 2;
pub const SUPERMAJORITY_DENOMINATOR: u32 = 3;

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
    pub type EmergencyCouncil = pallet_emergency_council::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type Block = Block;
}

impl pallet_emergency_council::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type MaxEmergencyBlocks = ConstU32<MAX_EMERGENCY_BLOCKS>;
    type MaxCouncilSize = ConstU32<MAX_COUNCIL_SIZE>;
    type SupermajorityNumerator = ConstU32<SUPERMAJORITY_NUMERATOR>;
    type SupermajorityDenominator = ConstU32<SUPERMAJORITY_DENOMINATOR>;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}
