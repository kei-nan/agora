// This is free and unencumbered software released into the public domain.
//
// Anyone is free to copy, modify, publish, use, compile, sell, or
// distribute this software, either in source code form or as a compiled
// binary, for any purpose, commercial or non-commercial, and by any
// means.
//
// In jurisdictions that recognize copyright laws, the author or authors
// of this software dedicate any and all copyright interest in the
// software to the public domain. We make this dedication for the benefit
// of the public at large and to the detriment of our heirs and
// successors. We intend this dedication to be an overt act of
// relinquishment in perpetuity of all present and future rights to this
// software under copyright law.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
// EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
// MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
// IN NO EVENT SHALL THE AUTHORS BE LIABLE FOR ANY CLAIM, DAMAGES OR
// OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE,
// ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
// OTHER DEALINGS IN THE SOFTWARE.
//
// For more information, please refer to <http://unlicense.org>

// Substrate and Polkadot dependencies
use frame_support::{
	derive_impl, parameter_types,
	traits::{ConstBool, ConstU128, ConstU32, ConstU64, ConstU8, Get, VariantCountOf},
	weights::{
		constants::{RocksDbWeight, WEIGHT_REF_TIME_PER_SECOND},
		IdentityFee, Weight,
	},
};
use frame_system::{limits::{BlockLength, BlockWeights}, EnsureRoot};
use pallet_transaction_payment::{ConstFeeMultiplier, FungibleAdapter, Multiplier};
use sp_consensus_aura::sr25519::AuthorityId as AuraId;
use sp_runtime::{traits::One, AccountId32, Perbill};
use sp_version::RuntimeVersion;

// Local module imports
use super::{
	AccountId, AccountabilityCouncil, Aura, Balance, Balances, Block, BlockNumber, Cabinet, Hash,
	Legislature, Nonce, PalletAntiCorruption, PalletInfo, Runtime, RuntimeCall, RuntimeEvent,
	RuntimeFreezeReason, RuntimeHoldReason, RuntimeOrigin, RuntimeTask, System, Timestamp, DAYS,
	EXISTENTIAL_DEPOSIT, MINUTES, SLOT_DURATION, VERSION,
};

const NORMAL_DISPATCH_RATIO: Perbill = Perbill::from_percent(75);

/// Seconds per day — `pallet_identity_zk::Config::MaxAnchorProofAge` is denominated in
/// (wall-clock) seconds, unlike `DAYS` above which counts blocks.
const DAYS_IN_SECONDS: u64 = 24 * 60 * 60;

parameter_types! {
	pub const BlockHashCount: BlockNumber = 2400;
	pub const Version: RuntimeVersion = VERSION;

	/// We allow for 2 seconds of compute with this chain's 12 second average block time
	/// (`MILLI_SECS_PER_BLOCK` in `runtime/src/lib.rs`).
	pub RuntimeBlockWeights: BlockWeights = BlockWeights::with_sensible_defaults(
		Weight::from_parts(2u64 * WEIGHT_REF_TIME_PER_SECOND, u64::MAX),
		NORMAL_DISPATCH_RATIO,
	);
	pub RuntimeBlockLength: BlockLength = BlockLength::max_with_normal_ratio(5 * 1024 * 1024, NORMAL_DISPATCH_RATIO);
	pub const SS58Prefix: u8 = 42;
}

/// The default types are being injected by [`derive_impl`](`frame_support::derive_impl`) from
/// [`SoloChainDefaultConfig`](`struct@frame_system::config_preludes::SolochainDefaultConfig`),
/// but overridden as needed.
#[derive_impl(frame_system::config_preludes::SolochainDefaultConfig)]
impl frame_system::Config for Runtime {
	/// The block type for the runtime.
	type Block = Block;
	/// Block & extrinsics weights: base values and limits.
	type BlockWeights = RuntimeBlockWeights;
	/// The maximum length of a block (in bytes).
	type BlockLength = RuntimeBlockLength;
	/// The identifier used to distinguish between accounts.
	type AccountId = AccountId;
	/// The type for storing how many extrinsics an account has signed.
	type Nonce = Nonce;
	/// The type for hashing blocks and tries.
	type Hash = Hash;
	/// Maximum number of block number to block hash mappings to keep (oldest pruned first).
	type BlockHashCount = BlockHashCount;
	/// The weight of database operations that the runtime can invoke.
	type DbWeight = RocksDbWeight;
	/// Version of the runtime.
	type Version = Version;
	/// The data to be stored in an account.
	type AccountData = pallet_balances::AccountData<Balance>;
	/// This is used as an identifier of the chain. 42 is the generic substrate prefix.
	type SS58Prefix = SS58Prefix;
	type MaxConsumers = frame_support::traits::ConstU32<16>;
}

impl pallet_aura::Config for Runtime {
	type AuthorityId = AuraId;
	type DisabledValidators = ();
	type MaxAuthorities = ConstU32<32>;
	type AllowMultipleBlocksPerSlot = ConstBool<false>;
	type SlotDuration = pallet_aura::MinimumPeriodTimesTwo<Runtime>;
}

impl pallet_grandpa::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;

	type WeightInfo = ();
	type MaxAuthorities = ConstU32<32>;
	type MaxNominators = ConstU32<0>;
	type MaxSetIdSessionEntries = ConstU64<0>;

	type KeyOwnerProof = sp_core::Void;
	type EquivocationReportSystem = ();
}

impl pallet_timestamp::Config for Runtime {
	/// A timestamp: milliseconds since the unix epoch.
	type Moment = u64;
	type OnTimestampSet = Aura;
	type MinimumPeriod = ConstU64<{ SLOT_DURATION / 2 }>;
	type WeightInfo = ();
}

impl pallet_balances::Config for Runtime {
	type MaxLocks = ConstU32<50>;
	type MaxReserves = ();
	type ReserveIdentifier = [u8; 8];
	/// The type for recording an account's balance.
	type Balance = Balance;
	/// The ubiquitous event type.
	type RuntimeEvent = RuntimeEvent;
	type DustRemoval = ();
	type ExistentialDeposit = ConstU128<EXISTENTIAL_DEPOSIT>;
	type AccountStore = System;
	type WeightInfo = pallet_balances::weights::SubstrateWeight<Runtime>;
	type FreezeIdentifier = RuntimeFreezeReason;
	type MaxFreezes = VariantCountOf<RuntimeFreezeReason>;
	type RuntimeHoldReason = RuntimeHoldReason;
	type RuntimeFreezeReason = RuntimeFreezeReason;
	type DoneSlashHandler = ();
}

parameter_types! {
	pub FeeMultiplier: Multiplier = Multiplier::one();
}

impl pallet_transaction_payment::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type OnChargeTransaction = FungibleAdapter<Balances, ()>;
	type OperationalFeeMultiplier = ConstU8<5>;
	type WeightToFee = IdentityFee<Balance>;
	type LengthToFee = IdentityFee<Balance>;
	type FeeMultiplierUpdate = ConstFeeMultiplier<FeeMultiplier>;
	type WeightInfo = pallet_transaction_payment::weights::SubstrateWeight<Runtime>;
}

impl pallet_sudo::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type WeightInfo = pallet_sudo::weights::SubstrateWeight<Runtime>;
}

/// Configure the pallet-template in pallets/template.
impl pallet_template::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = pallet_template::weights::SubstrateWeight<Runtime>;
}

// ── Randomness ───────────────────────────────────────────────────────────────
//
// Jury-selection randomness for pallet-courts no longer goes through a generic
// `frame_support::traits::Randomness` implementation. It previously did (mixing the last 81
// block hashes as of the block `select_jury` was submitted in), but that scheme's output was
// fully computable from already-mined history at the moment it was called — so any authorized
// caller (the appellant, the oracle) could grind for a favorable jury just by delaying
// submission across candidate blocks and checking each one's (already knowable) result. That
// is *not* meaningfully different from what `pallet_insecure_randomness_collective_flip` would
// have provided even if it built here (it doesn't — see below), since collective-flip has the
// same "mix N already-known past blocks" shape.
//
// Instead, pallet-courts now implements a commit-then-delayed-reveal scheme internally: filing
// an appeal (`appeal_ruling`) timestamps the case in `JuryRequestBlock`, and `select_jury` may
// only be called — and only derives its seed from — a fixed window of `JurySeedDelayBlocks`
// blocks starting immediately after that timestamp. None of those blocks exist (and their
// hashes are therefore unknowable to anyone) at appeal time, which removes the grind-by-delay
// hole. It does not remove all manipulation risk: a validator scheduled to author a block
// inside that window can still nudge that block's hash within the space of valid blocks they
// could produce (the same residual "last revealer" risk class as RANDAO). See the
// `JurySeedDelayBlocks` doc comment on `pallet_courts::Config` for the full writeup, and
// HANDOFF.md item 7.
//
// `pallet_insecure_randomness_collective_flip` (37.0.0, latest on crates.io) was re-checked
// against the dependency set pinned below and still cannot be added: it depends on
// `polkadot-sdk-frame` 0.18.0, which pulls in a parallel frame-support/frame-system/sp-io
// 48.0.0 stack alongside our pinned 40.x/40.0.1 one. `cargo tree -i sp-io --duplicates`
// confirms two resolved `sp-io` versions (40.0.1 and 48.0.0) once it's added, and the build
// hard-fails compiling the old `sp-runtime-interface` v29.0.1 pulled in transitively
// (`assert_eq_size!(usize, u32)` fails on a 64-bit host). This is the same conflict recorded
// in HANDOFF.md item 33/7, just reconfirmed against current versions — not something worth
// spending further effort on since, per above, it wouldn't have bought real security anyway.

// ── Agora pallets ────────────────────────────────────────────────────────────

/// Passthrough ZK verifier: accepts any proof during development.
/// Gated behind `dev-mode` feature — a production build without that feature will
/// fail to compile here, forcing the real ZKPassport UltraHonk verifier to be wired in.
#[cfg(feature = "dev-mode")]
pub struct PassthroughZkVerifier;

