use crate as pallet_elections;
use frame_support::{
    derive_impl,
    traits::{ConstU32, ConstU64, ConstU8},
};
use frame_system::EnsureRoot;
use sp_runtime::{BuildStorage, DispatchResult};
use std::cell::RefCell;
use std::collections::BTreeSet;

type Block = frame_system::mocking::MockBlock<Test>;
type Balance = u64;

// ── Test-only cross-pallet mocks ────────────────────────────────────────────
//
// pallet-elections depends on cross-pallet traits normally backed by
// pallet-identity (CitizenChecker) and pallet-legislature (SeatLegislature) in the
// real runtime. Here we back them with simple thread-local state so each test can
// freely control which accounts are "active citizens" and observe exactly what
// SeatLegislature::replace_members was called with.

thread_local! {
    static ACTIVE_CITIZENS: RefCell<BTreeSet<u64>> = RefCell::new(BTreeSet::new());
    static SEAT_CALLS: RefCell<Vec<Vec<u64>>> = RefCell::new(Vec::new());
}

/// Marks `who` as an active (non-suspended) registered citizen, or removes them.
pub fn set_active_citizen(who: u64, active: bool) {
    ACTIVE_CITIZENS.with(|c| {
        if active {
            c.borrow_mut().insert(who);
        } else {
            c.borrow_mut().remove(&who);
        }
    });
}

/// Every `winners` vector `SeatLegislature::replace_members` has been called with, in call order.
pub fn seat_calls() -> Vec<Vec<u64>> {
    SEAT_CALLS.with(|s| s.borrow().clone())
}

pub struct TestCitizenChecker;
impl pallet_elections::CitizenChecker<u64> for TestCitizenChecker {
    fn is_active_citizen(who: &u64) -> bool {
        ACTIVE_CITIZENS.with(|c| c.borrow().contains(who))
    }
}

pub struct TestSeatLegislature;
impl pallet_elections::SeatLegislature<u64> for TestSeatLegislature {
    fn replace_members(winners: alloc::vec::Vec<u64>) -> DispatchResult {
        SEAT_CALLS.with(|s| s.borrow_mut().push(winners));
        Ok(())
    }
}

// ── Mock runtime constants ──────────────────────────────────────────────────
//
// Small, deterministic values chosen for fast tests — not real-world day/year counts.
pub const CANDIDATE_DEPOSIT: Balance = 100;
pub const MAX_COMMISSIONERS: u32 = 5;
pub const MAX_CANDIDATES_PER_ELECTION: u32 = 5;
pub const MAX_DELEGATES: u32 = 20;
pub const DEFAULT_LEGISLATURE_SEATS: u32 = 3;
pub const DEFAULT_ELECTION_CYCLE_BLOCKS: u32 = 20;
// Deliberately larger than the "5-50 block" rule of thumb: on_initialize computes the term
// warning offset as `(term_length / 100) * (100 - warning_pct)` (divide-first, see lib.rs), so
// any term_length < 100 truncates the offset to 0 and the warning fires immediately at term
// start. A term_length of 100 keeps the warning window test meaningful while still being fast
// (block numbers are just counters in tests, not wall-clock time).
pub const DEFAULT_MAX_BACKINGS_PER_CITIZEN: u32 = 6;
pub const DEFAULT_BACKING_THRESHOLD: u32 = 3;
pub const DEFAULT_BACKING_THRESHOLD_FLOOR: u32 = 1;
pub const DEFAULT_BACKING_THRESHOLD_CEILING: u32 = 10;
pub const DEFAULT_TERM_LENGTH_BLOCKS: u32 = 100;
pub const DEFAULT_MAX_CONSECUTIVE_TERMS: u32 = 2;
pub const DEFAULT_MANDATORY_BREAK_BLOCKS: u32 = 10;
pub const DEFAULT_WARNING_WINDOW_PCT: u8 = 20;

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
    pub type Elections = pallet_elections::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type Block = Block;
    type AccountData = pallet_balances::AccountData<Balance>;
}

#[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
impl pallet_balances::Config for Test {
    type AccountStore = System;
    type Balance = Balance;
}

impl pallet_elections::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type CandidateDeposit = ConstU64<CANDIDATE_DEPOSIT>;
    type MaxCommissioners = ConstU32<MAX_COMMISSIONERS>;
    type MaxCandidatesPerElection = ConstU32<MAX_CANDIDATES_PER_ELECTION>;
    type MaxDelegates = ConstU32<MAX_DELEGATES>;
    type Currency = Balances;
    type CitizenChecker = TestCitizenChecker;
    // Root is authorized; any signed origin is not — lets tests drive both the
    // authorized and unauthorized-origin paths for the governance/constitutional calls.
    type GovernanceOrigin = EnsureRoot<u64>;
    type ConstitutionalOrigin = EnsureRoot<u64>;
    type LegislatureSeating = TestSeatLegislature;
    type DefaultLegislatureSeats = ConstU32<DEFAULT_LEGISLATURE_SEATS>;
    type DefaultElectionCycleBlocks = ConstU32<DEFAULT_ELECTION_CYCLE_BLOCKS>;
    type DefaultMaxBackingsPerCitizen = ConstU32<DEFAULT_MAX_BACKINGS_PER_CITIZEN>;
    type DefaultBackingThreshold = ConstU32<DEFAULT_BACKING_THRESHOLD>;
    type DefaultBackingThresholdFloor = ConstU32<DEFAULT_BACKING_THRESHOLD_FLOOR>;
    type DefaultBackingThresholdCeiling = ConstU32<DEFAULT_BACKING_THRESHOLD_CEILING>;
    type DefaultTermLengthBlocks = ConstU32<DEFAULT_TERM_LENGTH_BLOCKS>;
    type DefaultMaxConsecutiveTerms = ConstU32<DEFAULT_MAX_CONSECUTIVE_TERMS>;
    type DefaultMandatoryBreakBlocks = ConstU32<DEFAULT_MANDATORY_BREAK_BLOCKS>;
    type DefaultWarningWindowPct = ConstU8<DEFAULT_WARNING_WINDOW_PCT>;
}

// Build genesis storage according to the mock runtime. Accounts 1..=10 start with a
// balance comfortably above `CANDIDATE_DEPOSIT` so deposit-reservation tests don't need
// per-test funding boilerplate; account 99 is left unfunded for the insufficient-balance case.
pub fn new_test_ext() -> sp_io::TestExternalities {
    let mut storage = frame_system::GenesisConfig::<Test>::default().build_storage().unwrap();
    pallet_balances::GenesisConfig::<Test> {
        balances: (1..=10u64).map(|who| (who, 10_000u64)).collect(),
        ..Default::default()
    }
    .assimilate_storage(&mut storage)
    .unwrap();
    storage.into()
}
