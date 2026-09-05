use crate as pallet_constitution;
use crate::{
	AutoChallengeHook, CitizenChecker, FreshLegislatureChecker, PetitionApprover, TierConflictHook,
};
use frame_support::{derive_impl, traits::ConstU32, BoundedVec};
use frame_system::EnsureRoot;
use sp_runtime::{BuildStorage, DispatchResult};
use std::cell::RefCell;

type Block = frame_system::mocking::MockBlock<Test>;

// ── Test-controllable mocks for cross-pallet Config traits ─────────────────────
//
// These use thread-locals rather than pallet storage because the traits they mock
// (CitizenChecker, FreshLegislatureChecker, AutoChallengeHook, PetitionApprover) are
// implemented outside pallet-constitution in production (by pallet-identity,
// pallet-elections, pallet-courts, pallet-voting respectively) and are here modeled as
// plain Rust state the tests can set up/inspect directly.

thread_local! {
	static ACTIVE_CITIZENS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
	static FRESH_LEGISLATURE: RefCell<bool> = const { RefCell::new(false) };
	static AUTO_CHALLENGES: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
	static AUTO_CHALLENGE_SHOULD_FAIL: RefCell<bool> = const { RefCell::new(false) };
	static REFERENDA_CREATED: RefCell<Vec<(u32, [u8; 32])>> = const { RefCell::new(Vec::new()) };
	/// (filer, law_id) pairs `TestTierConflictHook::file_tier_conflict_case` has been called
	/// with, in call order. The zk_proof/public_inputs arguments aren't recorded — this mock
	/// only needs to prove `challenge_law_tier` reached the hook with the right (filer, law_id),
	/// not re-verify pallet-courts' own ZK logic, which is covered by that pallet's own tests.
	static TIER_CONFLICT_CHALLENGES: RefCell<Vec<(u64, u32)>> = const { RefCell::new(Vec::new()) };
	static TIER_CONFLICT_SHOULD_FAIL: RefCell<bool> = const { RefCell::new(false) };
}

/// Test helper: set the set of accounts considered active citizens by `TestCitizenChecker`.
pub fn set_active_citizens(citizens: Vec<u64>) {
	ACTIVE_CITIZENS.with(|c| *c.borrow_mut() = citizens);
}

/// Test helper: control whether `TestFreshLegislatureChecker::has_election_occurred_since`
/// returns true.
pub fn set_fresh_legislature(fresh: bool) {
	FRESH_LEGISLATURE.with(|f| *f.borrow_mut() = fresh);
}

/// Test helper: law_ids that `TestAutoChallengeHook::auto_challenge_law` has been called with,
/// in call order.
pub fn auto_challenges() -> Vec<u32> {
	AUTO_CHALLENGES.with(|v| v.borrow().clone())
}

/// Test helper: clears recorded auto-challenge calls (useful between sub-cases in one test).
pub fn clear_auto_challenges() {
	AUTO_CHALLENGES.with(|v| v.borrow_mut().clear());
}

/// Test helper: makes `TestAutoChallengeHook::auto_challenge_law` return `Err(..)` instead of
/// recording the call, so tests can exercise the `AutoChallengeFilingFailed` event path.
pub fn set_auto_challenge_should_fail(should_fail: bool) {
	AUTO_CHALLENGE_SHOULD_FAIL.with(|f| *f.borrow_mut() = should_fail);
}

/// Test helper: (petition_id, topic_hash) pairs that `TestPetitionApprover::create_referendum`
/// has been called with, in call order.
pub fn referenda_created() -> Vec<(u32, [u8; 32])> {
	REFERENDA_CREATED.with(|v| v.borrow().clone())
}

/// Test helper: (filer, law_id) pairs `TestTierConflictHook` has recorded, in call order.
pub fn tier_conflict_challenges() -> Vec<(u64, u32)> {
	TIER_CONFLICT_CHALLENGES.with(|v| v.borrow().clone())
}

/// Test helper: makes `TestTierConflictHook::file_tier_conflict_case` return `Err(..)` instead
/// of recording the call — mirrors `set_auto_challenge_should_fail`.
pub fn set_tier_conflict_should_fail(should_fail: bool) {
	TIER_CONFLICT_SHOULD_FAIL.with(|f| *f.borrow_mut() = should_fail);
}

pub struct TestCitizenChecker;
impl CitizenChecker<u64> for TestCitizenChecker {
	fn is_active_citizen(who: &u64) -> bool {
		ACTIVE_CITIZENS.with(|c| c.borrow().contains(who))
	}
}

pub struct TestFreshLegislatureChecker;
impl FreshLegislatureChecker<u64> for TestFreshLegislatureChecker {
	fn has_election_occurred_since(_proposed_at: u64) -> bool {
		FRESH_LEGISLATURE.with(|f| *f.borrow())
	}
}

pub struct TestAutoChallengeHook;
impl AutoChallengeHook for TestAutoChallengeHook {
	fn auto_challenge_law(law_id: u32) -> DispatchResult {
		if AUTO_CHALLENGE_SHOULD_FAIL.with(|f| *f.borrow()) {
			return Err(sp_runtime::DispatchError::Other("auto_challenge_law failed (test)"));
		}
		AUTO_CHALLENGES.with(|v| v.borrow_mut().push(law_id));
		Ok(())
	}
}