#[cfg(feature = "dev-mode")]
impl pallet_identity_zk::ZkProofVerifier for PassthroughZkVerifier {
	fn verify(_proof_bytes: &[u8], _public_inputs: &[[u8; 32]]) -> bool {
		true
	}
}

/// Same passthrough, reused for `pallet_elections::Config::ZkVerifier` (the outer proof a
/// `register_as_delegate` call submits — cryptographically the same kind of proof
/// `pallet_identity_zk::register_citizen` verifies) and `pallet_elections::Config::
/// BackingProofVerifier` (the standalone `backing-nullifier` proof) alike — both traits have
/// the identical `verify(proof_bytes, public_inputs) -> bool` shape as
/// `pallet_identity_zk::ZkProofVerifier` above, and dev-mode's whole point is "accept
/// everything", so one struct covers all three rather than three near-identical stubs.
#[cfg(feature = "dev-mode")]
impl pallet_elections::ZkProofVerifier for PassthroughZkVerifier {
	fn verify(_proof_bytes: &[u8], _public_inputs: &[[u8; 32]]) -> bool {
		true
	}
}

#[cfg(feature = "dev-mode")]
impl pallet_elections::BackingProofVerifier for PassthroughZkVerifier {
	fn verify(_proof_bytes: &[u8], _public_inputs: &[[u8; 32]]) -> bool {
		true
	}
}

/// Same passthrough, reused for `pallet_courts::Config::ZkVerifier` (the anonymized
/// `LawChallenge`/`TreasuryDispute`/`TierConflict` case-filing proof) — identical
/// `verify(proof_bytes, public_inputs) -> bool` shape, and dev-mode's whole point is "accept
/// everything", so this struct covers it too rather than adding a fourth near-identical stub.
#[cfg(feature = "dev-mode")]
impl pallet_courts::ZkProofVerifier for PassthroughZkVerifier {
	fn verify(_proof_bytes: &[u8], _public_inputs: &[[u8; 32]]) -> bool {
		true
	}
}

/// Passthrough OPRF identity-anchor verifier: accepts any registration/reverification/
/// migration proof. Only wired in for the `dev-mode` `pallet_identity_zk::Config` impl below
/// (this struct itself isn't `#[cfg]`-gated so it stays available to reference from doc comments
/// / tests in both build modes, but it is only ever assigned to `type AnchorVerifier` inside the
/// `#[cfg(feature = "dev-mode")]` impl). The non-dev-mode impl wires in the real
/// `crate::anchor_verifier::Poseidon2AnchorVerifier` instead, which genuinely recomputes the
/// Poseidon2 `param_commitment` against the already-verified outer proof (HANDOFF log #75/#76) —
/// see that type's doc comment for the full trail. So this stub provides no Sybil-resistance
/// guarantee only in dev-mode builds, not in both build modes.
pub struct PassthroughAnchorVerifier;

impl pallet_identity_zk::AnchorProofVerifier for PassthroughAnchorVerifier {
	fn verify_registration_anchor(
		_outer_public_inputs: &[[u8; 32]],
		_anchor: [u8; 32],
		_scheme_version: u32,
		_oprf_pk_hashes: [[u8; 32]; 5],
		_backing_commitment: [u8; 32],
	) -> bool {
		true
	}

	fn verify_reverification(
		_outer_public_inputs: &[[u8; 32]],
		_anchor: [u8; 32],
		_scheme_version: u32,
		_oprf_pk_hashes: [[u8; 32]; 5],
		_backing_commitment: [u8; 32],
	) -> bool {
		true
	}

	fn verify_migration(
		_outer_public_inputs: &[[u8; 32]],
		_old_anchor: [u8; 32],
		_new_anchor: [u8; 32],
		_old_scheme_version: u32,
		_new_scheme_version: u32,
		_old_oprf_pk_hashes: [[u8; 32]; 5],
		_new_oprf_pk_hashes: [[u8; 32]; 5],
	) -> bool {
		true
	}
}

/// Same passthrough, reused for `pallet_elections::Config::DelegatePersonaVerifier` — only ever
/// assigned inside the `dev-mode` `pallet_elections::Config` impl below, same restriction as
/// `AnchorProofVerifier` above. The non-dev-mode impl wires in the real
/// `crate::anchor_verifier::Poseidon2AnchorVerifier` instead (see that type's own
/// `DelegatePersonaVerifier` impl).
impl pallet_elections::DelegatePersonaVerifier for PassthroughAnchorVerifier {
	fn check_delegate_persona(
		_outer_public_inputs: &[[u8; 32]],
		_delegate_persona_id: [u8; 32],
		_persona_account: [u8; 32],
		_scheme_version: u32,
		_oprf_pk_hashes: [[u8; 32]; pallet_elections::NUM_COMMITTEES],
	) -> bool {
		true
	}
}

#[cfg(feature = "dev-mode")]
impl pallet_identity_zk::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type ZkVerifier = PassthroughZkVerifier;
	/// Court oracle may manually suspend citizens (auto-path uses suspend_citizen_internal via
	/// CitizenSuspender trait; this extrinsic is an explicit administrative override). Wired to
	/// `EnsureOracleCouncilApproved`, not the bare `EnsureOracle` membership check: this
	/// extrinsic only succeeds once the Oracle Council's M-of-N threshold has approved this
	/// exact call via `propose_admin_action`/`approve_admin_action`, closing the gap where a
	/// single member could suspend any citizen unilaterally.
	type SuspensionOrigin = pallet_courts::EnsureOracleCouncilApproved<Runtime>;
	/// Merkle root allowlist updates require a legislature vote.
	type AdminOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
	/// See `PassthroughAnchorVerifier`'s doc comment — no real OPRF verifier exists yet.
	type AnchorVerifier = PassthroughAnchorVerifier;
	/// Placeholder cadence (~1 year — `DAYS` is block-time-derived, so this stays accurate
	/// regardless of block time) pending the human decision flagged as open in HANDOFF log #67
	/// (whether the liveness re-verification cadence should be shorter than the 4-year
	/// OPRF-rotation cycle). Governance-tunable, not hardcoded logic.
	type ReverificationPeriod = ConstU32<{ 365 * DAYS }>;
	/// Wired to `pallet_emergency_council::EnsureActiveEmergency<Runtime>`: succeeds only when
	/// the caller is `Root` *and* `pallet_emergency_council::ActiveEmergency` is currently
	/// `Some(..)` — i.e. the Emergency Council has genuinely declared (supermajority vote) an
	/// emergency that has not yet been lifted or auto-sunset-expired. Root alone is no longer
	/// sufficient, unlike `pallet_constitution::Config::RevocationOrigin` below, which is still
	/// a bare `EnsureRoot` placeholder.
	type EmergencyRotationOrigin = pallet_emergency_council::EnsureActiveEmergency<Runtime>;
	type Now = Timestamp;
	/// 1 day: generous relative to how long a real registration flow (mobile proving, then
	/// submitting) should ever take, while still bounding replay of a stale-but-not-yet-
	/// passport-expired proof (HANDOFF log #75). Placeholder pending real-world timing data,
	/// same governance-tunable-not-hardcoded spirit as `ReverificationPeriod` above.
	type MaxAnchorProofAge = ConstU64<DAYS_IN_SECONDS>;
	/// 5 minutes of tolerance for ordinary clock skew between the prover and this chain's
	/// `T::Now`. Bounds the total exploitable replay window for a single proof to roughly
	/// `MaxAnchorProofClockSkew + MaxAnchorProofAge` instead of unbounded — without this, a
	/// future-dated `current_date` public input (fully prover-controlled, see
	/// `MaxAnchorProofClockSkew`'s doc comment in `pallet_identity_zk::Config`) would make
	/// `MaxAnchorProofAge`'s staleness check pass unconditionally, forever.
	type MaxAnchorProofClockSkew = ConstU64<300>;
	/// Room above the eventual ~35-member committees (changelog entry 73's 5-independent-
	/// committee governance topology) — see `pallet_identity_zk::CommitteeMembers`'s doc
	/// comment.
	type MaxCommitteeSize = ConstU32<50>;
	/// ~6 days at this chain's block time (`DAYS` is block-time-derived, see
	/// `block_times` above) — changelog entry 82's "~5-7 days" OPRF response SLA, a
	/// placeholder pending real pilot telemetry per that entry's "Still open" section.
	type OprfQuerySlaBlocks = ConstU32<{ 6 * DAYS }>;
	/// Changelog entry 73's stated "~12-of-35 (1/3)" threshold for the eventual full-scale
	/// committees — same placeholder caveat as that entry's own numbers pending real
	/// committee sizing (see `docs/project/research/oprf-alternatives/11-genuine-threshold-evaluation-design.md`).
	/// Must stay `<= MaxCommitteeSize` above.
	type OprfThreshold = ConstU32<12>;
	/// Generous headroom over a realistic number of concurrent in-flight registration/
	/// reverification/migration attempts a single citizen would ever have open at once
	/// (retries included), while still bounding per-citizen mailbox growth — see
	/// `pallet_identity_zk::Config::MaxPendingOprfQueriesPerCitizen`'s doc comment.
	type MaxPendingOprfQueriesPerCitizen = ConstU32<20>;
	/// ~7 days — blunts rapid-fire recovery abuse/griefing (see
	/// `pallet_identity_zk::Config::MinBlocksBetweenRecoveries`'s doc comment) while staying
	/// well clear of a genuine back-to-back device-loss scenario. Placeholder pending real
	/// pilot telemetry, same spirit as this pallet's other governance-tunable cadences above.
	type MinBlocksBetweenRecoveries = ConstU32<{ 7 * DAYS }>;
	/// Matches `pallet_voting::Config::MaxEpochDurationBlocks` (30 days, see below) — the
	/// longest a voting epoch can run, and therefore the longest a Merkle authentication path
	/// fetched at epoch-open might need to stay verifiable for (see
	/// `pallet_identity_zk::Config::BackingRootHistoryWindowBlocks`'s doc comment).
	type BackingRootHistoryWindowBlocks = ConstU32<{ 30 * DAYS }>;
	/// Generous headroom above any realistic registration/revocation volume within a 30-day
	/// window; see `pallet_identity_zk::Config::MaxBackingRootHistoryEntries`'s doc comment for
	/// why a large cap here stays cheap (a `StorageMap` ring, not a bounded vector).
	type MaxBackingRootHistoryEntries = ConstU32<100_000>;
	/// Real `Balances` -- backs `recover_account`'s `RecoveryBlockedNonzeroBalance` guard.
	/// Not dev-mode-gated: it's a plain balance read, not a proof/ZK check, so there is
	/// nothing for dev-mode to stub out (same reasoning as `CommitteeKeyChecker` below).
	type Currency = Balances;
	/// See `pallet_identity_zk::RecoveryStateChecker`'s doc comment. Not dev-mode-gated,
	/// same reasoning as `Currency` above -- these are plain storage reads against
	/// pallet-elections/pallet-legislature/pallet-executive, not proof checks.
	type RecoveryStateChecker = Runtime;
}

