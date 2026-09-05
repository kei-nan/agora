//! # Voting Pallet
//!
//! Three participation systems:
//! 1. MACI 1p1v (`commit_vote` / `submit_maci_tally`) — receipt-free anonymous voting for laws
//!    and elections. Votes are opaque encrypted commitments keyed by nullifier; the actual
//!    yes/no tally is computed off-chain and accepted on-chain only alongside a ZK proof
//!    (`Config::MACITallyVerifier`) attesting it is the correct decryption of those commitments.
//! 2. Referenda (`vote_referendum` / `finalize_referendum`) — plaintext, directly-signed 1p1v
//!    voting on a specific referendum, tallied on-chain in `ReferendumTally`.
//! 3. Budget QV — quadratic budget token allocation for fiscal priorities.
//!
//! ## Liquid democracy delegation
//! `Delegations` is wired into system 2 (Referenda) only: `finalize_referendum` calls
//! `apply_delegated_weight`, which adds a non-voting delegator's weight to whichever side their
//! (transitively resolved) delegate voted, if that delegate voted directly. See
//! `apply_delegated_weight`'s and `topic_id_of`'s doc comments for exactly how, and for the
//! known "topic" limitation.
//!
//! Delegation is deliberately **not** resolved on-chain for system 1 (MACI). MACI's whole point
//! is that `VoteCommitments` are opaque — an observer should not be able to tell what a
//! commitment decrypts to, or which encrypted commitments any two accounts' votes have any
//! relationship to. Cross-referencing the plaintext, publicly-readable `Delegations` graph
//! (which links real `AccountId`s to each other) against MACI commitments on-chain would leak
//! exactly the linkage MACI exists to hide — "these N encrypted commitments are all the same
//! block of delegated voting power" is itself a receipt. Real delegation-aware MACI tallying has
//! to happen off-chain, inside whatever computes `yes_votes`/`no_votes` before producing the ZK
//! proof passed to `submit_maci_tally` — i.e. inside a MACI coordinator service, which does not
//! exist yet in this repo (see `CLAUDE.md`'s "Remaining Work": no real MACI circuit verifier is
//! wired in either, `MACITallyVerifier` is fail-closed outside `dev-mode` — search
//! `FailClosedMACIVerifier` in `runtime/src/configs/mod.rs`). Tracked as a follow-up, not
//! attempted here.
//!
//! Suspended citizens are excluded from all systems via `Config::CitizenChecker::is_active_citizen`,
//! checked at the start of every citizen-facing call (including, for delegated weight, the
//! delegator — see `apply_delegated_weight`). Delegations also carry a bounded `expires_at`
//! ceiling (`MaxDelegationDurationBlocks`), so a suspended citizen's existing delegation lapses
//! on its own rather than persisting indefinitely.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {

    use codec::DecodeWithMemTracking;
    use frame_support::pallet_prelude::*;
    use frame_support::traits::EnsureOriginWithArg;
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::Saturating;

    /// Computes the domain-separated hash a legislature motion's `call_hash` must equal for
    /// `LegislatureOrigin` to authorize `tag`'s call with `params`. See
    /// `pallet_constitution::pallet::legislature_call_hash` for the full rationale.
    pub(crate) fn legislature_call_hash(tag: &'static [u8], params: impl Encode) -> [u8; 32] {
        let mut preimage = alloc::vec::Vec::from(tag);
        preimage.extend(params.encode());
        frame_support::Hashable::blake2_256(&preimage)
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// Cross-pallet hook: pallet-voting asks pallet-identity whether a citizen is
    /// both registered and has no active court-ordered suspension.
    pub trait CitizenChecker<AccountId> {
        fn is_active_citizen(who: &AccountId) -> bool;
        /// Total number of registered citizens. Used for percentage-based delegation cap.
        fn total_citizens() -> u32;
    }

    /// Returns the nullifier for a registered citizen, or None if not registered.
    /// Implemented by the runtime by reading CitizenNullifier from pallet-identity.
    pub trait NullifierProvider<AccountId> {
        fn nullifier_of(who: &AccountId) -> Option<[u8; 32]>;
    }

    /// Called by pallet-voting when a referendum passes — enacts the law in pallet-constitution.
    /// The tier is forwarded so pallet-constitution can record the correct law tier.
    pub trait LawEnactor {
        fn enact_law(tier: ReferendumTier, content_hash: [u8; 32]) -> DispatchResult;
    }

    /// Verifies an off-chain MACI tally ZK proof. The proof attests that
    /// (yes_votes, no_votes) is the correct decryption of all encrypted
    /// vote commitments whose Merkle root is commitment_root.
    pub trait MACITallyVerifier {
        fn verify_tally(
            proposal_id: u32,
            yes_votes: u64,
            no_votes: u64,
            commitment_root: [u8; 32],
            proof_bytes: &[u8],
        ) -> bool;
    }

    /// Stored per (delegator, topic_id). The delegation is invalid once `expires_at` is past.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct DelegationRecord<AccountId, BlockNumber> {
        pub delegate: AccountId,
        /// Delegation is valid for referenda whose close block ≤ expires_at.
        /// After this block the record is treated as absent and cleaned up lazily.
        pub expires_at: BlockNumber,
        /// The weight this delegation edge currently contributes to its resolved terminal
        /// delegate's `DelegatedWeight` entry (this delegator's own vote, plus whatever weight
        /// this delegator had itself accumulated as a terminal delegate for others before
        /// delegating onward). Snapshotted here — rather than recomputed on demand — so
        /// `revoke_delegation` and lazy expiry cleanup can remove *exactly* this amount later
        /// without drift; kept live thereafter by `propagate_weight_along_chain`, which updates
        /// this field on every account still along an affected chain whenever weight joins or
        /// leaves upstream of it (a later re-target trusts this field as-is, so it must stay
        /// current, not just accurate at creation time — see `DelegatedWeight`'s doc comment).
        pub resolved_weight: u32,
    }

    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum ReferendumState {
        Voting,
        Passed,
        Failed,
    }

    /// Which tier of law a referendum would enact.
    /// Each tier requires a higher passage threshold than the one before it.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum ReferendumTier {
        /// Simple majority (PassageThreshold %).
        Ordinary,
        /// Structural supermajority (ConstitutionalPassageThreshold %, e.g. 67%).
        Constitutional,
        /// Foundational supermajority (FoundationalPassageThreshold %, e.g. 75%).
        /// Only the legislature can open a Foundational referendum; no petition path.
        Foundational,
    }

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Max percentage of total voting power any single delegate may hold (0–100).
        #[pallet::constant]
        type DelegationCap: Get<u8>;
        /// Absolute max direct delegators any single delegate can hold per topic.
        #[pallet::constant]
        type MaxDelegationsPerDelegate: Get<u32>;
        /// Max chain depth when following transitive delegations (prevents O(n) DoS).
        #[pallet::constant]
        type MaxDelegationDepth: Get<u8>;
        /// Number of budget categories citizens can allocate tokens across.
        #[pallet::constant]
        type BudgetCategoryCount: Get<u32>;
        /// Minimum delegation duration in blocks. Prevents instant-expiry gaming.
        #[pallet::constant]
        type MinDelegationDurationBlocks: Get<u32>;
        /// Maximum delegation duration in blocks. Constitutional ceiling on how long
        /// a single delegation can run without renewal (prevents indefinite concentration).
        #[pallet::constant]
        type MaxDelegationDurationBlocks: Get<u32>;
        /// Minimum proposal duration in blocks. Prevents instantly-expired proposals.
        #[pallet::constant]
        type MinProposalDurationBlocks: Get<u32>;
        /// Maximum proposal duration in blocks. Prevents unbounded proposals.
        #[pallet::constant]
        type MaxProposalDurationBlocks: Get<u32>;
        /// Gate: returns false if the account is not a registered citizen or is suspended.
        type CitizenChecker: CitizenChecker<Self::AccountId>;
        /// Looks up a citizen's nullifier from pallet-identity storage.
        /// Returns None if the account has no registered nullifier.
        type NullifierProvider: NullifierProvider<Self::AccountId>;
        /// How long a referendum voting window stays open in blocks (e.g. 14 * DAYS).
        #[pallet::constant]
        type ReferendumDurationBlocks: Get<u32>;
        /// Percentage of yes votes required to pass an ordinary referendum (0–100).
        #[pallet::constant]
        type PassageThreshold: Get<u8>;
        /// Percentage of yes votes required to pass a constitutional (Structural-tier) referendum.
        /// Must be higher than PassageThreshold (e.g. 67 for 2/3 supermajority).
        #[pallet::constant]
        type ConstitutionalPassageThreshold: Get<u8>;
        /// Percentage of yes votes required to pass a foundational referendum (0–100).
        /// Must be higher than ConstitutionalPassageThreshold (e.g. 75 for 3/4 supermajority).
        #[pallet::constant]
        type FoundationalPassageThreshold: Get<u8>;
        /// Hook called when a referendum passes — enacts the law in pallet-constitution.
        type LawEnactor: LawEnactor;
        /// Verifier for MACI tally ZK proofs. Use PassthroughMACIVerifier in dev; wire in the
        /// real MACI verifier once circuit trusted setup is complete.
        type MACITallyVerifier: MACITallyVerifier;
        /// The origin permitted to start a new fiscal year (open a new budget epoch), submit
        /// MACI tallies, open voting epochs, and create constitutional/foundational
        /// referenda directly. Wired to the legislature motion origin so only a passed
        /// legislature vote can do these. `EnsureOriginWithArg` so each call site must pass
        /// the domain-separated hash of its own parameters (see `legislature_call_hash`);
        /// the origin then verifies that hash against the motion's approved `call_hash`, so
        /// a motion passed to authorize one call can never be replayed to execute another.
        /// Use EnsureRoot during development.
        type LegislatureOrigin: frame_support::traits::EnsureOriginWithArg<Self::RuntimeOrigin, [u8; 32]>;
        /// Minimum duration of a voting epoch in blocks.
        #[pallet::constant]
        type MinEpochDurationBlocks: Get<u32>;
        /// Maximum duration of a voting epoch in blocks (constitutional ceiling).
        #[pallet::constant]
        type MaxEpochDurationBlocks: Get<u32>;
        /// Maximum number of referenda whose automatic finalization can be scheduled for the
        /// same block via `PendingFinalization`. Bounds `on_initialize`'s worst-case per-block
        /// cost. If more referenda than this happen to share a finalization block, the excess
        /// simply aren't auto-scheduled — they still finalize via the permissionless
        /// `finalize_referendum` extrinsic fallback, just without the automatic close-the-gap
        /// guarantee `on_initialize` otherwise provides. See `PendingFinalization`'s doc comment.
        #[pallet::constant]
        type MaxReferendaPerBlock: Get<u32>;
        /// Bounds `OpenReferenda`, the flat list of referendum ids currently in
        /// `ReferendumState::Voting`. Exists so `RecoveryStateChecker::has_open_referendum_vote`
        /// (`pallet-identity-zk`) can check "has this account voted in any currently-open
        /// referendum" without an unbounded scan over `Referenda` (which grows with the total
        /// referendum count ever created, not just currently-open ones). Referendum creation is
        /// low-frequency (gated by a legislature motion or a petition threshold) and only
        /// referenda created within one `ReferendumDurationBlocks` window can be concurrently
        /// open, so this cap is not expected to bind in practice.
        ///
        /// Unlike `MaxReferendaPerBlock`/`PendingFinalization` above — where overflow is safe to
        /// silently skip because the permissionless `finalize_referendum` extrinsic is still a
        /// correct fallback — there is no equivalent fallback here: a referendum silently
        /// dropped from this list would become invisible to the recovery guard, reopening the
        /// double-vote gap it exists to close. So referendum creation itself fails outright
        /// (`Error::TooManyOpenReferenda`) if adding to this list would overflow the bound,
        /// rather than silently proceeding without tracking it.
        #[pallet::constant]
        type MaxConcurrentReferenda: Get<u32>;
    }

    // ── 1p1v / MACI storage ─────────────────────────────────────────────────

    /// Active proposals: proposal_id -> (end_block, topic_hash, tier).
    /// topic_hash and tier are needed by submit_maci_tally to enact the law on pass.
    #[pallet::storage]
    pub type Proposals<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, (BlockNumberFor<T>, [u8; 32], ReferendumTier)>;

    /// Per-proposal vote commitments (MACI-style): (proposal_id, nullifier) -> commitment.
    #[pallet::storage]
    pub type VoteCommitments<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, [u8; 32]), [u8; 32]>;

    /// Per-topic delegation: topic_id -> delegator -> DelegationRecord.
    /// Expired records (expires_at < now) are treated as absent and cleaned up lazily.
    /// Keyed topic-first (not delegator-first) so `apply_delegated_weight` can scan just one
    /// topic's delegators via `iter_prefix(topic_id)` instead of every delegation on every
    /// topic — see that function's doc comment for why an unbounded full-table scan there is a
    /// real weight/DoS concern, not just a style preference.
    #[pallet::storage]
    pub type Delegations<T: Config> = StorageDoubleMap<
        _,
        Blake2_128Concat,
        u32,
        Blake2_128Concat,
        T::AccountId,
        DelegationRecord<T::AccountId, BlockNumberFor<T>>,
    >;

    /// Number of direct delegators per (topic_id, delegate).
    #[pallet::storage]
    pub type DelegatorCount<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, T::AccountId), u32, ValueQuery>;

    /// Per-topic, transitively-resolved voting weight currently anchored at an account —
    /// i.e. how many citizens' votes would be credited to this account if it cast a direct
    /// vote, counting whole delegation chains that terminate here, not just direct fan-in.
    ///
    /// Invariant: any account with an active outgoing delegation on this topic always has 0
    /// here — its incoming weight is immediately re-attributed to whatever it resolves to
    /// instead, so only current *terminal* delegates ever carry a nonzero balance. Maintained
    /// incrementally by `delegate_vote`, `revoke_delegation`, and `has_delegation_cycle`'s
    /// lazy expiry cleanup, all via the shared `propagate_weight_along_chain` helper — see
    /// `delegate_vote`'s doc comment for exactly how. Every weight change (a new upstream join,
    /// a departure, a re-target) is walked and applied to *every* intermediate hop's own stored
    /// `DelegationRecord::resolved_weight` along the affected chain, not just the terminal's
    /// bucket here — this is what lets an intermediate delegate that later re-targets to a
    /// *different* new delegate trust its own stored snapshot as accurate, closing what used to
    /// be a silent `DelegationCap` bypass (an upstream account joining *after* an intermediate's
    /// own outgoing edge already existed left that intermediate's snapshot stale until it next
    /// re-targeted, understating the weight it actually carried).
    ///
    /// Used solely to enforce `DelegationCap` at delegation time. The actual vote tally in
    /// `apply_delegated_weight` never reads this — it always re-resolves the real
    /// `Delegations` graph fresh from scratch, so tallying correctness never depends on this
    /// cache staying perfectly accurate.
    #[pallet::storage]
    pub type DelegatedWeight<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, T::AccountId), u32, ValueQuery>;

    /// Next proposal id counter.
    #[pallet::storage]
    pub type NextProposalId<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Finalized MACI tally: proposal_id -> (yes_votes, no_votes, commitment_root).
    /// Set once per proposal by submit_maci_tally after the voting window closes.
    #[pallet::storage]
    pub type ProposalResults<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, (u64, u64, [u8; 32])>;

    // ── Budget QV storage ────────────────────────────────────────────────────

    /// Current fiscal year epoch. Incremented by start_fiscal_year.
    #[pallet::storage]
    pub type FiscalYearEpoch<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Budget tokens allocated per citizen for a given epoch.
    #[pallet::storage]
    pub type EpochTokenAllocation<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, u64>;

    /// Last epoch a citizen has claimed their budget tokens.
    #[pallet::storage]
    pub type CitizenClaimedEpoch<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32>;

    /// Remaining unspent budget tokens for a citizen in the current epoch.
    #[pallet::storage]
    pub type BudgetBalance<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u64, ValueQuery>;

    /// Quadratic votes cast: (account, epoch, category_id) -> vote_count.
    /// Token cost for this slot = vote_count². Refundable by reducing vote_count.
    #[pallet::storage]
    pub type CategoryVotes<T: Config> =
        StorageMap<_, Blake2_128Concat, (T::AccountId, u32, u32), u32, ValueQuery>;

    // ── Referendum storage ───────────────────────────────────────────────────

    /// referendum_id -> (petition_id, topic_hash, end_block, state, tier).
    #[pallet::storage]
    pub type Referenda<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, (u32, [u8; 32], BlockNumberFor<T>, ReferendumState, ReferendumTier)>;

    /// petition_id -> referendum_id. Prevents duplicate referenda for the same petition.
    #[pallet::storage]
    pub type PetitionReferendum<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, u32>;

    /// Running yes/no tally: referendum_id -> (yes_count, no_count).
    #[pallet::storage]
    pub type ReferendumTally<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, (u32, u32), ValueQuery>;

    /// Tracks which accounts have voted in which referendum (one vote per citizen).
    #[pallet::storage]
    pub type ReferendumHasVoted<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, T::AccountId), bool, ValueQuery>;

    /// Records each direct voter's `in_favor` choice: (referendum_id, voter) -> in_favor.
    /// Only meaningful when `ReferendumHasVoted` is true for the same key. Read by
    /// `apply_delegated_weight` at finalize time to know which side a delegate's transitive
    /// delegators' weight should be added to.
    #[pallet::storage]
    pub type ReferendumVoteChoice<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, T::AccountId), bool, ValueQuery>;

    #[pallet::storage]
    pub type NextReferendumId<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Schedules automatic finalization: block_number -> referendum ids that should be
    /// finalized (via the same logic `finalize_referendum` runs) during that block's
    /// `on_initialize`. Populated whenever a referendum's `end_block` is set (referendum
    /// creation, at all three call sites), keyed by `end_block + 1` — the first block at which
    /// `finalize_referendum`'s own `now > end_block` gate would allow finalization.
    ///
    /// Exists to close the gap the permissionless, never-automatically-called
    /// `finalize_referendum` extrinsic otherwise leaves open: without this, a referendum could
    /// sit finalized-but-not-finalized for an unbounded number of blocks after its voting window
    /// closes, during which an attacker who has seen the (already publicly readable) tally could
    /// delegate to a winning-side voter or revoke a delegation to a losing-side voter and change
    /// the outcome after the fact. Scheduling here plus the `on_initialize` hook that drains it
    /// closes that window to effectively zero: finalization now runs automatically in the very
    /// next block after voting closes, before any of that block's extrinsics (including a
    /// same-block delegation change) get to execute.
    ///
    /// Bounded per block by `MaxReferendaPerBlock` to keep `on_initialize`'s worst-case cost
    /// bounded; if a block's list is full, the referendum simply isn't auto-scheduled and instead
    /// relies on the permissionless `finalize_referendum` extrinsic as a fallback (still a much
    /// smaller window than before this fix, since referenda sharing an exact finalization block
    /// are the only ones that can overflow it).
    #[pallet::storage]
    pub type PendingFinalization<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        BlockNumberFor<T>,
        BoundedVec<u32, T::MaxReferendaPerBlock>,
        ValueQuery,
    >;

    /// Flat bounded list of referendum ids currently in `ReferendumState::Voting` — i.e.
    /// currently open for voting. Maintained by all three referendum-creation call sites
    /// (`create_constitutional_referendum`, `create_foundational_referendum`,
    /// `create_referendum_internal`, each of which pushes on creation) and by
    /// `do_finalize_referendum` (which removes an id when the referendum's state transitions
    /// out of `Voting`). See `Config::MaxConcurrentReferenda`'s doc comment for why this exists
    /// and why, unlike `PendingFinalization`, it fails closed on overflow instead of silently
    /// dropping entries.
    #[pallet::storage]
    pub type OpenReferenda<T: Config> =
        StorageValue<_, BoundedVec<u32, T::MaxConcurrentReferenda>, ValueQuery>;

    // ── Voting epoch storage ─────────────────────────────────────────────────

    /// Current active voting epoch: (start_block, end_block). None = no epoch open.
    /// Citizens may only cast referendum votes while this is Some and now is in [start, end].
    #[pallet::storage]
    pub type ActiveEpoch<T: Config> =
        StorageValue<_, (BlockNumberFor<T>, BlockNumberFor<T>)>;

    /// Monotonically increasing voting epoch counter. Incremented each time an epoch opens.
    #[pallet::storage]
    pub type EpochNumber<T: Config> = StorageValue<_, u32, ValueQuery>;

    // ── Events ───────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        ProposalCreated { id: u32, ends_at: BlockNumberFor<T>, topic_hash: [u8; 32], tier: ReferendumTier },
        VoteCommitted { proposal_id: u32, nullifier: [u8; 32] },
        /// A citizen overwrote their own previously-stored commitment for this
        /// (proposal_id, nullifier) with a new one before the proposal's voting window closed —
        /// e.g. to override a vote cast under coercion. Distinct from `VoteCommitted` (which
        /// only fires on a nullifier's first commitment) so first-time vs. override commits
        /// remain separately observable on-chain. See `commit_vote`'s doc comment.
        VoteResubmitted { proposal_id: u32, nullifier: [u8; 32] },
        DelegationSet {
            delegator: T::AccountId,
            delegate: T::AccountId,
            topic_id: u32,
            expires_at: BlockNumberFor<T>,
        },
        DelegationRevoked { delegator: T::AccountId, topic_id: u32 },
        /// Emitted lazily when an expired delegation is encountered and cleaned up.
        DelegationExpired { delegator: T::AccountId, topic_id: u32 },
        /// New fiscal year opened; all registered citizens may now claim budget tokens.
        FiscalYearStarted { epoch: u32, tokens_per_citizen: u64 },
        /// A citizen claimed their budget tokens for this epoch.
        BudgetTokensClaimed { who: T::AccountId, epoch: u32, tokens: u64 },
        /// A citizen updated their QV allocation for a budget category.
        /// vote_count is the new total; token cost for this slot = vote_count².
        BudgetAllocated { who: T::AccountId, epoch: u32, category_id: u32, vote_count: u32 },
        /// A referendum was automatically created when a petition hit PetitionThreshold.
        ReferendumCreated {
            referendum_id: u32,
            petition_id: u32,
            topic_hash: [u8; 32],
            ends_at: BlockNumberFor<T>,
            tier: ReferendumTier,
        },
        /// A periodic voting epoch opened — citizens may now vote on active referenda.
        VotingEpochOpened { epoch: u32, start: BlockNumberFor<T>, end: BlockNumberFor<T> },
        /// A voting epoch closed — no more votes accepted until the next epoch opens.
        VotingEpochClosed { epoch: u32 },
        /// A citizen cast a vote on a referendum.
        ReferendumVoteCast { referendum_id: u32, voter: T::AccountId, in_favor: bool },
        /// A referendum passed the passage threshold — law has been enacted.
        ReferendumPassed { referendum_id: u32, topic_hash: [u8; 32] },
        /// A referendum failed to reach the passage threshold.
        ReferendumFailed { referendum_id: u32 },
        /// Off-chain MACI tally submitted and verified for a proposal.
        TallySubmitted { proposal_id: u32, yes_votes: u64, no_votes: u64, commitment_root: [u8; 32] },
    }

    // ── Errors ───────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        ProposalNotFound,
        ProposalEnded,
        /// No longer raised by `commit_vote`: a resubmission for a nullifier that already has a
        /// commitment now overwrites it (see `commit_vote`'s doc comment and
        /// `Event::VoteResubmitted`) instead of being rejected. Kept as a variant for storage/
        /// metadata compatibility rather than removed.
        AlreadyVoted,
        DelegationCycleDetected,
        DelegationCapExceeded,
        NoDelegationOnTopic,
        /// duration_blocks is outside [MinDelegationDurationBlocks, MaxDelegationDurationBlocks].
        InvalidDelegationDuration,
        /// The delegation has already expired; it will be cleaned up automatically.
        DelegationExpired,
        NotRegisteredCitizen,
        /// Account is either not a registered citizen or has an active court-ordered suspension.
        CitizenNotActive,
        NoActiveFiscalYear,
        BudgetAlreadyClaimed,
        BudgetNotClaimed,
        InsufficientBudgetTokens,
        InvalidCategoryId,
        ReferendumNotFound,
        /// The referendum is not in Voting state (already finalized or doesn't exist).
        ReferendumNotActive,
        /// Citizen has already voted in this referendum.
        AlreadyVotedInReferendum,
        /// Voting window has not yet closed; cannot finalize.
        ReferendumStillActive,
        /// A referendum for this petition already exists.
        ReferendumAlreadyExists,
        /// `OpenReferenda` is already at `MaxConcurrentReferenda` capacity — see that storage
        /// item's and `Config::MaxConcurrentReferenda`'s doc comments for why referendum
        /// creation fails outright here instead of silently proceeding untracked.
        TooManyOpenReferenda,
        /// A tally has already been submitted for this proposal.
        TallyAlreadySubmitted,
        /// The MACI tally ZK proof did not verify.
        InvalidTallyProof,
        /// Voting window has not yet closed; cannot submit tally.
        ProposalStillActive,
        /// duration_blocks is outside [MinProposalDurationBlocks, MaxProposalDurationBlocks].
        InvalidProposalDuration,
        /// Voting is only permitted during an active voting epoch.
        VotingEpochNotActive,
        /// A voting epoch is already open; close it before opening a new one.
        EpochAlreadyActive,
        /// The voting epoch end block has not passed yet; cannot close early.
        EpochStillActive,
        /// duration_blocks is outside [MinEpochDurationBlocks, MaxEpochDurationBlocks].
        InvalidEpochDuration,
        /// `delegate_vote`: this topic has a referendum whose voting window has already closed
        /// (`end_block` has passed) but that has not yet been finalized (still
        /// `ReferendumState::Voting` — e.g. `PendingFinalization` overflowed and
        /// `finalize_referendum` hasn't been called yet). Creating or modifying a delegation for
        /// this topic is rejected until that referendum is finalized, closing the window where a
        /// delegation created after voting effectively ends could still swing that referendum's
        /// tally — see `delegate_vote`'s doc comment.
        ReferendumVotingWindowClosed,
    }

    // ── Calls ────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Submit a new MACI proposal. Caller must be an active citizen.
        /// `duration_blocks` must fall within [MinProposalDurationBlocks, MaxProposalDurationBlocks].
        /// `topic_hash` and `tier` are stored so that submit_maci_tally can enact the law on pass.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn submit_proposal(
            origin: OriginFor<T>,
            topic_hash: [u8; 32],
            tier: ReferendumTier,
            duration_blocks: u32,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive);
            ensure!(
                duration_blocks >= T::MinProposalDurationBlocks::get()
                    && duration_blocks <= T::MaxProposalDurationBlocks::get(),
                Error::<T>::InvalidProposalDuration
            );
            let id = NextProposalId::<T>::get();
            let ends_at = frame_system::Pallet::<T>::block_number()
                .saturating_add(BlockNumberFor::<T>::from(duration_blocks));
            Proposals::<T>::insert(id, (ends_at, topic_hash, tier.clone()));
            NextProposalId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::ProposalCreated { id, ends_at, topic_hash, tier });
            Ok(())
        }

        /// Commit an encrypted vote (MACI commitment). Actual tally done off-chain with ZK proof.
        /// The nullifier is derived from the caller's registered identity — callers cannot supply
        /// an arbitrary nullifier, which enforces 1-person-1-vote.
        ///
        /// ## Resubmission overwrites (coercion resistance)
        /// Calling this again for a `(proposal_id, nullifier)` that already has a stored
        /// commitment does **not** fail with an already-voted error — it overwrites the stored
        /// commitment with the new one (emitting `Event::VoteResubmitted` instead of
        /// `Event::VoteCommitted`), bounded by the same `ends_at` window check as a first-time
        /// commit. This lets a citizen coerced into voting a certain way quietly cast a
        /// superseding vote later. See the inline comment at the storage write below for what
        /// this does and does not cover (in short: on-chain override, yes; a tally that actually
        /// honors "only the last commitment per nullifier counts," not yet — that's the separate,
        /// already-tracked off-chain MACI coordinator work).
        ///
        /// ## Known sender-anonymity gap
        /// This call requires `ensure_signed`, so the extrinsic's signing `AccountId` (`who`
        /// below) is publicly visible on-chain — in the block/extrinsic itself, independent of
        /// anything this pallet stores. What actually gets *written to pallet storage* is
        /// already nullifier-scoped, not `who`-scoped (`VoteCommitments` is keyed by
        /// `(proposal_id, nullifier)`, and `Event::VoteCommitted` only carries the nullifier) —
        /// but pallet-identity's `CitizenNullifier` storage is a public, permanent map from
        /// `AccountId -> nullifier`. So anyone watching the chain can join "this signed extrinsic
        /// came from `AccountId` X" with "X's nullifier is N" (a public lookup) and learn "the
        /// citizen behind nullifier N participated in proposal P at block B" — i.e. *whether* and
        /// *when* someone voted is linkable to their real registered identity, even though *what*
        /// they voted stays hidden inside the encrypted `commitment`. This is weaker than the
        /// MACI/Semaphore "receipt-free, unlinkable" framing in `CLAUDE.md` implies, which
        /// expects participation itself to be unlinkable, not just vote content.
        ///
        /// A real fix needs the extrinsic to not carry the signer's `AccountId` at all — e.g. an
        /// unsigned extrinsic validated via a custom `ValidateUnsigned`/`SignedExtension` that
        /// checks a Semaphore-style ZK group-membership proof instead of a signature, or a
        /// relayer/mixnet that decouples submission from the signing key. Searched this codebase
        /// for either pattern before writing this comment: none exists yet. Every other
        /// citizen-facing extrinsic that also carries similarly sensitive identity material
        /// (`pallet-identity`'s `register_citizen`, `reverify_citizen`, `migrate_oprf_scheme`) is
        /// `ensure_signed` too, for the same reason: there's no unsigned/anonymous-submission
        /// infrastructure anywhere in this repo to reuse. Building one is a genuine architectural
        /// addition (new transaction-validity logic, new proof type, new client-side flow), not a
        /// local fix to this call — left as a tracked gap rather than force a partial change here.
        ///
        /// **Note (2026-08-22): the above is only the `CitizenNullifier`-map sub-case of a
        /// broader gap.** Even a signer account that was never registered as a citizen at all —
        /// so it never appears in `CitizenNullifier` — is not automatically safe: if it was
        /// funded by a direct on-chain transfer from the citizen's real account, or submits in
        /// close temporal proximity to other citizen-linked activity, ordinary chain analysis of
        /// that funding source or timing still deanonymizes it, independent of anything proved
        /// inside `commitment`. A relayer/mixnet fix needs to anonymize the funding path too, not
        /// just decouple the `AccountId` from the vote content. See `CLAUDE.md`'s Voting System
        /// section for the general writeup.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn commit_vote(
            origin: OriginFor<T>,
            proposal_id: u32,
            commitment: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive);
            let nullifier = T::NullifierProvider::nullifier_of(&who)
                .ok_or(Error::<T>::NotRegisteredCitizen)?;
            let (ends_at, _, _) = Proposals::<T>::get(proposal_id).ok_or(Error::<T>::ProposalNotFound)?;
            ensure!(frame_system::Pallet::<T>::block_number() < ends_at, Error::<T>::ProposalEnded);
            // ── Coercion-resistant vote override ────────────────────────────────────────
            // A resubmission for the same (proposal_id, nullifier) is deliberately allowed to
            // overwrite the previously stored commitment rather than being rejected with
            // `Error::AlreadyVoted`, so a citizen coerced into voting a certain way — by an
            // abuser, or under threat — can quietly cast a new commitment afterward that
            // supersedes the coerced one. `commit_vote` is a normal signed/proven extrinsic
            // applied to storage in block order, so on-chain state already has a natural "last
            // write wins" property: no off-chain coordinator is needed for *this* part. The
            // `ends_at` check above (identical to the one gating the original vote) is what
            // bounds this — a resubmission is rejected with `Error::ProposalEnded` once the
            // window has closed, exactly like a first-time commit, so the last commitment
            // written before window-close is what counts.
            //
            // This is on-chain override only. It does NOT provide a privacy-preserving *tally*
            // that only counts each nullifier's latest commitment — that requires the off-chain
            // MACI coordinator this repo doesn't have yet (see this pallet's module-level doc
            // comment and `runtime/src/configs/mod.rs`'s `FailClosedMACIVerifier`, which already
            // fails closed outside `dev-mode` because no real MACI tally verifier is wired in).
            // `submit_maci_tally` is what would need to know "only the last commitment per
            // nullifier counts" — that's a separate, larger, already-tracked follow-up, not
            // something this change attempts to solve.
            let is_resubmission = VoteCommitments::<T>::contains_key((proposal_id, nullifier));
            VoteCommitments::<T>::insert((proposal_id, nullifier), commitment);
            if is_resubmission {
                Self::deposit_event(Event::VoteResubmitted { proposal_id, nullifier });
            } else {
                Self::deposit_event(Event::VoteCommitted { proposal_id, nullifier });
            }
            Ok(())
        }

        /// Delegate voting power for a specific topic to another citizen for a fixed duration.
        ///
        /// `duration_blocks` must be within [MinDelegationDurationBlocks, MaxDelegationDurationBlocks].
        /// Replaces any existing (including expired) delegation for that topic.
        /// The delegation is valid for any referendum whose close block ≤ (now + duration_blocks).
        ///
        /// ## Rejected if the topic has a closed-but-unfinalized referendum outstanding
        /// `resolve_delegate_readonly`/`apply_delegated_weight` (used at finalization) validate a
        /// delegation only by `expires_at >= reference_block`, with no check on when the
        /// delegation was *created*. Left unguarded, that would let a delegation created after a
        /// referendum's `end_block` has already passed still swing that referendum's tally during
        /// the window between `end_block` and whenever `finalize_referendum` actually runs (normally
        /// closed to nothing by same-block `on_initialize` finalization, but `PendingFinalization`
        /// is capped by `MaxReferendaPerBlock` and silently falls back to the permissionless
        /// `finalize_referendum` extrinsic with no promptness guarantee on overflow — see that
        /// storage item's doc comment). `topic_has_closed_unfinalized_referendum` closes this: if
        /// `topic_id` currently has any `OpenReferenda` entry whose `end_block` has already passed,
        /// this call is rejected with `Error::ReferendumVotingWindowClosed` until that referendum is
        /// finalized.
        ///
        /// ## DelegationCap enforcement is against *transitively resolved* weight
        /// `DelegationCap` bounds the percentage of total voting power any single delegate may
        /// end up holding once chains are followed to their end — not just how many people
        /// delegate to them directly (see `DelegatedWeight`'s doc comment). So this call:
        /// 1. Walks forward from `delegate` (bounded by `MaxDelegationDepth`, same as
        ///    `resolve_delegate_readonly`) to find the terminal delegate this new edge would
        ///    ultimately route `who`'s vote to.
        /// 2. Computes what `who` is bringing to that terminal: `who`'s own vote, plus — if
        ///    `who` is currently itself a terminal delegate for others — all of the weight
        ///    already resolving to `who` (read from `DelegatedWeight`, an O(1) lookup; nobody
        ///    else's weight can be sitting "behind" `who` without already being reflected
        ///    there, see the invariant in `DelegatedWeight`'s doc comment).
        /// 3. Rejects the delegation if the terminal's resulting total would exceed the cap.
        ///
        /// This keeps the whole cost bounded by `MaxDelegationDepth` (a handful of bounded
        /// forward walks over storage) rather than rescanning every delegation on-chain, and it
        /// keeps the invariant "no delegate exceeds the cap" true for every edge that is itself
        /// being added or moved.
        ///
        /// Both the weight *added* by a new/moved edge (step 2 above) and the weight *removed*
        /// from an old edge being replaced (further down, when `maybe_old` is `Some`) are applied
        /// via `propagate_weight_along_chain`, which walks the whole affected chain — not just
        /// its terminal — and updates every intermediate hop's own stored
        /// `DelegationRecord::resolved_weight` by the same delta. This closes what used to be a
        /// real cap bypass: if an upstream account joins an intermediate delegate's chain
        /// *after* that intermediate's own outgoing edge already exists (e.g. B delegates to C,
        /// then A delegates to B), the intermediate's stored snapshot now reflects the join
        /// immediately, so if that intermediate later re-delegates to a *different* new target,
        /// the weight it carries there is the real, current total — not a stale figure captured
        /// only at the moment its own edge was first created.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn delegate_vote(
            origin: OriginFor<T>,
            delegate: T::AccountId,
            topic_id: u32,
            duration_blocks: u32,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive);
            ensure!(
                duration_blocks >= T::MinDelegationDurationBlocks::get()
                    && duration_blocks <= T::MaxDelegationDurationBlocks::get(),
                Error::<T>::InvalidDelegationDuration
            );
            ensure!(
                !Self::has_delegation_cycle(&who, &delegate, topic_id),
                Error::<T>::DelegationCycleDetected
            );

            let now = frame_system::Pallet::<T>::block_number();
            // Close the post-window delegation race: `resolve_delegate_readonly`/
            // `apply_delegated_weight` validate a delegation only by `expires_at >=
            // reference_block`, not by when the delegation was *created* — so without this
            // check, a delegation created after a referendum's `end_block` has already passed
            // could still swing that referendum's tally during the (normally near-zero, but not
            // zero — see `PendingFinalization`'s doc comment) window between `end_block` and
            // whenever `finalize_referendum` actually runs. Reject creating/modifying a
            // delegation for a topic that currently has such a closed-but-unfinalized
            // referendum outstanding.
            ensure!(
                !Self::topic_has_closed_unfinalized_referendum(topic_id, now),
                Error::<T>::ReferendumVotingWindowClosed
            );

            // Read the old delegation record without mutating yet.
            // We must do all cap checks before touching any storage, otherwise a failed cap
            // check would leave DelegatorCount decremented while the old Delegations record
            // still exists — an inconsistency.
            let maybe_old = Delegations::<T>::get(topic_id, who.clone());
            let replacing_same_delegate =
                maybe_old.as_ref().map_or(false, |r| r.delegate == delegate);

            // Compute the new count for `delegate`. If we're re-delegating to the same
            // delegate we decrement then increment, so the net count is unchanged.
            let base_count = DelegatorCount::<T>::get((topic_id, &delegate));
            let new_count = if replacing_same_delegate {
                base_count
            } else {
                base_count.saturating_add(1)
            };

            ensure!(
                new_count <= T::MaxDelegationsPerDelegate::get(),
                Error::<T>::DelegationCapExceeded
            );

            // The weight to record against the (possibly new) resolved terminal for this edge.
            // When replacing the same delegate, nothing about the resolved chain or its weight
            // changes (only expires_at does) — carry the existing snapshot forward unchanged
            // rather than recomputing, since `who`'s own `DelegatedWeight` bucket is already 0
            // (they have had an active outgoing delegation the whole time) and would otherwise
            // make it look like this edge only ever carried weight 1.
            let resolved_weight = if replacing_same_delegate {
                maybe_old.as_ref().expect("replacing_same_delegate implies Some").resolved_weight
            } else {
                // `who`'s own contribution to whichever terminal this edge resolves to.
                //
                // Two cases:
                //  - `who` has no existing outgoing delegation on this topic (first-time
                //    delegator, or their previous one already expired/was removed): their real
                //    weight is their own vote, plus (only if `who` is currently itself a
                //    terminal delegate for others) whatever weight has already accumulated
                //    behind them in `DelegatedWeight`. `DelegatedWeight[who]` accurately
                //    reflects that here, since `who` currently has no outgoing edge.
                //  - `who` already has an outgoing delegation to a *different* delegate
                //    (`replacing_same_delegate` is false here, so if `maybe_old` is `Some` its
                //    `delegate` field differs from the new `delegate`): per `DelegatedWeight`'s
                //    documented invariant, `DelegatedWeight[who]` is always 0 while `who` has an
                //    active outgoing delegation — it does NOT reflect `who`'s real transitively
                //    resolved weight. That real total is exactly `old_record.resolved_weight`,
                //    already snapshotted and available here (it's also what the old-edge unwind
                //    below uses). Using `1 + DelegatedWeight[who]` in this case would silently
                //    collapse `who_weight` to 1 regardless of `who`'s real backing, letting the
                //    cap check for the new edge pass trivially and permanently dropping the
                //    difference from cap-tracking entirely (not credited to either delegate).
                let who_weight = match maybe_old.as_ref() {
                    Some(old_record) => old_record.resolved_weight,
                    None => 1u32.saturating_add(DelegatedWeight::<T>::get((topic_id, &who))),
                };
                // Where the NEW edge would ultimately route weight to. Cycle detection above
                // already guarantees `who` cannot appear anywhere in this chain.
                let new_terminal = Self::resolve_delegate_readonly(&delegate, topic_id, now);
                let total = T::CitizenChecker::total_citizens();
                if total > 0 {
                    let projected =
                        DelegatedWeight::<T>::get((topic_id, &new_terminal)).saturating_add(who_weight);
                    // Use u64 to avoid overflow: DelegationCap * total can exceed u32::MAX
                    // for large citizenries (e.g. cap=50, total=100M → 5B overflows u32).
                    ensure!(
                        (projected as u64).saturating_mul(100)
                            <= (T::DelegationCap::get() as u64).saturating_mul(total as u64),
                        Error::<T>::DelegationCapExceeded
                    );
                }
                // Propagate `who_weight` all the way to the terminal, fixing up every
                // intermediate hop's own stored `resolved_weight` along the way (not just the
                // terminal's `DelegatedWeight` bucket) — see `propagate_weight_along_chain`'s
                // doc comment for why that matters.
                Self::propagate_weight_along_chain(&delegate, topic_id, now, who_weight as i64);
                who_weight
            };

            // All checks passed. Now perform state mutations atomically.
            // When replacing_same_delegate is true the net count is unchanged, so we must
            // skip BOTH the decrement (old) and the insert (new) to avoid a -1 drift.
            if let Some(old_record) = &maybe_old {
                if old_record.delegate != delegate {
                    DelegatorCount::<T>::mutate((topic_id, &old_record.delegate), |c| {
                        *c = c.saturating_sub(1)
                    });
                    // Move the weight this delegation used to carry away from wherever the OLD
                    // chain resolved to, fixing up every intermediate hop's own stored
                    // `resolved_weight` along that old chain too (see
                    // `propagate_weight_along_chain`'s doc comment).
                    Self::propagate_weight_along_chain(
                        &old_record.delegate,
                        topic_id,
                        now,
                        -(old_record.resolved_weight as i64),
                    );
                }
            }
            if !replacing_same_delegate {
                DelegatorCount::<T>::insert((topic_id, &delegate), new_count);
            }
            // `who` now has an active outgoing delegation (whether new or replaced), so any
            // weight that had accumulated at `who` as a terminal has just been forwarded above
            // — `who` itself must go back to 0, preserving DelegatedWeight's invariant.
            DelegatedWeight::<T>::remove((topic_id, &who));

            let expires_at = now.saturating_add(BlockNumberFor::<T>::from(duration_blocks));

            Delegations::<T>::insert(
                topic_id,
                who.clone(),
                DelegationRecord { delegate: delegate.clone(), expires_at, resolved_weight },
            );
            Self::deposit_event(Event::DelegationSet { delegator: who, delegate, topic_id, expires_at });
            Ok(())
        }

        /// Revoke an existing delegation for a specific topic (active or expired).
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn revoke_delegation(origin: OriginFor<T>, topic_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive);
            let record = Delegations::<T>::take(topic_id, who.clone())
                .ok_or(Error::<T>::NoDelegationOnTopic)?;
            DelegatorCount::<T>::mutate((topic_id, &record.delegate), |c| *c = c.saturating_sub(1));
            // Move the weight this delegation carried back from wherever it resolved to
            // onto `who` themselves — they are a terminal delegate again now that they have
            // no outgoing delegation on this topic. Also fixes up every intermediate hop's own
            // stored `resolved_weight` along the old chain (see
            // `propagate_weight_along_chain`'s doc comment).
            let now = frame_system::Pallet::<T>::block_number();
            Self::propagate_weight_along_chain(
                &record.delegate,
                topic_id,
                now,
                -(record.resolved_weight as i64),
            );
            DelegatedWeight::<T>::mutate((topic_id, &who), |w| {
                *w = w.saturating_add(record.resolved_weight)
            });
            Self::deposit_event(Event::DelegationRevoked { delegator: who, topic_id });
            Ok(())
        }

        /// Open a new fiscal year, making budget tokens available for citizens to claim.
        /// Tokens from the previous epoch cannot be carried over (expire on the old epoch).
        /// Origin: legislature motion (T::LegislatureOrigin).
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn start_fiscal_year(
            origin: OriginFor<T>,
            tokens_per_citizen: u64,
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-voting::start_fiscal_year", tokens_per_citizen),
            )?;
            let epoch = FiscalYearEpoch::<T>::get().saturating_add(1);
            FiscalYearEpoch::<T>::put(epoch);
            EpochTokenAllocation::<T>::insert(epoch, tokens_per_citizen);
            Self::deposit_event(Event::FiscalYearStarted { epoch, tokens_per_citizen });
            Ok(())
        }

        /// Claim budget tokens for the current fiscal year.
        /// Each citizen may claim once per epoch. Tokens expire with the epoch —
        /// they are non-transferable and cannot accumulate across years.
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn claim_fiscal_year_tokens(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive);
            let epoch = FiscalYearEpoch::<T>::get();
            ensure!(epoch > 0, Error::<T>::NoActiveFiscalYear);
            let last_claimed = CitizenClaimedEpoch::<T>::get(&who).unwrap_or(0);
            ensure!(last_claimed < epoch, Error::<T>::BudgetAlreadyClaimed);
            let tokens = EpochTokenAllocation::<T>::get(epoch)
                .ok_or(Error::<T>::NoActiveFiscalYear)?;
            CitizenClaimedEpoch::<T>::insert(&who, epoch);
            BudgetBalance::<T>::insert(&who, tokens);
            Self::deposit_event(Event::BudgetTokensClaimed { who, epoch, tokens });
            Ok(())
        }

        /// Allocate quadratic budget votes to a category.
        ///
        /// `vote_count` replaces the prior allocation for this (epoch, category).
        /// Marginal token cost = new_votes² − old_votes². Reducing vote_count refunds tokens.
        /// Passing vote_count = 0 refunds all tokens spent on that category.
        ///
        /// Legislature controls line items within each category; citizens control category weights.
        #[pallet::call_index(6)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn allocate_budget(
            origin: OriginFor<T>,
            category_id: u32,
            vote_count: u32,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive);
            ensure!(
                category_id < T::BudgetCategoryCount::get(),
                Error::<T>::InvalidCategoryId
            );
            let epoch = FiscalYearEpoch::<T>::get();
            ensure!(epoch > 0, Error::<T>::NoActiveFiscalYear);
            // Citizen must have claimed tokens for this epoch first.
            let last_claimed = CitizenClaimedEpoch::<T>::get(&who).unwrap_or(0);
            ensure!(last_claimed == epoch, Error::<T>::BudgetNotClaimed);

            let old_votes = CategoryVotes::<T>::get((who.clone(), epoch, category_id));
            let old_cost = (old_votes as u64).saturating_mul(old_votes as u64);
            let new_cost = (vote_count as u64).saturating_mul(vote_count as u64);

            if new_cost > old_cost {
                let extra = new_cost.saturating_sub(old_cost);
                let balance = BudgetBalance::<T>::get(&who);
                ensure!(balance >= extra, Error::<T>::InsufficientBudgetTokens);
                BudgetBalance::<T>::insert(&who, balance.saturating_sub(extra));
            } else {
                let refund = old_cost.saturating_sub(new_cost);
                BudgetBalance::<T>::mutate(&who, |b| *b = b.saturating_add(refund));
            }

            CategoryVotes::<T>::insert((who.clone(), epoch, category_id), vote_count);
            Self::deposit_event(Event::BudgetAllocated { who, epoch, category_id, vote_count });
            Ok(())
        }

        /// Cast a yes/no vote on an active referendum.
        /// One vote per active citizen. Voting closes at the referendum's end_block.
        #[pallet::call_index(7)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn vote_referendum(
            origin: OriginFor<T>,
            referendum_id: u32,
            in_favor: bool,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive);
            let (petition_id, topic_hash, end_block, state, tier) =
                Referenda::<T>::get(referendum_id).ok_or(Error::<T>::ReferendumNotFound)?;
            ensure!(state == ReferendumState::Voting, Error::<T>::ReferendumNotActive);
            ensure!(
                frame_system::Pallet::<T>::block_number() <= end_block,
                Error::<T>::ReferendumNotActive
            );
            // Voting is only permitted during an active epoch (Swiss batched-voting model).
            if let Some((epoch_start, epoch_end)) = ActiveEpoch::<T>::get() {
                let now = frame_system::Pallet::<T>::block_number();
                ensure!(now >= epoch_start && now <= epoch_end, Error::<T>::VotingEpochNotActive);
            } else {
                return Err(Error::<T>::VotingEpochNotActive.into());
            }
            ensure!(
                !ReferendumHasVoted::<T>::get((referendum_id, &who)),
                Error::<T>::AlreadyVotedInReferendum
            );
            ReferendumHasVoted::<T>::insert((referendum_id, &who), true);
            ReferendumVoteChoice::<T>::insert((referendum_id, &who), in_favor);
            ReferendumTally::<T>::mutate(referendum_id, |(yes, no)| {
                if in_favor { *yes = yes.saturating_add(1); } else { *no = no.saturating_add(1); }
            });
            let _ = (petition_id, topic_hash, end_block, tier);
            Self::deposit_event(Event::ReferendumVoteCast { referendum_id, voter: who, in_favor });
            Ok(())
        }

        /// Finalize a referendum once the voting window has closed.
        /// Anyone may call this. Passes if yes_votes * 100 >= PassageThreshold * total_votes.
        /// On pass, calls LawEnactor to enact the law in pallet-constitution.
        ///
        /// In the common case this extrinsic never needs to be called at all: `on_initialize`
        /// automatically runs the same finalization logic (`do_finalize_referendum`) at the
        /// first block after a referendum's `end_block`, via `PendingFinalization`. This call
        /// remains as a permissionless backstop for anything the hook misses — e.g. a block
        /// whose `PendingFinalization` list was already full when this referendum was created
        /// (bounded by `MaxReferendaPerBlock`) — so a referendum can never get permanently
        /// stuck in `Voting` if the automatic path doesn't reach it.
        ///
        /// Permissionless (`ensure_signed`, callable by anyone), so the declared weight must
        /// track the real cost of what this call does, not just its own body — see
        /// `finalize_referendum_weight`'s doc comment for the accounting.
        #[pallet::call_index(8)]
        #[pallet::weight(Pallet::<T>::finalize_referendum_weight())]
        pub fn finalize_referendum(origin: OriginFor<T>, referendum_id: u32) -> DispatchResult {
            let _who = ensure_signed(origin)?;
            Self::do_finalize_referendum(referendum_id)
        }

        /// Submit a verified MACI tally for a proposal whose voting window has closed.
        /// Only callable by the legislature origin (same authority that opens voting epochs).
        /// In production wire this to a dedicated MACI coordinator origin; using legislature
        /// origin here ensures at minimum a governance-controlled account submits tallies.
        #[pallet::call_index(9)]
        #[pallet::weight(Weight::from_parts(50_000, 0))]
        pub fn submit_maci_tally(
            origin: OriginFor<T>,
            proposal_id: u32,
            yes_votes: u64,
            no_votes: u64,
            commitment_root: [u8; 32],
            proof_bytes: BoundedVec<u8, ConstU32<4096>>,
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(
                    b"pallet-voting::submit_maci_tally",
                    (proposal_id, yes_votes, no_votes, commitment_root, proof_bytes.clone()),
                ),
            )?;
            let (end_block, topic_hash, tier) =
                Proposals::<T>::get(proposal_id).ok_or(Error::<T>::ProposalNotFound)?;
            ensure!(
                frame_system::Pallet::<T>::block_number() > end_block,
                Error::<T>::ProposalStillActive
            );
            ensure!(
                !ProposalResults::<T>::contains_key(proposal_id),
                Error::<T>::TallyAlreadySubmitted
            );
            ensure!(
                T::MACITallyVerifier::verify_tally(
                    proposal_id,
                    yes_votes,
                    no_votes,
                    commitment_root,
                    proof_bytes.as_slice(),
                ),
                Error::<T>::InvalidTallyProof
            );
            ProposalResults::<T>::insert(proposal_id, (yes_votes, no_votes, commitment_root));
            Self::deposit_event(Event::TallySubmitted { proposal_id, yes_votes, no_votes, commitment_root });

            // Enact the law if the tally meets the applicable passage threshold.
            let total = yes_votes.saturating_add(no_votes);
            let threshold = match &tier {
                ReferendumTier::Ordinary => T::PassageThreshold::get() as u64,
                ReferendumTier::Constitutional => T::ConstitutionalPassageThreshold::get() as u64,
                ReferendumTier::Foundational => T::FoundationalPassageThreshold::get() as u64,
            };
            if total > 0 && yes_votes.saturating_mul(100) >= threshold.saturating_mul(total) {
                T::LawEnactor::enact_law(tier, topic_hash)?;
            }
            Ok(())
        }

        /// Open a new Swiss-model voting epoch. Citizens may only vote on referenda during
        /// an active epoch. The legislature controls when epochs open and how long they last.
        /// Emits VotingEpochOpened. Fails if an epoch is already active.
        #[pallet::call_index(10)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn open_voting_epoch(
            origin: OriginFor<T>,
            duration_blocks: u32,
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-voting::open_voting_epoch", duration_blocks),
            )?;
            ensure!(ActiveEpoch::<T>::get().is_none(), Error::<T>::EpochAlreadyActive);
            ensure!(
                duration_blocks >= T::MinEpochDurationBlocks::get()
                    && duration_blocks <= T::MaxEpochDurationBlocks::get(),
                Error::<T>::InvalidEpochDuration
            );
            let now = frame_system::Pallet::<T>::block_number();
            let end = now.saturating_add(BlockNumberFor::<T>::from(duration_blocks));
            ActiveEpoch::<T>::put((now, end));
            let epoch = EpochNumber::<T>::get().saturating_add(1);
            EpochNumber::<T>::put(epoch);
            Self::deposit_event(Event::VotingEpochOpened { epoch, start: now, end });
            Ok(())
        }

        /// Create a Constitutional-tier referendum directly, without a citizen petition.
        /// Only the legislature (via a passed motion) may call this.
        /// Use for constitutional amendments and other matters requiring the
        /// ConstitutionalPassageThreshold supermajority to pass.
        /// petition_id is set to u32::MAX (sentinel — no backing petition).
        #[pallet::call_index(11)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn create_constitutional_referendum(
            origin: OriginFor<T>,
            topic_hash: [u8; 32],
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-voting::create_constitutional_referendum", topic_hash),
            )?;
            let id = NextReferendumId::<T>::get();
            let now = frame_system::Pallet::<T>::block_number();
            let ends_at = now.saturating_add(BlockNumberFor::<T>::from(T::ReferendumDurationBlocks::get()));
            // Checked before any storage write below, so a `TooManyOpenReferenda` failure here
            // leaves no partial state behind regardless of whether the caller happens to be
            // running inside its own transactional wrapper.
            Self::track_open_referendum(id)?;
            Referenda::<T>::insert(
                id,
                (u32::MAX, topic_hash, ends_at, ReferendumState::Voting, ReferendumTier::Constitutional),
            );
            NextReferendumId::<T>::put(id.saturating_add(1));
            Self::schedule_finalization(id, ends_at);
            Self::deposit_event(Event::ReferendumCreated {
                referendum_id: id,
                petition_id: u32::MAX,
                topic_hash,
                ends_at,
                tier: ReferendumTier::Constitutional,
            });
            Ok(())
        }

        /// Create a Foundational-tier referendum directly. Legislature only.
        ///
        /// Foundational referenda enact `LawTier::Foundational` laws and require
        /// `FoundationalPassageThreshold` (e.g. 75%) of votes cast to pass. They may only
        /// be created by a passed legislature motion — there is no citizen petition path to
        /// Foundational laws. This mirrors the German Basic Law eternity-clause model: the
        /// highest democratic bar, but still changeable through genuine democratic consensus.
        #[pallet::call_index(13)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn create_foundational_referendum(
            origin: OriginFor<T>,
            topic_hash: [u8; 32],
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-voting::create_foundational_referendum", topic_hash),
            )?;
            let id = NextReferendumId::<T>::get();
            let now = frame_system::Pallet::<T>::block_number();
            let ends_at = now.saturating_add(BlockNumberFor::<T>::from(T::ReferendumDurationBlocks::get()));
            // Checked before any storage write below — see the equivalent comment in
            // `create_constitutional_referendum` above for why.
            Self::track_open_referendum(id)?;
            Referenda::<T>::insert(
                id,
                (u32::MAX, topic_hash, ends_at, ReferendumState::Voting, ReferendumTier::Foundational),
            );
            NextReferendumId::<T>::put(id.saturating_add(1));
            Self::schedule_finalization(id, ends_at);
            Self::deposit_event(Event::ReferendumCreated {
                referendum_id: id,
                petition_id: u32::MAX,
                topic_hash,
                ends_at,
                tier: ReferendumTier::Foundational,
            });
            Ok(())
        }

        /// Manually close an expired voting epoch.
        /// Anyone may call this after the epoch's end block has passed.
        /// The on_initialize hook auto-closes epochs on the first block after expiry, so
        /// this call is a fallback for explicit finalization.
        #[pallet::call_index(12)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn close_voting_epoch(origin: OriginFor<T>) -> DispatchResult {
            let _who = ensure_signed(origin)?;
            let (_, end_block) = ActiveEpoch::<T>::get().ok_or(Error::<T>::VotingEpochNotActive)?;
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(now > end_block, Error::<T>::EpochStillActive);
            ActiveEpoch::<T>::kill();
            let epoch = EpochNumber::<T>::get();
            Self::deposit_event(Event::VotingEpochClosed { epoch });
            Ok(())
        }
    }

    // ── Hooks ────────────────────────────────────────────────────────────────

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// Auto-expires the active voting epoch on the first block after its end, and
        /// auto-finalizes any referenda scheduled (via `PendingFinalization`) to close at this
        /// block — see `PendingFinalization`'s doc comment for why this matters: it closes the
        /// window between a referendum's `end_block` and its finalization to effectively zero,
        /// rather than leaving it open indefinitely until someone happens to call the
        /// permissionless `finalize_referendum` extrinsic.
        fn on_initialize(now: BlockNumberFor<T>) -> Weight {
            let mut weight = Weight::zero();

            if let Some((_, end_block)) = ActiveEpoch::<T>::get() {
                if now > end_block {
                    ActiveEpoch::<T>::kill();
                    let epoch = EpochNumber::<T>::get();
                    Self::deposit_event(Event::VotingEpochClosed { epoch });
                    weight = weight.saturating_add(Weight::from_parts(10_000, 0));
                }
            }

            // Bounded by MaxReferendaPerBlock (the BoundedVec's capacity), so this loop's
            // worst-case cost per block is fixed regardless of how many referenda exist overall.
            let scheduled = PendingFinalization::<T>::take(now);
            for referendum_id in scheduled.into_iter() {
                weight = weight.saturating_add(Self::finalize_referendum_weight());
                // Run each finalization in its own storage transaction so a failure (e.g.
                // LawEnactor::enact_law erroring) rolls back just that referendum's partial
                // state instead of corrupting it or aborting the whole block — mirroring the
                // atomicity the `finalize_referendum` extrinsic gets for free from dispatch.
                let _ = frame_support::storage::with_transaction(|| {
                    match Self::do_finalize_referendum(referendum_id) {
                        Ok(()) => sp_runtime::TransactionOutcome::Commit(Ok(())),
                        Err(e) => sp_runtime::TransactionOutcome::Rollback(Err(e)),
                    }
                });
            }

            weight
        }
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
        /// Called by pallet-constitution (via PetitionApprover trait in the runtime) when a
        /// petition hits PetitionThreshold. Creates a timed referendum from the petition.
        /// `tier` controls whether simple majority or supermajority is required to pass.
        pub fn create_referendum_internal(
            petition_id: u32,
            topic_hash: [u8; 32],
            tier: ReferendumTier,
        ) -> DispatchResult {
            ensure!(
                !PetitionReferendum::<T>::contains_key(petition_id),
                Error::<T>::ReferendumAlreadyExists
            );
            let id = NextReferendumId::<T>::get();
            let now = frame_system::Pallet::<T>::block_number();
            // Always give the referendum a full ReferendumDurationBlocks window so that
            // a referendum created near the end of an epoch still has adequate voting time.
            // Citizens may vote in any overlapping future epoch within the window.
            let ends_at = now.saturating_add(BlockNumberFor::<T>::from(T::ReferendumDurationBlocks::get()));
            // Checked before any storage write below — see the equivalent comment in
            // `create_constitutional_referendum` above for why.
            Self::track_open_referendum(id)?;
            Referenda::<T>::insert(
                id,
                (petition_id, topic_hash, ends_at, ReferendumState::Voting, tier.clone()),
            );
            PetitionReferendum::<T>::insert(petition_id, id);
            NextReferendumId::<T>::put(id.saturating_add(1));
            Self::schedule_finalization(id, ends_at);
            Self::deposit_event(Event::ReferendumCreated {
                referendum_id: id,
                petition_id,
                topic_hash,
                ends_at,
                tier,
            });
            Ok(())
        }

        /// Schedules `referendum_id` (whose voting window closes at `end_block`) for automatic
        /// finalization in `on_initialize`, at `end_block + 1` — the first block at which
        /// `do_finalize_referendum`'s own `now > end_block` gate allows finalization to proceed.
        /// See `PendingFinalization`'s doc comment for why this exists.
        ///
        /// If that block's `PendingFinalization` list is already at `MaxReferendaPerBlock`
        /// capacity, this referendum is simply left unscheduled — it still finalizes correctly
        /// via the permissionless `finalize_referendum` extrinsic fallback, just without the
        /// automatic same-block guarantee. Never fails referendum creation itself.
        fn schedule_finalization(referendum_id: u32, end_block: BlockNumberFor<T>) {
            let finalize_at = end_block.saturating_add(BlockNumberFor::<T>::from(1u32));
            PendingFinalization::<T>::mutate(finalize_at, |scheduled| {
                let _ = scheduled.try_push(referendum_id);
            });
        }

        /// Adds `referendum_id` to `OpenReferenda`. Unlike `schedule_finalization` above, an
        /// overflow here is NOT silently swallowed: there is no permissionless fallback that
        /// re-derives `OpenReferenda`'s contents the way `finalize_referendum` backstops
        /// `PendingFinalization`, so an untracked open referendum would be invisible to
        /// `RecoveryStateChecker::has_open_referendum_vote` and reopen the double-vote gap that
        /// check exists to close. Called from all three referendum-creation call sites; a
        /// failure here aborts the whole creation extrinsic (dispatch is atomic), so
        /// `Referenda`/`NextReferendumId`/etc. are all rolled back together with it.
        fn track_open_referendum(referendum_id: u32) -> DispatchResult {
            OpenReferenda::<T>::try_mutate(|open| {
                open.try_push(referendum_id).map_err(|_| Error::<T>::TooManyOpenReferenda)
            })?;
            Ok(())
        }

        /// Removes `referendum_id` from `OpenReferenda`. Called once a referendum's state
        /// transitions out of `Voting` (`do_finalize_referendum`). A no-op if the id isn't
        /// present (e.g. it was never tracked because creation would have overflowed the bound —
        /// which can't happen since creation fails outright in that case, but this stays a safe
        /// no-op rather than panicking either way).
        fn untrack_open_referendum(referendum_id: u32) {
            OpenReferenda::<T>::mutate(|open| {
                open.retain(|id| *id != referendum_id);
            });
        }

        /// Hand-estimated weight for one finalization pass (delegation resolution + tally +
        /// state transition), shared by both the permissionless `finalize_referendum` extrinsic
        /// and `on_initialize`'s automatic scheduling — both run exactly the same
        /// `do_finalize_referendum` logic, so both need to account for the same worst-case cost.
        ///
        /// It internally calls `apply_delegated_weight`, which scans
        /// `Delegations::<T>::iter_prefix(topic_id)` — bounded to delegators on this
        /// referendum's specific topic (see that storage item's doc comment), but every
        /// registered citizen may hold at most one delegation per topic, so that scan can still
        /// cost up to `CitizenChecker::total_citizens()` iterations (e.g. many distinct
        /// delegates, each near `MaxDelegationsPerDelegate`, all delegated-to on the same
        /// topic). This adds a per-citizen component on top of the flat base cost — one flat
        /// `Weight::from_parts` estimate per potential delegator, scaled by `MaxDelegationDepth`
        /// to account for `resolve_delegate_readonly`'s chain walk, matching this file's other
        /// hand-estimated `Weight::from_parts` literals — so weight roughly tracks what the scan
        /// actually costs instead of a flat estimate that silently under-charges as the
        /// delegation graph grows. No formal benchmarking harness exists yet in this repo; this
        /// is a conservative hand-estimate, not a benchmarked figure.
        fn finalize_referendum_weight() -> Weight {
            // Per-delegator iteration cost: is_active_citizen + ReferendumHasVoted reads, the
            // resolve_delegate_readonly chain walk (up to MaxDelegationDepth Delegations reads),
            // plus a final ReferendumHasVoted/ReferendumVoteChoice read and a ReferendumTally
            // mutate. 1_000 per step is a conservative flat estimate, not a benchmarked figure.
            let per_delegator_steps = 4u64.saturating_add(T::MaxDelegationDepth::get() as u64);
            let per_delegator = Weight::from_parts(1_000, 0).saturating_mul(per_delegator_steps);
            let worst_case_delegators = T::CitizenChecker::total_citizens() as u64;
            Weight::from_parts(20_000, 0)
                .saturating_add(per_delegator.saturating_mul(worst_case_delegators))
        }

        /// Shared finalization logic for a referendum whose voting window has closed. Resolves
        /// liquid-democracy delegation into the tally, decides pass/fail against the tier's
        /// threshold, transitions `Referenda`'s state out of `Voting`, and enacts the law on
        /// pass. Used by both the permissionless `finalize_referendum` extrinsic and the
        /// automatic `on_initialize` scheduling (`PendingFinalization`) — see both call sites'
        /// doc comments for why finalization needs to happen at all and why it's now automatic.
        ///
        /// Safe to call at most once per referendum: the `ensure!(state == Voting)` guard below
        /// means a second call (whether from the extrinsic after the hook already ran, or vice
        /// versa) is a cheap no-op error rather than double-applying delegated weight or
        /// double-enacting a law.
        fn do_finalize_referendum(referendum_id: u32) -> DispatchResult {
            let (petition_id, topic_hash, end_block, state, tier) =
                Referenda::<T>::get(referendum_id).ok_or(Error::<T>::ReferendumNotFound)?;
            ensure!(state == ReferendumState::Voting, Error::<T>::ReferendumNotActive);
            ensure!(
                frame_system::Pallet::<T>::block_number() > end_block,
                Error::<T>::ReferendumStillActive
            );
            // Resolve liquid-democracy delegation into the tally before reading it. Any citizen
            // who delegated their vote on this referendum's topic and did NOT vote directly gets
            // their weight added on top of whichever side their (transitively resolved) delegate
            // voted, provided that delegate voted directly. Safe to run exactly once here: the
            // state transition below moves the referendum out of `Voting`, and the `ensure!`
            // above guarantees this can never re-enter this branch for the same referendum_id.
            // See `topic_id_of` and `apply_delegated_weight` doc comments.
            let topic_id = Self::topic_id_of(&topic_hash);
            Self::apply_delegated_weight(referendum_id, topic_id, end_block);
            let (yes_count, no_count) = ReferendumTally::<T>::get(referendum_id);
            let total = yes_count.saturating_add(no_count);
            let threshold = match tier {
                ReferendumTier::Ordinary => T::PassageThreshold::get() as u64,
                ReferendumTier::Constitutional => T::ConstitutionalPassageThreshold::get() as u64,
                ReferendumTier::Foundational => T::FoundationalPassageThreshold::get() as u64,
            };
            // Use u64 arithmetic to avoid overflow with large citizenries (yes_count * 100 would
            // saturate a u32 above ~42 million votes, causing the check to always pass).
            let passed = total > 0
                && (yes_count as u64).saturating_mul(100) >= threshold.saturating_mul(total as u64);
            let new_state = if passed { ReferendumState::Passed } else { ReferendumState::Failed };
            Referenda::<T>::insert(
                referendum_id,
                (petition_id, topic_hash, end_block, new_state, tier.clone()),
            );
            Self::untrack_open_referendum(referendum_id);
            if passed {
                T::LawEnactor::enact_law(tier, topic_hash)?;
                Self::deposit_event(Event::ReferendumPassed { referendum_id, topic_hash });
            } else {
                Self::deposit_event(Event::ReferendumFailed { referendum_id });
            }
            Ok(())
        }

        /// Walk the delegation chain from `delegate` up to MaxDelegationDepth steps.
        /// Expired delegations are treated as absent (lazy cleanup: removes record + adjusts count).
        /// Returns true if `who` appears in the chain, `who == delegate`, or the depth limit is
        /// reached (conservatively treats deep chains as cycles to bound on-chain computation).
        fn has_delegation_cycle(who: &T::AccountId, delegate: &T::AccountId, topic_id: u32) -> bool {
            if who == delegate {
                return true;
            }
            let now = frame_system::Pallet::<T>::block_number();
            let mut current = delegate.clone();
            for _ in 0..T::MaxDelegationDepth::get() {
                match Delegations::<T>::get(topic_id, current.clone()) {
                    Some(record) if record.expires_at >= now => {
                        if record.delegate == *who {
                            return true;
                        }
                        current = record.delegate;
                    }
                    Some(record) => {
                        // Expired — clean up lazily, including moving the weight this edge
                        // carried back onto `current` (which becomes a terminal delegate again)
                        // from wherever the now-invalid chain used to resolve to, fixing up every
                        // intermediate hop's own stored `resolved_weight` along that old chain too.
                        // Mirrors what `revoke_delegation` does for an explicit revoke.
                        Self::propagate_weight_along_chain(
                            &record.delegate,
                            topic_id,
                            now,
                            -(record.resolved_weight as i64),
                        );
                        DelegatedWeight::<T>::mutate((topic_id, &current), |w| {
                            *w = w.saturating_add(record.resolved_weight)
                        });
                        DelegatorCount::<T>::mutate((topic_id, &record.delegate), |c| {
                            *c = c.saturating_sub(1)
                        });
                        Delegations::<T>::remove(topic_id, current.clone());
                        Self::deposit_event(Event::DelegationExpired {
                            delegator: current,
                            topic_id,
                        });
                        return false;
                    }
                    None => return false,
                }
            }
            true
        }

        /// Derives the liquid-democracy `topic_id` for a proposal/referendum from its
        /// `topic_hash` (the content hash citizens already pass to `submit_proposal`,
        /// `create_constitutional_referendum`, `create_foundational_referendum`, and — via
        /// `PetitionApprover` — petitions).
        ///
        /// `Delegations` is keyed by a coarse `topic_id: u32` (e.g. an external "healthcare" /
        /// "budget" category — see the module doc comment and `DelegationRecord`), but nothing
        /// upstream of this pallet has ever assigned real category ids: every proposal and
        /// referendum is only tagged with `topic_hash`, a per-item content hash (the IPFS CID of
        /// the specific bill/petition text). Introducing a real category taxonomy would mean
        /// threading a new parameter through `submit_proposal`, both `create_*_referendum` calls,
        /// and the cross-pallet `PetitionApprover` trait implemented in `pallet-constitution` and
        /// the runtime — a bigger, multi-pallet change than this pass's scope (wiring
        /// `Delegations` into the actual tally, which previously had zero effect on any vote
        /// count). Deriving a stable id from the low 4 bytes of `topic_hash` keeps delegation
        /// self-contained to this pallet and makes it *functional* (a delegate's vote now counts
        /// for whoever delegated to them, transitively) — but note the known limitation: two
        /// referenda that a human would consider the same topic (e.g. two different healthcare
        /// bills) will not share a `topic_id` unless their hashes happen to collide in those 4
        /// bytes, so "per-topic" delegation today behaves closer to "per specific proposal
        /// content" than a durable category a citizen can set once and forget. A real fix needs
        /// an actual topic taxonomy chosen upstream (by the legislature/petition proposer) —
        /// tracked as follow-up, not attempted here.
        pub(crate) fn topic_id_of(topic_hash: &[u8; 32]) -> u32 {
            u32::from_le_bytes([topic_hash[0], topic_hash[1], topic_hash[2], topic_hash[3]])
        }

        /// Whether `topic_id` currently has a referendum that is still `ReferendumState::Voting`
        /// (i.e. not yet finalized) but whose voting window has already closed (`now >
        /// end_block`). `delegate_vote` uses this to reject creating/modifying a delegation for
        /// a topic in exactly this state — see its doc comment for the race this closes.
        ///
        /// Scans `OpenReferenda` — the flat, bounded (`MaxConcurrentReferenda`) list of
        /// currently-`Voting` referendum ids also used by `RecoveryStateChecker` — rather than
        /// all of `Referenda`, which grows with the total historical referendum count. This
        /// mirrors `vote_referendum`'s own pattern of reading a referendum's `(topic_hash,
        /// end_block, state)` tuple straight out of `Referenda` to decide whether it is still
        /// open, just applied across the bounded open set instead of one known id.
        fn topic_has_closed_unfinalized_referendum(topic_id: u32, now: BlockNumberFor<T>) -> bool {
            OpenReferenda::<T>::get().iter().any(|&referendum_id| {
                match Referenda::<T>::get(referendum_id) {
                    Some((_, topic_hash, end_block, state, _)) => {
                        state == ReferendumState::Voting
                            && Self::topic_id_of(&topic_hash) == topic_id
                            && now > end_block
                    }
                    None => false,
                }
            })
        }

        /// Applies a signed weight `delta` (positive = weight newly arriving from upstream,
        /// negative = weight departing) along the delegation chain starting at `start`, walking
        /// exactly the same hops `resolve_delegate_readonly` would (same expiry semantics, same
        /// `MaxDelegationDepth` bound) — but mutating storage instead of only reading it.
        ///
        /// Every intermediate hop on the way (an account whose own `Delegations` record is still
        /// active) has its *own* stored `resolved_weight` adjusted by `delta`, not just the final
        /// terminal's `DelegatedWeight` bucket. This is what closes the staleness gap described in
        /// `DelegatedWeight`'s doc comment: `resolved_weight` is the snapshot a later
        /// re-delegation or revocation by *that same account* will trust verbatim (see
        /// `delegate_vote`'s `who_weight` computation), so if an upstream join or departure
        /// changes how much weight is really flowing through an intermediate, that intermediate's
        /// own snapshot has to move too — not just the number sitting at whatever it currently
        /// forwards to. Without this, an intermediate that later re-targets to a *different* new
        /// delegate would carry forward its stale, pre-join/pre-departure snapshot, letting the
        /// cap check at the new target silently pass on less weight than is actually about to
        /// land there.
        ///
        /// Returns the terminal account reached (mirroring `resolve_delegate_readonly`'s return
        /// value), whose `DelegatedWeight` bucket receives the same `delta`.
        fn propagate_weight_along_chain(
            start: &T::AccountId,
            topic_id: u32,
            reference_block: BlockNumberFor<T>,
            delta: i64,
        ) -> T::AccountId {
            let mut current = start.clone();
            for _ in 0..T::MaxDelegationDepth::get() {
                match Delegations::<T>::get(topic_id, current.clone()) {
                    Some(mut record) if record.expires_at >= reference_block => {
                        record.resolved_weight = Self::apply_weight_delta(record.resolved_weight, delta);
                        let next = record.delegate.clone();
                        Delegations::<T>::insert(topic_id, current.clone(), record);
                        current = next;
                    }
                    _ => break,
                }
            }
            DelegatedWeight::<T>::mutate((topic_id, &current), |w| {
                *w = Self::apply_weight_delta(*w, delta);
            });
            current
        }

        /// Adds a signed `delta` to an unsigned weight value with saturating semantics in either
        /// direction. Shared by `propagate_weight_along_chain`'s per-hop and terminal updates.
        fn apply_weight_delta(base: u32, delta: i64) -> u32 {
            if delta >= 0 {
                base.saturating_add(delta as u32)
            } else {
                base.saturating_sub(delta.unsigned_abs() as u32)
            }
        }

        /// Read-only variant of the chain walk `has_delegation_cycle` performs: follows
        /// `Delegations` from `who` for up to `MaxDelegationDepth` hops and returns the final
        /// account the chain bottoms out at (i.e. the first account in the chain with no active
        /// outgoing delegation on `topic_id`). Returns `who` unchanged if they have no active
        /// delegation on this topic at all.
        ///
        /// A delegation is treated as active only while `reference_block <= record.expires_at`,
        /// matching `DelegationRecord`'s doc comment ("valid for referenda whose close block ≤
        /// expires_at") — callers doing referendum tallying should pass the referendum's
        /// `end_block`, not the current block, so a delegation that was active for the entire
        /// voting window still counts even if `finalize_referendum` itself is called later, after
        /// the delegation's `expires_at` has since passed.
        ///
        /// Deliberately does **not** mutate storage (unlike `has_delegation_cycle`'s lazy
        /// cleanup of expired records) so it is safe to call from within a loop over
        /// `Delegations::<T>::iter_prefix(topic_id)` in `apply_delegated_weight` without
        /// disturbing iteration. Expired records it walks past are simply left for the normal
        /// lazy-cleanup paths (`delegate_vote`, `revoke_delegation`, `has_delegation_cycle`) to
        /// remove later.
        fn resolve_delegate_readonly(
            who: &T::AccountId,
            topic_id: u32,
            reference_block: BlockNumberFor<T>,
        ) -> T::AccountId {
            let mut current = who.clone();
            for _ in 0..T::MaxDelegationDepth::get() {
                match Delegations::<T>::get(topic_id, current.clone()) {
                    Some(record) if record.expires_at >= reference_block => {
                        current = record.delegate;
                    }
                    _ => break,
                }
            }
            current
        }

        /// Adds delegated weight to `ReferendumTally` for `referendum_id` on behalf of every
        /// citizen who has an active delegation on `topic_id` and did not cast a direct vote on
        /// this referendum themselves. A direct vote always overrides delegation — only
        /// non-voting delegators are considered here.
        ///
        /// For each such delegator, resolves the delegation chain transitively (via
        /// `resolve_delegate_readonly`, reusing the same graph/expiry/depth-bound semantics
        /// `has_delegation_cycle` uses when a delegation is first set) to find the final
        /// delegate. If that final delegate cast a direct vote on this referendum, the
        /// delegator's weight is added to whichever side (`in_favor`/`ReferendumVoteChoice`) the
        /// delegate voted. Delegators who are currently suspended or unregistered are skipped —
        /// a suspended citizen shouldn't gain representation by proxy any more than they could by
        /// voting directly (`vote_referendum` already enforces this for direct votes via
        /// `CitizenChecker`).
        ///
        /// Revocation is respected automatically: `revoke_delegation` removes the `Delegations`
        /// entry outright, so a revoked delegation simply never appears in the iteration below.
        ///
        /// Called exactly once per referendum, from `finalize_referendum`, before that call
        /// transitions the referendum out of `ReferendumState::Voting` — see the `ensure!` guard
        /// there for why this can't double-apply.
        fn apply_delegated_weight(referendum_id: u32, topic_id: u32, reference_block: BlockNumberFor<T>) {
            for (delegator, _record) in Delegations::<T>::iter_prefix(topic_id) {
                if !T::CitizenChecker::is_active_citizen(&delegator) {
                    continue;
                }
                // Direct vote always overrides delegation.
                if ReferendumHasVoted::<T>::get((referendum_id, &delegator)) {
                    continue;
                }
                let final_delegate =
                    Self::resolve_delegate_readonly(&delegator, topic_id, reference_block);
                if final_delegate == delegator {
                    continue; // no active delegation after resolution (e.g. it expired)
                }
                if ReferendumHasVoted::<T>::get((referendum_id, &final_delegate)) {
                    let in_favor = ReferendumVoteChoice::<T>::get((referendum_id, &final_delegate));
                    ReferendumTally::<T>::mutate(referendum_id, |(yes, no)| {
                        if in_favor {
                            *yes = yes.saturating_add(1);
                        } else {
                            *no = no.saturating_add(1);
                        }
                    });
                }
            }
        }
    }
}
