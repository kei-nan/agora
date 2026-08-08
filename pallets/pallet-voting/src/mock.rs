use crate as pallet_voting;
use frame_support::{
    derive_impl,
    dispatch::DispatchResult,
    traits::{ConstU32, ConstU8},
};
use sp_runtime::BuildStorage;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

type Block = frame_system::mocking::MockBlock<Test>;

// ── Test-only cross-pallet mocks ────────────────────────────────────────────
//
// pallet-voting depends on several cross-pallet traits normally backed by
// pallet-identity / pallet-constitution / pallet-legislature in the real runtime
// (see runtime/src/configs/mod.rs). Here we back them with simple thread-local
// state so each test can freely control which accounts are "active citizens",
// which have a registered nullifier, and observe what LawEnactor was called with.

thread_local! {
    static ACTIVE_CITIZENS: RefCell<BTreeSet<u64>> = RefCell::new(BTreeSet::new());
    static TOTAL_CITIZENS: RefCell<u32> = RefCell::new(0);
    static NULLIFIERS: RefCell<BTreeMap<u64, [u8; 32]>> = RefCell::new(BTreeMap::new());
    static ENACTED_LAWS: RefCell<Vec<(pallet_voting::ReferendumTier, [u8; 32])>> =
        RefCell::new(Vec::new());
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

/// Sets the total registered citizen count used for the percentage-based delegation cap.
pub fn set_total_citizens(n: u32) {
    TOTAL_CITIZENS.with(|c| *c.borrow_mut() = n);
}

/// Deterministically derives and registers a nullifier for `who` so
/// `NullifierProvider::nullifier_of` returns `Some(_)` for them.
pub fn register_nullifier(who: u64) {
    let mut nullifier = [0u8; 32];
    nullifier[24..32].copy_from_slice(&who.to_be_bytes());
    NULLIFIERS.with(|n| {
        n.borrow_mut().insert(who, nullifier);
    });
}

/// Removes any registered nullifier for `who`.
#[allow(dead_code)]
pub fn clear_nullifier(who: u64) {
    NULLIFIERS.with(|n| {
        n.borrow_mut().remove(&who);
    });
}

/// Returns the nullifier registered for `who`, if any. Mirrors what
/// `TestNullifierProvider::nullifier_of` would return, for use in test assertions.
pub fn nullifier_of(who: u64) -> Option<[u8; 32]> {
    NULLIFIERS.with(|n| n.borrow().get(&who).copied())
}

/// Marks `who` as an active citizen with a registered nullifier, without touching the
/// total-citizen counter. Use `set_total_citizens` separately when a test cares about the
/// percentage-based delegation cap (leaving the total at 0 disables that check entirely,
/// which is useful when isolating the absolute `MaxDelegationsPerDelegate` cap instead).
pub fn activate_citizen(who: u64) {
    set_active_citizen(who, true);
    register_nullifier(who);
}

/// Convenience helper: marks `who` as an active citizen, registers their nullifier, and bumps
/// the total-citizen counter by one. Most tests just want "a citizen who can pass every gate".
#[allow(dead_code)]
pub fn register_citizen(who: u64) {
    activate_citizen(who);
    TOTAL_CITIZENS.with(|c| *c.borrow_mut() += 1);
}

/// Returns every `(tier, content_hash)` pair `TestLawEnactor::enact_law` has been called with,
/// in call order.
pub fn enacted_laws() -> Vec<(pallet_voting::ReferendumTier, [u8; 32])> {
    ENACTED_LAWS.with(|e| e.borrow().clone())
}

pub struct TestCitizenChecker;
impl pallet_voting::CitizenChecker<u64> for TestCitizenChecker {
    fn is_active_citizen(who: &u64) -> bool {
        ACTIVE_CITIZENS.with(|c| c.borrow().contains(who))
    }

    fn total_citizens() -> u32 {
        TOTAL_CITIZENS.with(|c| *c.borrow())
    }
}

pub struct TestNullifierProvider;
impl pallet_voting::NullifierProvider<u64> for TestNullifierProvider {
    fn nullifier_of(who: &u64) -> Option<[u8; 32]> {
        NULLIFIERS.with(|n| n.borrow().get(who).copied())
    }
}

pub struct TestLawEnactor;
impl pallet_voting::LawEnactor for TestLawEnactor {
    fn enact_law(tier: pallet_voting::ReferendumTier, content_hash: [u8; 32]) -> DispatchResult {
        ENACTED_LAWS.with(|e| e.borrow_mut().push((tier, content_hash)));
        Ok(())
    }
}

/// Test MACI tally verifier. Validity is controlled entirely by the proof bytes so tests stay
/// deterministic: a non-empty proof whose first byte is `1` verifies, anything else does not.
pub struct TestMACITallyVerifier;
impl pallet_voting::MACITallyVerifier for TestMACITallyVerifier {
    fn verify_tally(
        _proposal_id: u32,
        _yes_votes: u64,
        _no_votes: u64,
        _commitment_root: [u8; 32],
        proof_bytes: &[u8],
    ) -> bool {
        matches!(proof_bytes.first(), Some(1))
    }
}

/// Byte marker used by `TestMACITallyVerifier` to signal a proof that should pass verification.
pub const VALID_TALLY_PROOF: [u8; 1] = [1];
/// Byte marker used by `TestMACITallyVerifier` to signal a proof that should fail verification.
pub const INVALID_TALLY_PROOF: [u8; 1] = [0];

// ── Mock runtime constants ──────────────────────────────────────────────────
//
// Small, deterministic values chosen for fast tests — not real-world day counts.
pub const DELEGATION_CAP_PCT: u8 = 40;
pub const MAX_DELEGATIONS_PER_DELEGATE: u32 = 5;
pub const MAX_DELEGATION_DEPTH: u8 = 3;
pub const BUDGET_CATEGORY_COUNT: u32 = 4;
pub const MIN_DELEGATION_DURATION: u32 = 5;
pub const MAX_DELEGATION_DURATION: u32 = 100;
pub const MIN_PROPOSAL_DURATION: u32 = 5;
pub const MAX_PROPOSAL_DURATION: u32 = 100;
pub const REFERENDUM_DURATION: u32 = 20;
pub const PASSAGE_THRESHOLD: u8 = 50;
pub const CONSTITUTIONAL_PASSAGE_THRESHOLD: u8 = 67;
pub const FOUNDATIONAL_PASSAGE_THRESHOLD: u8 = 75;
pub const MIN_EPOCH_DURATION: u32 = 5;
pub const MAX_EPOCH_DURATION: u32 = 50;

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
    pub type Voting = pallet_voting::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type Block = Block;
}