#[cfg(not(feature = "dev-mode"))]
impl pallet_identity_zk::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// Real ZKPassport UltraHonk verifier (replaces the dropped Rarimo Groth16 one). Performs a
	/// genuine bb 5.0.0 UltraHonk pairing check via `ultrahonk-no-std` (changelog #72) — its own
	/// test suite proves real proofs verify (`verifies_a_real_bb5_zk_proof`) and mutated ones are
	/// rejected. What's still outstanding is only that no real end-to-end ZKPassport proof has
	/// been run through it yet (gated on the OPRF committee, see `docs/project/next-steps.md`),
	/// not a missing pairing check. See `crate::verifier`'s module docs for the full detail.
	type ZkVerifier = crate::verifier::ZkPassportUltraHonkVerifier;
	/// Court oracle may manually suspend citizens (auto-path uses suspend_citizen_internal via
	/// CitizenSuspender trait; this extrinsic is an explicit administrative override). Wired to
	/// `EnsureOracleCouncilApproved`, not the bare `EnsureOracle` membership check: this
	/// extrinsic only succeeds once the Oracle Council's M-of-N threshold has approved this
	/// exact call via `propose_admin_action`/`approve_admin_action`, closing the gap where a
	/// single member could suspend any citizen unilaterally.
	type SuspensionOrigin = pallet_courts::EnsureOracleCouncilApproved<Runtime>;
	/// Merkle root allowlist updates require a legislature vote.
	type AdminOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
	/// Real for all three methods (Poseidon2 `param_commitment` recomputation against the
	/// already-verified outer proof — registration/reverification via `disclosure`,
	/// migration via `migrate-disclosure`; HANDOFF log #75/#76) — see
	/// `crate::anchor_verifier::Poseidon2AnchorVerifier`'s doc comment for the full trail.
	type AnchorVerifier = crate::anchor_verifier::Poseidon2AnchorVerifier;
	/// See the `dev-mode` impl above for the placeholder-cadence rationale.
	type ReverificationPeriod = ConstU32<{ 365 * DAYS }>;
	/// See the `dev-mode` impl above — same `EnsureActiveEmergency` wiring.
	type EmergencyRotationOrigin = pallet_emergency_council::EnsureActiveEmergency<Runtime>;
	type Now = Timestamp;
	/// See the `dev-mode` impl above for the same rationale.
	type MaxAnchorProofAge = ConstU64<DAYS_IN_SECONDS>;
	/// See the `dev-mode` impl above for the same rationale.
	type MaxAnchorProofClockSkew = ConstU64<300>;
	/// See the `dev-mode` impl above for the same rationale.
	type MaxCommitteeSize = ConstU32<50>;
	/// See the `dev-mode` impl above for the same rationale.
	type OprfQuerySlaBlocks = ConstU32<{ 6 * DAYS }>;
	/// See the `dev-mode` impl above for the same rationale.
	type OprfThreshold = ConstU32<12>;
	/// See the `dev-mode` impl above for the same rationale.
	type MaxPendingOprfQueriesPerCitizen = ConstU32<20>;
	/// See the `dev-mode` impl above for the same rationale.
	type MinBlocksBetweenRecoveries = ConstU32<{ 7 * DAYS }>;
	/// See the `dev-mode` impl above for the same rationale.
	type BackingRootHistoryWindowBlocks = ConstU32<{ 30 * DAYS }>;
	/// See the `dev-mode` impl above for the same rationale.
	type MaxBackingRootHistoryEntries = ConstU32<100_000>;
	/// See the `dev-mode` impl above for the same rationale.
	type Currency = Balances;
	/// See the `dev-mode` impl above for the same rationale.
	type RecoveryStateChecker = Runtime;
}

/// Passthrough MACI tally verifier — accepts all proofs. Dev-mode only, same mechanism as
/// `PassthroughZkVerifier`/`PassthroughAntiCorruptionZkVerifier` elsewhere in this file: gated
/// behind the `dev-mode` feature, so a production build without that feature will fail to
/// compile here, forcing the choice below (`FailClosedMACIVerifier`) to be replaced with a real
/// MACI circuit verifier before it can be used to accept a tally in production.
#[cfg(feature = "dev-mode")]
pub struct PassthroughMACIVerifier;

#[cfg(feature = "dev-mode")]
impl pallet_voting::MACITallyVerifier for PassthroughMACIVerifier {
	fn verify_tally(
		_proposal_id: u32,
		_yes_votes: u64,
		_no_votes: u64,
		_commitment_root: [u8; 32],
		_proof_bytes: &[u8],
	) -> bool {
		true
	}
}

/// Fail-closed MACI tally verifier for non-`dev-mode` builds.
///
/// Unlike the identity-anchor path (`PassthroughAnchorVerifier` is dev-mode-only; non-dev-mode
/// already uses the real `crate::anchor_verifier::Poseidon2AnchorVerifier`, see its own doc
/// comment), no real MACI circuit verifier exists yet at all, so there's nothing to force in
/// place of a passthrough here. A fabricated MACI tally directly drives `enact_law` inside
/// `submit_maci_tally` — silently accepting one would let any `LegislatureOrigin`-controlled
/// account enact a law on fake vote counts. Rather than inventing a "real" check that doesn't
/// actually verify anything (trusted setup / circuit work not started, see CLAUDE.md's
/// "Remaining Work"), this rejects every tally. `submit_maci_tally` is effectively unusable in
/// non-dev builds until a genuine MACI tally verifier is wired in here to replace it.
#[cfg(not(feature = "dev-mode"))]
pub struct FailClosedMACIVerifier;

#[cfg(not(feature = "dev-mode"))]
impl pallet_voting::MACITallyVerifier for FailClosedMACIVerifier {
	fn verify_tally(
		_proposal_id: u32,
		_yes_votes: u64,
		_no_votes: u64,
		_commitment_root: [u8; 32],
		_proof_bytes: &[u8],
	) -> bool {
		false
	}
}

/// `cargo test -p agora-runtime --features dev-mode` runs `dev_mode_passthrough_accepts_anything`;
/// `cargo test -p agora-runtime` (default features — `dev-mode` is no longer one of them, see
/// `runtime/Cargo.toml`) runs `non_dev_mode_rejects_a_fabricated_tally` — proving a fabricated
/// tally that would previously have silently passed (unconditional `PassthroughMACIVerifier`) is
/// now rejected outside dev-mode.
#[cfg(all(test, feature = "dev-mode"))]
mod maci_verifier_tests {
	use super::*;
	use pallet_voting::MACITallyVerifier;

	#[test]
	fn dev_mode_passthrough_accepts_anything() {
		// A garbage "proof" for a fabricated tally — dev-mode's passthrough still accepts it
		// (expected/documented; dev-mode is explicitly not a security boundary).
		assert!(PassthroughMACIVerifier::verify_tally(
			0,
			1_000_000,
			0,
			[0u8; 32],
			&[0xde, 0xad, 0xbe, 0xef],
		));
	}
}

#[cfg(all(test, not(feature = "dev-mode")))]
mod maci_verifier_tests {
	use super::*;
	use pallet_voting::MACITallyVerifier;

	#[test]
	fn non_dev_mode_rejects_a_fabricated_tally() {
		// Same fabricated "proof" as the dev-mode test — must be rejected outside dev-mode,
		// closing the CRITICAL gap where PassthroughMACIVerifier used to accept it unconditionally.
		assert!(!FailClosedMACIVerifier::verify_tally(
			0,
			1_000_000,
			0,
			[0u8; 32],
			&[0xde, 0xad, 0xbe, 0xef],
		));
	}
}

/// Runtime implements CitizenChecker by calling pallet-identity's is_active_citizen.
/// Returns false for both unregistered accounts and accounts with active suspensions.
impl pallet_voting::CitizenChecker<AccountId> for Runtime {
	fn is_active_citizen(who: &AccountId) -> bool {
		pallet_identity_zk::Pallet::<Runtime>::is_active_citizen(who)
	}

	fn total_citizens() -> u32 {
		pallet_identity_zk::TotalCitizens::<Runtime>::get()
	}
}

/// Runtime implements NullifierProvider by reading CitizenNullifier from pallet-identity.
impl pallet_voting::NullifierProvider<AccountId> for Runtime {
	fn nullifier_of(who: &AccountId) -> Option<[u8; 32]> {
		pallet_identity_zk::CitizenNullifier::<Runtime>::get(who)
	}
}

/// Runtime implements LawEnactor: when a referendum passes, enact the law at the correct tier.
/// The referendum tier is forwarded so constitutional referenda enact constitutional laws.
impl pallet_voting::LawEnactor for Runtime {
	fn enact_law(
		tier: pallet_voting::ReferendumTier,
		content_hash: [u8; 32],
	) -> sp_runtime::DispatchResult {
		let law_tier = match tier {
			pallet_voting::ReferendumTier::Ordinary => pallet_constitution::LawTier::Ordinary,
			pallet_voting::ReferendumTier::Constitutional => pallet_constitution::LawTier::Structural,
			pallet_voting::ReferendumTier::Foundational => pallet_constitution::LawTier::Foundational,
		};
		pallet_constitution::Pallet::<Runtime>::enact_law_internal(law_tier, content_hash)
	}
}

