use crate as pallet_elections;
use frame_support::{derive_impl, traits::{ConstU32, ConstU8}};
use frame_system::EnsureRoot;
use sp_runtime::{BuildStorage, DispatchResult};
use std::cell::RefCell;
use std::collections::BTreeSet;

type Block = frame_system::mocking::MockBlock<Test>;

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
    static CURRENT_DISCLOSURES: RefCell<BTreeSet<u64>> = RefCell::new(BTreeSet::new());
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

/// Marks `who` as having a current asset disclosure on file, or removes them (lapsed/never
/// filed). Defaults to false (not current) for any account never passed here, mirroring
/// `set_active_citizen`'s default -- tests that seat delegates must opt them in explicitly.
pub fn set_current_disclosure(who: u64, current: bool) {
    CURRENT_DISCLOSURES.with(|c| {
        if current {
            c.borrow_mut().insert(who);
        } else {
            c.borrow_mut().remove(&who);
        }
    });
}

pub struct TestCitizenChecker;
impl pallet_elections::CitizenChecker<u64> for TestCitizenChecker {
    fn is_active_citizen(who: &u64) -> bool {
        ACTIVE_CITIZENS.with(|c| c.borrow().contains(who))
    }
}

pub struct TestDisclosureChecker;
impl pallet_elections::DisclosureChecker<u64> for TestDisclosureChecker {
    fn has_current_disclosure(who: &u64) -> bool {
        CURRENT_DISCLOSURES.with(|c| c.borrow().contains(who))
    }
}

pub struct TestSeatLegislature;
impl pallet_elections::SeatLegislature<u64> for TestSeatLegislature {
    fn replace_members(winners: alloc::vec::Vec<u64>) -> DispatchResult {
        SEAT_CALLS.with(|s| s.borrow_mut().push(winners));
        Ok(())
    }
}

// ── Test-only ZK verifier mocks ─────────────────────────────────────────────
//
// Mirrors pallet-identity-zk's own `mock.rs` (`TestZkVerifier`/`TestAnchorVerifier`):
// deterministic, proof-bytes/public-inputs-driven doubles, not real cryptography -- real
// cryptographic correctness for these exact circuit shapes is already covered by
// `runtime/src/anchor_verifier.rs`'s and `runtime/src/backing_nullifier_verifier.rs`'s own
// real-bb-proof test suites. These mocks only need to let pallet-level tests drive both the
// accept and reject paths of *this pallet's* logic.

/// Byte marker `TestZkVerifier`/`TestBackingProofVerifier` treat as "this proof passes".
pub const VALID_PROOF_MARKER: u8 = 1;
/// Any other first byte (including this one) is treated as "this proof fails".
pub const INVALID_PROOF_MARKER: u8 = 0;

/// Outer-proof pairing check double: valid iff `proof_bytes[0] == VALID_PROOF_MARKER`.
pub struct TestZkVerifier;
impl pallet_elections::ZkProofVerifier for TestZkVerifier {
    fn verify(proof_bytes: &[u8], _public_inputs: &[[u8; 32]]) -> bool {
        matches!(proof_bytes.first(), Some(&VALID_PROOF_MARKER))
    }
}

/// `delegate-persona` commitment-check double: valid iff `outer_public_inputs` contains both
/// the claimed `delegate_persona_id` and the claimed `persona_account` bytes anywhere -- not
/// how the real `check_delegate_persona` computes a match (a single Poseidon2 recomputation),
/// but deterministic and lets tests drive both the accept path and a persona/account mismatch
/// rejection without depending on the crypto crate.
pub struct TestDelegatePersonaVerifier;
impl pallet_elections::DelegatePersonaVerifier for TestDelegatePersonaVerifier {
    fn check_delegate_persona(
        outer_public_inputs: &[[u8; 32]],
        delegate_persona_id: [u8; 32],
        persona_account: [u8; 32],
        _scheme_version: u32,
        _oprf_pk_hashes: [[u8; 32]; pallet_elections::NUM_COMMITTEES],
    ) -> bool {
        outer_public_inputs.contains(&delegate_persona_id)
            && outer_public_inputs.contains(&persona_account)
    }
}

/// Committee-key-approval double: approves everything by default so tests that don't care
/// about this check don't need to thread anything extra through. A slot's key hash with its
/// first byte set to `UNAPPROVED_COMMITTEE_KEY_MARKER` is treated as unapproved, for the one
/// test that exercises the rejection path.
pub const UNAPPROVED_COMMITTEE_KEY_MARKER: u8 = 0xFF;

