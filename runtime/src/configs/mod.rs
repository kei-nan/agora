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
	traits::{ConstBool, ConstU128, ConstU32, ConstU64, ConstU8, VariantCountOf},
	weights::{
		constants::{RocksDbWeight, WEIGHT_REF_TIME_PER_SECOND},
		IdentityFee, Weight,
	},
};
use frame_system::{limits::{BlockLength, BlockWeights}, EnsureRoot};
use pallet_transaction_payment::{ConstFeeMultiplier, FungibleAdapter, Multiplier};
use sp_consensus_aura::sr25519::AuthorityId as AuraId;
use sp_runtime::{traits::{BlakeTwo256, Hash as HashT, One}, AccountId32, Perbill};
use sp_version::RuntimeVersion;

// Local module imports
use super::{
	AccountId, Aura, Balance, Balances, Block, BlockNumber, Cabinet, Hash, Legislature, Nonce, PalletInfo,
	Runtime, RuntimeCall, RuntimeEvent, RuntimeFreezeReason, RuntimeHoldReason, RuntimeOrigin,
	RuntimeTask, System, DAYS, EXISTENTIAL_DEPOSIT, SLOT_DURATION, VERSION,
};

const NORMAL_DISPATCH_RATIO: Perbill = Perbill::from_percent(75);

parameter_types! {
	pub const BlockHashCount: BlockNumber = 2400;
	pub const Version: RuntimeVersion = VERSION;

	/// We allow for 2 seconds of compute with a 6 second average block time.
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

/// Block-hash-based randomness for jury selection.
/// Mixes the last 81 block hashes (matching the official pallet_insecure_randomness_collective_flip
/// history length) with the subject bytes for domain separation.
/// Still vulnerable to block-author manipulation — replace with BABE/SASSAFRAS VRF before mainnet.
/// (pallet-insecure-randomness-collective-flip cannot be added as a dep because it transitively
/// requires an older sp-io version incompatible with our frame-support 40.x stack.)
pub struct BlockHashRandomness;

impl frame_support::traits::Randomness<[u8; 32], BlockNumber> for BlockHashRandomness {
	fn random(subject: &[u8]) -> ([u8; 32], BlockNumber) {
		let current = frame_system::Pallet::<Runtime>::block_number();
		let mut entropy = [0u8; 32];
		for lag in 0u32..81 {
			let n = current.saturating_sub(lag);
			let h = frame_system::Pallet::<Runtime>::block_hash(n);
			for (i, b) in h.as_ref().iter().enumerate() {
				entropy[i % 32] ^= b;
			}
		}
		for (i, b) in subject.iter().enumerate() {
			entropy[i % 32] ^= b;
		}
		let out_hash = BlakeTwo256::hash(&entropy);
		let mut out = [0u8; 32];
		out.copy_from_slice(out_hash.as_ref());
		(out, current)
	}
}

// ── Agora pallets ────────────────────────────────────────────────────────────

/// Passthrough ZK verifier: accepts any proof during development.
/// Gated behind `dev-mode` feature — a production build without that feature will
/// fail to compile here, forcing a real Rarimo Groth16 verifier to be wired in.
#[cfg(feature = "dev-mode")]
pub struct PassthroughZkVerifier;

#[cfg(feature = "dev-mode")]
impl pallet_identity_zk::ZkProofVerifier for PassthroughZkVerifier {
	fn verify(_proof_bytes: &[u8], _public_inputs: &[[u8; 32]]) -> bool {
		true
	}
}

#[cfg(feature = "dev-mode")]
impl pallet_identity_zk::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type ZkVerifier = PassthroughZkVerifier;
	/// Court oracle may manually suspend citizens (auto-path uses suspend_citizen_internal via
	/// CitizenSuspender trait; this extrinsic is an explicit administrative override).
	type SuspensionOrigin = pallet_courts::EnsureOracle<Runtime>;
	/// Merkle root allowlist updates require a legislature vote.
	type AdminOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
}