impl pallet_voting::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// No single delegate may hold more than 33% of voting power.
	type DelegationCap = ConstU8<33>;
	/// Absolute ceiling: at most 1 000 direct delegators per (topic, delegate).
	type MaxDelegationsPerDelegate = ConstU32<1_000>;
	/// Walk at most 10 hops when checking for delegation cycles.
	type MaxDelegationDepth = ConstU8<10>;
	/// Minimum delegation duration: 1 voting epoch (~30 days). Prevents instant-expiry gaming.
	type MinDelegationDurationBlocks = ConstU32<{ 30 * DAYS }>;
	/// Maximum delegation duration: 2 years. Constitutional ceiling; prevents indefinite concentration.
	type MaxDelegationDurationBlocks = ConstU32<{ 2 * 365 * DAYS }>;
	/// Number of budget categories citizens can allocate QV tokens across.
	type BudgetCategoryCount = ConstU32<10>;
	type CitizenChecker = Runtime;
	type NullifierProvider = Runtime;
	/// Minimum 1-day voting window prevents spam proposals that expire instantly.
	type MinProposalDurationBlocks = ConstU32<{ 1 * DAYS }>;
	/// Maximum 90-day cap prevents proposals from lingering indefinitely.
	type MaxProposalDurationBlocks = ConstU32<{ 90 * DAYS }>;
	/// Referendum voting window: 14 days.
	type ReferendumDurationBlocks = ConstU32<{ 14 * DAYS }>;
	/// Simple majority required to pass an ordinary referendum.
	type PassageThreshold = ConstU8<51>;
	/// 2/3 supermajority required to pass a constitutional (Structural-tier) referendum.
	type ConstitutionalPassageThreshold = ConstU8<67>;
	/// 3/4 supermajority required to pass a foundational referendum.
	type FoundationalPassageThreshold = ConstU8<75>;
	type LawEnactor = Runtime;
	/// Dev builds pass every tally through unverified (`PassthroughMACIVerifier`); non-dev
	/// builds reject every tally until a real MACI circuit verifier replaces
	/// `FailClosedMACIVerifier`. See both structs' doc comments above.
	#[cfg(feature = "dev-mode")]
	type MACITallyVerifier = PassthroughMACIVerifier;
	#[cfg(not(feature = "dev-mode"))]
	type MACITallyVerifier = FailClosedMACIVerifier;
	/// Fiscal year start is a legislature motion — wired to the same origin as
	/// pallet-constitution's law-enactment gate so budget epochs are on-chain governed.
	type LegislatureOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
	/// Voting epochs must be at least 7 days long.
	type MinEpochDurationBlocks = ConstU32<{ 7 * DAYS }>;
	/// Voting epochs may not exceed 30 days (constitutional ceiling).
	type MaxEpochDurationBlocks = ConstU32<{ 30 * DAYS }>;
	/// Bounds how many referenda sharing an exact finalization block can be auto-scheduled via
	/// `PendingFinalization`. Generous relative to realistic referendum creation rates (this
	/// many would all have to be created within the same block to overflow it); any overflow
	/// still finalizes correctly via the permissionless `finalize_referendum` extrinsic.
	type MaxReferendaPerBlock = ConstU32<500>;
	/// Bounds `pallet_voting::OpenReferenda`, the flat list of currently-open referendum ids
	/// `RecoveryStateChecker::has_open_referendum_vote` scans below — see that storage item's
	/// and `pallet_voting::Config::MaxConcurrentReferenda`'s doc comments. 500 mirrors
	/// `MaxReferendaPerBlock` above: generous relative to realistic referendum creation rates
	/// (referendum creation is gated by a legislature motion or a petition threshold, and only
	/// referenda created within one `ReferendumDurationBlocks` — 14-day — window can be
	/// concurrently open), while unlike `MaxReferendaPerBlock`, overflowing this one fails
	/// referendum creation outright rather than silently proceeding untracked.
	type MaxConcurrentReferenda = ConstU32<500>;
}

/// Type alias for the audit pallet used in cross-pallet trait wiring.
/// The canonical `PalletAudit` alias for `construct_runtime!` lives in `runtime/src/lib.rs`.
type PalletAuditImpl = pallet_audit::Pallet<Runtime>;

/// Runtime implements pallet-audit's `TreasuryFreezer` by calling pallet-treasury-ledger's
/// audit-specific internal freeze/unfreeze functions — deliberately the `audit_*` pair, not
/// the plain `freeze_department_internal`/`unfreeze_department_internal` that `TreasuryEnforcer`
/// below uses for pallet-courts. The two authorities now write to independent storage axes
/// (`AuditFrozenDepartments` vs. `CourtFrozenDepartments`) so pallet-audit resolving its own
/// flags can never silently lift a pallet-courts freeze, and vice versa.
impl pallet_audit::TreasuryFreezer for Runtime {
	fn freeze_department(department_id: u32) -> sp_runtime::DispatchResult {
		pallet_treasury_ledger::Pallet::<Runtime>::audit_freeze_department_internal(department_id)
	}
	fn unfreeze_department(department_id: u32) -> sp_runtime::DispatchResult {
		pallet_treasury_ledger::Pallet::<Runtime>::audit_unfreeze_department_internal(department_id)
	}
}

impl pallet_audit::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// At most 10 registered auditors.
	type MaxAuditors = ConstU32<10>;
	type TreasuryFreezer = Runtime;
	/// Auditor appointment now requires the independent Accountability Council's own 2/3
	/// supermajority approval for the exact call, not bare `Root` — see
	/// `pallet_accountability_council`'s module doc comment and the "Accountability Council"
	/// section below for the self-oversight rationale.
	type AppointmentOrigin = pallet_accountability_council::EnsureAccountabilityCouncilApproved<Runtime>;
}

impl pallet_treasury_ledger::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type Balance = Balance;
	/// Wire the audit pallet as the expenditure hook.
	type AuditHook = PalletAuditImpl;
	/// Budget allocation requires a passed legislature motion, not just sudo.
	type LegislatureOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
	/// Manually clearing a freeze (`unfreeze_department`) requires the Oracle Council's
	/// M-of-N approval, not bare root — mirrors exactly how `SuspensionOrigin`/`CourtOrigin`
	/// are wired for `pallet_identity_zk`/`pallet_constitution` above. A court-ordered freeze
	/// (`CourtFrozenDepartments`) is only ever set via the M-of-N-gated `TreasuryEnforcer`
	/// path (an actual court ruling); before this, `unfreeze_department` was gated only by
	/// `EnsureRoot`, letting a single Root/sudo key silently reverse an already-adjudicated
	/// ruling with no council or jury involvement.
	type CourtOrigin = pallet_courts::EnsureOracleCouncilApproved<Runtime>;
}

/// Runtime implements CitizenSelector by reading pallet-identity's indexed storage.
impl pallet_courts::CitizenSelector<AccountId> for Runtime {
	fn citizen_at(index: u32) -> Option<AccountId> {
		pallet_identity_zk::CitizenIndex::<Runtime>::get(index)
	}
	fn total_citizens() -> u32 {
		pallet_identity_zk::TotalCitizens::<Runtime>::get()
	}
}

/// Runtime implements CitizenChecker for pallet-courts (file_case active-citizen gate, and
/// appeal_ruling's ruled-against-party nullifier check).
impl pallet_courts::CitizenChecker<AccountId> for Runtime {
	fn is_active_citizen(who: &AccountId) -> bool {
		pallet_identity_zk::Pallet::<Runtime>::is_active_citizen(who)
	}

	fn citizen_nullifier(who: &AccountId) -> Option<[u8; 32]> {
		pallet_identity_zk::CitizenNullifier::<Runtime>::get(who)
	}
}

/// Runtime implements LawEnforcer by calling pallet-constitution's internal function.
impl pallet_courts::LawEnforcer for Runtime {
	fn invalidate_law(law_id: u32) -> sp_runtime::DispatchResult {
		pallet_constitution::Pallet::<Runtime>::invalidate_law_internal(law_id)
	}
}

/// Runtime implements TreasuryEnforcer by calling pallet-treasury-ledger's court-specific
/// internal function (`freeze_department_internal`, writing `CourtFrozenDepartments` — the
/// axis pallet-audit's `TreasuryFreezer` wiring above never touches).
impl pallet_courts::TreasuryEnforcer for Runtime {
	fn freeze_department(department_id: u32) -> sp_runtime::DispatchResult {
		pallet_treasury_ledger::Pallet::<Runtime>::freeze_department_internal(department_id)
	}
}

/// Runtime implements CitizenSuspender by calling pallet-identity's internal suspension function.
/// `suspension_until` is an absolute block number computed by pallet-courts before this call.
impl pallet_courts::CitizenSuspender<BlockNumber> for Runtime {
	fn suspend_citizen(
		nullifier: [u8; 32],
		suspension_until: Option<BlockNumber>,
		jury_reviewed: bool,
	) -> sp_runtime::DispatchResult {
		pallet_identity_zk::Pallet::<Runtime>::suspend_citizen_internal(
			nullifier,
			suspension_until,
			jury_reviewed,
		)
	}
}

/// Runtime implements `RecoveryStateChecker` by reading pallet-elections/pallet-legislature/
/// pallet-executive/pallet-voting's own storage directly — see
/// `pallet_identity_zk::RecoveryStateChecker`'s doc comment for why this lives on `Runtime`
/// rather than as a direct impl on any of those pallets' own `Pallet<T>` (a circular crate
/// dependency back onto pallet-identity-zk, the same reason `CitizenSuspender` above is
/// Runtime-glue rather than a direct impl).
impl pallet_identity_zk::RecoveryStateChecker<AccountId> for Runtime {
	fn is_registered_delegate(who: &AccountId) -> bool {
		pallet_elections::Delegates::<Runtime>::contains_key(who)
	}

	fn holds_legislature_seat(who: &AccountId) -> bool {
		pallet_legislature::Members::<Runtime>::get().contains(who)
	}

	fn holds_cabinet_role(who: &AccountId) -> bool {
		pallet_executive::MinisterPortfolio::<Runtime>::contains_key(who)
			|| pallet_executive::PrimeMinister::<Runtime>::get().as_ref() == Some(who)
	}

