//! # Constitution Pallet
//!
//! Versioned on-chain law ledger. Every law has an IPFS content hash + on-chain record.
//! Two tiers: Constitutional (supermajority + deliberation period) and Ordinary (simple majority).
//! Laws can be invalidated by pallet-courts rulings (auto-enforcement).
#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {

    use codec::DecodeWithMemTracking;
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum LawTier {
        Ordinary,
        Constitutional,
    }

    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum LawStatus {
        Active,
        Paused,    // court-invalidated pending review
        Repealed,
    }

    /// Called by pallet-constitution when a petition crosses PetitionThreshold.
    /// Implemented by the runtime — delegates to pallet-voting's referendum storage.
    pub trait PetitionApprover {
        fn create_referendum(petition_id: u32, topic_hash: [u8; 32]) -> DispatchResult;
    }

    /// Gate: returns false if the account is not a registered active citizen.
    pub trait CitizenChecker<AccountId> {
        fn is_active_citizen(who: &AccountId) -> bool;
    }

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Minimum blocks of deliberation before a constitutional amendment can be ratified.
        #[pallet::constant]
        type ConstitutionalDeliberationBlocks: Get<u32>;
        /// The origin that represents the legislature (e.g. a referendum or collective).
        type LegislatureOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;
        /// Minimum number of citizen signatures required for a petition to trigger a referendum.
        #[pallet::constant]
        type PetitionThreshold: Get<u32>;
        /// Hook called when a petition hits PetitionThreshold — schedules a referendum.
        type PetitionApprover: PetitionApprover;
        /// Gate that checks whether an account is a registered active citizen.
        /// Prevents Sybil attacks on petitions.
        type CitizenChecker: CitizenChecker<Self::AccountId>;
        /// Minimum blocks of deliberation before an Ordinary law amendment can be ratified.
        /// Typically 0 (no wait) for ordinary laws.
        #[pallet::constant]
        type OrdinaryAmendmentDeliberationBlocks: Get<u32>;
        /// Origin representing the Human Rights Commission. Can veto a newly enacted law within HRCVetoWindowBlocks.
        type HumanRightsOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;
        /// Blocks after law enactment during which the HRC may veto. After this window only courts can invalidate.
        #[pallet::constant]
        type HRCVetoWindowBlocks: Get<u32>;
        /// The origin permitted to call `invalidate_law` (pause a law via courts ruling).
        /// Wire to a dedicated courts origin type once pallet-courts exposes one.
        /// Use EnsureRoot for now to preserve current behaviour while making it swappable.
        type CourtOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;
    }

    /// law_id -> (tier, status, version, ipfs_content_hash).
    #[pallet::storage]
    pub type Laws<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, (LawTier, LawStatus, u32, [u8; 32])>;

    /// Pending amendment: law_id -> (proposed_hash, proposed_at_block).
    #[pallet::storage]
    pub type PendingAmendments<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, ([u8; 32], BlockNumberFor<T>)>;

    /// Block at which each law was enacted. Used to enforce the HRC veto window.
    #[pallet::storage]
    pub type LawEnactedAt<T: Config> = StorageMap<_, Blake2_128Concat, u32, BlockNumberFor<T>>;

    #[pallet::storage]
    pub type NextLawId<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// petition_id -> (proposer, topic_hash [u8;32], signature_count, submitted_at_block)
    #[pallet::storage]
    pub type Petitions<T: Config> =
        StorageMap<_, Blake2_128Concat, u32,
            (T::AccountId, [u8; 32], u32, BlockNumberFor<T>)>;

    /// Tracks which accounts have signed which petition. Prevents double-signing.
    #[pallet::storage]
    pub type PetitionSignatures<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, T::AccountId), bool, ValueQuery>;

    #[pallet::storage]
    pub type NextPetitionId<T: Config> = StorageValue<_, u32, ValueQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        LawEnacted { law_id: u32, tier: LawTier, content_hash: [u8; 32] },
        LawInvalidated { law_id: u32 },
        LawRepealed { law_id: u32 },
        AmendmentProposed { law_id: u32, proposed_hash: [u8; 32] },
        AmendmentRatified { law_id: u32, new_hash: [u8; 32] },
        PetitionSubmitted { petition_id: u32, proposer: T::AccountId, topic_hash: [u8; 32] },
        PetitionSigned { petition_id: u32, signer: T::AccountId, signature_count: u32 },
        /// Emitted when a petition crosses PetitionThreshold — signals the legislature
        /// to schedule a referendum. Off-chain indexers and on-chain governance act on this.
        PetitionThresholdReached { petition_id: u32, topic_hash: [u8; 32] },
        LawVetoed { law_id: u32 },
    }

    #[pallet::error]
    pub enum Error<T> {
        LawNotFound,
        LawNotActive,
        AmendmentNotFound,
        AmendmentAlreadyPending,
        DeliberationPeriodActive,
        UnauthorizedAmendment,
        PetitionNotFound,
        AlreadySigned,
        PetitionAlreadyPassed,
        VetoWindowExpired,
        CitizenNotActive,
        LawAlreadyRepealed,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Enact a new law. Root origin = legislature approval.
        /// content_hash is Poseidon2 or SHA-256 of the IPFS document CID.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn enact_law(
            origin: OriginFor<T>,
            tier: LawTier,
            content_hash: [u8; 32],
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(origin)?;
            let id = NextLawId::<T>::get();
            Laws::<T>::insert(id, (tier.clone(), LawStatus::Active, 1u32, content_hash));
            LawEnactedAt::<T>::insert(id, frame_system::Pallet::<T>::block_number());
            NextLawId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::LawEnacted { law_id: id, tier, content_hash });
            Ok(())
        }

        /// Pause a law (called by courts pallet on invalidation ruling).
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(6_000, 0))]
        pub fn invalidate_law(origin: OriginFor<T>, law_id: u32) -> DispatchResult {
            T::CourtOrigin::ensure_origin(origin)?;
            Laws::<T>::try_mutate(law_id, |maybe_law| {
                let law = maybe_law.as_mut().ok_or(Error::<T>::LawNotFound)?;
                ensure!(law.1 == LawStatus::Active, Error::<T>::LawNotActive);
                law.1 = LawStatus::Paused;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::LawInvalidated { law_id });
            Ok(())
        }

        /// Propose an amendment to an existing law. Starts the deliberation clock.
        /// Only active laws can receive amendments. A pending amendment must be withdrawn
        /// (by ratifying or abandoning) before a new one can be submitted for the same law.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn propose_amendment(
            origin: OriginFor<T>,
            law_id: u32,
            proposed_hash: [u8; 32],
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(origin)?;
            let law = Laws::<T>::get(law_id).ok_or(Error::<T>::LawNotFound)?;
            ensure!(law.1 == LawStatus::Active, Error::<T>::LawNotActive);
            ensure!(
                !PendingAmendments::<T>::contains_key(law_id),
                Error::<T>::AmendmentAlreadyPending
            );
            let proposed_at = frame_system::Pallet::<T>::block_number();
            PendingAmendments::<T>::insert(law_id, (proposed_hash, proposed_at));
            Self::deposit_event(Event::AmendmentProposed { law_id, proposed_hash });
            Ok(())
        }

        /// Ratify an amendment after the deliberation period expires.
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn ratify_amendment(origin: OriginFor<T>, law_id: u32) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(origin)?;
            let (new_hash, proposed_at) =
                PendingAmendments::<T>::take(law_id).ok_or(Error::<T>::AmendmentNotFound)?;
            let law = Laws::<T>::get(law_id).ok_or(Error::<T>::LawNotFound)?;
            let deliberation_blocks = match law.0 {
                LawTier::Constitutional => T::ConstitutionalDeliberationBlocks::get(),
                LawTier::Ordinary => T::OrdinaryAmendmentDeliberationBlocks::get(),
            };
            let deliberation = BlockNumberFor::<T>::from(deliberation_blocks);
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(now >= proposed_at + deliberation, Error::<T>::DeliberationPeriodActive);
            Laws::<T>::try_mutate(law_id, |maybe_law| {
                let law = maybe_law.as_mut().ok_or(Error::<T>::LawNotFound)?;
                law.2 = law.2.saturating_add(1); // bump version
                law.3 = new_hash;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::AmendmentRatified { law_id, new_hash });
            Ok(())
        }

        /// Submit a new petition. topic_hash is the IPFS CID of the petition text.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn submit_petition(
            origin: OriginFor<T>,
            topic_hash: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive);
            let id = NextPetitionId::<T>::get();
            let now = frame_system::Pallet::<T>::block_number();
            Petitions::<T>::insert(id, (who.clone(), topic_hash, 0u32, now));
            NextPetitionId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::PetitionSubmitted { petition_id: id, proposer: who, topic_hash });
            Ok(())
        }

        /// Sign an existing petition. Each account may sign once.
        /// When the signature count crosses PetitionThreshold, emits PetitionThresholdReached.
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn sign_petition(
            origin: OriginFor<T>,
            petition_id: u32,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive);
            ensure!(
                !PetitionSignatures::<T>::get((petition_id, &who)),
                Error::<T>::AlreadySigned
            );
            let mut petition = Petitions::<T>::get(petition_id)
                .ok_or(Error::<T>::PetitionNotFound)?;
            let new_count = petition.2.saturating_add(1);
            petition.2 = new_count;
            Petitions::<T>::insert(petition_id, &petition);
            PetitionSignatures::<T>::insert((petition_id, &who), true);
            Self::deposit_event(Event::PetitionSigned {
                petition_id,
                signer: who,
                signature_count: new_count,
            });
            if new_count == T::PetitionThreshold::get() {
                Self::deposit_event(Event::PetitionThresholdReached {
                    petition_id,
                    topic_hash: petition.1,
                });
                // Auto-create a referendum in pallet-voting. Failure here is non-fatal
                // (e.g., duplicate petition) — we don't roll back the signature.
                let _ = T::PetitionApprover::create_referendum(petition_id, petition.1);
            }
            Ok(())
        }

        /// Veto a newly enacted law on human rights grounds. Only callable by HumanRightsOrigin
        /// within HRCVetoWindowBlocks of the law's enactment. After the window, use pallet-courts.
        #[pallet::call_index(6)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn veto_law(origin: OriginFor<T>, law_id: u32) -> DispatchResult {
            T::HumanRightsOrigin::ensure_origin(origin)?;
            let enacted_at = LawEnactedAt::<T>::get(law_id).ok_or(Error::<T>::LawNotFound)?;
            let veto_window = BlockNumberFor::<T>::from(T::HRCVetoWindowBlocks::get());
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(now <= enacted_at + veto_window, Error::<T>::VetoWindowExpired);
            Laws::<T>::try_mutate(law_id, |maybe_law| {
                let law = maybe_law.as_mut().ok_or(Error::<T>::LawNotFound)?;
                ensure!(law.1 == LawStatus::Active, Error::<T>::LawNotActive);
                law.1 = LawStatus::Paused;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::LawVetoed { law_id });
            Ok(())
        }

        /// Repeal a law entirely. A repealed law is terminal: it cannot be amended or re-enacted
        /// by the same id. Only Active or Paused laws may be repealed.
        #[pallet::call_index(7)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn repeal_law(origin: OriginFor<T>, law_id: u32) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(origin)?;
            Laws::<T>::try_mutate(law_id, |maybe_law| {
                let law = maybe_law.as_mut().ok_or(Error::<T>::LawNotFound)?;
                ensure!(law.1 != LawStatus::Repealed, Error::<T>::LawAlreadyRepealed);
                law.1 = LawStatus::Repealed;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::LawRepealed { law_id });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Called by pallet-voting (via LawEnactor trait in the runtime) when a referendum passes.
        /// Enacts a new Ordinary law from the petition's IPFS content hash.
        pub fn enact_law_internal(tier: LawTier, content_hash: [u8; 32]) -> DispatchResult {
            let id = NextLawId::<T>::get();
            Laws::<T>::insert(id, (tier.clone(), LawStatus::Active, 1u32, content_hash));
            LawEnactedAt::<T>::insert(id, frame_system::Pallet::<T>::block_number());
            NextLawId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::LawEnacted { law_id: id, tier, content_hash });
            Ok(())
        }

        /// Called by pallet-courts (via LawEnforcer trait in the runtime) when a ruling
        /// overturns a law. Pauses the law pending legislature review.
        /// Returns LawNotActive if the law is already Paused or Repealed — prevents
        /// a court ruling from silently resurrecting a dead law.
        pub fn invalidate_law_internal(law_id: u32) -> DispatchResult {
            Laws::<T>::try_mutate(law_id, |maybe_law| {
                let law = maybe_law.as_mut().ok_or(Error::<T>::LawNotFound)?;
                ensure!(law.1 == LawStatus::Active, Error::<T>::LawNotActive);
                law.1 = LawStatus::Paused;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::LawInvalidated { law_id });
            Ok(())
        }
    }
}
