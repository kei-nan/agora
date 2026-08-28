use crate as pallet_executive;
use frame_support::{derive_impl, traits::ConstU32};
use sp_runtime::BuildStorage;
use std::cell::RefCell;
use std::collections::BTreeSet;

type Block = frame_system::mocking::MockBlock<Test>;

/// Cooldown after an emergency ends before another can be declared, used across tests.
pub const EMERGENCY_COOLDOWN_BLOCKS: u32 = 20;

// ── Test-only cross-pallet mocks ────────────────────────────────────────────
//
// pallet-executive depends on cross-pallet traits normally backed by pallet-identity
// (CitizenChecker) and pallet-legislature (LegislatureMembership) in the real runtime. Here
// we back them with simple thread-local state so each test can freely control which
// accounts are "active citizens" / "legislature members".

thread_local! {
    // Tracked as *suspended* accounts (opt-in), not active ones — so an account nobody
    // has called `set_active_citizen(_, false)` on defaults to active. That default matters:
    // the daily vacancy sweep runs unconditionally from block 0 in every test, and without
    // this default every test that appoints a PM/minister and then calls `on_initialize`
    // would find them "not active" and silently vacate them.
    static SUSPENDED_CITIZENS: RefCell<BTreeSet<u64>> = RefCell::new(BTreeSet::new());
    static LEGISLATURE_MEMBERS: RefCell<BTreeSet<u64>> = RefCell::new(BTreeSet::new());
    // Accounts whose *current* suspension (if any) came from a jury-reviewed conviction, not
    // a bare unappealed AI ruling. Defaults to false: suspending an account via
    // `set_active_citizen(_, false)` alone models the AI-only path, which must NOT be enough
    // on its own to trigger the vacancy sweep — see `is_suspended_by_jury_reviewed_conviction`.
    static JURY_REVIEWED_SUSPENSIONS: RefCell<BTreeSet<u64>> = RefCell::new(BTreeSet::new());
    static ACCOUNTABILITY_COUNCIL_MEMBERS: RefCell<BTreeSet<u64>> = RefCell::new(BTreeSet::new());
}

/// Marks `who` as an active (non-suspended) registered citizen, or suspended (modeling the
/// unappealed-AI-ruling path unless paired with `set_jury_reviewed_suspension`). Defaults to
/// active for any account this is never called for.
pub fn set_active_citizen(who: u64, active: bool) {
    SUSPENDED_CITIZENS.with(|c| {
        if active {
            c.borrow_mut().remove(&who);
        } else {
            c.borrow_mut().insert(who);
        }
    });
}

/// Marks whether `who`'s current suspension (set via `set_active_citizen(_, false)`) came
/// from a jury-reviewed conviction. Meaningless if `who` isn't currently suspended.
pub fn set_jury_reviewed_suspension(who: u64, jury_reviewed: bool) {
    JURY_REVIEWED_SUSPENSIONS.with(|j| {
        if jury_reviewed {
            j.borrow_mut().insert(who);
        } else {
            j.borrow_mut().remove(&who);
        }
    });
}

/// Marks `who` as a current legislature member, or removes them.
pub fn set_legislature_member(who: u64, member: bool) {
    LEGISLATURE_MEMBERS.with(|m| {
        if member {
            m.borrow_mut().insert(who);
        } else {
            m.borrow_mut().remove(&who);
        }
    });
}

/// Marks `who` as a current Accountability Council member, or removes them. Defaults to false
/// (not a member) for any account this is never called for.
pub fn set_accountability_council_member(who: u64, member: bool) {
    ACCOUNTABILITY_COUNCIL_MEMBERS.with(|c| {
        if member {
            c.borrow_mut().insert(who);
        } else {
            c.borrow_mut().remove(&who);
        }
    });
}

pub struct TestAccountabilityCouncilChecker;
impl pallet_executive::AccountabilityCouncilChecker<u64> for TestAccountabilityCouncilChecker {
    fn is_current_member(who: &u64) -> bool {
        ACCOUNTABILITY_COUNCIL_MEMBERS.with(|c| c.borrow().contains(who))
    }
}

