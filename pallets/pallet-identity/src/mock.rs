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
        _backing_commitment: [u8; 32],
    ) -> bool {
        outer_public_inputs.contains(&anchor)
    }

    fn verify_reverification(
        outer_public_inputs: &[[u8; 32]],
        anchor: [u8; 32],
        _scheme_version: u32,
        _oprf_pk_hashes: [[u8; 32]; 5],
        _backing_commitment: [u8; 32],
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

    // Wired only so `EmergencyRotationOrigin` below can be tested against the real
    // `EnsureActiveEmergency` origin (see its `try_origin` in
    // `pallet_emergency_council::pallet`), not a hand-rolled stand-in — this mirrors how the
    // runtime actually wires `EmergencyRotationOrigin` in `runtime/src/configs/mod.rs`.
    #[runtime::pallet_index(2)]
    pub type EmergencyCouncil = pallet_emergency_council::Pallet<Test>;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type Block = Block;
}

impl pallet_emergency_council::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type MaxEmergencyBlocks = frame_support::traits::ConstU32<100>;
    // Pre-existing gap fixed incidentally here: this mock didn't implement
    // `EmergencyCooldownBlocks` after it was added to `pallet_emergency_council::Config`
    // (see that pallet's own mock for the equivalent value), which left this crate's `--tests`
    // build broken regardless of this file's other contents.
    type EmergencyCooldownBlocks = frame_support::traits::ConstU32<50>;
    type MaxCouncilSize = frame_support::traits::ConstU32<10>;
    type SupermajorityNumerator = frame_support::traits::ConstU32<2>;
    type SupermajorityDenominator = frame_support::traits::ConstU32<3>;
    type WeightInfo = ();
}

impl pallet_identity_zk::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type ZkVerifier = TestZkVerifier;
    // Root is authorized; any signed origin is not — lets tests drive both the
    // authorized and unauthorized-origin paths for suspend/restore and the admin calls.
    // `AsEnsureOriginWithArg` ignores the call-hash argument `SuspensionOrigin`/`AdminOrigin`
    // now require -- this pallet's own tests exercise this pallet's logic, not the call-hash
    // binding invariant (covered by pallet-courts'/pallet-legislature's own test suites).
    type SuspensionOrigin = frame_support::traits::AsEnsureOriginWithArg<frame_system::EnsureRoot<u64>>;
    type AdminOrigin = frame_support::traits::AsEnsureOriginWithArg<frame_system::EnsureRoot<u64>>;
    type AnchorVerifier = TestAnchorVerifier;
    // Short period so tests can cross a reverification deadline without huge block numbers.
    type ReverificationPeriod = frame_support::traits::ConstU32<10>;
    // Wired to the real `pallet_emergency_council::EnsureActiveEmergency`, mirroring the
    // runtime's actual wiring (see `runtime/src/configs/mod.rs`), so this pallet's own tests
    // can prove `emergency_rotate_oprf_scheme` genuinely requires an active,
    // council-declared emergency rather than a bare root call. Unlike SuspensionOrigin/
    // AdminOrigin above (which stay `EnsureRoot` because this pallet's tests aren't meant to
    // re-exercise pallet-legislature's own call-hash-binding invariant), the entire point of
    // this Config field's test coverage is the cross-pallet emergency-gating behavior itself.
    type EmergencyRotationOrigin = pallet_emergency_council::EnsureActiveEmergency<Test>;
    type Now = TestNow;
    // 1 hour — generous enough that every fixture using the default fresh `current_date`
    // (see `tests.rs::public_inputs`) stays comfortably inside the window, while still
    // leaving room for a dedicated staleness test to use an obviously-expired one.
    type MaxAnchorProofAge = frame_support::traits::ConstU64<3600>;
    // 5 minutes — generous enough that every fixture using the default fresh `current_date`
    // stays inside it, while still leaving room for a dedicated future-dated test to use an
    // obviously-out-of-tolerance `current_date`.
    type MaxAnchorProofClockSkew = frame_support::traits::ConstU64<300>;
    // Small on purpose: big enough for tests to exercise a full roster plus one rejected
    // over-capacity add, without needing a large fixture.
    type MaxCommitteeSize = frame_support::traits::ConstU32<3>;
    // Short window (mirrors ReverificationPeriod's convention above) so tests can cross an
    // OPRF query's expiry deadline without huge block numbers.
    type OprfQuerySlaBlocks = frame_support::traits::ConstU32<10>;
    // Strictly less than MaxCommitteeSize (3) so tests can distinguish "set locked at
    // threshold" from "roster physically full" as two different conditions.
    type OprfThreshold = frame_support::traits::ConstU32<2>;
    // Small on purpose (mirrors MaxCommitteeSize's convention above): big enough for tests to
    // submit a handful of queries and exercise the over-cap rejection without a large fixture.
    type MaxPendingOprfQueriesPerCitizen = frame_support::traits::ConstU32<3>;
    // Short cooldown (mirrors ReverificationPeriod/OprfQuerySlaBlocks' convention above) so
    // tests can cross it without huge block numbers, while still being nonzero so a
    // back-to-back double-recovery test has something real to assert against.
    type MinBlocksBetweenRecoveries = frame_support::traits::ConstU32<5>;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}