pub struct TestCommitteeKeyChecker;
impl pallet_elections::CommitteeKeyChecker for TestCommitteeKeyChecker {
    fn are_committee_keys_approved(
        _scheme_version: u32,
        oprf_pk_hashes: &[[u8; 32]; pallet_elections::NUM_COMMITTEES],
    ) -> bool {
        oprf_pk_hashes.iter().all(|h| h[0] != UNAPPROVED_COMMITTEE_KEY_MARKER)
    }
}

/// `u64` `AccountId` -> 32 bytes: the value right-aligned into the low 8 bytes, zero elsewhere.
/// Not how the real runtime's `AccountId32` conversion works (that's a genuine byte-identity
/// mapping); just deterministic and injective enough to exercise this pallet's logic.
pub struct TestAccountIdToBytes;
impl pallet_elections::AccountIdToBytes<u64> for TestAccountIdToBytes {
    fn to_bytes(who: &u64) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        bytes[24..32].copy_from_slice(&who.to_be_bytes());
        bytes
    }
}

/// Backing-nullifier pairing-check double: same marker convention as `TestZkVerifier`.
pub struct TestBackingProofVerifier;
impl pallet_elections::BackingProofVerifier for TestBackingProofVerifier {
    fn verify(proof_bytes: &[u8], _public_inputs: &[[u8; 32]]) -> bool {
        matches!(proof_bytes.first(), Some(&VALID_PROOF_MARKER))
    }
}

thread_local! {
    static INVALID_BACKING_ROOTS: RefCell<BTreeSet<[u8; 32]>> = RefCell::new(BTreeSet::new());
}

/// Marks `root` as one `TestBackingRootChecker` should reject, for the one test that exercises
/// that rejection path. Every other root is accepted by default.
pub fn set_invalid_backing_root(root: [u8; 32]) {
    INVALID_BACKING_ROOTS.with(|r| { r.borrow_mut().insert(root); });
}

pub struct TestBackingRootChecker;
impl pallet_elections::BackingRootChecker for TestBackingRootChecker {
    fn is_valid_backing_commitment_root(root: [u8; 32]) -> bool {
        INVALID_BACKING_ROOTS.with(|r| !r.borrow().contains(&root))
    }
}

// ── Mock runtime constants ──────────────────────────────────────────────────
//
// Small, deterministic values chosen for fast tests — not real-world day/year counts.
pub const MAX_DELEGATES: u32 = 20;
// Comfortably above the delegate counts used by existing sweep-behavior tests (a handful of
// delegates each), so a single `on_initialize` call still sweeps all of them in those tests.
// `pagination_*` tests below deliberately register more than this to exercise the
// multi-block sweep behavior itself.
pub const MAX_DELEGATE_SWEEP_PER_BLOCK: u32 = 10;
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
    pub type Elections = pallet_elections::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type Block = Block;
}

impl pallet_elections::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type MaxDelegates = ConstU32<MAX_DELEGATES>;
    type MaxDelegateSweepPerBlock = ConstU32<MAX_DELEGATE_SWEEP_PER_BLOCK>;
    type CitizenChecker = TestCitizenChecker;
    // Root is authorized; any signed origin is not — lets tests drive both the
    // authorized and unauthorized-origin paths for the governance/constitutional calls.
    // `AsEnsureOriginWithArg` ignores the call-hash argument `GovernanceOrigin` now
    // requires -- this pallet's own tests exercise this pallet's logic, not the call-hash
    // binding invariant (covered by pallet-legislature's own test suite).
    type GovernanceOrigin = frame_support::traits::AsEnsureOriginWithArg<EnsureRoot<u64>>;
    type ConstitutionalOrigin = EnsureRoot<u64>;
    type LegislatureSeating = TestSeatLegislature;
    type DisclosureChecker = TestDisclosureChecker;
    type AccountIdToBytes = TestAccountIdToBytes;
    type ZkVerifier = TestZkVerifier;
    type DelegatePersonaVerifier = TestDelegatePersonaVerifier;
    type CommitteeKeyChecker = TestCommitteeKeyChecker;
    type BackingProofVerifier = TestBackingProofVerifier;
    type BackingRootChecker = TestBackingRootChecker;
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
    type WeightInfo = ();
    #[cfg(feature = "runtime-benchmarks")]
    type BenchmarkHelper = TestBenchmarkHelper;
}

#[cfg(feature = "runtime-benchmarks")]
pub struct TestBenchmarkHelper;
#[cfg(feature = "runtime-benchmarks")]
impl pallet_elections::BenchmarkHelper<u64> for TestBenchmarkHelper {
    fn make_active_citizen(who: &u64) {
        set_active_citizen(*who, true);
    }
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}