	/// Scans `pallet_voting::OpenReferenda` — a bounded list (see
	/// `pallet_voting::Config::MaxConcurrentReferenda`) of referendum ids currently in
	/// `ReferendumState::Voting`, not an unbounded scan over `Referenda` — checking
	/// `ReferendumHasVoted` for `who` against each. Bounded worst-case cost:
	/// `MaxConcurrentReferenda` storage reads.
	fn has_open_referendum_vote(who: &AccountId) -> bool {
		pallet_voting::OpenReferenda::<Runtime>::get()
			.iter()
			.any(|referendum_id| pallet_voting::ReferendumHasVoted::<Runtime>::get((*referendum_id, who.clone())))
	}

	/// O(1): `true` iff `who` has already claimed the currently active fiscal epoch's budget
	/// allocation.
	fn has_unclaimed_current_epoch_budget(who: &AccountId) -> bool {
		pallet_voting::CitizenClaimedEpoch::<Runtime>::get(who)
			== Some(pallet_voting::FiscalYearEpoch::<Runtime>::get())
	}
}

impl pallet_courts::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// Citizens have 7 days to appeal an AI ruling.
	type AppealWindowBlocks = ConstU32<{ 7 * DAYS }>;
	type CitizenSelector = Runtime;
	type CitizenChecker = Runtime;
	type LawEnforcer = Runtime;
	type TreasuryEnforcer = Runtime;
	/// Oracle Council membership stored in OracleMembers; managed via add_oracle_member /
	/// remove_oracle_member (root-only). `EnsureOracle<Runtime>` accepts a signed member; the
	/// actual M-of-N gate is enforced separately by submit_ai_ruling/approve_ai_ruling/
	/// finalize_ruling against OracleApprovalNumerator/Denominator below — see
	/// pallet_courts::Config::OracleOrigin's doc comment for why this replaced the earlier
	/// single-OracleAccount design (a project review flagged it as a single point of failure).
	type OracleOrigin = pallet_courts::EnsureOracle<Runtime>;
	/// Oracle Council capped at 7 seats — matches a Level-1 appeal jury's size
	/// (`select_jury`'s non-LawChallenge branch); see `pallet_courts::Config::MaxOracleMembers`.
	type MaxOracleMembers = ConstU32<7>;
	/// Simple majority (strictly more than half) of the Oracle Council must approve a ruling
	/// submission or finalization before it takes effect — see
	/// `pallet_courts::Config::OracleApprovalNumerator`'s doc comment for why this uses `>`
	/// rather than the `>=` this pallet's own AI-model-approval supermajority below uses.
	type OracleApprovalNumerator = ConstU32<1>;
	type OracleApprovalDenominator = ConstU32<2>;
	/// 14 days, matching `pallet_legislature::Config::PendingApprovalExpiryBlocks` — the same
	/// stuck-proposer deadlock this closes (see `EnsureOracleCouncilApproved`'s doc comment).
	type AdminActionExpiryBlocks = ConstU32<{ 14 * DAYS }>;
	type CitizenSuspender = Runtime;
	/// 10 minutes' worth of blocks after an appeal is filed before jury selection can use
	/// the resulting (delayed-reveal) seed. See `pallet_courts::Config::JurySeedDelayBlocks`
	/// for what this buys and its residual risk — it is not VRF-grade.
	type JurySeedDelayBlocks = ConstU32<{ 10 * MINUTES }>;
	/// Bounds how many cases may have their jury-seed capture scheduled for the same block
	/// (see `pallet_courts::Config::MaxCasesPerBlock`). 128 is generous headroom relative to
	/// this runtime's expected appeal volume, sized the same order of magnitude as this
	/// runtime's other generic per-block/per-round bounds.
	type MaxCasesPerBlock = ConstU32<128>;
	/// Zero account used as filer for system-initiated LawChallenge cases.
	type AutoChallengeAccount = AutoChallengeAccount;
	type Currency = Balances;
	/// 1 AGR — a plain spam-prevention deposit sized the same as this runtime's other
	/// single-AGR bonds/deposits; no documented reason court filings should be priced
	/// differently. (Until the Elections Commission subsystem was removed from
	/// pallet-elections — see `docs/project/pallets/elections.md` — this literal matched
	/// its since-deleted `CandidateDeposit`; that pallet no longer has any deposit of its
	/// own to stay consistent with.) `auto_file_case` (system-initiated) never reserves
	/// this — see that call's own doc comment in `pallets/pallet-courts/src/lib.rs`.
	type CaseFilingBond = ConstU128<1_000_000_000_000>;
	/// AI Model Governance Council capped at 35 seats — same order of magnitude as the
	/// single-committee size used elsewhere in this codebase's OPRF committee governance
	/// design; no stronger documented reason to pick a different number here.
	type MaxAIGovernanceCouncilSize = ConstU32<35>;
	/// 2/3 supermajority required to approve a new AI model version, matching
	/// CLAUDE.md's "AI model updates require on-chain governance vote (supermajority)"
	/// and pallet-executive's identical cabinet-emergency threshold below.
	type AIModelSupermajorityNumerator = ConstU32<2>;
	type AIModelSupermajorityDenominator = ConstU32<3>;
	/// Dev builds accept every anonymized case-filing proof unconditionally
	/// (`PassthroughZkVerifier`); non-dev builds run the real bb 5.0.0 UltraHonk pairing check —
	/// the same verifier `pallet_identity_zk`/`pallet_elections::Config::ZkVerifier` use, since
	/// this is cryptographically just another outer ZKPassport proof (see
	/// `crate::verifier::ZkPassportUltraHonkVerifier`'s `pallet_courts::ZkProofVerifier` impl).
	/// Mirrors `pallet_elections::Config::ZkVerifier`'s identical per-field cfg-gating pattern
	/// elsewhere in this file, rather than duplicating this whole `impl` for one differing line.
	#[cfg(feature = "dev-mode")]
	type ZkVerifier = PassthroughZkVerifier;
	#[cfg(not(feature = "dev-mode"))]
	type ZkVerifier = crate::verifier::ZkPassportUltraHonkVerifier;
}

/// Runtime implements CitizenChecker for pallet-constitution (petition/sign gating).
impl pallet_constitution::CitizenChecker<AccountId> for Runtime {
	fn is_active_citizen(who: &AccountId) -> bool {
		pallet_identity_zk::Pallet::<Runtime>::is_active_citizen(who)
	}
}

/// Runtime implements PetitionApprover: when a petition hits threshold, create an Ordinary
/// referendum. Constitutional referenda require a separate legislature motion.
impl pallet_constitution::PetitionApprover for Runtime {
	fn create_referendum(petition_id: u32, topic_hash: [u8; 32]) -> sp_runtime::DispatchResult {
		pallet_voting::Pallet::<Runtime>::create_referendum_internal(
			petition_id,
			topic_hash,
			pallet_voting::ReferendumTier::Ordinary,
		)
	}
}

// ── Auto-challenge (replaces HRC) ───────────────────────────────────────────

parameter_types! {
	/// Zero account used as the system filer for auto-initiated LawChallenge cases.
	/// Structural and Foundational laws automatically open a court case on enactment.
	pub const AutoChallengeAccount: AccountId = AccountId32::new([0u8; 32]);
}

/// Runtime implements AutoChallengeHook by calling pallet-courts::auto_file_case.
impl pallet_constitution::AutoChallengeHook for Runtime {
	fn auto_challenge_law(law_id: u32) -> sp_runtime::DispatchResult {
		pallet_courts::Pallet::<Runtime>::auto_file_case(
			pallet_courts::CaseSubject::LawChallenge { law_id },
		)
	}
}

/// Runtime implements TierConflictHook by calling pallet-courts::file_case_for with
/// `CaseSubject::TierConflict` — the citizen-initiated, anonymized counterpart to
/// `AutoChallengeHook` above. See `pallet_constitution::TierConflictHook`'s doc comment.
impl pallet_constitution::TierConflictHook<AccountId> for Runtime {
	fn file_tier_conflict_case(
		filer: AccountId,
		law_id: u32,
		zk_proof: frame_support::BoundedVec<u8, ConstU32<4096>>,
		public_inputs: frame_support::BoundedVec<[u8; 32], ConstU32<16>>,
	) -> sp_runtime::DispatchResult {
		pallet_courts::Pallet::<Runtime>::file_case_for(
			filer,
			pallet_courts::CaseSubject::TierConflict { law_id },
			zk_proof,
			public_inputs,
		)
	}
}

/// Runtime implements FreshLegislatureChecker by reading LastElectionBlock from pallet-elections.
impl pallet_constitution::FreshLegislatureChecker<BlockNumber> for Runtime {
	fn has_election_occurred_since(proposed_at: BlockNumber) -> bool {
		pallet_elections::LastElectionBlock::<Runtime>::get() > proposed_at
	}
}