pub struct TestTierConflictHook;
impl TierConflictHook<u64> for TestTierConflictHook {
	fn file_tier_conflict_case(
		filer: u64,
		law_id: u32,
		_zk_proof: BoundedVec<u8, ConstU32<4096>>,
		_public_inputs: BoundedVec<[u8; 32], ConstU32<16>>,
	) -> DispatchResult {
		if TIER_CONFLICT_SHOULD_FAIL.with(|f| *f.borrow()) {
			return Err(sp_runtime::DispatchError::Other("file_tier_conflict_case failed (test)"));
		}
		TIER_CONFLICT_CHALLENGES.with(|v| v.borrow_mut().push((filer, law_id)));
		Ok(())
	}
}

pub struct TestPetitionApprover;
impl PetitionApprover for TestPetitionApprover {
	fn create_referendum(petition_id: u32, topic_hash: [u8; 32]) -> DispatchResult {
		REFERENDA_CREATED.with(|v| v.borrow_mut().push((petition_id, topic_hash)));
		Ok(())
	}
}

#[cfg(feature = "runtime-benchmarks")]
pub struct TestBenchmarkHelper;
#[cfg(feature = "runtime-benchmarks")]
impl pallet_constitution::BenchmarkHelper<u64> for TestBenchmarkHelper {
	fn make_active_citizen(who: &u64) {
		ACTIVE_CITIZENS.with(|c| {
			let mut list = c.borrow_mut();
			if !list.contains(who) {
				list.push(*who);
			}
		});
	}
	fn make_legislature_fresh() {
		set_fresh_legislature(true);
	}
}

// ── Config constants (small values for fast, deterministic tests) ──────────────

pub const ORDINARY_DELIBERATION_BLOCKS: u32 = 5;
pub const PROVISIONING_PERIOD_BLOCKS: u32 = 10;
pub const CONFIRMATION_PERIOD_BLOCKS: u32 = 6;
pub const PETITION_THRESHOLD: u32 = 3;

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
	pub type Constitution = pallet_constitution::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = Block;
}

impl pallet_constitution::Config for Test {
	type RuntimeEvent = RuntimeEvent;

	// Root is authorized; any signed origin is not — lets tests drive both the
	// authorized and unauthorized-origin paths for every privileged call.
	// `AsEnsureOriginWithArg` adapts the plain `EnsureRoot` origin (which only cares
	// about Root-ness) to the `EnsureOriginWithArg<_, [u8; 32]>` bound `LegislatureOrigin`
	// now requires, ignoring the call-hash argument. That's intentional: this pallet's own
	// unit tests exercise this pallet's logic, not the call-hash binding invariant, which
	// is covered by pallet-legislature's own test suite (the real
	// `EnsureLegislatureMotion` origin wired in the runtime enforces it for real).
	type LegislatureOrigin = frame_support::traits::AsEnsureOriginWithArg<EnsureRoot<u64>>;
	// Same `AsEnsureOriginWithArg` adaptation as `LegislatureOrigin`/`CourtOrigin` below, now
	// that `RevocationOrigin` requires `EnsureOriginWithArg<_, u8>` (the stage-dependent
	// threshold argument is exercised by this pallet's own `revocation_threshold_for_stage`
	// unit tests, not by this mock, which stays Root-authorized regardless of the argument).
	type RevocationOrigin = frame_support::traits::AsEnsureOriginWithArg<EnsureRoot<u64>>;
	// Same `AsEnsureOriginWithArg` adaptation as `LegislatureOrigin` above, now that
	// `CourtOrigin` requires `EnsureOriginWithArg<_, [u8; 32]>` too (the call-hash binding
	// itself is exercised by pallet-courts' own `EnsureOracleCouncilApproved` test suite).
	type CourtOrigin = frame_support::traits::AsEnsureOriginWithArg<EnsureRoot<u64>>;

	// Tier-aware legislature passage thresholds (see `required_threshold` in lib.rs). Values
	// mirror the runtime's real wiring (51/67/75) even though this mock's `LegislatureOrigin`
	// ignores the threshold argument (Root-authorized regardless) — pallet-legislature's own
	// test suite is what exercises the actual threshold-checking logic end to end.
	type OrdinaryPassageThreshold = frame_support::traits::ConstU8<51>;
	type ConstitutionalPassageThreshold = frame_support::traits::ConstU8<67>;
	type FoundationalPassageThreshold = frame_support::traits::ConstU8<75>;

	type OrdinaryAmendmentDeliberationBlocks = frame_support::traits::ConstU32<ORDINARY_DELIBERATION_BLOCKS>;
	type ProvisioningPeriodBlocks = frame_support::traits::ConstU32<PROVISIONING_PERIOD_BLOCKS>;
	type ConfirmationPeriodBlocks = frame_support::traits::ConstU32<CONFIRMATION_PERIOD_BLOCKS>;
	type FreshLegislatureChecker = TestFreshLegislatureChecker;

	type PetitionThreshold = frame_support::traits::ConstU32<PETITION_THRESHOLD>;
	type PetitionApprover = TestPetitionApprover;
	type CitizenChecker = TestCitizenChecker;

	type AutoChallengeHook = TestAutoChallengeHook;
	type TierConflictHook = TestTierConflictHook;
	type WeightInfo = ();
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper = TestBenchmarkHelper;
}

// Build genesis storage according to the mock runtime, resetting all thread-local
// test-mock state so tests don't leak state into one another.
pub fn new_test_ext() -> sp_io::TestExternalities {
	set_active_citizens(Vec::new());
	set_fresh_legislature(false);
	clear_auto_challenges();
	set_auto_challenge_should_fail(false);
	REFERENDA_CREATED.with(|v| v.borrow_mut().clear());
	TIER_CONFLICT_CHALLENGES.with(|v| v.borrow_mut().clear());
	set_tier_conflict_should_fail(false);
	frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}
