//! # Executive Pallet
//!
//! Parliamentary executive for Agora. The legislature appoints ministers to named portfolios
//! via a passed motion. A Prime Minister coordinates the cabinet.
//!
//! ## Separation-of-powers enforcement
//!
//! Active ministers (portfolio holders and the Prime Minister) are blocked from voting on
//! legislature motions. This is enforced by pallet-legislature calling
//! `MinisterChecker::is_active_minister` before recording a vote.
//!
//! ## Emergency powers
//!
//! The cabinet (PM + supermajority of ministers) can declare a time-limited state of
//! emergency. Once declared, the legislature has `RatificationWindowBlocks` to ratify it via
//! a passed motion — if they don't, the emergency lapses automatically. The emergency also
//! auto-expires at its hard sunset block regardless of ratification.
//!
//! Flow:
//!   1. Any minister or PM calls `vote_declare_emergency(reason_hash, duration_blocks)`.
//!   2. When a 2/3 supermajority of the cabinet has voted, the emergency activates.
//!   3. Legislature must call `ratify_emergency` within `RatificationWindowBlocks` or it lapses.
//!   4. The emergency auto-expires at `expires_at` (checked in `on_initialize`).
//!   5. Cabinet may vote to end it early via `vote_end_emergency`.
//!
//! `ActiveEmergency::get().is_some()` is the canonical "is there an emergency?" signal for
//! other pallets to check. Only ratified emergencies remain in storage past the window.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod weights;
pub use weights::WeightInfo;

#[frame_support::pallet]
pub mod pallet {
    use codec::{Decode, DecodeWithMemTracking, Encode};
    use frame_support::pallet_prelude::*;
    use frame_support::traits::EnsureOriginWithArg;
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::{Saturating, Zero};
    use crate::weights::WeightInfo;

    /// Computes the domain-separated hash a legislature motion's `call_hash` must equal for
    /// `LegislatureOrigin` to authorize `tag`'s call with `params`. See
    /// `pallet_constitution::pallet::legislature_call_hash` for the full rationale.
    pub(crate) fn legislature_call_hash(tag: &'static [u8], params: impl Encode) -> [u8; 32] {
        let mut preimage = alloc::vec::Vec::from(tag);
        preimage.extend(params.encode());
        frame_support::Hashable::blake2_256(&preimage)
    }

    // ── Pallet ───────────────────────────────────────────────────────────────────

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    // ── Origin ───────────────────────────────────────────────────────────────────

    /// Origin that passes only when the signer is a current minister or Prime Minister.
    pub struct EnsureExecutiveMinister<T>(core::marker::PhantomData<T>);

    impl<T: Config> frame_support::traits::EnsureOrigin<T::RuntimeOrigin>
        for EnsureExecutiveMinister<T>
    {
        type Success = T::AccountId;

        fn try_origin(o: T::RuntimeOrigin) -> Result<Self::Success, T::RuntimeOrigin> {
            use frame_system::RawOrigin;
            match o.clone().into() {
                Ok(RawOrigin::Signed(who))
                    if MinisterPortfolio::<T>::contains_key(&who)
                        || PrimeMinister::<T>::get().as_ref() == Some(&who) =>
                {
                    Ok(who)
                }
                _ => Err(o),
            }
        }

        #[cfg(feature = "runtime-benchmarks")]
        fn try_successful_origin() -> Result<T::RuntimeOrigin, ()> {
            let pm = PrimeMinister::<T>::get().ok_or(())?;
            Ok(frame_system::RawOrigin::Signed(pm).into())
        }
    }

    // ── Cross-pallet traits ─────────────────────────────────────────────────────

    /// Checks whether an account is currently an active (non-suspended) registered
    /// citizen, and separately whether any current suspension came from a jury-reviewed
    /// conviction rather than a bare, unappealed AI ruling. Implemented by
    /// pallet-identity-zk in the runtime. The daily vacancy sweep (`on_initialize`)
    /// deliberately requires the *second* check, not just the first, before auto-removing
    /// a sitting PM/minister — see `run_vacancy_sweep`'s doc comment for why: the
    /// consequence (losing the office) is large enough to warrant the same evidentiary
    /// bar this project's court system already applies elsewhere, and requiring it here
    /// also closes off filing a bogus, never-appealed case as a way to force someone out.
    pub trait CitizenChecker<AccountId> {
        fn is_active_citizen(who: &AccountId) -> bool;
        fn is_suspended_by_jury_reviewed_conviction(who: &AccountId) -> bool;
    }

    /// Checks whether an account currently holds a legislature seat. Implemented by
    /// pallet-legislature in the runtime. PM nominees, ballot casters, and
    /// no-confidence successors must all be current members — the PM is chosen by and
    /// from the legislature, not the general citizenry.
    pub trait LegislatureMembership<AccountId> {
        fn is_member(who: &AccountId) -> bool;
    }

    /// Benchmark-only seeding for `CitizenChecker`/`LegislatureMembership` state, which the
    /// real runtime backs with pallet-identity-zk/pallet-legislature storage this pallet
    /// otherwise has no generic way to reach into from a benchmark. Same pattern as
    /// `pallet_elections::BenchmarkHelper` — see its doc comment.
    #[cfg(feature = "runtime-benchmarks")]
    pub trait BenchmarkHelper<AccountId> {
        fn make_active_citizen(who: &AccountId);
        fn make_legislature_member(who: &AccountId);
    }