impl pallet_constitution::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// `EnsureLegislatureMotion`'s `([u8; 32], u8)` overload — the same origin type used
	/// elsewhere, but here the required-percentage argument is checked against the tally
	/// frozen when the authorizing motion closed (see that overload's doc comment in
	/// pallet-legislature, and `pallet_constitution`'s module doc comment for the full
	/// tier-aware-threshold rationale).
	type LegislatureOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
	/// Ordinary-tier legislature motions: 51%, matching pallet-voting's Ordinary referendum
	/// `PassageThreshold` below.
	type OrdinaryPassageThreshold = ConstU8<51>;
	/// Structural-tier legislature motions: 67%, matching pallet-voting's
	/// `ConstitutionalPassageThreshold` below.
	type ConstitutionalPassageThreshold = ConstU8<67>;
	/// Foundational-tier legislature motions: 75%, matching pallet-voting's
	/// `FoundationalPassageThreshold` below.
	type FoundationalPassageThreshold = ConstU8<75>;
	/// Ordinary law amendments take effect immediately (no deliberation window).
	type OrdinaryAmendmentDeliberationBlocks = ConstU32<0>;
	/// Structural/Foundational amendments stay Provisional for ~2 years before reaffirmation opens.
	type ProvisioningPeriodBlocks = ConstU32<{ 2 * 365 * DAYS }>;
	/// After Confirmed, ~4 more years before Entrenched can be claimed (6 years total pipeline).
	type ConfirmationPeriodBlocks = ConstU32<{ 4 * 365 * DAYS }>;
	type FreshLegislatureChecker = Runtime;
	/// Revocation origin: EnsureRoot for dev. Wire to a minority collective (30–40%) for mainnet.
	type RevocationOrigin = EnsureRoot<AccountId>;
	/// 1 000 citizen signatures required to trigger a referendum.
	type PetitionThreshold = ConstU32<1_000>;
	type PetitionApprover = Runtime;
	type CitizenChecker = Runtime;
	/// Structural/Foundational laws auto-open a court case on enactment for AI review.
	type AutoChallengeHook = Runtime;
	/// Citizen-initiated, permissionless `challenge_law_tier` — see
	/// `pallet_constitution::TierConflictHook`'s doc comment.
	type TierConflictHook = Runtime;
	/// Courts origin for invalidate_law (manual override). The auto-enforcement path
	/// uses invalidate_law_internal via the LawEnforcer trait. Wired to
	/// `EnsureOracleCouncilApproved`, not the bare `EnsureOracle` membership check: this
	/// extrinsic only succeeds once the Oracle Council's M-of-N threshold has approved this
	/// exact call via `propose_admin_action`/`approve_admin_action`, closing the gap where a
	/// single member could pause any law unilaterally.
	type CourtOrigin = pallet_courts::EnsureOracleCouncilApproved<Runtime>;
	type WeightInfo = pallet_constitution::weights::SubstrateWeight<Runtime>;
	/// `submit_petition`/`sign_petition`/`reaffirm_amendment` benchmarks need a way to mark an
	/// account as an active citizen and to fast-forward `FreshLegislatureChecker` — this runtime
	/// has no such hook yet (would need an equivalent benchmark-only entry point on
	/// pallet-identity-zk/pallet-elections), so those three benchmarks will fail if actually run
	/// via `benchmark pallet` against this runtime until that hook exists. See
	/// `pallet_constitution::benchmarking`'s module doc comment.
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper = RuntimeBenchmarkHelper;
}

/// No-op `BenchmarkHelper` for pallets whose benchmarks need cross-pallet citizen/election state
/// this runtime has no benchmark-only hook for yet (see doc comments on the `Config` impls that
/// use it). Running `benchmark pallet` against a real built runtime will fail exactly the
/// extrinsics that depend on these methods, not silently produce wrong numbers — documented
/// rather than papered over.
#[cfg(feature = "runtime-benchmarks")]
pub struct RuntimeBenchmarkHelper;
#[cfg(feature = "runtime-benchmarks")]
impl pallet_constitution::BenchmarkHelper<AccountId> for RuntimeBenchmarkHelper {
	fn make_active_citizen(_who: &AccountId) {}
	fn make_legislature_fresh() {}
}
#[cfg(feature = "runtime-benchmarks")]
impl pallet_elections::BenchmarkHelper<AccountId> for RuntimeBenchmarkHelper {
	fn make_active_citizen(_who: &AccountId) {}
}

impl pallet_legislature::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// Legislature has at most 500 seats.
	type MaxMembers = ConstU32<500>;
	/// Members have 7 days to vote on a motion.
	type MotionDurationBlocks = ConstU32<{ 7 * DAYS }>;
	/// Floor: 51% of total members required to plant an approval token at all, matching the
	/// referendum path's Ordinary tier. This is only the minimum — pallet-constitution demands
	/// more for Structural/Foundational calls via `EnsureLegislatureMotion`'s tier-aware
	/// `([u8; 32], u8)` origin overload (see `pallet_constitution::Config::LegislatureOrigin`).
	type PassageThreshold = ConstU8<51>;
	/// An unconsumed approval token can be discarded by any member after 14 days, recovering
	/// the legislature if a proposer never executes it (offline, lost key, or removed).
	type PendingApprovalExpiryBlocks = ConstU32<{ 14 * DAYS }>;
	/// Active ministers are blocked from legislature votes (incompatibility rule).
	type MinisterChecker = Cabinet;
	type WeightInfo = pallet_legislature::weights::SubstrateWeight<Runtime>;
}

// ── Parliamentary Executive ──────────────────────────────────────────────────

impl pallet_executive::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// Only a passed legislature motion can appoint/dismiss ministers or ratify an emergency.
	type LegislatureOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
	/// Maximum 20 cabinet portfolios.
	type MaxPortfolios = ConstU32<20>;
	/// 30 days, constitutional ceiling on emergency duration. Expressed via `DAYS` (block-time-
	/// derived, see `pallet_emergency_council::Config::MaxEmergencyBlocks` below for the sibling
	/// fix and full rationale) rather than a hardcoded literal, so it can't silently drift out of
	/// sync with `MILLI_SECS_PER_BLOCK` again — a bare `432_000` here was previously "30 days at
	/// 6s/block" but this runtime's actual block time is 12s, making that literal a 60-day cap.
	type MaxEmergencyBlocks = ConstU32<{ 30 * DAYS }>;
	/// Legislature has 72 hours to ratify after cabinet declares emergency (`DAYS` is block-time-
	/// derived, so this stays correct regardless of block time).
	type RatificationWindowBlocks = ConstU32<{ 3 * DAYS }>;
	/// 7-day cooldown after an emergency ends (lapse, sunset expiry, or early
	/// `vote_end_emergency`) before the cabinet can declare another one — mirrors
	/// `pallet_emergency_council::Config::EmergencyCooldownBlocks` below for the same reason:
	/// without it, the same supermajority could chain back-to-back emergencies into de-facto
	/// indefinite emergency powers.
	type EmergencyCooldownBlocks = ConstU32<{ 7 * DAYS }>;
	/// Cross-pallet coordination with pallet-emergency-council's own, independent cooldown —
	/// see `pallet_executive::SiblingEmergencyCooldown`'s doc comment. Implemented on `Runtime`
	/// itself, just below (a "Runtime-level delegating impl", the same escape hatch
	/// `CitizenChecker`/`LegislatureMembership` above use, needed here — unlike
	/// `DisclosureChecker` elsewhere in this file — because the relationship is symmetric and a
	/// direct pallet-to-pallet crate dependency in both directions would cycle).
	type SiblingEmergencyCooldown = Runtime;
	/// 2/3 cabinet supermajority required to declare or end an emergency.
	type SupermajorityNumerator = ConstU32<2>;
	type SupermajorityDenominator = ConstU32<3>;
	/// Daily re-check for a court-suspended PM/minister (see `pallet_executive`'s
	/// `run_vacancy_sweep` doc comment for why this is a periodic poll, not a hook).
	/// Implemented on `Runtime` itself, just below.
	type CitizenChecker = Runtime;
	/// The PM/successor/nominee/voter must currently hold a legislature seat.
	/// Implemented on `Runtime` itself, just below.
	type LegislatureMembership = Runtime;
	/// Gates PM/minister appointment on the reverse-direction executive/Accountability-Council
	/// overlap bar (see `pallet_executive::AccountabilityCouncilChecker`'s doc comment) —
	/// `pallet_accountability_council::add_member` only blocks the other direction (a current
	/// minister/PM joining the Council) at join time; this closes the gap where a sitting
	/// Council member could later be nominated/invested PM or appointed a minister.
	type AccountabilityCouncilChecker = AccountabilityCouncil;
	/// 7 days to nominate PM candidates once an investiture round opens, matching
	/// pallet-legislature's own motion-duration convention.
	type PmNominationWindowBlocks = ConstU32<{ 7 * DAYS }>;
	/// 7 further days to cast ranked ballots once nominations close.
	type PmVotingWindowBlocks = ConstU32<{ 7 * DAYS }>;
	/// Bounds ballot/tally size for a single investiture round.
	type MaxPmCandidates = ConstU32<20>;
	/// Rolling-window occupancy cap on the PM office (replaces an earlier consecutive-terms
	/// counter that a one-block puppet reinstall could reset — see
	/// `pallet_executive::pallet::Pallet::pm_occupancy_in_window`'s doc comment for the full
	/// rationale). Tunable policy constants, not derived from first principles: no account may
	/// hold the PM office for more than ~74% of the trailing year (270 of 365 days).
	type PmOccupancyWindowBlocks = ConstU32<{ 365 * DAYS }>;
	type MaxPmOccupancyBlocks = ConstU32<{ 270 * DAYS }>;
	type MaxPmTenureHistory = ConstU32<128>;
	/// Daily conviction-vacancy sweep.
	type VacancySweepIntervalBlocks = ConstU32<{ 1 * DAYS }>;
	type WeightInfo = pallet_executive::weights::SubstrateWeight<Runtime>;
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper = RuntimeBenchmarkHelper;
}

/// Runtime implements pallet_executive::CitizenChecker by delegating to pallet-identity —
/// same suspension check (`SuspendedNullifiers`) every other citizen-facing pallet uses.
impl pallet_executive::CitizenChecker<AccountId> for Runtime {
	fn is_active_citizen(who: &AccountId) -> bool {
		pallet_identity_zk::Pallet::<Runtime>::is_active_citizen(who)
	}
	fn is_suspended_by_jury_reviewed_conviction(who: &AccountId) -> bool {
		pallet_identity_zk::Pallet::<Runtime>::is_suspended_by_jury_reviewed_conviction(who)
	}
}

/// Runtime implements pallet_executive::LegislatureMembership by reading pallet-legislature's
/// `Members` directly — the PM is chosen by and from the legislature, not the citizenry.
impl pallet_executive::LegislatureMembership<AccountId> for Runtime {
	fn is_member(who: &AccountId) -> bool {
		pallet_legislature::Members::<Runtime>::get().contains(who)
	}
}
#[cfg(feature = "runtime-benchmarks")]
impl pallet_executive::BenchmarkHelper<AccountId> for RuntimeBenchmarkHelper {
	fn make_active_citizen(_who: &AccountId) {}
	fn make_legislature_member(_who: &AccountId) {}
}