#[cfg(not(feature = "dev-mode"))]
impl pallet_identity_zk::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// Real Rarimo Groth16 BN254 verifier. Requires runtime/assets/vk_sha256.bin and
	/// vk_sha1.bin to be populated (see scripts/convert_vk.py).
	type ZkVerifier = crate::verifier::RarimoGroth16Verifier;
	/// Court oracle may manually suspend citizens (auto-path uses suspend_citizen_internal via
	/// CitizenSuspender trait; this extrinsic is an explicit administrative override).
	type SuspensionOrigin = pallet_courts::EnsureOracle<Runtime>;
	/// Merkle root allowlist updates require a legislature vote.
	type AdminOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
}

/// Passthrough MACI tally verifier — accepts all proofs.
/// TODO: replace with the real MACI circuit verifier once trusted setup is complete.
pub struct PassthroughMACIVerifier;
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
	type MACITallyVerifier = PassthroughMACIVerifier;
	/// Fiscal year start is a legislature motion — wired to the same origin as
	/// pallet-constitution's law-enactment gate so budget epochs are on-chain governed.
	type LegislatureOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
	/// Voting epochs must be at least 7 days long.
	type MinEpochDurationBlocks = ConstU32<{ 7 * DAYS }>;
	/// Voting epochs may not exceed 30 days (constitutional ceiling).
	type MaxEpochDurationBlocks = ConstU32<{ 30 * DAYS }>;
}

/// Type alias for the audit pallet used in cross-pallet trait wiring.
/// The canonical `PalletAudit` alias for `construct_runtime!` lives in `runtime/src/lib.rs`.
type PalletAuditImpl = pallet_audit::Pallet<Runtime>;

impl pallet_audit::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// At most 10 registered auditors.
	type MaxAuditors = ConstU32<10>;
}

impl pallet_treasury_ledger::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type Balance = Balance;
	/// Wire the audit pallet as the expenditure hook.
	type AuditHook = PalletAuditImpl;
	/// Budget allocation requires a passed legislature motion, not just sudo.
	type LegislatureOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
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

/// Runtime implements CitizenChecker for pallet-courts (file_case active-citizen gate).
impl pallet_courts::CitizenChecker<AccountId> for Runtime {
	fn is_active_citizen(who: &AccountId) -> bool {
		pallet_identity_zk::Pallet::<Runtime>::is_active_citizen(who)
	}
}

/// Runtime implements LawEnforcer by calling pallet-constitution's internal function.
impl pallet_courts::LawEnforcer for Runtime {
	fn invalidate_law(law_id: u32) -> sp_runtime::DispatchResult {
		pallet_constitution::Pallet::<Runtime>::invalidate_law_internal(law_id)
	}
}

/// Runtime implements TreasuryEnforcer by calling pallet-treasury-ledger's internal function.
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
	) -> sp_runtime::DispatchResult {
		pallet_identity_zk::Pallet::<Runtime>::suspend_citizen_internal(nullifier, suspension_until)
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
	/// Oracle account stored in OracleAccount storage; set via set_oracle_account (root-only).
	type OracleOrigin = pallet_courts::EnsureOracle<Runtime>;
	type CitizenSuspender = Runtime;
	/// Mixes 81 historical block hashes; still insecure against block-author manipulation.
	/// Replace with BABE/SASSAFRAS VRF before mainnet.
	type Randomness = BlockHashRandomness;
	/// Zero account used as filer for system-initiated LawChallenge cases.
	type AutoChallengeAccount = AutoChallengeAccount;
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

/// Runtime implements FreshLegislatureChecker by reading LastElectionBlock from pallet-elections.
impl pallet_constitution::FreshLegislatureChecker<BlockNumber> for Runtime {
	fn has_election_occurred_since(proposed_at: BlockNumber) -> bool {
		pallet_elections::LastElectionBlock::<Runtime>::get() > proposed_at
	}
}

impl pallet_constitution::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type LegislatureOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
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
	/// Courts origin for invalidate_law (manual override). The auto-enforcement path
	/// uses invalidate_law_internal via the LawEnforcer trait.
	type CourtOrigin = pallet_courts::EnsureOracle<Runtime>;
}


