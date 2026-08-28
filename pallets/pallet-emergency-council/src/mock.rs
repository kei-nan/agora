use crate as pallet_emergency_council;
use frame_support::{derive_impl, traits::ConstU32};
use sp_runtime::BuildStorage;
use std::cell::RefCell;

type Block = frame_system::mocking::MockBlock<Test>;

/// Constitutional ceiling on emergency duration used across tests: 100 blocks.
pub const MAX_EMERGENCY_BLOCKS: u32 = 100;
/// Cooldown after an emergency ends before another can be declared, used across tests.
pub const EMERGENCY_COOLDOWN_BLOCKS: u32 = 20;
/// Maximum council size used across tests.
pub const MAX_COUNCIL_SIZE: u32 = 10;
/// Supermajority fraction used across tests: 2/3.
pub const SUPERMAJORITY_NUMERATOR: u32 = 2;
pub const SUPERMAJORITY_DENOMINATOR: u32 = 3;

// ── Mock cross-pallet sibling cooldown (stands in for pallet-executive) ─────
//
// pallet-executive depends on `pallet_emergency_council::SiblingEmergencyCooldown` in the
// real runtime (implemented on `Runtime`, see `runtime/src/configs/mod.rs`). Here it's backed
// by simple thread-local state, same idiom as `pallet-executive/src/mock.rs`'s own
// `CitizenChecker`/`LegislatureMembership` mocks. Defaults (cooldown-until 0, no notification
// recorded) make this behave exactly like the no-op `()` impl unless a test explicitly calls
// `set_sibling_cooldown_until`, so ordinary tests don't need to know pallet-executive exists.
thread_local! {
    static SIBLING_COOLDOWN_UNTIL: RefCell<u64> = RefCell::new(0);
    static SIBLING_NOTIFIED_AT: RefCell<Option<u64>> = RefCell::new(None);
}

/// Simulates pallet-executive currently being in cooldown until block `until`.
pub fn set_sibling_cooldown_until(until: u64) {
    SIBLING_COOLDOWN_UNTIL.with(|c| *c.borrow_mut() = until);
}

/// The block number this pallet last told its mock sibling to start its cooldown at, if this
/// pallet's own cooldown has ever ended since the mock was last reset.
pub fn sibling_notified_at() -> Option<u64> {
    SIBLING_NOTIFIED_AT.with(|c| *c.borrow())
}

pub struct MockSiblingCooldown;
impl pallet_emergency_council::SiblingEmergencyCooldown<u64> for MockSiblingCooldown {
    fn is_in_cooldown(now: u64) -> bool {
        now < SIBLING_COOLDOWN_UNTIL.with(|c| *c.borrow())
    }
    fn notify_emergency_ended(now: u64) {
        SIBLING_NOTIFIED_AT.with(|c| *c.borrow_mut() = Some(now));
    }
}

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
    type EmergencyCooldownBlocks = ConstU32<EMERGENCY_COOLDOWN_BLOCKS>;
    type MaxCouncilSize = ConstU32<MAX_COUNCIL_SIZE>;
    type SupermajorityNumerator = ConstU32<SUPERMAJORITY_NUMERATOR>;
    type SupermajorityDenominator = ConstU32<SUPERMAJORITY_DENOMINATOR>;
    // See the mock's doc comment above — behaves as a no-op by default, controllable per test
    // via `set_sibling_cooldown_until`/`sibling_notified_at`.
    type SiblingEmergencyCooldown = MockSiblingCooldown;
    type WeightInfo = ();
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}
