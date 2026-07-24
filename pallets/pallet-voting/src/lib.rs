//! # Voting Pallet
//!
//! Two separate participation systems:
//! 1. MACI 1p1v — receipt-free anonymous voting for laws and elections.
//! 2. Budget QV — quadratic budget token allocation for fiscal priorities.
//!
//! Liquid democracy delegation applies to system 1 only.
//! Suspended citizens are excluded from both systems (TODO: wire cross-pallet check).
#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {

    use codec::DecodeWithMemTracking;
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;

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
    pub trait LawEnactor {
        fn enact_law(content_hash: [u8; 32]) -> DispatchResult;
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

    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum ReferendumState {
        Voting,
        Passed,
        Failed,
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
        /// Percentage of yes votes required for a referendum to pass (0-100). e.g. 50 = majority.
        #[pallet::constant]
        type PassageThreshold: Get<u8>;
        /// Hook called when a referendum passes — enacts the law in pallet-constitution.
        type LawEnactor: LawEnactor;
        /// Verifier for MACI tally ZK proofs. Use PassthroughMACIVerifier in dev; wire in the
        /// real MACI verifier once circuit trusted setup is complete.
        type MACITallyVerifier: MACITallyVerifier;
        /// The origin permitted to start a new fiscal year (open a new budget epoch).
        /// Wired to the legislature motion origin so only a passed legislature vote
        /// can open a new fiscal year. Use EnsureRoot during development.
        type LegislatureOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;
    }

    // ── 1p1v / MACI storage ─────────────────────────────────────────────────

    /// Active proposals: proposal_id -> end block.
    #[pallet::storage]
    pub type Proposals<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, BlockNumberFor<T>>;

    /// Per-proposal vote commitments (MACI-style): (proposal_id, nullifier) -> commitment.
    #[pallet::storage]
    pub type VoteCommitments<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, [u8; 32]), [u8; 32]>;

    /// Per-topic delegation: (delegator, topic_id) -> delegate AccountId.
    #[pallet::storage]
    pub type Delegations<T: Config> =
        StorageMap<_, Blake2_128Concat, (T::AccountId, u32), T::AccountId>;

    /// Number of direct delegators per (topic_id, delegate).
    #[pallet::storage]
    pub type DelegatorCount<T: Config> =
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

    /// referendum_id -> (petition_id, topic_hash, end_block, state).
    #[pallet::storage]
    pub type Referenda<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, (u32, [u8; 32], BlockNumberFor<T>, ReferendumState)>;

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

    #[pallet::storage]
    pub type NextReferendumId<T: Config> = StorageValue<_, u32, ValueQuery>;

    // ── Events ───────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        ProposalCreated { id: u32, ends_at: BlockNumberFor<T> },
        VoteCommitted { proposal_id: u32, nullifier: [u8; 32] },
        DelegationSet { delegator: T::AccountId, delegate: T::AccountId, topic_id: u32 },
        DelegationRevoked { delegator: T::AccountId, topic_id: u32 },
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
        },
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
        AlreadyVoted,
        DelegationCycleDetected,
        DelegationCapExceeded,
        NoDelegationOnTopic,
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
        /// A tally has already been submitted for this proposal.
        TallyAlreadySubmitted,
        /// The MACI tally ZK proof did not verify.
        InvalidTallyProof,
        /// Voting window has not yet closed; cannot submit tally.
        ProposalStillActive,
        /// duration_blocks is outside [MinProposalDurationBlocks, MaxProposalDurationBlocks].
        InvalidProposalDuration,
    }

    // ── Calls ────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Submit a new proposal for the current voting epoch.
        /// Caller must be an active citizen. `duration_blocks` must fall within
        /// [MinProposalDurationBlocks, MaxProposalDurationBlocks].
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn submit_proposal(
            origin: OriginFor<T>,
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
            let ends_at = frame_system::Pallet::<T>::block_number() +
                BlockNumberFor::<T>::from(duration_blocks);
            Proposals::<T>::insert(id, ends_at);
            NextProposalId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::ProposalCreated { id, ends_at });
            Ok(())
        }

        /// Commit an encrypted vote (MACI commitment). Actual tally done off-chain with ZK proof.
        /// The nullifier is derived from the caller's registered identity — callers cannot supply
        /// an arbitrary nullifier, which enforces 1-person-1-vote.
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
            let ends_at = Proposals::<T>::get(proposal_id).ok_or(Error::<T>::ProposalNotFound)?;
            ensure!(frame_system::Pallet::<T>::block_number() < ends_at, Error::<T>::ProposalEnded);
            ensure!(!VoteCommitments::<T>::contains_key((proposal_id, nullifier)), Error::<T>::AlreadyVoted);
            VoteCommitments::<T>::insert((proposal_id, nullifier), commitment);
            Self::deposit_event(Event::VoteCommitted { proposal_id, nullifier });
            Ok(())
        }

        /// Delegate voting power for a specific topic to another citizen.
        /// Replaces any existing delegation for that topic.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn delegate_vote(
            origin: OriginFor<T>,
            delegate: T::AccountId,
            topic_id: u32,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive);
            ensure!(
                !Self::has_delegation_cycle(&who, &delegate, topic_id),
                Error::<T>::DelegationCycleDetected
            );
            if let Some(old_delegate) = Delegations::<T>::get((who.clone(), topic_id)) {
                DelegatorCount::<T>::mutate((topic_id, &old_delegate), |c| {
                    *c = c.saturating_sub(1)
                });
            }
            let new_count =
                DelegatorCount::<T>::get((topic_id, &delegate)).saturating_add(1);
            // Absolute delegator count ceiling.
            ensure!(
                new_count <= T::MaxDelegationsPerDelegate::get(),
                Error::<T>::DelegationCapExceeded
            );
            // Percentage cap: delegate may not hold more than DelegationCap% of all citizens.
            let total = T::CitizenChecker::total_citizens();
            if total > 0 {
                ensure!(
                    new_count.saturating_mul(100) <= T::DelegationCap::get() as u32 * total,
                    Error::<T>::DelegationCapExceeded
                );
            }
            DelegatorCount::<T>::insert((topic_id, &delegate), new_count);
            Delegations::<T>::insert((who.clone(), topic_id), delegate.clone());
            Self::deposit_event(Event::DelegationSet { delegator: who, delegate, topic_id });
            Ok(())
        }

        /// Revoke an existing delegation for a specific topic.
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn revoke_delegation(origin: OriginFor<T>, topic_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let delegate = Delegations::<T>::take((who.clone(), topic_id))
                .ok_or(Error::<T>::NoDelegationOnTopic)?;
            DelegatorCount::<T>::mutate((topic_id, &delegate), |c| *c = c.saturating_sub(1));
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
            T::LegislatureOrigin::ensure_origin(origin)?;
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
                let extra = new_cost - old_cost;
                let balance = BudgetBalance::<T>::get(&who);
                ensure!(balance >= extra, Error::<T>::InsufficientBudgetTokens);
                BudgetBalance::<T>::insert(&who, balance - extra);
            } else {
                let refund = old_cost - new_cost;
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
            let (petition_id, topic_hash, end_block, state) =
                Referenda::<T>::get(referendum_id).ok_or(Error::<T>::ReferendumNotFound)?;
            ensure!(state == ReferendumState::Voting, Error::<T>::ReferendumNotActive);
            ensure!(
                frame_system::Pallet::<T>::block_number() <= end_block,
                Error::<T>::ReferendumNotActive
            );
            ensure!(
                !ReferendumHasVoted::<T>::get((referendum_id, &who)),
                Error::<T>::AlreadyVotedInReferendum
            );
            ReferendumHasVoted::<T>::insert((referendum_id, &who), true);
            ReferendumTally::<T>::mutate(referendum_id, |(yes, no)| {
                if in_favor { *yes = yes.saturating_add(1); } else { *no = no.saturating_add(1); }
            });
            // Re-insert unchanged fields to satisfy the borrow checker.
            let _ = (petition_id, topic_hash, end_block);
            Self::deposit_event(Event::ReferendumVoteCast { referendum_id, voter: who, in_favor });
            Ok(())
        }

        /// Finalize a referendum once the voting window has closed.
        /// Anyone may call this. Passes if yes_votes * 100 >= PassageThreshold * total_votes.
        /// On pass, calls LawEnactor to enact the law in pallet-constitution.
        #[pallet::call_index(8)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn finalize_referendum(origin: OriginFor<T>, referendum_id: u32) -> DispatchResult {
            let _who = ensure_signed(origin)?;
            let (petition_id, topic_hash, end_block, state) =
                Referenda::<T>::get(referendum_id).ok_or(Error::<T>::ReferendumNotFound)?;
            ensure!(state == ReferendumState::Voting, Error::<T>::ReferendumNotActive);
            ensure!(
                frame_system::Pallet::<T>::block_number() > end_block,
                Error::<T>::ReferendumStillActive
            );
            let (yes_count, no_count) = ReferendumTally::<T>::get(referendum_id);
            let total = yes_count.saturating_add(no_count);
            let passed = total > 0
                && yes_count.saturating_mul(100) >= T::PassageThreshold::get() as u32 * total;
            let new_state = if passed { ReferendumState::Passed } else { ReferendumState::Failed };
            Referenda::<T>::insert(referendum_id, (petition_id, topic_hash, end_block, new_state));
            if passed {
                T::LawEnactor::enact_law(topic_hash)?;
                Self::deposit_event(Event::ReferendumPassed { referendum_id, topic_hash });
            } else {
                Self::deposit_event(Event::ReferendumFailed { referendum_id });
            }
            Ok(())
        }

        /// Submit a verified MACI tally for a proposal whose voting window has closed.
        /// The off-chain MACI coordinator calls this after decrypting all vote commitments
        /// and generating a ZK proof of correct tallying.
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
            let _who = ensure_signed(origin)?;
            let end_block = Proposals::<T>::get(proposal_id).ok_or(Error::<T>::ProposalNotFound)?;
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
            Ok(())
        }
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
        /// Called by pallet-constitution (via PetitionApprover trait in the runtime) when a
        /// petition hits PetitionThreshold. Creates a timed referendum from the petition.
        pub fn create_referendum_internal(petition_id: u32, topic_hash: [u8; 32]) -> DispatchResult {
            ensure!(
                !PetitionReferendum::<T>::contains_key(petition_id),
                Error::<T>::ReferendumAlreadyExists
            );
            let id = NextReferendumId::<T>::get();
            let now = frame_system::Pallet::<T>::block_number();
            let ends_at = now + BlockNumberFor::<T>::from(T::ReferendumDurationBlocks::get());
            Referenda::<T>::insert(id, (petition_id, topic_hash, ends_at, ReferendumState::Voting));
            PetitionReferendum::<T>::insert(petition_id, id);
            NextReferendumId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::ReferendumCreated { referendum_id: id, petition_id, topic_hash, ends_at });
            Ok(())
        }

        /// Walk the delegation chain from `delegate` up to MaxDelegationDepth steps.
        /// Returns true if `who` appears in the chain, `who == delegate`, or the depth limit is
        /// reached without a clean termination (conservatively treats deep chains as cycles).
        fn has_delegation_cycle(who: &T::AccountId, delegate: &T::AccountId, topic_id: u32) -> bool {
            if who == delegate {
                return true;
            }
            let mut current = delegate.clone();
            for _ in 0..T::MaxDelegationDepth::get() {
                match Delegations::<T>::get((current.clone(), topic_id)) {
                    Some(next) => {
                        if next == *who {
                            return true;
                        }
                        current = next;
                    }
                    None => return false,
                }
            }
            // Depth exhausted without finding a clean chain end — treat as potential cycle.
            true
        }
    }
}
