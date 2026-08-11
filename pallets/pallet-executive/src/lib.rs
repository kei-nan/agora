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
    use sp_runtime::traits::Saturating;
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
        /// Weight functions needed for this pallet's extrinsics.
        type WeightInfo: crate::weights::WeightInfo;
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

    // ── Storage ──────────────────────────────────────────────────────────────────

    /// The current Prime Minister. None = no PM appointed.
    #[pallet::storage]
    pub type PrimeMinister<T: Config> = StorageValue<_, T::AccountId, OptionQuery>;

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

        /// Appoint a Prime Minister. LegislatureOrigin only.
        #[pallet::call_index(1)]
        #[pallet::weight(T::WeightInfo::appoint_prime_minister())]
        pub fn appoint_prime_minister(
            origin: OriginFor<T>,
            who: T::AccountId,
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-executive::appoint_prime_minister", who.clone()),
            )?;
            if let Some(old) = PrimeMinister::<T>::get() {
                // Invalidate the outgoing PM's pending emergency declaration vote.
                DeclareVotes::<T>::remove(&old);
                Self::deposit_event(Event::PrimeMinisterDismissed { who: old });
            }
            // Clear any stale vote the incoming account may carry from a prior cabinet tenure.
            DeclareVotes::<T>::remove(&who);
            PrimeMinister::<T>::put(who.clone());
            Self::deposit_event(Event::PrimeMinisterAppointed { who });
            Ok(())
        }

        /// Dismiss the Prime Minister. LegislatureOrigin only.
        #[pallet::call_index(2)]
        #[pallet::weight(T::WeightInfo::dismiss_prime_minister())]
        pub fn dismiss_prime_minister(origin: OriginFor<T>) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-executive::dismiss_prime_minister", ()),
            )?;
            let who = PrimeMinister::<T>::take().ok_or(Error::<T>::NoPrimeMinister)?;
            // Invalidate the dismissed PM's pending emergency declaration vote.
            DeclareVotes::<T>::remove(&who);
            Self::deposit_event(Event::PrimeMinisterDismissed { who });
            Ok(())
        }

        /// Appoint a minister to a portfolio. LegislatureOrigin only.
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

            // Vacate whoever currently holds this portfolio.
            if let Some(old) = PortfolioMinister::<T>::get(portfolio_id) {
                MinisterPortfolio::<T>::remove(&old);
                // Invalidate the outgoing minister's pending emergency declaration vote.
                DeclareVotes::<T>::remove(&old);
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
            // Invalidate the dismissed minister's pending emergency declaration vote.
            DeclareVotes::<T>::remove(&who);
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
            // Invalidate the resigned minister's pending emergency declaration vote.
            DeclareVotes::<T>::remove(&who);
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
        #[pallet::call_index(7)]
        #[pallet::weight(T::WeightInfo::ratify_emergency())]
        pub fn ratify_emergency(origin: OriginFor<T>) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-executive::ratify_emergency", ()),
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
