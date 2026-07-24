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
use frame_system::{limits::{BlockLength, BlockWeights}, EnsureRoot, EnsureSignedBy};
use pallet_transaction_payment::{ConstFeeMultiplier, FungibleAdapter, Multiplier};
use sp_consensus_aura::sr25519::AuthorityId as AuraId;
use sp_runtime::{traits::{BlakeTwo256, Hash as HashT, One}, AccountId32, Perbill};
use sp_version::RuntimeVersion;

// Local module imports
use super::{
	AccountId, Aura, Balance, Balances, Block, BlockNumber, Hash, Nonce, PalletInfo, Runtime,
	RuntimeCall, RuntimeEvent, RuntimeFreezeReason, RuntimeHoldReason, RuntimeOrigin, RuntimeTask,
	System, DAYS, EXISTENTIAL_DEPOSIT, SLOT_DURATION, VERSION,
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

/// Block-hash-based randomness source wired into pallets via the `Randomness` trait.
///
/// Mixes the 5 most recent block hashes with the caller's subject bytes, making
/// manipulation require control of 5 consecutive authorship slots rather than one.
/// This is still NOT safe against a determined adversary with significant stake.
/// TODO: Replace with Babe/SASSAFRAS VRF randomness before any real deployment.
pub struct BlockHashRandomness;

impl frame_support::traits::Randomness<[u8; 32], BlockNumber> for BlockHashRandomness {
	fn random(subject: &[u8]) -> ([u8; 32], BlockNumber) {
		let current = frame_system::Pallet::<Runtime>::block_number();
		let mut entropy = [0u8; 32];
		for lag in 0u32..5 {
			let n = current.saturating_sub(lag);
			let h = frame_system::Pallet::<Runtime>::block_hash(n);
			for (i, b) in h.as_ref().iter().enumerate() {
				entropy[i % 32] ^= b;
			}
		}
		// Mix in subject bytes for domain separation so different callers get different seeds.
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
	/// TODO: replace with a court-controlled multisig origin once pallet-courts has a dedicated
	/// SuspensionOrigin council. Using root for now.
	type SuspensionOrigin = EnsureRoot<AccountId>;
	/// Merkle root allowlist management. Root for now; swap to a governance collective later.
	type AdminOrigin = EnsureRoot<AccountId>;
}

#[cfg(not(feature = "dev-mode"))]
impl pallet_identity_zk::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// Real Rarimo Groth16 BN254 verifier. Requires runtime/assets/vk_sha256.bin and
	/// vk_sha1.bin to be populated (see scripts/convert_vk.py).
	type ZkVerifier = crate::verifier::RarimoGroth16Verifier;
	type SuspensionOrigin = EnsureRoot<AccountId>;
	/// Merkle root allowlist management. Root for now; swap to a governance collective later.
	type AdminOrigin = EnsureRoot<AccountId>;
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

/// Runtime implements LawEnactor: when a referendum passes, enact an Ordinary law.
impl pallet_voting::LawEnactor for Runtime {
	fn enact_law(content_hash: [u8; 32]) -> sp_runtime::DispatchResult {
		pallet_constitution::Pallet::<Runtime>::enact_law_internal(
			pallet_constitution::LawTier::Ordinary,
			content_hash,
		)
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
	/// Simple majority required to pass.
	type PassageThreshold = ConstU8<51>;
	type LawEnactor = Runtime;
	type MACITallyVerifier = PassthroughMACIVerifier;
	/// Fiscal year start is a legislature motion — wired to the same origin as
	/// pallet-constitution's law-enactment gate so budget epochs are on-chain governed.
	type LegislatureOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
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
impl pallet_courts::CitizenSuspender for Runtime {
	fn suspend_citizen(nullifier: [u8; 32], until: Option<u32>) -> sp_runtime::DispatchResult {
		pallet_identity_zk::Pallet::<Runtime>::suspend_citizen_internal(
			nullifier,
			until.map(|u| u.into()),
		)
	}
}

impl pallet_courts::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// Citizens have 7 days to appeal an AI ruling.
	type AppealWindowBlocks = ConstU32<{ 7 * DAYS }>;
	type CitizenSelector = Runtime;
	type LawEnforcer = Runtime;
	type TreasuryEnforcer = Runtime;
	/// Oracle account stored in OracleAccount storage; set via set_oracle_account (root-only).
	type OracleOrigin = pallet_courts::EnsureOracle<Runtime>;
	type CitizenSuspender = Runtime;
	/// TODO: replace with Babe/SASSAFRAS VRF randomness before mainnet.
	type Randomness = BlockHashRandomness;
}

/// Runtime implements CitizenChecker for pallet-constitution (petition/sign gating).
impl pallet_constitution::CitizenChecker<AccountId> for Runtime {
	fn is_active_citizen(who: &AccountId) -> bool {
		pallet_identity_zk::Pallet::<Runtime>::is_active_citizen(who)
	}
}

/// Runtime implements PetitionApprover: when a petition hits threshold, create a referendum.
impl pallet_constitution::PetitionApprover for Runtime {
	fn create_referendum(petition_id: u32, topic_hash: [u8; 32]) -> sp_runtime::DispatchResult {
		pallet_voting::Pallet::<Runtime>::create_referendum_internal(petition_id, topic_hash)
	}
}

// ── HRC (Human Rights Commission) origin ────────────────────────────────────

/// The Human Rights Commission seat — currently a single well-known dev account (//Eve),
/// distinct from Alice/sudo. HRC may veto newly enacted laws within HRCVetoWindowBlocks.
///
/// TODO (mainnet): replace with a pallet-collective instance appointed by supermajority vote.
pub struct HrcCouncil;

impl frame_support::traits::SortedMembers<AccountId32> for HrcCouncil {
	fn sorted_members() -> alloc::vec::Vec<AccountId32> {
		// SR25519 "//Eve" public key (subkey inspect //Eve --scheme sr25519)
		alloc::vec![AccountId32::from([
			0xe6, 0x59, 0xa7, 0xa1, 0x62, 0x8c, 0xdd, 0x93,
			0xfe, 0xbc, 0x04, 0xa4, 0xe0, 0x64, 0x6e, 0xa2,
			0x0e, 0x9f, 0x5f, 0x0c, 0xe0, 0x97, 0xd9, 0xa0,
			0x52, 0x90, 0xd4, 0xa9, 0xe0, 0x54, 0xdf, 0x4e,
		])]
	}
}

impl pallet_constitution::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// Constitutional amendments require 30 days of deliberation before ratification.
	type ConstitutionalDeliberationBlocks = ConstU32<{ 30 * DAYS }>;
	type LegislatureOrigin = pallet_legislature::EnsureLegislatureMotion<Runtime>;
	/// 1 000 citizen signatures required to trigger a referendum.
	type PetitionThreshold = ConstU32<1_000>;
	type PetitionApprover = Runtime;
	type CitizenChecker = Runtime;
	/// Ordinary law amendments take effect immediately (no deliberation window).
	type OrdinaryAmendmentDeliberationBlocks = ConstU32<0>;
	/// HRC veto: the //Eve dev account acts as the HRC seat.
	/// TODO (mainnet): replace with a pallet-collective HRC instance.
	type HumanRightsOrigin = EnsureSignedBy<HrcCouncil, AccountId>;
	/// HRC has 14 days to veto a newly enacted law on human rights grounds.
	type HRCVetoWindowBlocks = ConstU32<{ 14 * DAYS }>;
	/// Courts origin for invalidate_law. EnsureRoot for now; swap to a dedicated
	/// pallet-courts origin once that pallet exposes a standalone origin type.
	type CourtOrigin = EnsureRoot<AccountId>;
}

impl pallet_emergency_council::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// 30 days at 6s/block — constitutionally hard-coded ceiling on emergency duration.
	type MaxEmergencyBlocks = ConstU32<432_000>;
	/// Emergency Council may have at most 15 members.
	type MaxCouncilSize = ConstU32<15>;
	/// Supermajority numerator: 2 (for 2/3 majority).
	type SupermajorityNumerator = ConstU32<2>;
	/// Supermajority denominator: 3 (for 2/3 majority).
	type SupermajorityDenominator = ConstU32<3>;
}

impl pallet_legislature::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	/// Legislature has at most 500 seats.
	type MaxMembers = ConstU32<500>;
	/// Members have 7 days to vote on a motion.
	type MotionDurationBlocks = ConstU32<{ 7 * DAYS }>;
	/// Simple majority (50%+1) required to pass a motion.
	type PassageThreshold = ConstU8<50>;
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
	/// Candidate deposit: 1 AGR token (1_000_000_000_000 planck). Refunded after election certified.
	type CandidateDeposit = ConstU128<1_000_000_000_000>;
	/// Up to 20 commissioners on the Elections Commission.
	type MaxCommissioners = ConstU32<20>;
	/// Up to 100 candidates per election.
	type MaxCandidatesPerElection = ConstU32<100>;
	/// Use the chain's native Balances pallet for deposits.
	type Currency = Balances;
	/// Citizen eligibility gated on active passport registration.
	type CitizenChecker = Runtime;
}
