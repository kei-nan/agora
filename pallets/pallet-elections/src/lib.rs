//! # Elections Pallet
//!
//! Manages two separate concerns:
//!
//! ## A) Elections Commission
//! Handles office elections (commissioners, candidates, results certification).
//! Flow: root adds commissioners → commissioner creates election → citizens register as
//! candidates → commissioner certifies eligible candidates → commissioner submits results
//! → commissioner certifies results.
//!
//! ## B) Liquid Democracy Delegates + Legislature Elections
//! Manages the public delegate registry for vote delegation, and runs periodic elections
//! to seat the top-N delegates (by backing count) into pallet-legislature.
//!
//! ### Delegate identity
//! Separate from citizen identity: uses `Poseidon2(national_id || country_code || "delegate")`
//! as nullifier, so the citizen and delegate on-chain identities are cryptographically unlinked.
//! The delegate voluntarily publishes their real name; citizens remain anonymous.
//!
//! ### Backing threshold
//! A delegate becomes Active only when they have ≥ `BackingThreshold` citizen backers.
//! Each citizen may back at most `MaxBackingsPerCitizen` delegates (constitutional parameter,
//! default 5). This makes backing a meaningful signal rather than noise.
//!
//! ### Legislature elections
//! Every `ElectionCycleBlocks` blocks, `on_initialize` ranks all Active delegates by backing
//! count and seats the top `LegislatureSeats` into pallet-legislature via `SeatLegislature`.
//! All three parameters are constitutional (supermajority to change).
//! Defaults: 100 seats, 2-year cycle, max 5 backings per citizen.
//!
//! ### Term limits
//! All parameters are constitutional (supermajority to change):
//! - `TermLengthBlocks`: length of a single term.
//! - `MaxConsecutiveTerms`: max back-to-back terms before a mandatory break.
//! - `MandatoryBreakBlocks`: how long the break must last.
//! - `WarningWindowPct`: what fraction of the final term triggers a warning event (1–50 %).
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
    use sp_runtime::traits::{Saturating, Zero};

    // ── Balance type alias ─────────────────────────────────────────────────────

    pub type BalanceOf<T> = <<T as Config>::Currency as Currency<
        <T as frame_system::Config>::AccountId,
    >>::Balance;

    // ── Cross-pallet traits ────────────────────────────────────────────────────

    pub trait CitizenChecker<AccountId> {
        fn is_active_citizen(who: &AccountId) -> bool;
    }

    /// Called by pallet-elections at the end of each election cycle.
    /// The implementation in pallet-legislature replaces the full Members set.
    pub trait SeatLegislature<AccountId> {
        fn replace_members(winners: alloc::vec::Vec<AccountId>) -> DispatchResult;
    }

    // ── Data types: Elections Commission ──────────────────────────────────────

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum CandidateStatus {
        Registered,
        Certified,
        Disqualified,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct CandidateInfo<AccountId, Balance> {
        pub profile_ipfs_hash: [u8; 32],
        pub status: CandidateStatus,
        pub deposit: Balance,
        pub account: AccountId,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum ElectionStatus {
        Open,
        ResultsSubmitted,
        Certified,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct ElectionInfo<AccountId, BlockNumber> {
        pub office: BoundedVec<u8, ConstU32<64>>,
        pub start_block: BlockNumber,
        pub end_block: BlockNumber,
        pub status: ElectionStatus,
        pub winner: Option<AccountId>,
        pub results_ipfs_hash: Option<[u8; 32]>,
    }

    // ── Data types: Delegates ──────────────────────────────────────────────────

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum DelegateStatus {
        /// Registered but below the backing threshold — cannot yet receive delegations.
        Pending,
        /// At or above the backing threshold and within term limits — can receive delegations.
        Active,
        /// Served `MaxConsecutiveTerms` back-to-back; must wait until `break_until_block`.
        OnBreak,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct DelegateInfo<BlockNumber> {
        /// Publicly visible real name.
        pub display_name: BoundedVec<u8, ConstU32<64>>,
        /// IPFS hash of the delegate's public profile / policy positions document.
        pub profile_ipfs_hash: [u8; 32],
        pub status: DelegateStatus,
        /// How many consecutive terms this delegate has served.
        pub consecutive_terms: u32,
        /// Block at which the current term started. None when Pending or OnBreak.
        pub term_start_block: Option<BlockNumber>,
        /// Total blocks spent Active in the current term (for the >50% rule).
        pub active_blocks_this_term: BlockNumber,
        /// Block after which this delegate may return from a mandatory break.
        pub break_until_block: Option<BlockNumber>,
        /// Whether the warning event for the current term has already been emitted.
        pub warning_emitted: bool,
    }

    // ── Pallet struct ──────────────────────────────────────────────────────────

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    // ── Config ─────────────────────────────────────────────────────────────────

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        #[pallet::constant]
        type CandidateDeposit: Get<BalanceOf<Self>>;

        #[pallet::constant]
        type MaxCommissioners: Get<u32>;

        #[pallet::constant]
        type MaxCandidatesPerElection: Get<u32>;

        /// Hard cap on the number of registered delegates (bounds on_initialize iteration).
        #[pallet::constant]
        type MaxDelegates: Get<u32>;

        type Currency: ReservableCurrency<Self::AccountId>;
        type CitizenChecker: CitizenChecker<Self::AccountId>;

        /// Origin that can change `BackingThreshold` (ordinary supermajority governance).
        type GovernanceOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Origin that can change constitutional parameters (supermajority + HRC veto).
        type ConstitutionalOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Cross-pallet hook: called at the end of each election cycle to install winners.
        type LegislatureSeating: SeatLegislature<Self::AccountId>;

        /// Default number of legislature seats (constitutional, default 100).
        #[pallet::constant]
        type DefaultLegislatureSeats: Get<u32>;

        /// Default election cycle length in blocks (constitutional, default 2 years).
        #[pallet::constant]
        type DefaultElectionCycleBlocks: Get<u32>;

        /// Default max number of delegates a citizen may back simultaneously (constitutional, default 5).
        #[pallet::constant]
        type DefaultMaxBackingsPerCitizen: Get<u32>;
    }

    // ── Storage: Elections Commission ──────────────────────────────────────────

    #[pallet::storage]
    pub type Commissioners<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxCommissioners>, ValueQuery>;

    #[pallet::storage]
    pub type Elections<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, ElectionInfo<T::AccountId, BlockNumberFor<T>>>;

    #[pallet::storage]
    pub type Candidates<T: Config> = StorageDoubleMap<
        _,
        Blake2_128Concat, u32,
        Blake2_128Concat, T::AccountId,
        CandidateInfo<T::AccountId, BalanceOf<T>>,
    >;

    #[pallet::storage]
    pub type NextElectionId<T: Config> = StorageValue<_, u32, ValueQuery>;

    // ── Storage: Delegate registry ─────────────────────────────────────────────

    /// All registered delegates.
    #[pallet::storage]
    pub type Delegates<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, DelegateInfo<BlockNumberFor<T>>>;

    /// Number of citizens currently backing each delegate.
    #[pallet::storage]
    pub type BackingCount<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    /// (backer, delegate) → () — prevents a citizen from backing the same delegate twice.
    #[pallet::storage]
    pub type BackingOf<T: Config> = StorageDoubleMap<
        _,
        Blake2_128Concat, T::AccountId,
        Blake2_128Concat, T::AccountId,
        (),
    >;

    /// Number of delegates each citizen is currently backing.
    /// Enforced against MaxBackingsPerCitizen on every back_delegate call.
    #[pallet::storage]
    pub type CitizenBackingCount<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    // ── Storage: Governance-controlled parameters ──────────────────────────────

    /// Minimum citizen backers required to hold Active delegate status.
    #[pallet::storage]
    pub type BackingThreshold<T: Config> = StorageValue<_, u32, ValueQuery>;

    #[pallet::storage]
    pub type BackingThresholdFloor<T: Config> = StorageValue<_, u32, ValueQuery>;

    #[pallet::storage]
    pub type BackingThresholdCeiling<T: Config> = StorageValue<_, u32, ValueQuery>;

    #[pallet::storage]
    pub type TermLengthBlocks<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

    #[pallet::storage]
    pub type MaxConsecutiveTerms<T: Config> = StorageValue<_, u32, ValueQuery>;

    #[pallet::storage]
    pub type MandatoryBreakBlocks<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

    #[pallet::storage]
    pub type WarningWindowPct<T: Config> = StorageValue<_, u8, ValueQuery>;

    // ── Storage: Legislature election parameters (constitutional) ──────────────

    #[pallet::type_value]
    pub fn DefaultSeats<T: Config>() -> u32 { T::DefaultLegislatureSeats::get() }

    /// Number of legislature seats filled at each election. Constitutional.
    #[pallet::storage]
    pub type LegislatureSeats<T: Config> =
        StorageValue<_, u32, ValueQuery, DefaultSeats<T>>;

    #[pallet::type_value]
    pub fn DefaultCycle<T: Config>() -> u32 { T::DefaultElectionCycleBlocks::get() }

    /// Blocks between legislature elections. Constitutional.
    #[pallet::storage]
    pub type ElectionCycleBlocks<T: Config> =
        StorageValue<_, u32, ValueQuery, DefaultCycle<T>>;

    #[pallet::type_value]
    pub fn DefaultMaxBackings<T: Config>() -> u32 { T::DefaultMaxBackingsPerCitizen::get() }

    /// Max delegates a single citizen may back simultaneously. Constitutional.
    #[pallet::storage]
    pub type MaxBackingsPerCitizen<T: Config> =
        StorageValue<_, u32, ValueQuery, DefaultMaxBackings<T>>;

    /// Block at which the last legislature election ran. 0 = no election yet.
    /// First election fires at block ElectionCycleBlocks.
    #[pallet::storage]
    pub type LastElectionBlock<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

    // ── Events ─────────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        // ── Elections Commission ──
        CommissionerAdded { account: T::AccountId },
        CommissionerRemoved { account: T::AccountId },
        ElectionCreated { id: u32 },
        CandidateRegistered { election_id: u32, candidate: T::AccountId },
        CandidateCertified { election_id: u32, candidate: T::AccountId },
        ResultsSubmitted { election_id: u32, winner: T::AccountId },
        ElectionCertified { election_id: u32, winner: T::AccountId },

        // ── Delegates ──
        DelegateRegistered { delegate: T::AccountId, display_name: BoundedVec<u8, ConstU32<64>> },
        DelegateActivated { delegate: T::AccountId },
        DelegateDeactivated { delegate: T::AccountId },
        DelegateBacked { delegate: T::AccountId, backer: T::AccountId },
        DelegateBackingRemoved { delegate: T::AccountId, backer: T::AccountId },
        DelegateTermWarning { delegate: T::AccountId, blocks_remaining: BlockNumberFor<T> },
        DelegateTermExpired { delegate: T::AccountId },
        DelegateBreakEnded { delegate: T::AccountId },

        // ── Legislature elections ──
        /// Periodic election ran; `seated` delegates installed into the legislature.
        LegislatureElectionRun { at_block: BlockNumberFor<T>, seated: u32 },
        /// Constitutional election parameters were updated.
        ElectionParamsChanged { seats: u32, cycle_blocks: u32, max_backings_per_citizen: u32 },

        // ── Governance parameters ──
        BackingThresholdChanged { new_threshold: u32 },
        TermParamsChanged {
            term_length: BlockNumberFor<T>,
            max_consecutive: u32,
            mandatory_break: BlockNumberFor<T>,
            warning_pct: u8,
        },
        BackingBoundsChanged { floor: u32, ceiling: u32 },
    }

    // ── Errors ─────────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        NotCommissioner,
        NotActiveCitizen,
        ElectionNotFound,
        CandidateAlreadyRegistered,
        ResultsAlreadySubmitted,
        ElectionNotOpen,
        AlreadyCertified,
        CandidateNotFound,
        TooManyCommissioners,
        InsufficientBalance,
        AlreadyRegisteredAsDelegate,
        DelegateNotFound,
        AlreadyBacking,
        NotBacking,
        CannotBackSelf,
        DelegateOnBreak,
        BackingThresholdOutOfBounds,
        WarningPctInvalid,
        FloorExceedsCeiling,
        ThresholdBelowFloor,
        ThresholdAboveCeiling,
        /// Citizen has reached MaxBackingsPerCitizen and cannot back more delegates.
        BackingLimitReached,
        /// Legislature seat count must be at least 1.
        ElectionSeatsZero,
    }

    // ── on_initialize: term warnings, expirations, and legislature elections ───

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        fn on_initialize(now: BlockNumberFor<T>) -> Weight {
            let mut weight = Weight::zero();

            // ── Legislature election cycle ──────────────────────────────────────
            let last = LastElectionBlock::<T>::get();
            let cycle: BlockNumberFor<T> = ElectionCycleBlocks::<T>::get().into();
            if !cycle.is_zero() && !now.is_zero() && now >= last.saturating_add(cycle) {
                weight = weight.saturating_add(Self::run_election(now));
            }

            // ── Term warnings and expirations ───────────────────────────────────
            let term_length = TermLengthBlocks::<T>::get();
            let max_consecutive = MaxConsecutiveTerms::<T>::get();
            let break_blocks = MandatoryBreakBlocks::<T>::get();
            let warning_pct = WarningWindowPct::<T>::get();

            let hundred: BlockNumberFor<T> = 100u32.into();
            let complement: BlockNumberFor<T> = (100u32.saturating_sub(warning_pct as u32)).into();
            let warning_offset: BlockNumberFor<T> = (term_length / hundred) * complement;

            let delegates: alloc::vec::Vec<_> = Delegates::<T>::iter().collect();
            for (account, mut info) in delegates {
                weight = weight.saturating_add(T::DbWeight::get().reads(1));

                match info.status {
                    DelegateStatus::Active => {
                        let term_start = match info.term_start_block {
                            Some(b) => b,
                            None => continue,
                        };
                        let blocks_elapsed = now.saturating_sub(term_start);

                        if !info.warning_emitted && blocks_elapsed >= warning_offset {
                            let blocks_remaining = term_length.saturating_sub(blocks_elapsed);
                            Self::deposit_event(Event::DelegateTermWarning {
                                delegate: account.clone(),
                                blocks_remaining,
                            });
                            info.warning_emitted = true;
                            Delegates::<T>::insert(&account, &info);
                            weight = weight.saturating_add(T::DbWeight::get().writes(1));
                        }

                        if blocks_elapsed >= term_length {
                            let half: BlockNumberFor<T> = 2u32.into();
                            let counts_as_full =
                                info.active_blocks_this_term >= term_length / half;

                            if counts_as_full {
                                info.consecutive_terms =
                                    info.consecutive_terms.saturating_add(1);
                            }

                            if info.consecutive_terms >= max_consecutive {
                                info.status = DelegateStatus::OnBreak;
                                info.break_until_block =
                                    Some(now.saturating_add(break_blocks));
                            } else {
                                info.term_start_block = Some(now);
                                info.active_blocks_this_term = 0u32.into();
                                info.warning_emitted = false;
                            }

                            Self::deposit_event(Event::DelegateTermExpired {
                                delegate: account.clone(),
                            });
                            Delegates::<T>::insert(&account, &info);
                            weight = weight.saturating_add(T::DbWeight::get().writes(1));
                        } else {
                            let one: BlockNumberFor<T> = 1u32.into();
                            info.active_blocks_this_term =
                                info.active_blocks_this_term.saturating_add(one);
                            Delegates::<T>::insert(&account, &info);
                            weight = weight.saturating_add(T::DbWeight::get().writes(1));
                        }
                    }
                    DelegateStatus::OnBreak => {
                        if let Some(until) = info.break_until_block {
                            if now >= until {
                                info.status = DelegateStatus::Pending;
                                info.consecutive_terms = 0;
                                info.term_start_block = None;
                                info.active_blocks_this_term = 0u32.into();
                                info.break_until_block = None;
                                info.warning_emitted = false;
                                Delegates::<T>::insert(&account, &info);
                                Self::deposit_event(Event::DelegateBreakEnded {
                                    delegate: account.clone(),
                                });
                                weight = weight.saturating_add(T::DbWeight::get().writes(1));

                                let count = BackingCount::<T>::get(&account);
                                if count >= BackingThreshold::<T>::get() {
                                    Self::activate_delegate(&account);
                                    weight =
                                        weight.saturating_add(T::DbWeight::get().writes(1));
                                }
                            }
                        }
                    }
                    DelegateStatus::Pending => {}
                }
            }

            weight
        }
    }

    // ── Calls ──────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        // ── Elections Commission ───────────────────────────────────────────────

        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn add_commissioner(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            Commissioners::<T>::try_mutate(|commissioners| {
                if commissioners.contains(&account) { return Ok(()); }
                commissioners.try_push(account.clone()).map_err(|_| Error::<T>::TooManyCommissioners)?;
                Self::deposit_event(Event::CommissionerAdded { account });
                Ok(())
            })
        }

        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn remove_commissioner(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            Commissioners::<T>::try_mutate(|commissioners| {
                if let Some(idx) = commissioners.iter().position(|a| a == &account) {
                    commissioners.remove(idx);
                    Self::deposit_event(Event::CommissionerRemoved { account });
                }
                Ok(())
            })
        }

        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn create_election(
            origin: OriginFor<T>,
            office: BoundedVec<u8, ConstU32<64>>,
            start_block: BlockNumberFor<T>,
            end_block: BlockNumberFor<T>,
        ) -> DispatchResult {
            match ensure_signed(origin.clone()) {
                Ok(signed) => Self::ensure_commissioner_account(&signed)?,
                Err(_) => ensure_root(origin)?,
            }
            let id = NextElectionId::<T>::get();
            Elections::<T>::insert(id, ElectionInfo {
                office, start_block, end_block,
                status: ElectionStatus::Open, winner: None, results_ipfs_hash: None,
            });
            NextElectionId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::ElectionCreated { id });
            Ok(())
        }

        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn register_candidate(
            origin: OriginFor<T>,
            election_id: u32,
            profile_ipfs_hash: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::NotActiveCitizen);
            let election = Elections::<T>::get(election_id).ok_or(Error::<T>::ElectionNotFound)?;
            ensure!(election.status == ElectionStatus::Open, Error::<T>::ElectionNotOpen);
            ensure!(!Candidates::<T>::contains_key(election_id, &who), Error::<T>::CandidateAlreadyRegistered);
            let deposit = T::CandidateDeposit::get();
            T::Currency::reserve(&who, deposit).map_err(|_| Error::<T>::InsufficientBalance)?;
            Candidates::<T>::insert(election_id, &who, CandidateInfo {
                profile_ipfs_hash, status: CandidateStatus::Registered, deposit, account: who.clone(),
            });
            Self::deposit_event(Event::CandidateRegistered { election_id, candidate: who });
            Ok(())
        }

        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn certify_candidate(
            origin: OriginFor<T>, election_id: u32, candidate: T::AccountId,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_commissioner_account(&who)?;
            ensure!(Elections::<T>::contains_key(election_id), Error::<T>::ElectionNotFound);
            Candidates::<T>::try_mutate(election_id, &candidate, |maybe| {
                let info = maybe.as_mut().ok_or(Error::<T>::CandidateNotFound)?;
                info.status = CandidateStatus::Certified;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::CandidateCertified { election_id, candidate });
            Ok(())
        }

        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn submit_results(
            origin: OriginFor<T>, election_id: u32, winner: T::AccountId,
            results_ipfs_hash: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_commissioner_account(&who)?;
            Elections::<T>::try_mutate(election_id, |maybe| {
                let election = maybe.as_mut().ok_or(Error::<T>::ElectionNotFound)?;
                ensure!(election.status == ElectionStatus::Open, Error::<T>::ElectionNotOpen);
                election.status = ElectionStatus::ResultsSubmitted;
                election.winner = Some(winner.clone());
                election.results_ipfs_hash = Some(results_ipfs_hash);
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::ResultsSubmitted { election_id, winner });
            Ok(())
        }

        #[pallet::call_index(6)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn certify_results(origin: OriginFor<T>, election_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_commissioner_account(&who)?;
            let winner = Elections::<T>::try_mutate(election_id, |maybe| {
                let election = maybe.as_mut().ok_or(Error::<T>::ElectionNotFound)?;
                ensure!(election.status != ElectionStatus::Certified, Error::<T>::AlreadyCertified);
                ensure!(election.status == ElectionStatus::ResultsSubmitted, Error::<T>::ResultsAlreadySubmitted);
                election.status = ElectionStatus::Certified;
                Ok::<Option<T::AccountId>, DispatchError>(election.winner.clone())
            })?;
            let candidates: alloc::vec::Vec<_> =
                Candidates::<T>::iter_prefix(election_id).collect();
            for (account, info) in candidates {
                T::Currency::unreserve(&account, info.deposit);
            }
            let winner_account = winner.ok_or(Error::<T>::ElectionNotFound)?;
            Self::deposit_event(Event::ElectionCertified { election_id, winner: winner_account });
            Ok(())
        }

        // ── Delegate registry ──────────────────────────────────────────────────

        #[pallet::call_index(7)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn register_as_delegate(
            origin: OriginFor<T>,
            display_name: BoundedVec<u8, ConstU32<64>>,
            profile_ipfs_hash: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::NotActiveCitizen);
            ensure!(!Delegates::<T>::contains_key(&who), Error::<T>::AlreadyRegisteredAsDelegate);
            Delegates::<T>::insert(&who, DelegateInfo {
                display_name: display_name.clone(),
                profile_ipfs_hash,
                status: DelegateStatus::Pending,
                consecutive_terms: 0,
                term_start_block: None,
                active_blocks_this_term: Zero::zero(),
                break_until_block: None,
                warning_emitted: false,
            });
            Self::deposit_event(Event::DelegateRegistered { delegate: who, display_name });
            Ok(())
        }

        /// Back a delegate. Each citizen may back at most `MaxBackingsPerCitizen` delegates.
        /// If this backing pushes the delegate to or above the threshold, they become Active.
        #[pallet::call_index(8)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn back_delegate(origin: OriginFor<T>, delegate: T::AccountId) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::NotActiveCitizen);
            ensure!(who != delegate, Error::<T>::CannotBackSelf);
            let info = Delegates::<T>::get(&delegate).ok_or(Error::<T>::DelegateNotFound)?;
            ensure!(info.status != DelegateStatus::OnBreak, Error::<T>::DelegateOnBreak);
            ensure!(!BackingOf::<T>::contains_key(&who, &delegate), Error::<T>::AlreadyBacking);

            // Enforce per-citizen backing cap.
            let citizen_count = CitizenBackingCount::<T>::get(&who);
            ensure!(citizen_count < MaxBackingsPerCitizen::<T>::get(), Error::<T>::BackingLimitReached);

            BackingOf::<T>::insert(&who, &delegate, ());
            CitizenBackingCount::<T>::mutate(&who, |c| *c = c.saturating_add(1));
            let new_count = BackingCount::<T>::get(&delegate).saturating_add(1);
            BackingCount::<T>::insert(&delegate, new_count);
            Self::deposit_event(Event::DelegateBacked { delegate: delegate.clone(), backer: who });

            if new_count >= BackingThreshold::<T>::get()
                && Delegates::<T>::get(&delegate)
                    .map_or(false, |d| d.status == DelegateStatus::Pending)
            {
                Self::activate_delegate(&delegate);
            }
            Ok(())
        }

        /// Remove backing from a delegate. Frees one slot in the citizen's backing allowance.
        #[pallet::call_index(9)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn remove_backing(origin: OriginFor<T>, delegate: T::AccountId) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(BackingOf::<T>::contains_key(&who, &delegate), Error::<T>::NotBacking);

            BackingOf::<T>::remove(&who, &delegate);
            CitizenBackingCount::<T>::mutate(&who, |c| *c = c.saturating_sub(1));
            let new_count = BackingCount::<T>::get(&delegate).saturating_sub(1);
            BackingCount::<T>::insert(&delegate, new_count);
            Self::deposit_event(Event::DelegateBackingRemoved {
                delegate: delegate.clone(), backer: who,
            });

            if new_count < BackingThreshold::<T>::get()
                && Delegates::<T>::get(&delegate)
                    .map_or(false, |d| d.status == DelegateStatus::Active)
            {
                Delegates::<T>::mutate(&delegate, |maybe| {
                    if let Some(d) = maybe { d.status = DelegateStatus::Pending; }
                });
                Self::deposit_event(Event::DelegateDeactivated { delegate });
            }
            Ok(())
        }

        // ── Governance: ordinary supermajority ────────────────────────────────

        #[pallet::call_index(10)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn set_backing_threshold(origin: OriginFor<T>, threshold: u32) -> DispatchResult {
            T::GovernanceOrigin::ensure_origin(origin)?;
            ensure!(threshold >= BackingThresholdFloor::<T>::get(), Error::<T>::ThresholdBelowFloor);
            ensure!(threshold <= BackingThresholdCeiling::<T>::get(), Error::<T>::ThresholdAboveCeiling);
            BackingThreshold::<T>::put(threshold);
            Self::deposit_event(Event::BackingThresholdChanged { new_threshold: threshold });
            Ok(())
        }

        // ── Governance: constitutional supermajority ──────────────────────────

        #[pallet::call_index(11)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn set_backing_bounds(origin: OriginFor<T>, floor: u32, ceiling: u32) -> DispatchResult {
            T::ConstitutionalOrigin::ensure_origin(origin)?;
            ensure!(floor <= ceiling, Error::<T>::FloorExceedsCeiling);
            BackingThresholdFloor::<T>::put(floor);
            BackingThresholdCeiling::<T>::put(ceiling);
            let current = BackingThreshold::<T>::get();
            if current < floor { BackingThreshold::<T>::put(floor); }
            if current > ceiling { BackingThreshold::<T>::put(ceiling); }
            Self::deposit_event(Event::BackingBoundsChanged { floor, ceiling });
            Ok(())
        }

        #[pallet::call_index(12)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn set_term_params(
            origin: OriginFor<T>,
            term_length: BlockNumberFor<T>,
            max_consecutive: u32,
            mandatory_break: BlockNumberFor<T>,
            warning_pct: u8,
        ) -> DispatchResult {
            T::ConstitutionalOrigin::ensure_origin(origin)?;
            ensure!(warning_pct >= 1 && warning_pct <= 50, Error::<T>::WarningPctInvalid);
            TermLengthBlocks::<T>::put(term_length);
            MaxConsecutiveTerms::<T>::put(max_consecutive);
            MandatoryBreakBlocks::<T>::put(mandatory_break);
            WarningWindowPct::<T>::put(warning_pct);
            Self::deposit_event(Event::TermParamsChanged {
                term_length, max_consecutive, mandatory_break, warning_pct,
            });
            Ok(())
        }

        /// Update constitutional election parameters. Any field left as None is unchanged.
        #[pallet::call_index(13)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn set_election_params(
            origin: OriginFor<T>,
            seats: Option<u32>,
            cycle_blocks: Option<u32>,
            max_backings_per_citizen: Option<u32>,
        ) -> DispatchResult {
            T::ConstitutionalOrigin::ensure_origin(origin)?;
            if let Some(s) = seats {
                ensure!(s > 0, Error::<T>::ElectionSeatsZero);
                LegislatureSeats::<T>::put(s);
            }
            if let Some(c) = cycle_blocks {
                ElectionCycleBlocks::<T>::put(c);
            }
            if let Some(m) = max_backings_per_citizen {
                MaxBackingsPerCitizen::<T>::put(m);
            }
            Self::deposit_event(Event::ElectionParamsChanged {
                seats: LegislatureSeats::<T>::get(),
                cycle_blocks: ElectionCycleBlocks::<T>::get(),
                max_backings_per_citizen: MaxBackingsPerCitizen::<T>::get(),
            });
            Ok(())
        }
    }

    // ── Internal helpers ───────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
        fn ensure_commissioner_account(account: &T::AccountId) -> DispatchResult {
            let commissioners = Commissioners::<T>::get();
            ensure!(commissioners.contains(account), Error::<T>::NotCommissioner);
            Ok(())
        }

        fn activate_delegate(delegate: &T::AccountId) {
            let now = frame_system::Pallet::<T>::block_number();
            Delegates::<T>::mutate(delegate, |maybe| {
                if let Some(d) = maybe {
                    d.status = DelegateStatus::Active;
                    d.term_start_block = Some(now);
                    d.active_blocks_this_term = 0u32.into();
                    d.warning_emitted = false;
                }
            });
            Self::deposit_event(Event::DelegateActivated { delegate: delegate.clone() });
        }

        /// Run a legislature election: rank Active delegates by backing count, seat the top N.
        fn run_election(now: BlockNumberFor<T>) -> Weight {
            let seats = LegislatureSeats::<T>::get() as usize;

            let mut candidates: alloc::vec::Vec<(T::AccountId, u32)> = Delegates::<T>::iter()
                .filter_map(|(addr, info)| {
                    if info.status == DelegateStatus::Active {
                        Some((addr.clone(), BackingCount::<T>::get(&addr)))
                    } else {
                        None
                    }
                })
                .collect();

            // Stable sort by backing count descending — ties broken by storage order.
            candidates.sort_by(|a, b| b.1.cmp(&a.1));

            let winners: alloc::vec::Vec<T::AccountId> = candidates
                .into_iter()
                .take(seats)
                .map(|(addr, _)| addr)
                .collect();

            let seated = winners.len() as u32;
            let _ = T::LegislatureSeating::replace_members(winners);
            LastElectionBlock::<T>::put(now);

            Self::deposit_event(Event::LegislatureElectionRun { at_block: now, seated });

            T::DbWeight::get().reads_writes(3, 2)
        }
    }
}