impl pallet_voting::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type DelegationCap = ConstU8<DELEGATION_CAP_PCT>;
    type MaxDelegationsPerDelegate = ConstU32<MAX_DELEGATIONS_PER_DELEGATE>;
    type MaxDelegationDepth = ConstU8<MAX_DELEGATION_DEPTH>;
    type BudgetCategoryCount = ConstU32<BUDGET_CATEGORY_COUNT>;
    type MinDelegationDurationBlocks = ConstU32<MIN_DELEGATION_DURATION>;
    type MaxDelegationDurationBlocks = ConstU32<MAX_DELEGATION_DURATION>;
    type MinProposalDurationBlocks = ConstU32<MIN_PROPOSAL_DURATION>;
    type MaxProposalDurationBlocks = ConstU32<MAX_PROPOSAL_DURATION>;
    type CitizenChecker = TestCitizenChecker;
    type NullifierProvider = TestNullifierProvider;
    type ReferendumDurationBlocks = ConstU32<REFERENDUM_DURATION>;
    type PassageThreshold = ConstU8<PASSAGE_THRESHOLD>;
    type ConstitutionalPassageThreshold = ConstU8<CONSTITUTIONAL_PASSAGE_THRESHOLD>;
    type FoundationalPassageThreshold = ConstU8<FOUNDATIONAL_PASSAGE_THRESHOLD>;
    type LawEnactor = TestLawEnactor;
    type MACITallyVerifier = TestMACITallyVerifier;
    // `AsEnsureOriginWithArg` ignores the call-hash argument `LegislatureOrigin` now
    // requires -- this pallet's own tests exercise this pallet's logic, not the call-hash
    // binding invariant (covered by pallet-legislature's own test suite).
    type LegislatureOrigin =
        frame_support::traits::AsEnsureOriginWithArg<frame_system::EnsureRoot<u64>>;
    type MinEpochDurationBlocks = ConstU32<MIN_EPOCH_DURATION>;
    type MaxEpochDurationBlocks = ConstU32<MAX_EPOCH_DURATION>;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}