pub struct TestCitizenChecker;
impl pallet_executive::CitizenChecker<u64> for TestCitizenChecker {
    fn is_active_citizen(who: &u64) -> bool {
        !SUSPENDED_CITIZENS.with(|c| c.borrow().contains(who))
    }
    fn is_suspended_by_jury_reviewed_conviction(who: &u64) -> bool {
        SUSPENDED_CITIZENS.with(|c| c.borrow().contains(who))
            && JURY_REVIEWED_SUSPENSIONS.with(|j| j.borrow().contains(who))
    }
}

pub struct TestLegislatureMembership;
impl pallet_executive::LegislatureMembership<u64> for TestLegislatureMembership {
    fn is_member(who: &u64) -> bool {
        LEGISLATURE_MEMBERS.with(|m| m.borrow().contains(who))
    }
}

// ── Mock cross-pallet sibling cooldown (stands in for pallet-emergency-council) ─
//
// In the real runtime, pallet-executive depends on
// `pallet_executive::SiblingEmergencyCooldown` being backed by pallet-emergency-council's own
// `CooldownUntil` (implemented on `Runtime`, see `runtime/src/configs/mod.rs`). Here it's
// backed by simple thread-local state, same idiom as the mocks above. Defaults (cooldown-until
// 0, no notification recorded) make this behave exactly like the no-op `()` impl unless a test
// explicitly calls `set_sibling_cooldown_until`, so ordinary tests don't need to know
// pallet-emergency-council exists.
thread_local! {
    static SIBLING_COOLDOWN_UNTIL: RefCell<u64> = RefCell::new(0);
    static SIBLING_NOTIFIED_AT: RefCell<Option<u64>> = RefCell::new(None);
}

/// Simulates pallet-emergency-council currently being in cooldown until block `until`.
pub fn set_sibling_cooldown_until(until: u64) {
    SIBLING_COOLDOWN_UNTIL.with(|c| *c.borrow_mut() = until);
}

/// The block number this pallet last told its mock sibling to start its cooldown at, if this
/// pallet's own cooldown has ever ended since the mock was last reset.
pub fn sibling_notified_at() -> Option<u64> {
    SIBLING_NOTIFIED_AT.with(|c| *c.borrow())
}

pub struct MockSiblingCooldown;
impl pallet_executive::SiblingEmergencyCooldown<u64> for MockSiblingCooldown {
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
    type EmergencyCooldownBlocks = ConstU32<EMERGENCY_COOLDOWN_BLOCKS>;
    // See the mock's doc comment above — behaves as a no-op by default, controllable per test
    // via `set_sibling_cooldown_until`/`sibling_notified_at`.
    type SiblingEmergencyCooldown = MockSiblingCooldown;
    type SupermajorityNumerator = ConstU32<2>;
    type SupermajorityDenominator = ConstU32<3>;
    type CitizenChecker = TestCitizenChecker;
    type LegislatureMembership = TestLegislatureMembership;
    type AccountabilityCouncilChecker = TestAccountabilityCouncilChecker;
    type PmNominationWindowBlocks = ConstU32<5>;
    type PmVotingWindowBlocks = ConstU32<5>;
    type MaxPmCandidates = ConstU32<8>;
    type PmOccupancyWindowBlocks = ConstU32<50>;
    type MaxPmOccupancyBlocks = ConstU32<30>;
    type MaxPmTenureHistory = ConstU32<16>;
    type VacancySweepIntervalBlocks = ConstU32<20>;
    type WeightInfo = ();
    #[cfg(feature = "runtime-benchmarks")]
    type BenchmarkHelper = TestBenchmarkHelper;
}

#[cfg(feature = "runtime-benchmarks")]
pub struct TestBenchmarkHelper;
#[cfg(feature = "runtime-benchmarks")]
impl pallet_executive::BenchmarkHelper<u64> for TestBenchmarkHelper {
    fn make_active_citizen(who: &u64) {
        set_active_citizen(*who, true);
    }
    fn make_legislature_member(who: &u64) {
        set_legislature_member(*who, true);
    }
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}