impl pallet_legislature::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// Legislature has at most 500 seats.
	type MaxMembers = ConstU32<500>;
	/// Members have 7 days to vote on a motion.
	type MotionDurationBlocks = ConstU32<{ 7 * DAYS }>;
	/// Simple majority (50%+1) required to pass a motion.
	type PassageThreshold = ConstU8<50>;
	/// Active ministers are blocked from legislature votes (incompatibility rule).
	type MinisterChecker = Cabinet;
}

// ── Parliamentary Executive ──────────────────────────────────────────────────

impl pallet_executive::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// Only a passed legislature motion can appoint/dismiss ministers or ratify an emergency.
	type LegislatureOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
	/// Maximum 20 cabinet portfolios.
	type MaxPortfolios = ConstU32<20>;
	/// 30 days at 6s/block — constitutional ceiling on emergency duration.
	type MaxEmergencyBlocks = ConstU32<432_000>;
	/// Legislature has 72 hours (at 6s/block) to ratify after cabinet declares emergency.
	type RatificationWindowBlocks = ConstU32<{ 3 * DAYS }>;
	/// 2/3 cabinet supermajority required to declare or end an emergency.
	type SupermajorityNumerator = ConstU32<2>;
	type SupermajorityDenominator = ConstU32<3>;
}

// ── Elections Commission ─────────────────────────────────────────────────────

/// Runtime implements pallet_elections::CitizenChecker by delegating to pallet-identity.
impl pallet_elections::CitizenChecker<AccountId> for Runtime {
	fn is_active_citizen(who: &AccountId) -> bool {
		pallet_identity_zk::Pallet::<Runtime>::is_active_citizen(who)
	}
}

impl pallet_elections::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type CandidateDeposit = ConstU128<1_000_000_000_000>;
	type MaxCommissioners = ConstU32<20>;
	type MaxCandidatesPerElection = ConstU32<100>;
	/// Hard cap on registered delegates; bounds on_initialize iteration.
	type MaxDelegates = ConstU32<10_000>;
	type Currency = Balances;
	type CitizenChecker = Runtime;
	/// Ordinary supermajority legislature motion can adjust BackingThreshold within bounds.
	type GovernanceOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
	/// Constitutional supermajority (via legislature) gates term-limit parameters and bounds.
	type ConstitutionalOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
	/// Seats the top-N backed delegates into pallet-legislature at each election.
	type LegislatureSeating = Legislature;
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
}

// ── Anti-Corruption module ───────────────────────────────────────────────────

/// Passthrough ZK verifier for the anti-corruption pallet (dev mode only).
/// In production, wire in the same Rarimo Groth16 verifier used by pallet-identity.
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
	/// Asset disclosures must be renewed every ~1 year (5_256_000 blocks at 6s/block).
	type AssetDisclosureRenewalBlocks = ConstU32<5_256_000>;
}

#[cfg(not(feature = "dev-mode"))]
pub struct RarimoAntiCorruptionZkVerifier;

#[cfg(not(feature = "dev-mode"))]
impl pallet_anticorruption::ZkProofVerifier for RarimoAntiCorruptionZkVerifier {
	fn verify(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> bool {
		// Reuse the same Rarimo Groth16 circuit that pallet-identity uses.
		<crate::verifier::RarimoGroth16Verifier as pallet_identity_zk::ZkProofVerifier>::verify(
			proof_bytes,
			public_inputs,
		)
	}
}

#[cfg(not(feature = "dev-mode"))]
impl pallet_anticorruption::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type ZkVerifier = RarimoAntiCorruptionZkVerifier;
	type MaxInvestigators = ConstU32<20>;
	type AssetDisclosureRenewalBlocks = ConstU32<5_256_000>;
}
