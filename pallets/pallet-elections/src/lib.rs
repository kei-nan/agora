//! # Elections Pallet
//!
//! Elections Commission pallet for Agora. Manages commissioners, elections, candidate
//! registration, candidate certification, and result certification with deposit handling.
//!
//! ## Flow
//! 1. Root adds commissioners.
//! 2. Commissioner (or root) creates an election with office name + block window.
//! 3. Active citizens register as candidates (reserves `CandidateDeposit`).
//! 4. Commissioners certify eligible candidates.
//! 5. Commissioner submits results (winner + IPFS results hash) → status → ResultsSubmitted.
//! 6. Commissioner certifies results → status → Certified; all deposits unreserved.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
    use codec::DecodeWithMemTracking;
    use frame_support::{
        pallet_prelude::*,
        traits::{Currency, ReservableCurrency},
    };
    use frame_system::pallet_prelude::*;

    // ── Balance type alias ─────────────────────────────────────────────────────

    pub type BalanceOf<T> = <<T as Config>::Currency as Currency<
        <T as frame_system::Config>::AccountId,
    >>::Balance;

    // ── Local trait: CitizenChecker ────────────────────────────────────────────

    /// Gate: returns false if the account is not a registered active citizen.
    /// Defined locally to avoid a dependency on pallet-voting or pallet-identity.
    /// The runtime implements this by delegating to pallet-identity's `is_active_citizen`.
    pub trait CitizenChecker<AccountId> {
        fn is_active_citizen(who: &AccountId) -> bool;
    }

    // ── Data types ─────────────────────────────────────────────────────────────

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum CandidateStatus {
        /// Registered but not yet reviewed by the commission.
        Registered,
        /// Certified as eligible by a commissioner.
        Certified,
        /// Disqualified by the commission (deposit still held until explicit release).
        Disqualified,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct CandidateInfo<AccountId, Balance> {
        /// IPFS content hash of the candidate profile document.
        pub profile_ipfs_hash: [u8; 32],
        pub status: CandidateStatus,
        /// Amount reserved from the candidate's balance.
        pub deposit: Balance,
        /// Redundant but convenient for iteration / off-chain queries.
        pub account: AccountId,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum ElectionStatus {
        /// Accepting candidate registrations and votes.
        Open,
        /// Commissioner has submitted results; awaiting certification.
        ResultsSubmitted,
        /// Results certified; deposits returned.
        Certified,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct ElectionInfo<AccountId, BlockNumber> {
        /// Short human-readable label for the office (e.g. "President", "District 3 MP").
        pub office: BoundedVec<u8, ConstU32<64>>,
        pub start_block: BlockNumber,
        pub end_block: BlockNumber,
        pub status: ElectionStatus,
        /// Set when results are submitted.
        pub winner: Option<AccountId>,
        /// IPFS hash of the full results document.
        pub results_ipfs_hash: Option<[u8; 32]>,
    }

    // ── Pallet struct ──────────────────────────────────────────────────────────

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    // ── Config ─────────────────────────────────────────────────────────────────

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Deposit reserved from a candidate when they register. Unreserved after certification.
        #[pallet::constant]
        type CandidateDeposit: Get<BalanceOf<Self>>;

        /// Maximum number of election commissioners.
        #[pallet::constant]
        type MaxCommissioners: Get<u32>;

        /// Maximum number of candidates per election.
        #[pallet::constant]
        type MaxCandidatesPerElection: Get<u32>;

        /// Currency used for candidate deposits.
        type Currency: ReservableCurrency<Self::AccountId>;

        /// Checks whether an account is an active registered citizen.
        type CitizenChecker: CitizenChecker<Self::AccountId>;
    }

    // ── Storage ────────────────────────────────────────────────────────────────

    /// Ordered list of current election commissioners.
    #[pallet::storage]
    pub type Commissioners<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxCommissioners>, ValueQuery>;

    /// election_id → ElectionInfo
    #[pallet::storage]
    pub type Elections<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, ElectionInfo<T::AccountId, BlockNumberFor<T>>>;

    /// (election_id, candidate_account) → CandidateInfo
    #[pallet::storage]
    pub type Candidates<T: Config> = StorageDoubleMap<
        _,
        Blake2_128Concat,
        u32,
        Blake2_128Concat,
        T::AccountId,
        CandidateInfo<T::AccountId, BalanceOf<T>>,
    >;

    /// Monotonically increasing counter for election IDs.
    #[pallet::storage]
    pub type NextElectionId<T: Config> = StorageValue<_, u32, ValueQuery>;

    // ── Events ─────────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// A new commissioner was added by root.
        CommissionerAdded { account: T::AccountId },
        /// A commissioner was removed by root.
        CommissionerRemoved { account: T::AccountId },
        /// A new election was created.
        ElectionCreated { id: u32 },
        /// A citizen registered as a candidate.
        CandidateRegistered { election_id: u32, candidate: T::AccountId },
        /// A commissioner certified a candidate as eligible.
        CandidateCertified { election_id: u32, candidate: T::AccountId },
        /// A commissioner submitted election results.
        ResultsSubmitted { election_id: u32, winner: T::AccountId },
        /// A commissioner certified the election results; deposits returned.
        ElectionCertified { election_id: u32, winner: T::AccountId },
    }

    // ── Errors ─────────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        /// The caller is not a commissioner.
        NotCommissioner,
        /// The account is not a registered active citizen.
        NotActiveCitizen,
        /// The election ID does not exist.
        ElectionNotFound,
        /// The candidate has already registered for this election.
        CandidateAlreadyRegistered,
        /// Results have already been submitted for this election.
        ResultsAlreadySubmitted,
        /// The election is not in the Open state.
        ElectionNotOpen,
        /// The election has already been certified.
        AlreadyCertified,
        /// The candidate was not found for this election.
        CandidateNotFound,
        /// The commissioners list is already at maximum capacity.
        TooManyCommissioners,
        /// The balance is insufficient to cover the candidate deposit.
        InsufficientBalance,
    }

    // ── Calls ──────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Add an account as an election commissioner. Only root.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn add_commissioner(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            Commissioners::<T>::try_mutate(|commissioners| {
                if commissioners.contains(&account) {
                    return Ok(()); // idempotent
                }
                commissioners
                    .try_push(account.clone())
                    .map_err(|_| Error::<T>::TooManyCommissioners)?;
                Self::deposit_event(Event::CommissionerAdded { account });
                Ok(())
            })
        }

        /// Remove an account from the commissioners list. Only root.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn remove_commissioner(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            Commissioners::<T>::try_mutate(|commissioners| {
                let pos = commissioners.iter().position(|a| a == &account);
                if let Some(idx) = pos {
                    commissioners.remove(idx);
                    Self::deposit_event(Event::CommissionerRemoved { account });
                }
                Ok(())
            })
        }

        /// Create a new election. Callable by root or a commissioner.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn create_election(
            origin: OriginFor<T>,
            office: BoundedVec<u8, ConstU32<64>>,
            start_block: BlockNumberFor<T>,
            end_block: BlockNumberFor<T>,
        ) -> DispatchResult {
            // Accept either root or a signed commissioner.
            match ensure_signed(origin.clone()) {
                Ok(signed) => Self::ensure_commissioner_account(&signed)?,
                Err(_) => ensure_root(origin)?,
            }

            let id = NextElectionId::<T>::get();
            Elections::<T>::insert(
                id,
                ElectionInfo {
                    office,
                    start_block,
                    end_block,
                    status: ElectionStatus::Open,
                    winner: None,
                    results_ipfs_hash: None,
                },
            );
            NextElectionId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::ElectionCreated { id });
            Ok(())
        }

        /// Register as a candidate for an election.
        /// The caller must be an active citizen. Reserves `CandidateDeposit`.
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn register_candidate(
            origin: OriginFor<T>,
            election_id: u32,
            profile_ipfs_hash: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(
                T::CitizenChecker::is_active_citizen(&who),
                Error::<T>::NotActiveCitizen
            );
            let election = Elections::<T>::get(election_id)
                .ok_or(Error::<T>::ElectionNotFound)?;
            ensure!(election.status == ElectionStatus::Open, Error::<T>::ElectionNotOpen);
            ensure!(
                !Candidates::<T>::contains_key(election_id, &who),
                Error::<T>::CandidateAlreadyRegistered
            );

            let deposit = T::CandidateDeposit::get();
            T::Currency::reserve(&who, deposit).map_err(|_| Error::<T>::InsufficientBalance)?;

            Candidates::<T>::insert(
                election_id,
                &who,
                CandidateInfo {
                    profile_ipfs_hash,
                    status: CandidateStatus::Registered,
                    deposit,
                    account: who.clone(),
                },
            );
            Self::deposit_event(Event::CandidateRegistered { election_id, candidate: who });
            Ok(())
        }

        /// Certify a candidate as eligible. Commissioner only.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn certify_candidate(
            origin: OriginFor<T>,
            election_id: u32,
            candidate: T::AccountId,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_commissioner_account(&who)?;
            ensure!(Elections::<T>::contains_key(election_id), Error::<T>::ElectionNotFound);
            Candidates::<T>::try_mutate(election_id, &candidate, |maybe_info| {
                let info = maybe_info.as_mut().ok_or(Error::<T>::CandidateNotFound)?;
                info.status = CandidateStatus::Certified;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::CandidateCertified { election_id, candidate });
            Ok(())
        }

        /// Submit election results. Commissioner only. Election must be Open.
        /// Sets election status to ResultsSubmitted.
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn submit_results(
            origin: OriginFor<T>,
            election_id: u32,
            winner: T::AccountId,
            results_ipfs_hash: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_commissioner_account(&who)?;
            Elections::<T>::try_mutate(election_id, |maybe_election| {
                let election = maybe_election.as_mut().ok_or(Error::<T>::ElectionNotFound)?;
                ensure!(
                    election.status == ElectionStatus::Open,
                    Error::<T>::ElectionNotOpen
                );
                election.status = ElectionStatus::ResultsSubmitted;
                election.winner = Some(winner.clone());
                election.results_ipfs_hash = Some(results_ipfs_hash);
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::ResultsSubmitted { election_id, winner });
            Ok(())
        }

        /// Certify the election results. Commissioner only.
        /// Sets status to Certified and unreserves all candidate deposits.
        #[pallet::call_index(6)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn certify_results(origin: OriginFor<T>, election_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_commissioner_account(&who)?;
            let winner = Elections::<T>::try_mutate(election_id, |maybe_election| {
                let election = maybe_election.as_mut().ok_or(Error::<T>::ElectionNotFound)?;
                ensure!(
                    election.status != ElectionStatus::Certified,
                    Error::<T>::AlreadyCertified
                );
                ensure!(
                    election.status == ElectionStatus::ResultsSubmitted,
                    Error::<T>::ResultsAlreadySubmitted
                );
                election.status = ElectionStatus::Certified;
                Ok::<Option<T::AccountId>, DispatchError>(election.winner.clone())
            })?;

            // Unreserve deposits for all candidates in this election.
            let candidates: alloc::vec::Vec<_> =
                Candidates::<T>::iter_prefix(election_id).collect();
            for (account, info) in candidates {
                T::Currency::unreserve(&account, info.deposit);
            }

            // winner is always Some when status was ResultsSubmitted (set by submit_results).
            let winner_account = winner.ok_or(Error::<T>::ElectionNotFound)?;
            Self::deposit_event(Event::ElectionCertified {
                election_id,
                winner: winner_account,
            });
            Ok(())
        }
    }

    // ── Internal helpers ───────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
        /// Check that `account` is in the Commissioners list.
        fn ensure_commissioner_account(account: &T::AccountId) -> DispatchResult {
            let commissioners = Commissioners::<T>::get();
            ensure!(commissioners.contains(account), Error::<T>::NotCommissioner);
            Ok(())
        }
    }
}