/// Runtime-level delegating impl of `pallet_executive::SiblingEmergencyCooldown` — see that
/// trait's doc comment for the full rationale (mutual coordination between pallet-executive's
/// and pallet-emergency-council's independently-cooled-down emergency mechanisms; a
/// `Runtime`-level impl rather than a direct one on either pallet's `Pallet<T>` because the
/// relationship is symmetric and a direct crate dependency in both directions would cycle).
/// Reuses pallet-emergency-council's own `CooldownUntil` storage item directly rather than
/// adding a new "cooldown imposed by the sibling" item — see the trait's doc comment for why
/// that's safe (neither pallet's internal logic distinguishes *why* `CooldownUntil` was set).
impl pallet_executive::SiblingEmergencyCooldown<BlockNumber> for Runtime {
	fn is_in_cooldown(now: BlockNumber) -> bool {
		now < pallet_emergency_council::CooldownUntil::<Runtime>::get()
	}
	fn notify_emergency_ended(now: BlockNumber) {
		pallet_emergency_council::CooldownUntil::<Runtime>::put(
			now.saturating_add(
				<Runtime as pallet_emergency_council::Config>::EmergencyCooldownBlocks::get(),
			),
		);
	}
}

/// Mirror-image of the impl above: lets pallet-emergency-council also honor, and also start,
/// pallet-executive's own cooldown. See `pallet_executive::SiblingEmergencyCooldown`'s doc
/// comment just above for the shared rationale.
impl pallet_emergency_council::SiblingEmergencyCooldown<BlockNumber> for Runtime {
	fn is_in_cooldown(now: BlockNumber) -> bool {
		now < pallet_executive::CooldownUntil::<Runtime>::get()
	}
	fn notify_emergency_ended(now: BlockNumber) {
		pallet_executive::CooldownUntil::<Runtime>::put(
			now.saturating_add(
				<Runtime as pallet_executive::Config>::EmergencyCooldownBlocks::get(),
			),
		);
	}
}

// ── Liquid Democracy Delegates / Legislature Elections ──────────────────────
//
// pallet-elections used to also run a separate "Elections Commission" subsystem
// (commissioners, named "office" elections, candidate registration/certification) —
// removed: it certified an election's outcome on nothing but a commissioner's say-so, with
// no on-chain tally behind it, and nothing in this system's design turned out to need a
// citizen-facing "elect one person to a named office" mechanism. Legislature seats fill
// automatically via the delegate/backing mechanism below; the Prime Minister is chosen by
// the legislature itself via pallet-executive's ranked-choice investiture (see
// `pallet_executive::Config` above). See docs/project/changelog/ for the removal rationale.

/// Runtime implements pallet_elections::CitizenChecker by delegating to pallet-identity.
impl pallet_elections::CitizenChecker<AccountId> for Runtime {
	fn is_active_citizen(who: &AccountId) -> bool {
		pallet_identity_zk::Pallet::<Runtime>::is_active_citizen(who)
	}
}

/// `AccountId` (`<<Signature as Verify>::Signer as IdentifyAccount>::AccountId`, `sp_runtime::
/// AccountId32` under `MultiSignature`) is genuinely 32 raw bytes end-to-end, so this is a real
/// byte-identity conversion, not a placeholder — see `pallet_elections::AccountIdToBytes`'s doc
/// comment for why it's a pluggable Config item rather than a bare trait bound.
impl pallet_elections::AccountIdToBytes<AccountId> for Runtime {
	fn to_bytes(who: &AccountId) -> [u8; 32] {
		who.clone().into()
	}
}

/// Delegates to the same `OprfCommitteeKeys`-backed check `pallet_identity_zk::register_citizen`
/// performs on itself — see `pallet_identity_zk::Pallet::are_committee_keys_approved`'s doc
/// comment.
impl pallet_elections::CommitteeKeyChecker for Runtime {
	fn are_committee_keys_approved(
		scheme_version: u32,
		oprf_pk_hashes: &[[u8; 32]; pallet_elections::NUM_COMMITTEES],
	) -> bool {
		pallet_identity_zk::Pallet::<Runtime>::are_committee_keys_approved(
			scheme_version,
			oprf_pk_hashes,
		)
	}
}

/// Delegates to pallet-identity's own backing-commitment root history — see
/// `pallet_identity_zk::Pallet::is_valid_backing_commitment_root`'s doc comment.
impl pallet_elections::BackingRootChecker for Runtime {
	fn is_valid_backing_commitment_root(root: [u8; 32]) -> bool {
		pallet_identity_zk::Pallet::<Runtime>::is_valid_backing_commitment_root(root)
	}
}

impl pallet_elections::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// Hard cap on registered delegates.
	type MaxDelegates = ConstU32<10_000>;
	/// Bounds on_initialize's per-block delegate sweep (term warnings/expirations,
	/// break-endings) to a constant amount of work regardless of how many of the up-to-10,000
	/// `MaxDelegates` are actually registered — a full sweep completes within
	/// `MaxDelegates / MaxDelegateSweepPerBlock` = 100 blocks (~10-20 min at this chain's
	/// block time) in the worst case.
	type MaxDelegateSweepPerBlock = ConstU32<100>;
	/// Bounds `run_election`'s per-block ranking scan the same way — a full scan-and-seat
	/// cycle completes within `MaxDelegates / MaxElectionScanPerBlock` = 100 blocks in the
	/// worst case, instead of doing an unbounded `Delegates::iter()` + sort in one block.
	type MaxElectionScanPerBlock = ConstU32<100>;
	/// Flash-backing defense (see `pallet_elections::Config::MinBackingDurationBlocks`'s doc
	/// comment): a `BackingCount` checkpoint must be at least 30 days old before `run_election`
	/// will use it for seating. Matches `pallet_voting::Config::MinDelegationDurationBlocks`
	/// (same 30-day figure, same "stop a flash-style manipulation of a governance-weight
	/// signal" rationale), and stays comfortably below `DefaultElectionCycleBlocks` (2 years)
	/// so a checkpoint always has an opportunity to mature within a single election cycle.
	type MinBackingDurationBlocks = ConstU32<{ 30 * DAYS }>;
	type CitizenChecker = Runtime;
	/// Ordinary supermajority legislature motion can adjust BackingThreshold within bounds.
	type GovernanceOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
	/// Constitutional parameters require EnsureRoot for now. Production should wire this to
	/// a dedicated constitutional collective with a 2/3 supermajority threshold.
	type ConstitutionalOrigin = EnsureRoot<AccountId>;
	/// Seats the top-N backed delegates into pallet-legislature at each election.
	type LegislatureSeating = Legislature;
	/// Gates seating on a current pallet-anticorruption asset disclosure — an account without
	/// one is skipped in favor of the next-highest-backed eligible delegate (see
	/// `pallet_elections::DisclosureChecker`'s doc comment for the full rationale). Points at
	/// the real pallet-anticorruption implementation, not a no-op.
	type DisclosureChecker = PalletAntiCorruption;
	/// Gates seating on the reverse-direction legislature/Accountability-Council overlap bar —
	/// an account currently sitting on the Council is skipped in favor of the next-highest-
	/// backed eligible delegate (see `pallet_elections::AccountabilityCouncilChecker`'s doc
	/// comment). `pallet_accountability_council::add_member` only blocks the other direction
	/// (a current legislature member joining the Council) at join time; this closes the gap
	/// where a sitting Council member could later be automatically seated here.
	type AccountabilityCouncilChecker = AccountabilityCouncil;
	/// Real byte-identity `AccountId` conversion — see `AccountIdToBytes`'s impl comment above.
	/// Not dev-mode-gated: it performs no cryptographic verification, just a structural
	/// conversion, so there is nothing for dev-mode to stub out.
	type AccountIdToBytes = Runtime;
	/// Dev builds accept every `register_as_delegate` outer proof unconditionally
	/// (`PassthroughZkVerifier`); non-dev builds run the real bb 5.0.0 UltraHonk pairing check
	/// — the same verifier `pallet_identity_zk::Config::ZkVerifier` uses, since a delegate-
	/// persona proof is cryptographically just another outer ZKPassport proof.
	#[cfg(feature = "dev-mode")]
	type ZkVerifier = PassthroughZkVerifier;
	#[cfg(not(feature = "dev-mode"))]
	type ZkVerifier = crate::verifier::ZkPassportUltraHonkVerifier;
	/// Dev builds accept every delegate-persona commitment unconditionally
	/// (`PassthroughAnchorVerifier`); non-dev builds genuinely recompute and check the
	/// Poseidon2 `param_commitment` (`crate::anchor_verifier::Poseidon2AnchorVerifier`, see its
	/// `DelegatePersonaVerifier` impl).
	#[cfg(feature = "dev-mode")]
	type DelegatePersonaVerifier = PassthroughAnchorVerifier;
	#[cfg(not(feature = "dev-mode"))]
	type DelegatePersonaVerifier = crate::anchor_verifier::Poseidon2AnchorVerifier;
	/// Not dev-mode-gated: delegates to pallet-identity-zk's own real `OprfCommitteeKeys`
	/// storage check regardless of build mode, same as the two checkers below — there is no
	/// "passthrough" version of a storage lookup.
	type CommitteeKeyChecker = Runtime;
	/// Dev builds accept every `backing-nullifier` proof unconditionally
	/// (`PassthroughZkVerifier`); non-dev builds run the real standalone UltraHonk pairing
	/// check (`crate::backing_nullifier_verifier::BackingNullifierVerifier`).
	#[cfg(feature = "dev-mode")]
	type BackingProofVerifier = PassthroughZkVerifier;
	#[cfg(not(feature = "dev-mode"))]
	type BackingProofVerifier = crate::backing_nullifier_verifier::BackingNullifierVerifier;
	/// Not dev-mode-gated — see `CommitteeKeyChecker` above.
	type BackingRootChecker = Runtime;
	/// 100 legislature seats (constitutional, changeable via set_election_params).
	type DefaultLegislatureSeats = ConstU32<100>;
	/// 2-year election cycle: 2 * 365 * 7200 blocks at 12 s/block.
	type DefaultElectionCycleBlocks = ConstU32<{ 2 * 365 * DAYS }>;
	/// Each citizen may back at most 5 delegates simultaneously (constitutional).
	type DefaultMaxBackingsPerCitizen = ConstU32<5>;
	/// Governance-controlled parameter defaults — stored in storage and changeable by governance,
	/// but these values apply from genesis so the chain is functional without a governance vote.
	type DefaultBackingThreshold = ConstU32<10>;
	type DefaultBackingThresholdFloor = ConstU32<5>;
	type DefaultBackingThresholdCeiling = ConstU32<500>;
	/// 1-year term: 365 * 7200 blocks at 12 s/block.
	type DefaultTermLengthBlocks = ConstU32<{ 365 * DAYS }>;
	/// Delegates must take a break after 2 consecutive terms.
	type DefaultMaxConsecutiveTerms = ConstU32<2>;
	/// Mandatory break = 1 year.
	type DefaultMandatoryBreakBlocks = ConstU32<{ 365 * DAYS }>;
	/// Warn delegates when 10% of their term remains.
	type DefaultWarningWindowPct = ConstU8<10>;
	type WeightInfo = pallet_elections::weights::SubstrateWeight<Runtime>;
	/// See `RuntimeBenchmarkHelper`'s doc comment (defined above, next to
	/// `pallet_constitution::Config`'s `BenchmarkHelper` wiring).
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper = RuntimeBenchmarkHelper;
}