    // ── Config ───────────────────────────────────────────────────────────────────

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Origin that may appoint/dismiss ministers. Wire to EnsureLegislatureMotion.
        /// `EnsureOriginWithArg` so each call site must pass the domain-separated hash of
        /// its own parameters (see `legislature_call_hash`); the origin then verifies that
        /// hash against the motion's approved `call_hash`, so a motion passed to authorize
        /// one call (e.g. `enact_law` in another pallet) can never be replayed to execute
        /// an unrelated one here (e.g. `appoint_minister`), and vice versa.
        type LegislatureOrigin: frame_support::traits::EnsureOriginWithArg<Self::RuntimeOrigin, [u8; 32]>;
        /// Maximum number of portfolios (cabinet size ceiling).
        #[pallet::constant]
        type MaxPortfolios: Get<u32>;
        /// Constitutional ceiling on emergency duration in blocks (e.g. 30 days).
        /// Clamped on declaration — cannot be exceeded even by unanimous cabinet vote.
        #[pallet::constant]
        type MaxEmergencyBlocks: Get<u32>;
        /// Blocks the legislature has to ratify after the cabinet declares an emergency.
        /// If they don't pass `ratify_emergency` in time, the emergency lapses.
        #[pallet::constant]
        type RatificationWindowBlocks: Get<u32>;
        /// Numerator of the cabinet supermajority required to declare/end an emergency (e.g. 2).
        #[pallet::constant]
        type SupermajorityNumerator: Get<u32>;
        /// Denominator of the cabinet supermajority (e.g. 3 for 2/3).
        #[pallet::constant]
        type SupermajorityDenominator: Get<u32>;
        /// Checks suspension status for the daily PM/minister vacancy sweep.
        type CitizenChecker: CitizenChecker<Self::AccountId>;
        /// Checks legislature membership for PM nomination/voting/succession.
        type LegislatureMembership: LegislatureMembership<Self::AccountId>;
        /// How many blocks a PM-investiture nomination window stays open once opened.
        #[pallet::constant]
        type PmNominationWindowBlocks: Get<u32>;
        /// How many blocks the ranked-ballot voting window runs for, starting right
        /// after the nomination window closes.
        #[pallet::constant]
        type PmVotingWindowBlocks: Get<u32>;
        /// Cap on candidates a single investiture round can hold (bounds ballot/tally size).
        #[pallet::constant]
        type MaxPmCandidates: Get<u32>;
        /// Size (in blocks) of the trailing window used to cap how much of recent
        /// history any single account may have spent as Prime Minister.
        #[pallet::constant]
        type PmOccupancyWindowBlocks: Get<u32>;
        /// Maximum total blocks any single account may have served as Prime
        /// Minister within the trailing `PmOccupancyWindowBlocks` window. Must be
        /// less than `PmOccupancyWindowBlocks` to have any effect.
        #[pallet::constant]
        type MaxPmOccupancyBlocks: Get<u32>;
        /// Bound on the ring buffer of past PM tenure records retained for
        /// computing rolling occupancy. Must be set generously relative to
        /// `PmOccupancyWindowBlocks` divided by the shortest realistic tenure
        /// length, so that a burst of rapid turnover within one window cannot
        /// evict still-in-window history before it naturally ages out (see the
        /// eviction policy on the storage item below).
        #[pallet::constant]
        type MaxPmTenureHistory: Get<u32>;
        /// Blocks between automatic PM/minister eligibility sweeps (e.g. ~1 day).
        #[pallet::constant]
        type VacancySweepIntervalBlocks: Get<u32>;
        /// Weight functions needed for this pallet's extrinsics.
        type WeightInfo: crate::weights::WeightInfo;
        /// See `BenchmarkHelper`'s doc comment.
        #[cfg(feature = "runtime-benchmarks")]
        type BenchmarkHelper: BenchmarkHelper<Self::AccountId>;
    }

    // ── Types ────────────────────────────────────────────────────────────────────

    /// A defined cabinet portfolio slot.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct Portfolio {
        /// IPFS hash of the portfolio name / terms of reference document.
        pub name_hash: [u8; 32],
    }

    /// A live emergency declaration.
    #[derive(Clone, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct EmergencyInfo<BlockNumber> {
        /// Block at which the emergency was declared.
        pub declared_at: BlockNumber,
        /// Block at which the emergency auto-expires (constitutionally hard-bounded).
        pub expires_at: BlockNumber,
        /// Block by which the legislature must ratify, or the emergency lapses.
        pub ratify_by: BlockNumber,
        /// IPFS hash of the reason document.
        pub reason_hash: [u8; 32],
        /// True once the legislature has passed a ratification motion.
        pub ratified: bool,
    }

    /// A currently-open PM investiture round.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct InvestitureRoundInfo<BlockNumber> {
        /// Nominations are accepted strictly before this block.
        pub nomination_end: BlockNumber,
        /// Ranked ballots are accepted from `nomination_end` strictly before this block.
        pub voting_end: BlockNumber,
    }

    /// A completed Prime Minister tenure, retained to compute rolling-window
    /// occupancy (see `PmTenureHistory`).
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct PmTenureRecord<AccountId, BlockNumber> {
        pub who: AccountId,
        pub start: BlockNumber,
        pub end: BlockNumber,
    }

    /// Which office the daily vacancy sweep removed someone from.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum VacatedRole {
        PrimeMinister,
        Minister { portfolio_id: u32 },
    }

    // ── Storage ──────────────────────────────────────────────────────────────────

    /// The current Prime Minister. None = no PM appointed.
    #[pallet::storage]
    pub type PrimeMinister<T: Config> = StorageValue<_, T::AccountId, OptionQuery>;

    /// Ring buffer of completed PM tenures, used to compute how many blocks
    /// within the trailing `PmOccupancyWindowBlocks` window any given account
    /// has spent as PM (see `pm_occupancy_in_window`). Eviction policy on
    /// overflow: first drop any record whose `end` already predates the
    /// current occupancy window (safe — it can no longer contribute to any
    /// future occupancy calculation), and only if none are prunable, drop the
    /// single oldest record as a last-resort safety valve. `MaxPmTenureHistory`
    /// must be sized generously enough in practice that this last resort is
    /// never hit by legitimate turnover within one window.
    #[pallet::storage]
    pub type PmTenureHistory<T: Config> = StorageValue<
        _,
        BoundedVec<PmTenureRecord<T::AccountId, BlockNumberFor<T>>, T::MaxPmTenureHistory>,
        ValueQuery,
    >;

    /// Start block of the currently sitting PM's in-progress tenure. `None`
    /// while the seat is vacant.
    #[pallet::storage]
    pub type CurrentPmTenureStart<T: Config> = StorageValue<_, BlockNumberFor<T>, OptionQuery>;

    /// portfolio_id → Portfolio definition.
    #[pallet::storage]
    pub type Portfolios<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, Portfolio>;

    /// portfolio_id → current minister AccountId.
    #[pallet::storage]
    pub type PortfolioMinister<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, T::AccountId>;

    /// minister AccountId → their portfolio_id. Enables O(1) is_active_minister checks.
    #[pallet::storage]
    pub type MinisterPortfolio<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32>;

    /// Monotonic counter for portfolio IDs.
    #[pallet::storage]
    pub type NextPortfolioId<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Active emergency, if any.
    #[pallet::storage]
    pub type ActiveEmergency<T: Config> =
        StorageValue<_, EmergencyInfo<BlockNumberFor<T>>, OptionQuery>;

    /// Cabinet members who have voted to declare the pending emergency.
    #[pallet::storage]
    pub type DeclareVotes<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, bool, ValueQuery>;

    /// Cabinet members who have voted to end the active emergency early.
    #[pallet::storage]
    pub type EndVotes<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, bool, ValueQuery>;

    /// Proposal terms locked in by the first cabinet member to vote for an emergency.
    /// Subsequent voters' `reason_hash` and `duration_blocks` args are ignored; these
    /// stored terms are used when the supermajority is reached. Cleared on activation,
    /// and on retract when no votes remain.
    #[pallet::storage]
    pub type PendingEmergencyProposal<T: Config> = StorageValue<_, ([u8; 32], u32), OptionQuery>;

    // ── Storage: minister nomination (PM nominates, legislature confirms) ──────────

    /// portfolio_id → the PM's currently staged nominee, awaiting legislature
    /// confirmation via `appoint_minister`. Cleared on confirmation, or whenever the PM
    /// office changes hands (see `clear_pending_minister_nominations`) so an outgoing
    /// PM's unconfirmed pick can never be confirmed under a successor's watch.
    #[pallet::storage]
    pub type PendingMinisterNomination<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, T::AccountId>;

    // ── Storage: PM investiture (ranked-choice among legislature members) ──────────

    /// The currently open investiture round, if any. Opens only while the PM seat is
    /// vacant — see `open_pm_investiture`.
    #[pallet::storage]
    pub type InvestitureRound<T: Config> =
        StorageValue<_, InvestitureRoundInfo<BlockNumberFor<T>>, OptionQuery>;

    /// Nominees for the current investiture round, in nomination order — that order is
    /// also the deterministic tie-break the instant-runoff tally eliminates by.
    #[pallet::storage]
    pub type PmNominees<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxPmCandidates>, ValueQuery>;

    /// Legislature member → their ranked ballot for the current investiture round,
    /// most-preferred candidate first.
    #[pallet::storage]
    pub type PmBallots<T: Config> = StorageMap<
        _, Blake2_128Concat, T::AccountId, BoundedVec<T::AccountId, T::MaxPmCandidates>,
    >;

    // ── Storage: conviction-triggered vacancy sweep ─────────────────────────────────

    /// Block the next automatic PM/minister eligibility sweep should run at.
    #[pallet::storage]
    pub type NextVacancySweepBlock<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

    // ── Hooks ────────────────────────────────────────────────────────────────────

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        fn on_initialize(now: BlockNumberFor<T>) -> Weight {
            let mut weight = T::DbWeight::get().reads(1);
            if let Some(info) = ActiveEmergency::<T>::get() {
                // Lapse: emergency declared but legislature didn't ratify in time.
                if !info.ratified && now > info.ratify_by {
                    ActiveEmergency::<T>::kill();
                    let _ = DeclareVotes::<T>::clear(u32::MAX, None);
                    let _ = EndVotes::<T>::clear(u32::MAX, None);
                    Self::deposit_event(Event::EmergencyLapsed);
                    weight = weight.saturating_add(T::DbWeight::get().writes(3));
                // Expire: emergency ran its full constitutional duration.
                } else if now >= info.expires_at {
                    ActiveEmergency::<T>::kill();
                    let _ = DeclareVotes::<T>::clear(u32::MAX, None);
                    let _ = EndVotes::<T>::clear(u32::MAX, None);
                    Self::deposit_event(Event::EmergencyExpired { at_block: now });
                    weight = weight.saturating_add(T::DbWeight::get().writes(3));
                }
            }

            // Daily (config-driven) sweep: auto-vacate a PM/minister a court has
            // suspended since they took office. See `run_vacancy_sweep`'s doc comment
            // for why this is a periodic re-check rather than a cross-pallet hook.
            if now >= NextVacancySweepBlock::<T>::get() {
                weight = weight.saturating_add(Self::run_vacancy_sweep());
                NextVacancySweepBlock::<T>::put(
                    now.saturating_add(BlockNumberFor::<T>::from(T::VacancySweepIntervalBlocks::get())),
                );
                weight = weight.saturating_add(T::DbWeight::get().reads_writes(1, 1));
            }

            weight
        }
    }

    // ── Events ───────────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// A new portfolio was defined.
        PortfolioDefined { portfolio_id: u32, name_hash: [u8; 32] },
        /// The Prime Minister was appointed.
        PrimeMinisterAppointed { who: T::AccountId },
        /// The Prime Minister was dismissed.
        PrimeMinisterDismissed { who: T::AccountId },
        /// A minister was appointed to a portfolio.
        MinisterAppointed { portfolio_id: u32, who: T::AccountId },
        /// A minister was dismissed from a portfolio.
        MinisterDismissed { portfolio_id: u32, who: T::AccountId },
        /// A minister resigned from their portfolio.
        MinisterResigned { portfolio_id: u32, who: T::AccountId },
        /// Cabinet member voted to declare an emergency.
        EmergencyVoteCast { who: T::AccountId, vote_count: u32 },
        /// Cabinet supermajority reached — emergency declared, awaiting legislature ratification.
        EmergencyDeclared { expires_at: BlockNumberFor<T>, ratify_by: BlockNumberFor<T>, reason_hash: [u8; 32] },
        /// Legislature ratified the emergency — it is now fully active.
        EmergencyRatified,
        /// Legislature didn't ratify in time — emergency lapsed without taking effect.
        EmergencyLapsed,
        /// Emergency reached its sunset block and expired.
        EmergencyExpired { at_block: BlockNumberFor<T> },
        /// Cabinet voted to end the emergency early; supermajority reached.
        EmergencyLifted,
        /// The PM staged a nominee for a portfolio, awaiting legislature confirmation.
        MinisterNominated { portfolio_id: u32, nominee: T::AccountId },
        /// The Prime Minister resigned voluntarily.
        PrimeMinisterResigned { who: T::AccountId },
        /// A constructive no-confidence motion passed: old PM removed, successor
        /// installed atomically in the same call — the office is never left vacant by
        /// this path.
        PrimeMinisterRemovedAndReplaced { old: T::AccountId, new: T::AccountId },
        /// A PM investiture round opened; nominations close at `nomination_end`, voting
        /// closes at `voting_end`.
        PmInvestitureOpened { nomination_end: BlockNumberFor<T>, voting_end: BlockNumberFor<T> },
        /// A candidate was nominated for the current PM investiture round.
        PmCandidateNominated { candidate: T::AccountId },
        /// A legislature member cast (or replaced) their ranked ballot.
        PmBallotCast { voter: T::AccountId },
        /// The investiture round closed with a winner, who is now Prime Minister.
        PmInvestitureFinalized { winner: T::AccountId },
        /// The investiture round closed with no winner (no candidates were nominated, or
        /// no ballots were cast) — the seat remains vacant; a new round must be opened.
        PmInvestitureFailedNoWinner,
        /// The daily vacancy sweep removed a PM/minister a court has suspended.
        OfficeVacatedForConviction { who: T::AccountId, role: VacatedRole },
    }

    // ── Errors ───────────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        /// Portfolio ID does not exist.
        PortfolioNotFound,
        /// Maximum number of portfolios already defined.
        PortfolioCapacityReached,
        /// No Prime Minister is currently appointed.
        NoPrimeMinister,
        /// The caller does not hold any portfolio.
        NotAMinister,
        /// The portfolio exists but currently has no minister assigned.
        PortfolioVacant,
        /// Caller is not a minister or the Prime Minister.
        NotCabinetMember,
        /// An emergency is already active — only one at a time.
        AlreadyActiveEmergency,
        /// This cabinet member has already voted to declare the current emergency.
        AlreadyVotedToDeclare,
        /// This cabinet member has already voted to end the current emergency.
        AlreadyVotedToEnd,
        /// There is no active emergency.
        NoActiveEmergency,
        /// Legislature ratification window has already closed.
        RatificationWindowClosed,
        /// Emergency has already been ratified.
        AlreadyRatified,
        /// Cabinet member has not cast an emergency declaration vote to retract.
        NotYetVoted,
        /// Cabinet has fewer than 2 members — a supermajority ratio check is meaningless
        /// (and trivially satisfiable) against a denominator of 1, e.g. a freshly-invested
        /// PM with no confirmed ministers yet. Emergency declaration/ending votes require
        /// a real cabinet, not just a single office-holder.
        CabinetTooSmallForEmergencyVote,
        /// Caller is not the current Prime Minister (`nominate_minister` is PM-only).
        NotPrimeMinister,
        /// `appoint_minister` was called for an account the PM never staged via
        /// `nominate_minister` — legislature can only confirm, not originate, a pick.
        NoNominationPending,
        /// Candidate/nominee/successor must currently hold a legislature seat — the PM
        /// is chosen by and from the legislature, not the general citizenry.
        NotLegislatureMember,
        /// The named successor is already the current Prime Minister.
        SuccessorIsCurrentPm,
        /// Installing this account would push its total time served as Prime
        /// Minister within the trailing occupancy window at or above the cap.
        PmOccupancyLimitReached,
        /// `open_pm_investiture` was called while a round is already open.
        InvestitureAlreadyOpen,
        /// `nominate_pm`/`cast_pm_ballot`/`finalize_pm_investiture` was called with no
        /// investiture round currently open.
        NoInvestitureOpen,
        /// Investiture only opens on a vacancy — a Prime Minister is already appointed.
        PmSeatNotVacant,
        /// The current investiture round's nomination window has already closed.
        NominationWindowClosed,
        /// Still inside the nomination window — voting hasn't opened yet.
        NominationWindowStillOpen,
        /// The current investiture round's voting window has already closed.
        VotingWindowClosed,
        /// Still inside the voting window — the round can't be finalized yet.
        VotingWindowStillOpen,
        /// This candidate has already been nominated for the current round.
        AlreadyNominated,
        /// The current investiture round already holds `MaxPmCandidates` nominees.
        TooManyPmCandidates,
        /// A ranked ballot named an account that isn't a nominee this round.
        NotANominee,
        /// A ranked ballot named the same candidate more than once.
        DuplicateCandidateInBallot,
    }

    // ── Calls ────────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Define a new cabinet portfolio. LegislatureOrigin only.
        /// name_hash is the IPFS CID of the portfolio's terms of reference.
        #[pallet::call_index(0)]
        #[pallet::weight(T::WeightInfo::define_portfolio())]
        pub fn define_portfolio(
            origin: OriginFor<T>,
            name_hash: [u8; 32],
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-executive::define_portfolio", name_hash),
            )?;
            let id = NextPortfolioId::<T>::get();
            ensure!(id < T::MaxPortfolios::get(), Error::<T>::PortfolioCapacityReached);
            Portfolios::<T>::insert(id, Portfolio { name_hash });
            NextPortfolioId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::PortfolioDefined { portfolio_id: id, name_hash });
            Ok(())
        }

        /// Removed: direct single-candidate PM appointment. Superseded entirely by
        /// `open_pm_investiture`/`nominate_pm`/`cast_pm_ballot`/`finalize_pm_investiture`
        /// — a ranked-choice vote among legislature members, not a single up/down motion.
        /// Keeping both would let legislature bypass the fair investiture process
        /// whenever convenient. call_index 1 is deliberately left unused rather than
        /// reassigned (see `#[pallet::call_index]`'s docs — indices need not be
        /// contiguous, and reassigning a removed call's index to new, unrelated
        /// behavior would be more confusing than leaving a gap).

        /// Constructive vote of no confidence: removes the current Prime Minister and
        /// installs `successor` atomically in the same motion. LegislatureOrigin only.
        ///
        /// Deliberately not a plain no-confidence vote (remove with no successor) —
        /// that pattern lets a majority topple a government for purely obstructive
        /// reasons with no obligation to agree on a replacement, which is what destabilized
        /// interwar parliamentary systems. Requiring the successor to be named *in the same
        /// vote* means removal is only possible when a replacement already has support.
        #[pallet::call_index(2)]
        #[pallet::weight(T::WeightInfo::remove_and_replace_prime_minister())]
        pub fn remove_and_replace_prime_minister(
            origin: OriginFor<T>,
            successor: T::AccountId,
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(
                    b"pallet-executive::remove_and_replace_prime_minister",
                    successor.clone(),
                ),
            )?;
            let old = PrimeMinister::<T>::get().ok_or(Error::<T>::NoPrimeMinister)?;
            ensure!(successor != old, Error::<T>::SuccessorIsCurrentPm);
            ensure!(
                T::LegislatureMembership::is_member(&successor),
                Error::<T>::NotLegislatureMember
            );
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(
                Self::pm_occupancy_in_window(&successor, now)
                    < BlockNumberFor::<T>::from(T::MaxPmOccupancyBlocks::get()),
                Error::<T>::PmOccupancyLimitReached
            );
            Self::deposit_event(Event::PrimeMinisterDismissed { who: old.clone() });
            Self::install_pm(successor.clone());
            Self::deposit_event(Event::PrimeMinisterRemovedAndReplaced { old, new: successor });
            Ok(())
        }

        /// Resign from being Prime Minister. Only the sitting PM may call this. Opens
        /// the seat for a fresh investiture round — see `open_pm_investiture`.
        #[pallet::call_index(10)]
        #[pallet::weight(T::WeightInfo::resign_as_pm())]
        pub fn resign_as_pm(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(PrimeMinister::<T>::get().as_ref() == Some(&who), Error::<T>::NotPrimeMinister);
            let now = frame_system::Pallet::<T>::block_number();
            PrimeMinister::<T>::kill();
            Self::close_current_pm_tenure(who.clone(), now);
            DeclareVotes::<T>::remove(&who);
            EndVotes::<T>::remove(&who);
            Self::clear_pending_minister_nominations();
            Self::deposit_event(Event::PrimeMinisterResigned { who });
            Ok(())
        }

        /// Stage a nominee for a portfolio. Prime Minister only. Legislature must then
        /// confirm via `appoint_minister` naming this exact account before it takes effect.
        #[pallet::call_index(11)]
        #[pallet::weight(T::WeightInfo::nominate_minister())]
        pub fn nominate_minister(
            origin: OriginFor<T>,
            portfolio_id: u32,
            candidate: T::AccountId,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(PrimeMinister::<T>::get().as_ref() == Some(&who), Error::<T>::NotPrimeMinister);
            ensure!(Portfolios::<T>::contains_key(portfolio_id), Error::<T>::PortfolioNotFound);
            PendingMinisterNomination::<T>::insert(portfolio_id, candidate.clone());
            Self::deposit_event(Event::MinisterNominated { portfolio_id, nominee: candidate });
            Ok(())
        }

        /// Open a PM investiture round. Anyone may call this, but only while the PM
        /// seat is actually vacant and no round is already open — starting the clock on
        /// an objective vacancy isn't a judgment call that needs a legislature vote.
        #[pallet::call_index(12)]
        #[pallet::weight(T::WeightInfo::open_pm_investiture())]
        pub fn open_pm_investiture(origin: OriginFor<T>) -> DispatchResult {
            let _ = ensure_signed(origin)?;
            ensure!(PrimeMinister::<T>::get().is_none(), Error::<T>::PmSeatNotVacant);
            ensure!(InvestitureRound::<T>::get().is_none(), Error::<T>::InvestitureAlreadyOpen);
            let now = frame_system::Pallet::<T>::block_number();
            let nomination_end =
                now.saturating_add(BlockNumberFor::<T>::from(T::PmNominationWindowBlocks::get()));
            let voting_end = nomination_end
                .saturating_add(BlockNumberFor::<T>::from(T::PmVotingWindowBlocks::get()));
            InvestitureRound::<T>::put(InvestitureRoundInfo { nomination_end, voting_end });
            Self::deposit_event(Event::PmInvestitureOpened { nomination_end, voting_end });
            Ok(())
        }

        /// Nominate a candidate for the open PM investiture round. Caller and candidate
        /// must both currently hold a legislature seat — the PM is chosen by and from
        /// the legislature.
        #[pallet::call_index(13)]
        #[pallet::weight(T::WeightInfo::nominate_pm())]
        pub fn nominate_pm(origin: OriginFor<T>, candidate: T::AccountId) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(
                T::LegislatureMembership::is_member(&who),
                Error::<T>::NotLegislatureMember
            );
            let round = InvestitureRound::<T>::get().ok_or(Error::<T>::NoInvestitureOpen)?;
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(now < round.nomination_end, Error::<T>::NominationWindowClosed);
            ensure!(
                T::LegislatureMembership::is_member(&candidate),
                Error::<T>::NotLegislatureMember
            );
            ensure!(
                Self::pm_occupancy_in_window(&candidate, now)
                    < BlockNumberFor::<T>::from(T::MaxPmOccupancyBlocks::get()),
                Error::<T>::PmOccupancyLimitReached
            );
            PmNominees::<T>::try_mutate(|nominees| {
                ensure!(!nominees.contains(&candidate), Error::<T>::AlreadyNominated);
                nominees.try_push(candidate.clone()).map_err(|_| Error::<T>::TooManyPmCandidates)?;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::PmCandidateNominated { candidate });
            Ok(())
        }

        /// Cast (or replace) a ranked ballot for the open PM investiture round. Must be
        /// a current legislature member; every ranked candidate must be a nominee this
        /// round, with no duplicates.
        #[pallet::call_index(14)]
        #[pallet::weight(T::WeightInfo::cast_pm_ballot())]
        pub fn cast_pm_ballot(
            origin: OriginFor<T>,
            ranked_candidates: BoundedVec<T::AccountId, T::MaxPmCandidates>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(
                T::LegislatureMembership::is_member(&who),
                Error::<T>::NotLegislatureMember
            );
            let round = InvestitureRound::<T>::get().ok_or(Error::<T>::NoInvestitureOpen)?;
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(now >= round.nomination_end, Error::<T>::NominationWindowStillOpen);
            ensure!(now < round.voting_end, Error::<T>::VotingWindowClosed);
            let nominees = PmNominees::<T>::get();
            let mut seen: alloc::vec::Vec<T::AccountId> = alloc::vec::Vec::new();
            for c in ranked_candidates.iter() {
                ensure!(nominees.contains(c), Error::<T>::NotANominee);
                ensure!(!seen.contains(c), Error::<T>::DuplicateCandidateInBallot);
                seen.push(c.clone());
            }
            PmBallots::<T>::insert(&who, ranked_candidates);
            Self::deposit_event(Event::PmBallotCast { voter: who });
            Ok(())
        }

        /// Close the PM investiture round once its voting window has passed, tally
        /// ranked ballots by instant-runoff, and install the winner. Anyone may call
        /// this. Emits `PmInvestitureFailedNoWinner` (seat stays vacant, a fresh round
        /// must be opened) if no candidates were nominated or no ballots were cast.
        #[pallet::call_index(15)]
        #[pallet::weight(T::WeightInfo::finalize_pm_investiture())]
        pub fn finalize_pm_investiture(origin: OriginFor<T>) -> DispatchResult {
            let _ = ensure_signed(origin)?;
            let round = InvestitureRound::<T>::get().ok_or(Error::<T>::NoInvestitureOpen)?;
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(now >= round.voting_end, Error::<T>::VotingWindowStillOpen);

            let winner = Self::run_instant_runoff();

            InvestitureRound::<T>::kill();
            let _ = PmBallots::<T>::clear(u32::MAX, None);
            PmNominees::<T>::kill();

            match winner {
                Some(w) => {
                    Self::install_pm(w.clone());
                    Self::deposit_event(Event::PmInvestitureFinalized { winner: w });
                }
                None => {
                    Self::deposit_event(Event::PmInvestitureFailedNoWinner);
                }
            }
            Ok(())
        }

        /// Appoint a minister to a portfolio. LegislatureOrigin only, and only for the
        /// account the Prime Minister has staged via `nominate_minister` for this
        /// portfolio — legislature confirms the PM's pick, it doesn't originate one.
        ///
        /// If the portfolio is already occupied, the previous holder is automatically dismissed.
        /// If the incoming account already holds a different portfolio, they are vacated first.
        #[pallet::call_index(3)]
        #[pallet::weight(T::WeightInfo::appoint_minister())]
        pub fn appoint_minister(
            origin: OriginFor<T>,
            portfolio_id: u32,
            who: T::AccountId,
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(
                    b"pallet-executive::appoint_minister",
                    (portfolio_id, who.clone()),
                ),
            )?;
            ensure!(Portfolios::<T>::contains_key(portfolio_id), Error::<T>::PortfolioNotFound);
            let nominee = PendingMinisterNomination::<T>::get(portfolio_id)
                .ok_or(Error::<T>::NoNominationPending)?;
            ensure!(nominee == who, Error::<T>::NoNominationPending);
            PendingMinisterNomination::<T>::remove(portfolio_id);

            // Vacate whoever currently holds this portfolio.
            if let Some(old) = PortfolioMinister::<T>::get(portfolio_id) {
                MinisterPortfolio::<T>::remove(&old);
                // Invalidate the outgoing minister's pending emergency declaration/end votes.
                DeclareVotes::<T>::remove(&old);
                EndVotes::<T>::remove(&old);
                Self::deposit_event(Event::MinisterDismissed { portfolio_id, who: old });
            }

            // Vacate any other portfolio the incoming account currently holds.
            if let Some(old_pid) = MinisterPortfolio::<T>::get(&who) {
                PortfolioMinister::<T>::remove(old_pid);
                Self::deposit_event(Event::MinisterDismissed { portfolio_id: old_pid, who: who.clone() });
            }

            // Clear any stale emergency vote the incoming account may carry from a prior tenure.
            DeclareVotes::<T>::remove(&who);
            PortfolioMinister::<T>::insert(portfolio_id, who.clone());
            MinisterPortfolio::<T>::insert(who.clone(), portfolio_id);
            Self::deposit_event(Event::MinisterAppointed { portfolio_id, who });
            Ok(())
        }

        /// Dismiss the minister currently holding a portfolio. LegislatureOrigin only.
        #[pallet::call_index(4)]
        #[pallet::weight(T::WeightInfo::dismiss_minister())]
        pub fn dismiss_minister(origin: OriginFor<T>, portfolio_id: u32) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-executive::dismiss_minister", portfolio_id),
            )?;
            ensure!(Portfolios::<T>::contains_key(portfolio_id), Error::<T>::PortfolioNotFound);
            let who = PortfolioMinister::<T>::take(portfolio_id)
                .ok_or(Error::<T>::PortfolioVacant)?;
            MinisterPortfolio::<T>::remove(&who);
            // Invalidate the dismissed minister's pending emergency declaration/end votes.
            DeclareVotes::<T>::remove(&who);
            EndVotes::<T>::remove(&who);
            Self::deposit_event(Event::MinisterDismissed { portfolio_id, who });
            Ok(())
        }

        /// Resign from one's own portfolio. Any active minister may call this.
        #[pallet::call_index(5)]
        #[pallet::weight(T::WeightInfo::resign())]
        pub fn resign(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let portfolio_id =
                MinisterPortfolio::<T>::take(&who).ok_or(Error::<T>::NotAMinister)?;
            PortfolioMinister::<T>::remove(portfolio_id);
            // Invalidate the resigned minister's pending emergency declaration/end votes.
            DeclareVotes::<T>::remove(&who);
            EndVotes::<T>::remove(&who);
            Self::deposit_event(Event::MinisterResigned { portfolio_id, who });
            Ok(())
        }

        /// Vote to declare a state of emergency. Any minister or PM may call.
        ///
        /// `duration_blocks` is clamped to `MaxEmergencyBlocks`. When a 2/3 supermajority
        /// of the cabinet has voted, the emergency activates and the legislature has
        /// `RatificationWindowBlocks` to ratify it or it lapses automatically.
        #[pallet::call_index(6)]
        #[pallet::weight(T::WeightInfo::vote_declare_emergency())]
        pub fn vote_declare_emergency(
            origin: OriginFor<T>,
            reason_hash: [u8; 32],
            duration_blocks: u32,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_cabinet_member(&who), Error::<T>::NotCabinetMember);
            ensure!(Self::cabinet_size() >= 2, Error::<T>::CabinetTooSmallForEmergencyVote);
            ensure!(ActiveEmergency::<T>::get().is_none(), Error::<T>::AlreadyActiveEmergency);
            ensure!(!DeclareVotes::<T>::get(&who), Error::<T>::AlreadyVotedToDeclare);

            // Lock in the proposal terms from the first vote. Subsequent voters' args are
            // ignored so a decisive late voter cannot override the agreed-upon reason or duration.
            if PendingEmergencyProposal::<T>::get().is_none() {
                PendingEmergencyProposal::<T>::put((reason_hash, duration_blocks));
            }
            let (agreed_reason, agreed_duration) =
                PendingEmergencyProposal::<T>::get().unwrap_or((reason_hash, duration_blocks));
            let clamped = agreed_duration.min(T::MaxEmergencyBlocks::get());

            DeclareVotes::<T>::insert(&who, true);

            let cabinet_size = Self::cabinet_size();
            let vote_count = Self::count_declare_votes();

            Self::deposit_event(Event::EmergencyVoteCast { who, vote_count });

            if Self::supermajority_reached(vote_count, cabinet_size) {
                let now = frame_system::Pallet::<T>::block_number();
                let expires_at = now.saturating_add(BlockNumberFor::<T>::from(clamped));
                let ratify_by = now.saturating_add(
                    BlockNumberFor::<T>::from(T::RatificationWindowBlocks::get())
                );

                ActiveEmergency::<T>::put(EmergencyInfo {
                    declared_at: now,
                    expires_at,
                    ratify_by,
                    reason_hash: agreed_reason,
                    ratified: false,
                });

                // Proposal has been consumed; clear it.
                // DeclareVotes are intentionally kept so the same member can't re-vote
                // if the emergency lapses and a new one is started immediately.
                // They are cleared on lapse/expire/lift.
                PendingEmergencyProposal::<T>::kill();
                Self::deposit_event(Event::EmergencyDeclared { expires_at, ratify_by, reason_hash: agreed_reason });
            }

            Ok(())
        }

        /// Ratify the active emergency. LegislatureOrigin only.
        ///
        /// Must be called within `RatificationWindowBlocks` of the emergency being declared.
        /// Once ratified, the emergency remains active until `expires_at` or early termination.
        ///
        /// The required `legislature_call_hash` is bound to the specific active emergency's
        /// `(reason_hash, declared_at)`, matching the pattern every other `LegislatureOrigin`
        /// call in this pallet uses to bind its hash to its own parameters (e.g.
        /// `remove_and_replace_prime_minister` binds `successor`). Without this, a single
        /// constant hash would let a legislature motion approved to ratify one emergency be
        /// replayed later to ratify a completely different, unrelated one the legislature
        /// never actually voted on — `PendingLegislatureApproval` only tracks one outstanding
        /// token system-wide and doesn't invalidate it when the emergency it was meant for
        /// lapses.
        #[pallet::call_index(7)]
        #[pallet::weight(T::WeightInfo::ratify_emergency())]
        pub fn ratify_emergency(origin: OriginFor<T>) -> DispatchResult {
            let active = ActiveEmergency::<T>::get().ok_or(Error::<T>::NoActiveEmergency)?;
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(
                    b"pallet-executive::ratify_emergency",
                    (active.reason_hash, active.declared_at),
                ),
            )?;
            ActiveEmergency::<T>::try_mutate(|maybe| {
                let info = maybe.as_mut().ok_or(Error::<T>::NoActiveEmergency)?;
                ensure!(!info.ratified, Error::<T>::AlreadyRatified);
                let now = frame_system::Pallet::<T>::block_number();
                ensure!(now <= info.ratify_by, Error::<T>::RatificationWindowClosed);
                info.ratified = true;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::EmergencyRatified);
            Ok(())
        }

        /// Vote to end the active emergency early. Any minister or PM may call.
        ///
        /// When a cabinet supermajority votes to end, the emergency is cleared immediately.
        #[pallet::call_index(8)]
        #[pallet::weight(T::WeightInfo::vote_end_emergency())]
        pub fn vote_end_emergency(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_cabinet_member(&who), Error::<T>::NotCabinetMember);
            ensure!(Self::cabinet_size() >= 2, Error::<T>::CabinetTooSmallForEmergencyVote);
            ensure!(ActiveEmergency::<T>::get().is_some(), Error::<T>::NoActiveEmergency);
            ensure!(!EndVotes::<T>::get(&who), Error::<T>::AlreadyVotedToEnd);

            EndVotes::<T>::insert(&who, true);

            let cabinet_size = Self::cabinet_size();
            let vote_count = EndVotes::<T>::iter().filter(|(_, v)| *v).count() as u32;

            if Self::supermajority_reached(vote_count, cabinet_size) {
                ActiveEmergency::<T>::kill();
                let _ = DeclareVotes::<T>::clear(u32::MAX, None);
                let _ = EndVotes::<T>::clear(u32::MAX, None);
                PendingEmergencyProposal::<T>::kill();
                Self::deposit_event(Event::EmergencyLifted);
            }

            Ok(())
        }

        /// Retract a previously cast emergency declaration vote.
        ///
        /// Allows a cabinet member to withdraw their vote before the emergency activates.
        /// Once `ActiveEmergency` is set this is no longer possible.
        #[pallet::call_index(9)]
        #[pallet::weight(T::WeightInfo::retract_emergency_vote())]
        pub fn retract_emergency_vote(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_cabinet_member(&who), Error::<T>::NotCabinetMember);
            ensure!(ActiveEmergency::<T>::get().is_none(), Error::<T>::AlreadyActiveEmergency);
            ensure!(DeclareVotes::<T>::get(&who), Error::<T>::NotYetVoted);
            DeclareVotes::<T>::remove(&who);
            // If no cabinet votes remain, reset the proposal terms so the next first voter
            // can establish fresh ones rather than inheriting the retracted voter's params.
            if DeclareVotes::<T>::iter().filter(|(_, v)| *v).count() == 0 {
                PendingEmergencyProposal::<T>::kill();
            }
            Ok(())
        }
    }

    // ── Private helpers ───────────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
        fn is_cabinet_member(who: &T::AccountId) -> bool {
            MinisterPortfolio::<T>::contains_key(who)
                || PrimeMinister::<T>::get().as_ref() == Some(who)
        }

        /// Total cabinet size = number of portfolio holders + 1 if a PM is appointed.
        fn cabinet_size() -> u32 {
            let ministers = PortfolioMinister::<T>::iter().count() as u32;
            let pm_bonus = if PrimeMinister::<T>::get().is_some() { 1 } else { 0 };
            ministers.saturating_add(pm_bonus)
        }

        fn count_declare_votes() -> u32 {
            DeclareVotes::<T>::iter().filter(|(_, v)| *v).count() as u32
        }

        /// `votes * denominator >= cabinet_size * numerator`
        /// e.g. 2/3 supermajority: votes * 3 >= size * 2
        fn supermajority_reached(votes: u32, cabinet_size: u32) -> bool {
            if cabinet_size == 0 {
                return false;
            }
            votes.saturating_mul(T::SupermajorityDenominator::get())
                >= cabinet_size.saturating_mul(T::SupermajorityNumerator::get())
        }

        /// Installs `new_pm` as Prime Minister, handling all of the bookkeeping any path
        /// that changes who holds the office needs: clears the outgoing PM's stale
        /// emergency vote and any pending minister nomination they staged (so it can
        /// never be confirmed under a successor's watch — see
        /// `clear_pending_minister_nominations`), and starts `new_pm`'s in-progress
        /// tenure clock.
        ///
        /// Rolling-window occupancy semantics: this function no longer tracks anything
        /// per-account itself. If a live sitting PM is being directly displaced (no
        /// intervening vacancy), their in-progress tenure is closed out into
        /// `PmTenureHistory` via `close_current_pm_tenure` before the new tenure starts.
        /// `CurrentPmTenureStart` is then set to `now` for `new_pm`. How much of the
        /// trailing `PmOccupancyWindowBlocks` window any account has actually occupied is
        /// derived purely from `PmTenureHistory` plus this in-progress tenure — see
        /// `pm_occupancy_in_window` — and is what `remove_and_replace_prime_minister` and
        /// `nominate_pm` check against `MaxPmOccupancyBlocks` before allowing (re)install.
        fn install_pm(new_pm: T::AccountId) {
            let now = frame_system::Pallet::<T>::block_number();
            let live_previous = PrimeMinister::<T>::get();
            if let Some(prev) = &live_previous {
                DeclareVotes::<T>::remove(prev);
                EndVotes::<T>::remove(prev);
                if prev != &new_pm {
                    Self::close_current_pm_tenure(prev.clone(), now);
                }
            }
            Self::clear_pending_minister_nominations();

            CurrentPmTenureStart::<T>::put(now);
            DeclareVotes::<T>::remove(&new_pm);
            PrimeMinister::<T>::put(new_pm.clone());
            Self::deposit_event(Event::PrimeMinisterAppointed { who: new_pm });
        }

        /// Sum, over `PmTenureHistory` plus the in-progress tenure if `who` is the
        /// sitting PM, of each tenure's overlap with the trailing
        /// `[now - PmOccupancyWindowBlocks, now]` window.
        ///
        /// `pub(crate)` (rather than fully private, like most other helpers here) so
        /// `tests.rs` can assert directly on rolling occupancy rather than only on
        /// externally-observable call outcomes.
        pub(crate) fn pm_occupancy_in_window(who: &T::AccountId, now: BlockNumberFor<T>) -> BlockNumberFor<T> {
            let window_start =
                now.saturating_sub(BlockNumberFor::<T>::from(T::PmOccupancyWindowBlocks::get()));
            let mut total = BlockNumberFor::<T>::zero();
            for record in PmTenureHistory::<T>::get().iter() {
                if &record.who == who {
                    let overlap_start = record.start.max(window_start);
                    let overlap_end = record.end.min(now);
                    if overlap_end > overlap_start {
                        total = total.saturating_add(overlap_end - overlap_start);
                    }
                }
            }
            if PrimeMinister::<T>::get().as_ref() == Some(who) {
                if let Some(start) = CurrentPmTenureStart::<T>::get() {
                    let overlap_start = start.max(window_start);
                    if now > overlap_start {
                        total = total.saturating_add(now - overlap_start);
                    }
                }
            }
            total
        }

        /// Closes out the in-progress PM tenure (if any) into `PmTenureHistory`
        /// and clears `CurrentPmTenureStart`. Call this on every path that ends a
        /// PM's time in office: `resign_as_pm`, `run_vacancy_sweep`'s forced
        /// removal, and `install_pm` when displacing a sitting PM directly (no
        /// intervening vacancy).
        fn close_current_pm_tenure(who: T::AccountId, now: BlockNumberFor<T>) {
            let start = CurrentPmTenureStart::<T>::take().unwrap_or(now);
            let record = PmTenureRecord { who, start, end: now };
            PmTenureHistory::<T>::mutate(|history| {
                let window_start = now
                    .saturating_sub(BlockNumberFor::<T>::from(T::PmOccupancyWindowBlocks::get()));
                if history.is_full() {
                    if let Some(pos) = history.iter().position(|r| r.end <= window_start) {
                        history.remove(pos);
                    } else {
                        history.remove(0);
                    }
                }
                let _ = history.try_push(record);
            });
        }

        /// Clears every staged-but-unconfirmed minister nomination. Called whenever the
        /// PM office changes hands, so an outgoing PM's pick can never be confirmed by
        /// legislature after they've left — confirmation should always reflect the
        /// *current* PM's judgment.
        fn clear_pending_minister_nominations() {
            let _ = PendingMinisterNomination::<T>::clear(u32::MAX, None);
        }

        /// Re-checks the sitting PM and every sitting minister's citizen-suspension
        /// status and auto-vacates anyone suspended by a *jury-reviewed* conviction since
        /// they took office. Run periodically from `on_initialize` (see
        /// `VacancySweepIntervalBlocks`) rather than wired as a cross-pallet hook off
        /// pallet-courts/pallet-identity: the set being checked is always tiny (one PM, a
        /// handful of ministers), so polling costs nothing meaningful, and it keeps this
        /// pallet self-contained instead of threading a hook through two other pallets.
        ///
        /// Deliberately checks `is_suspended_by_jury_reviewed_conviction`, not just
        /// `is_active_citizen`: a bare, unappealed AI ruling is *not* enough on its own to
        /// remove a sitting PM/minister — only a suspension a jury actually reviewed is.
        /// This is the same reasoning as requiring a named successor for a no-confidence
        /// vote (`remove_and_replace_prime_minister`): the consequence is large, so the
        /// bar for triggering it automatically is the highest one this system has,
        /// rather than the lowest. A suspended-but-not-jury-reviewed office holder isn't
        /// exempt from consequences — legislature can still remove them via a
        /// no-confidence vote at any time, and they're simply blocked from being
        /// nominated again (see `nominate_pm`) if their suspension outlives their term.
        fn run_vacancy_sweep() -> Weight {
            let mut weight = Weight::zero();
            let now = frame_system::Pallet::<T>::block_number();
            if let Some(pm) = PrimeMinister::<T>::get() {
                weight = weight.saturating_add(T::DbWeight::get().reads(2));
                if T::CitizenChecker::is_suspended_by_jury_reviewed_conviction(&pm) {
                    PrimeMinister::<T>::kill();
                    Self::close_current_pm_tenure(pm.clone(), now);
                    DeclareVotes::<T>::remove(&pm);
                    EndVotes::<T>::remove(&pm);
                    Self::clear_pending_minister_nominations();
                    Self::deposit_event(Event::OfficeVacatedForConviction {
                        who: pm,
                        role: VacatedRole::PrimeMinister,
                    });
                    weight = weight.saturating_add(T::DbWeight::get().writes(5));
                }
            }
            let ministers: alloc::vec::Vec<(u32, T::AccountId)> =
                PortfolioMinister::<T>::iter().collect();
            for (portfolio_id, minister) in ministers {
                weight = weight.saturating_add(T::DbWeight::get().reads(2));
                if T::CitizenChecker::is_suspended_by_jury_reviewed_conviction(&minister) {
                    PortfolioMinister::<T>::remove(portfolio_id);
                    MinisterPortfolio::<T>::remove(&minister);
                    DeclareVotes::<T>::remove(&minister);
                    EndVotes::<T>::remove(&minister);
                    Self::deposit_event(Event::OfficeVacatedForConviction {
                        who: minister,
                        role: VacatedRole::Minister { portfolio_id },
                    });
                    weight = weight.saturating_add(T::DbWeight::get().writes(4));
                }
            }
            weight
        }

        /// Instant-runoff tally over the current investiture round's nominees/ballots.
        /// Returns `None` if there are no nominees or no ballots were cast at all
        /// (rather than declaring an arbitrary winner by tie-break alone).
        ///
        /// Each round: tally every ballot's most-preferred still-active candidate: if
        /// one has a strict majority of ballots still counting toward *some* active
        /// candidate, they win. Otherwise eliminate whoever has the fewest such votes
        /// and repeat. Ties for fewest are broken toward eliminating whoever was
        /// nominated later — deterministic, no on-chain randomness needed for a
        /// governance tally. `active` strictly shrinks by exactly one every iteration,
        /// so this always terminates within `nominees.len()` rounds.
        fn run_instant_runoff() -> Option<T::AccountId> {
            let mut active: alloc::vec::Vec<T::AccountId> = PmNominees::<T>::get().into_inner();
            if active.is_empty() {
                return None;
            }
            let ballots: alloc::vec::Vec<alloc::vec::Vec<T::AccountId>> =
                PmBallots::<T>::iter().map(|(_, b)| b.into_inner()).collect();
            if ballots.is_empty() {
                return None;
            }

            while active.len() > 1 {
                let mut tally: alloc::vec::Vec<(T::AccountId, u32)> =
                    active.iter().cloned().map(|c| (c, 0u32)).collect();
                for ballot in ballots.iter() {
                    if let Some(pref) = ballot.iter().find(|c| active.contains(c)) {
                        if let Some(entry) = tally.iter_mut().find(|(c, _)| c == pref) {
                            entry.1 = entry.1.saturating_add(1);
                        }
                    }
                }
                let total_counted: u32 = tally.iter().map(|(_, c)| *c).sum();

                if let Some((winner, count)) = tally.iter().max_by_key(|(_, c)| *c) {
                    if total_counted > 0 && (*count as u64).saturating_mul(2) > total_counted as u64 {
                        return Some(winner.clone());
                    }
                }

                let min_count = tally.iter().map(|(_, c)| *c).min().unwrap_or(0);
                let elimination = active
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| {
                        tally.iter().find(|(cand, _)| cand == *c).map(|(_, ct)| *ct)
                            == Some(min_count)
                    })
                    .map(|(i, _)| i)
                    .last();
                match elimination {
                    Some(pos) => {
                        active.remove(pos);
                    }
                    None => return None, // unreachable: tally is built from active itself.
                }
            }

            active.into_iter().next()
        }
    }
}

/// Implement MinisterChecker so pallet-legislature can enforce the incompatibility rule.
impl<T: pallet::Config> pallet_legislature::pallet::MinisterChecker<T::AccountId>
    for pallet::Pallet<T>
{
    fn is_active_minister(who: &T::AccountId) -> bool {
        pallet::MinisterPortfolio::<T>::contains_key(who)
            || pallet::PrimeMinister::<T>::get().as_ref() == Some(who)
    }
}
