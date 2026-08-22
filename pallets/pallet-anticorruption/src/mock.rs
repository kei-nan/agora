use crate as pallet_anticorruption;
use frame_support::derive_impl;
use frame_system::EnsureRoot;
use sp_runtime::BuildStorage;

type Block = frame_system::mocking::MockBlock<Test>;

/// Test ZK proof verifier. Validity is controlled entirely by the proof bytes so tests
/// stay deterministic and don't need any shared/thread-local state: a non-empty proof
/// whose first byte is `1` is treated as valid, anything else (including an empty proof
/// or a first byte of `0`) is treated as invalid.
pub struct TestZkVerifier;

impl pallet_anticorruption::ZkProofVerifier for TestZkVerifier {
    fn verify(proof_bytes: &[u8], _public_inputs: &[[u8; 32]]) -> bool {
        matches!(proof_bytes.first(), Some(1))
    }
}

/// Byte marker used by `TestZkVerifier` to signal a proof that should pass verification.
pub const VALID_PROOF_MARKER: u8 = 1;
/// Byte marker used by `TestZkVerifier` to signal a proof that should fail verification.
pub const INVALID_PROOF_MARKER: u8 = 0;

pub const MAX_INVESTIGATORS: u32 = 4;
pub const RENEWAL_BLOCKS: u32 = 100;

frame_support::parameter_types! {
    pub const MaxInvestigators: u32 = MAX_INVESTIGATORS;
    pub const AssetDisclosureRenewalBlocks: u32 = RENEWAL_BLOCKS;
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
    pub type AntiCorruption = pallet_anticorruption::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type Block = Block;
}

impl pallet_anticorruption::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type ZkVerifier = TestZkVerifier;
    type MaxInvestigators = MaxInvestigators;
    type AssetDisclosureRenewalBlocks = AssetDisclosureRenewalBlocks;
    // `AsEnsureOriginWithArg` adapts the plain `EnsureRoot` origin (which only cares about
    // Root-ness) to the `EnsureOriginWithArg<_, [u8; 32]>` bound `AppointmentOrigin` now
    // requires — ignoring the call-hash argument entirely. Mirrors
    // `pallet_treasury_ledger`'s mock `LegislatureOrigin` wiring: this pallet's own tests
    // exercise this pallet's investigator-registry logic, not the call-hash-binding/
    // supermajority invariant of the real
    // `pallet_accountability_council::EnsureAccountabilityCouncilApproved` origin, which is
    // covered by that pallet's own test suite. Keeping this permissive (Root still succeeds)
    // lets the pre-existing `RuntimeOrigin::root()`-based tests keep working unchanged.
    type AppointmentOrigin = frame_support::traits::AsEnsureOriginWithArg<EnsureRoot<u64>>;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}