// ── Emergency Council ─────────────────────────────────────────────────────────

impl pallet_emergency_council::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// 30 days, constitutional ceiling. The pallet doc comment (and docs/project/pallets/
	/// emergency-council.md) say "432_000 (30 days at 6s/block)", but this runtime's actual
	/// block time is 12s (`MILLI_SECS_PER_BLOCK` in `runtime/src/lib.rs`), i.e. `DAYS` = 7_200
	/// blocks/day, not 14_400. Expressed via `DAYS` (as the rest of this file does for
	/// wall-clock-denominated constants) rather than a hardcoded literal so it stays correct
	/// if the block time ever changes; this correctly comes out to 216_000 blocks, not 432_000.
	type MaxEmergencyBlocks = ConstU32<{ 30 * DAYS }>;
	/// 7-day cooldown after an emergency ends (sunset expiry or early `vote_end_emergency`)
	/// before the council can declare another one. Without this, the same supermajority that
	/// declares an emergency could re-declare a fresh one the block after the previous one
	/// ends, chaining into de-facto indefinite emergency powers despite `MaxEmergencyBlocks`
	/// capping each individual window.
	type EmergencyCooldownBlocks = ConstU32<{ 7 * DAYS }>;
	/// Cross-pallet coordination with pallet-executive's own, independent cooldown — see
	/// `pallet_emergency_council::SiblingEmergencyCooldown`'s doc comment. Implemented on
	/// `Runtime` itself, just below, for the same reason `pallet_executive::Config::
	/// SiblingEmergencyCooldown` above is (a "Runtime-level delegating impl", needed because
	/// this relationship is symmetric and a direct pallet-to-pallet crate dependency in both
	/// directions would cycle).
	type SiblingEmergencyCooldown = Runtime;
	/// Council capped at 15 members (docs/project/pallets/emergency-council.md).
	type MaxCouncilSize = ConstU32<15>;
	/// 2/3 supermajority required to declare or end an emergency.
	type SupermajorityNumerator = ConstU32<2>;
	type SupermajorityDenominator = ConstU32<3>;
	type WeightInfo = pallet_emergency_council::weights::SubstrateWeight<Runtime>;
}

// ── Anti-Corruption module ───────────────────────────────────────────────────

/// Passthrough ZK verifier for the anti-corruption pallet (dev mode only).
/// In production, wire in the same ZKPassport UltraHonk verifier used by pallet-identity.
#[cfg(feature = "dev-mode")]
pub struct PassthroughAntiCorruptionZkVerifier;

#[cfg(feature = "dev-mode")]
impl pallet_anticorruption::ZkProofVerifier for PassthroughAntiCorruptionZkVerifier {
	fn verify(_proof_bytes: &[u8], _public_inputs: &[[u8; 32]]) -> bool {
		true
	}
}

#[cfg(feature = "dev-mode")]
impl pallet_anticorruption::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type ZkVerifier = PassthroughAntiCorruptionZkVerifier;
	/// Up to 20 designated investigators.
	type MaxInvestigators = ConstU32<20>;
	/// Asset disclosures must be renewed every ~1 year. Expressed via `DAYS` (block-time-derived)
	/// rather than a hardcoded literal — a bare `5_256_000` here was previously "~1 year at
	/// 6s/block", but this runtime's actual block time is 12s, making that literal ~2 years,
	/// contradicting docs/project/pallets/anticorruption.md's "mandatory annual renewal" claim.
	type AssetDisclosureRenewalBlocks = ConstU32<{ 365 * DAYS }>;
	/// Investigator appointment now requires the independent Accountability Council's own 2/3
	/// supermajority approval for the exact call, not bare `Root` — see
	/// `pallet_accountability_council`'s module doc comment and the "Accountability Council"
	/// section below for the self-oversight rationale.
	type AppointmentOrigin = pallet_accountability_council::EnsureAccountabilityCouncilApproved<Runtime>;
}

#[cfg(not(feature = "dev-mode"))]
pub struct ZkPassportAntiCorruptionZkVerifier;

#[cfg(not(feature = "dev-mode"))]
impl pallet_anticorruption::ZkProofVerifier for ZkPassportAntiCorruptionZkVerifier {
	fn verify(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> bool {
		// Reuse the same ZKPassport outer circuit that pallet-identity uses. Note this
		// inherits its fail-closed behaviour too — see `crate::verifier`'s module docs.
		// The whistleblower circuit this pallet eventually wants is a different circuit
		// anyway (HANDOFF item 8); this binding only keeps the two paths consistent.
		<crate::verifier::ZkPassportUltraHonkVerifier as pallet_identity_zk::ZkProofVerifier>::verify(
			proof_bytes,
			public_inputs,
		)
	}
}

#[cfg(not(feature = "dev-mode"))]
impl pallet_anticorruption::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type ZkVerifier = ZkPassportAntiCorruptionZkVerifier;
	type MaxInvestigators = ConstU32<20>;
	/// See the `dev-mode` impl above for the block-time rationale.
	type AssetDisclosureRenewalBlocks = ConstU32<{ 365 * DAYS }>;
	/// See the `dev-mode` impl above.
	type AppointmentOrigin = pallet_accountability_council::EnsureAccountabilityCouncilApproved<Runtime>;
}

// ── Accountability Council ───────────────────────────────────────────────────
//
// Independent oversight body governing pallet-audit auditor and pallet-anticorruption
// investigator appointment — see docs/project/pallets/accountability-council.md and
// `pallet_accountability_council`'s module doc comment for why this is a dedicated council
// rather than routed through pallet-legislature (self-oversight risk) or pallet-executive.
//
// Now wired as `pallet_audit::Config::AppointmentOrigin` / `pallet_anticorruption::Config::
// AppointmentOrigin` (both `= pallet_accountability_council::EnsureAccountabilityCouncilApproved
// <Runtime>`, see the two `impl ... ::Config for Runtime` blocks above): those pallets'
// `add_auditor`/`remove_auditor`/`add_investigator`/`remove_investigator` gained a genuine
// `EnsureOriginWithArg<Self::RuntimeOrigin, [u8; 32]>` associated type (previously bare
// `ensure_root` with no configurable `EnsureOrigin` at all), bound to the call-hash-domain-
// separated tags `pallet-audit::add_auditor`/`::remove_auditor` and
// `pallet-anticorruption::add_investigator`/`::remove_investigator` via
// `pallet_accountability_council::accountability_call_hash` (imported directly — pallet-audit
// and pallet-anticorruption both now depend on the pallet-accountability-council crate for
// this one shared hash function, so the two sides can never silently drift apart the way two
// independently-reimplemented hash functions could).

/// Runtime implements `pallet_accountability_council::LegislatureChecker` by reading
/// pallet-legislature's `Members` directly — same approach as the identical check
/// `pallet_executive::LegislatureMembership` already performs for the runtime.
impl pallet_accountability_council::LegislatureChecker<AccountId> for Runtime {
	fn is_legislature_member(who: &AccountId) -> bool {
		pallet_legislature::Members::<Runtime>::get().contains(who)
	}
}

/// Runtime implements `pallet_accountability_council::ExecutiveChecker` by reading
/// pallet-executive's minister/Prime-Minister storage directly — the same two storage items
/// `pallet_legislature::pallet::MinisterChecker for pallet_executive::Pallet<T>` checks
/// (`is_active_minister`), just read from `Runtime` instead of delegated to a pallet-local impl,
/// consistent with how every other cross-pallet "Checker" trait in this file is wired.
impl pallet_accountability_council::ExecutiveChecker<AccountId> for Runtime {
	fn is_active_minister(who: &AccountId) -> bool {
		pallet_executive::MinisterPortfolio::<Runtime>::contains_key(who)
			|| pallet_executive::PrimeMinister::<Runtime>::get().as_ref() == Some(who)
	}
}

impl pallet_accountability_council::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// 7-9 members, matching the Oracle Council's size range (see
	/// `pallet_courts::Config::MaxOracleMembers`).
	type MaxCouncilSize = ConstU32<9>;
	/// Genuine 2/3 supermajority — not the Oracle Council's simple >1/2 majority — for both
	/// membership changes (post-bootstrap) and any external action this Council approves.
	type SupermajorityNumerator = ConstU32<2>;
	type SupermajorityDenominator = ConstU32<3>;
	/// An unconsumed approved action can be discarded by any member after 14 days, mirroring
	/// pallet-legislature's `PendingApprovalExpiryBlocks` / pallet-courts'
	/// `AdminActionExpiryBlocks`.
	type ApprovalExpiryBlocks = ConstU32<{ 14 * DAYS }>;
	type LegislatureChecker = Runtime;
	type ExecutiveChecker = Runtime;
}
