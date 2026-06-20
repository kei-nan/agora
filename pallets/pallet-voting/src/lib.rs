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
        /// Gate: returns false if the account is not a registered citizen or is suspended.
        type CitizenChecker: CitizenChecker<Self::AccountId>;
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
    }

    // ── Calls ────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Submit a new proposal for the current voting epoch.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn submit_proposal(
            origin: OriginFor<T>,
            duration_blocks: u32,
        ) -> DispatchResult {
            let _who = ensure_signed(origin)?;
            let id = NextProposalId::<T>::get();
            let ends_at = frame_system::Pallet::<T>::block_number() +
                BlockNumberFor::<T>::from(duration_blocks);
            Proposals::<T>::insert(id, ends_at);
            NextProposalId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::ProposalCreated { id, ends_at });
            Ok(())
        }

        /// Commit an encrypted vote (MACI commitment). Actual tally done off-chain with ZK proof.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn commit_vote(
            origin: OriginFor<T>,
            proposal_id: u32,
            nullifier: [u8; 32],
            commitment: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive);
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
        /// Origin: root (TODO: legislature origin).
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn start_fiscal_year(
            origin: OriginFor<T>,
            tokens_per_citizen: u64,
        ) -> DispatchResult {
            ensure_root(origin)?;
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
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
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
