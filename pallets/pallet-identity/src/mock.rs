use crate as pallet_identity_zk;
use frame_support::derive_impl;
use sp_runtime::BuildStorage;

type Block = frame_system::mocking::MockBlock<Test>;

/// Test ZK proof verifier. Validity is controlled entirely by the proof bytes so tests
/// stay deterministic and don't need any shared/thread-local state: a non-empty proof
/// whose first byte is `1` is treated as valid, anything else (including an empty proof
/// or a first byte of `0`) is treated as invalid.
pub struct TestZkVerifier;

impl pallet_identity_zk::ZkProofVerifier for TestZkVerifier {
    fn verify(proof_bytes: &[u8], _public_inputs: &[[u8; 32]]) -> bool {
        matches!(proof_bytes.first(), Some(1))
    }
}

/// Byte marker used by `TestZkVerifier` to signal a proof that should pass verification.
pub const VALID_PROOF_MARKER: u8 = 1;
/// Byte marker used by `TestZkVerifier` to signal a proof that should fail verification.
pub const INVALID_PROOF_MARKER: u8 = 0;

/// Test anchor-proof verifier. None of the three methods take proof bytes any more (HANDOFF
/// log #75/#76 — the disclosure/migrate-disclosure subproof rides inside the already-verified
/// outer proof, so there is no separate anchor SNARK); this mock instead treats a proof as
/// valid whenever `outer_public_inputs` contains a `param_commitments[i]` equal to the
/// claimed anchor(s). That is not how the real `Poseidon2AnchorVerifier` computes a match (see
/// `runtime/src/anchor_verifier.rs`), but it is deterministic and lets pallet-level tests
/// drive both the accept and reject paths without depending on the crypto crate.
pub struct TestAnchorVerifier;

impl pallet_identity_zk::AnchorProofVerifier for TestAnchorVerifier {
    fn verify_registration_anchor(
        outer_public_inputs: &[[u8; 32]],
        anchor: [u8; 32],
        _scheme_version: u32,
        _oprf_pk_hashes: [[u8; 32]; 5],
    ) -> bool {
        outer_public_inputs.contains(&anchor)
    }

    fn verify_reverification(
        outer_public_inputs: &[[u8; 32]],
        anchor: [u8; 32],
        _scheme_version: u32,
        _oprf_pk_hashes: [[u8; 32]; 5],
    ) -> bool {
        outer_public_inputs.contains(&anchor)
    }

    fn verify_migration(
        outer_public_inputs: &[[u8; 32]],
        old_anchor: [u8; 32],
        new_anchor: [u8; 32],
        _old_scheme_version: u32,
        _new_scheme_version: u32,
        _old_oprf_pk_hashes: [[u8; 32]; 5],
        _new_oprf_pk_hashes: [[u8; 32]; 5],
    ) -> bool {
        outer_public_inputs.contains(&old_anchor) && outer_public_inputs.contains(&new_anchor)
    }
}

/// Fixed test clock for `pallet_identity_zk::Config::Now` — always reports
/// `TEST_NOW_UNIX_SECS`, so freshness checks in tests are driven entirely by the
/// `current_date` each test puts in its `public_inputs` fixture rather than by wall-clock
/// time.
pub const TEST_NOW_UNIX_SECS: u64 = 1_000_000;

pub struct TestNow;

impl frame_support::traits::UnixTime for TestNow {
    fn now() -> core::time::Duration {
        core::time::Duration::from_secs(TEST_NOW_UNIX_SECS)
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
    pub type Identity = pallet_identity_zk::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type Block = Block;
}

impl pallet_identity_zk::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type ZkVerifier = TestZkVerifier;
    // Root is authorized; any signed origin is not — lets tests drive both the
    // authorized and unauthorized-origin paths for suspend/restore and the admin calls.
    type SuspensionOrigin = frame_system::EnsureRoot<u64>;
    type AdminOrigin = frame_system::EnsureRoot<u64>;
    type AnchorVerifier = TestAnchorVerifier;
    // Short period so tests can cross a reverification deadline without huge block numbers.
    type ReverificationPeriod = frame_support::traits::ConstU32<10>;
    // Same convention as SuspensionOrigin/AdminOrigin above.
    type EmergencyRotationOrigin = frame_system::EnsureRoot<u64>;
    type Now = TestNow;
    // 1 hour — generous enough that every fixture using the default fresh `current_date`
    // (see `tests.rs::public_inputs`) stays comfortably inside the window, while still
    // leaving room for a dedicated staleness test to use an obviously-expired one.
    type MaxAnchorProofAge = frame_support::traits::ConstU64<3600>;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}
