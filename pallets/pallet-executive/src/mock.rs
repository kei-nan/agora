use crate as pallet_executive;
use frame_support::derive_impl;
use frame_support::traits::ConstU32;
use sp_runtime::BuildStorage;

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
    pub type Executive = pallet_executive::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type Block = Block;
}

impl pallet_executive::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    // Root is authorized (stand-in for EnsureLegislatureMotion); any signed origin is
    // not — lets tests drive both the authorized and unauthorized-origin paths.
    // `AsEnsureOriginWithArg` ignores the call-hash argument `LegislatureOrigin` now
    // requires -- this pallet's own tests exercise this pallet's logic, not the call-hash
    // binding invariant (covered by pallet-legislature's own test suite).
    type LegislatureOrigin =
        frame_support::traits::AsEnsureOriginWithArg<frame_system::EnsureRoot<u64>>;
    type MaxPortfolios = ConstU32<5>;
    type MaxEmergencyBlocks = ConstU32<100>;
    type RatificationWindowBlocks = ConstU32<10>;
    type SupermajorityNumerator = ConstU32<2>;
    type SupermajorityDenominator = ConstU32<3>;
    type WeightInfo = ();
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}
