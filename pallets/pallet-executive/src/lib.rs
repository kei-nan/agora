//! # Executive Pallet
//!
//! Parliamentary executive for Agora. The legislature appoints ministers to named portfolios
//! via a passed motion. A Prime Minister coordinates the cabinet.
//!
//! ## Separation-of-powers enforcement
//!
//! Active ministers (portfolio holders and the Prime Minister) are blocked from voting on
//! legislature motions. This is enforced by pallet-legislature calling
//! `MinisterChecker::is_active_minister` before recording a vote. Ministers may still
//! propose motions and observe debates, but hold no legislative voting power while in office.
//!
//! One account holds at most one portfolio at a time. Appointing an existing minister to a
//! new portfolio automatically vacates their old one.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
    use codec::{Decode, DecodeWithMemTracking, Encode};
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;

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
        type LegislatureOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;
        /// Maximum number of portfolios (cabinet size ceiling).
        #[pallet::constant]
        type MaxPortfolios: Get<u32>;
    }

    // ── Types ────────────────────────────────────────────────────────────────────

    /// A defined cabinet portfolio slot.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct Portfolio {
        /// IPFS hash of the portfolio name / terms of reference document.
        pub name_hash: [u8; 32],
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
    }

    // ── Errors ───────────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        /// Portfolio ID does not exist.
        PortfolioNotFound,
        /// Portfolio already has a minister — dismiss them first, or use appoint_minister
        /// which auto-vacates the old holder.
        PortfolioOccupied,
        /// Maximum number of portfolios already defined.
        PortfolioCapacityReached,
        /// No Prime Minister is currently appointed.
        NoPrimeMinister,
        /// The caller does not hold any portfolio.
        NotAMinister,
    }

    // ── Calls ────────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Define a new cabinet portfolio. LegislatureOrigin only.
        /// name_hash is the IPFS CID of the portfolio's terms of reference.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn define_portfolio(
            origin: OriginFor<T>,
            name_hash: [u8; 32],
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(origin)?;
            let id = NextPortfolioId::<T>::get();
            ensure!(id < T::MaxPortfolios::get(), Error::<T>::PortfolioCapacityReached);
            Portfolios::<T>::insert(id, Portfolio { name_hash });
            NextPortfolioId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::PortfolioDefined { portfolio_id: id, name_hash });
            Ok(())
        }

        /// Appoint a Prime Minister. LegislatureOrigin only.
        /// The PM is blocked from legislature votes while in office.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn appoint_prime_minister(
            origin: OriginFor<T>,
            who: T::AccountId,
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(origin)?;
            if let Some(old) = PrimeMinister::<T>::get() {
                Self::deposit_event(Event::PrimeMinisterDismissed { who: old });
            }
            PrimeMinister::<T>::put(who.clone());
            Self::deposit_event(Event::PrimeMinisterAppointed { who });
            Ok(())
        }

        /// Dismiss the Prime Minister. LegislatureOrigin only.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(6_000, 0))]
        pub fn dismiss_prime_minister(origin: OriginFor<T>) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(origin)?;
            let who = PrimeMinister::<T>::take().ok_or(Error::<T>::NoPrimeMinister)?;
            Self::deposit_event(Event::PrimeMinisterDismissed { who });
            Ok(())
        }

        /// Appoint a minister to a portfolio. LegislatureOrigin only.
        ///
        /// If the portfolio is already occupied, the previous holder is automatically
        /// dismissed. If the incoming account already holds a different portfolio,
        /// they are vacated from it first (one portfolio per person).
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn appoint_minister(
            origin: OriginFor<T>,
            portfolio_id: u32,
            who: T::AccountId,
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(origin)?;
            ensure!(Portfolios::<T>::contains_key(portfolio_id), Error::<T>::PortfolioNotFound);

            // Vacate whoever currently holds this portfolio.
            if let Some(old) = PortfolioMinister::<T>::get(portfolio_id) {
                MinisterPortfolio::<T>::remove(&old);
                Self::deposit_event(Event::MinisterDismissed { portfolio_id, who: old });
            }

            // Vacate any other portfolio the incoming account currently holds.
            if let Some(old_pid) = MinisterPortfolio::<T>::get(&who) {
                PortfolioMinister::<T>::remove(old_pid);
                Self::deposit_event(Event::MinisterDismissed { portfolio_id: old_pid, who: who.clone() });
            }

            PortfolioMinister::<T>::insert(portfolio_id, who.clone());
            MinisterPortfolio::<T>::insert(who.clone(), portfolio_id);
            Self::deposit_event(Event::MinisterAppointed { portfolio_id, who });
            Ok(())
        }

        /// Dismiss the minister currently holding a portfolio. LegislatureOrigin only.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn dismiss_minister(origin: OriginFor<T>, portfolio_id: u32) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(origin)?;
            let who = PortfolioMinister::<T>::take(portfolio_id)
                .ok_or(Error::<T>::PortfolioNotFound)?;
            MinisterPortfolio::<T>::remove(&who);
            Self::deposit_event(Event::MinisterDismissed { portfolio_id, who });
            Ok(())
        }

        /// Resign from one's own portfolio. Any active minister may call this.
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(6_000, 0))]
        pub fn resign(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let portfolio_id =
                MinisterPortfolio::<T>::take(&who).ok_or(Error::<T>::NotAMinister)?;
            PortfolioMinister::<T>::remove(portfolio_id);
            Self::deposit_event(Event::MinisterResigned { portfolio_id, who });
            Ok(())
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
