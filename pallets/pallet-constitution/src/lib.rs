//! # Constitution Pallet
//!
//! Versioned on-chain law ledger with three tiers:
//!   - Ordinary: legislature simple-majority; amendments take effect after OrdinaryAmendmentDeliberationBlocks.
//!   - Structural: high-threshold tier; amendments enter the Provisional → Confirmed → Entrenched pipeline.
//!   - Foundational: same pipeline as Structural; higher passage thresholds enforced by the legislature origin.
//!
//! Constitutional (Structural/Foundational) amendment lifecycle:
//!   1. propose_constitutional_amendment: hash applied immediately, record enters Provisional stage.
//!   2. reaffirm_amendment: callable after ProvisioningPeriodBlocks by a legislature that has held
//!      at least one election since the proposal — advances to Confirmed.
//!   3. advance_to_entrenched: permissionless once ProvisioningPeriodBlocks + ConfirmationPeriodBlocks
//!      have elapsed — advances to Entrenched.
//!   4. revoke_amendment: RevocationOrigin may revert the amendment at any stage.
//!      The required revocation threshold (30 / 35 / 40 %) grows by stage and is
//!      enforced externally by the RevocationOrigin collective configuration.
//!
//! HRC may veto any newly enacted law within HRCVetoWindowBlocks.
//! Courts may pause any Active law via CourtOrigin.
#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
    use codec::DecodeWithMemTracking;
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;

    // ── Types ────────────────────────────────────────────────────────────────────

    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum LawTier {
        /// Legislature simple-majority motion; amendments ratified after OrdinaryAmendmentDeliberationBlocks.
        Ordinary,
        /// High-threshold tier. Amendments enter the Provisional → Confirmed → Entrenched pipeline.
        Structural,
        /// Highest tier. Same pipeline as Structural; higher passage thresholds enforced externally.
        Foundational,
    }

    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum LawStatus {
        Active,
        Paused,   // court-invalidated or HRC-vetoed, pending review
        Repealed,
    }

    /// Maturity stage of a Structural/Foundational amendment.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum MaturityStage {
        /// Newly proposed. Hash applied; RevocationOrigin may revert (30% threshold intended).
        Provisional,
        /// Fresh-legislature reaffirmation received. Revocation threshold rises to 35%.
        Confirmed,
        /// Full pipeline elapsed. Revocation threshold rises to 40%.
        Entrenched,
    }

    /// Live record of a Structural/Foundational amendment in its maturing pipeline.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    #[scale_info(skip_type_params(T))]
    pub struct ConstitutionalAmendmentRecord<T: Config> {
        /// Content hash before this amendment — restored on revocation.
        pub previous_hash: [u8; 32],
        /// Content hash applied at proposal time.
        pub new_hash: [u8; 32],
        /// Block at which the amendment was proposed.
        pub proposed_at: BlockNumberFor<T>,
        /// Current maturity stage.
        pub stage: MaturityStage,
        /// True once a post-election legislature has called reaffirm_amendment.
        pub legislature_reaffirmed: bool,
    }

    // ── Traits ───────────────────────────────────────────────────────────────────

    /// Called by pallet-constitution when a petition crosses PetitionThreshold.
    pub trait PetitionApprover {
        fn create_referendum(petition_id: u32, topic_hash: [u8; 32]) -> DispatchResult;
    }

    /// Gate: returns false if the account is not a registered active citizen.
    pub trait CitizenChecker<AccountId> {
        fn is_active_citizen(who: &AccountId) -> bool;
    }

    /// Checks whether at least one election has occurred since a given block.
    /// Implemented by the runtime via pallet-elections::LastElectionBlock.
    pub trait FreshLegislatureChecker<BlockNumber> {
        fn has_election_occurred_since(proposed_at: BlockNumber) -> bool;
    }

    /// Called when a Structural or Foundational law is enacted, to automatically open a
    /// court case for AI review. Implemented by the runtime via pallet-courts::auto_file_case.
    pub trait AutoChallengeHook {
        fn auto_challenge_law(law_id: u32) -> DispatchResult;
    }

    // ── Pallet ───────────────────────────────────────────────────────────────────

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    // ── Config ───────────────────────────────────────────────────────────────────

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        // ── Legislature ──────────────────────────────────────────────────────────
        /// Origin representing a passed legislature motion (law enactment + Ordinary amendments).
        type LegislatureOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;

        // ── Ordinary amendments ──────────────────────────────────────────────────
        /// Deliberation blocks before an Ordinary amendment can be ratified (may be 0).
        #[pallet::constant]
        type OrdinaryAmendmentDeliberationBlocks: Get<u32>;

        // ── Constitutional amendment pipeline ────────────────────────────────────
        /// Blocks from proposal until the fresh-legislature reaffirmation window opens.
        /// Intended: 2 * 365 * DAYS ≈ 2 years.
        #[pallet::constant]
        type ProvisioningPeriodBlocks: Get<u32>;
        /// Additional blocks after Confirmed before Entrenched can be claimed.
        /// Intended: 4 * 365 * DAYS ≈ 4 years (total pipeline ≈ 6 years).
        #[pallet::constant]
        type ConfirmationPeriodBlocks: Get<u32>;
        /// Checks whether at least one election occurred since the given block.
        type FreshLegislatureChecker: FreshLegislatureChecker<BlockNumberFor<Self>>;
        /// Origin permitted to revoke a constitutional amendment.
        /// Wire to a minority collective (30–40% of legislature, stage-dependent) in production.
        type RevocationOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;

        // ── Petitions ────────────────────────────────────────────────────────────
        /// Minimum citizen signatures required for a petition to trigger a referendum.
        #[pallet::constant]
        type PetitionThreshold: Get<u32>;
        type PetitionApprover: PetitionApprover;
        type CitizenChecker: CitizenChecker<Self::AccountId>;

        // ── Auto-challenge (replaces HRC) ────────────────────────────────────────
        /// Called when a Structural or Foundational law is enacted.
        /// Automatically opens a court case for AI review — the opposition can also file
        /// challenges manually via pallet-courts at any time.
        type AutoChallengeHook: AutoChallengeHook;

        // ── Courts ───────────────────────────────────────────────────────────────
        /// Origin permitted to pause a law via court ruling.
        type CourtOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;
    }

    // ── Storage ──────────────────────────────────────────────────────────────────

    /// law_id → (tier, status, version, content_hash).
    #[pallet::storage]
    pub type Laws<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, (LawTier, LawStatus, u32, [u8; 32])>;

    /// Pending Ordinary amendments: law_id → (proposed_hash, proposed_at_block).
    #[pallet::storage]
    pub type PendingAmendments<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, ([u8; 32], BlockNumberFor<T>)>;

    /// Live Structural/Foundational amendments in the maturing pipeline.
    #[pallet::storage]
    pub type ConstitutionalAmendments<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, ConstitutionalAmendmentRecord<T>>;

    #[pallet::storage]
    pub type NextLawId<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// petition_id → (proposer, topic_hash, signature_count, submitted_at_block).
    #[pallet::storage]
    pub type Petitions<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, (T::AccountId, [u8; 32], u32, BlockNumberFor<T>)>;

    /// Tracks which accounts have signed which petition. Prevents double-signing.
    #[pallet::storage]
    pub type PetitionSignatures<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, T::AccountId), bool, ValueQuery>;

    #[pallet::storage]
    pub type NextPetitionId<T: Config> = StorageValue<_, u32, ValueQuery>;

    // ── Events ───────────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        LawEnacted { law_id: u32, tier: LawTier, content_hash: [u8; 32] },
        LawInvalidated { law_id: u32 },
        LawRepealed { law_id: u32 },
        // Ordinary amendments
        AmendmentProposed { law_id: u32, proposed_hash: [u8; 32] },
        AmendmentRatified { law_id: u32, new_hash: [u8; 32] },
        // Constitutional (Structural/Foundational) amendment pipeline
        ConstitutionalAmendmentProposed { law_id: u32, new_hash: [u8; 32], tier: LawTier },
        AmendmentReaffirmed { law_id: u32 },
        AmendmentAdvancedToEntrenched { law_id: u32 },
        AmendmentRevoked { law_id: u32, restored_hash: [u8; 32] },
        // Petitions
        PetitionSubmitted { petition_id: u32, proposer: T::AccountId, topic_hash: [u8; 32] },
        PetitionSigned { petition_id: u32, signer: T::AccountId, signature_count: u32 },
        PetitionThresholdReached { petition_id: u32, topic_hash: [u8; 32] },
    }

    // ── Errors ───────────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        LawNotFound,
        LawNotActive,
        LawAlreadyRepealed,
        // Ordinary amendment errors
        AmendmentNotFound,
        AmendmentAlreadyPending,
        DeliberationPeriodActive,
        /// propose_amendment is for Ordinary laws only. Use propose_constitutional_amendment instead.
        UseConstitutionalAmendmentCall,
        // Constitutional amendment errors
        /// propose_constitutional_amendment is for Structural/Foundational laws only.
        UseOrdinaryAmendmentCall,
        ConstitutionalAmendmentAlreadyPending,
        ConstitutionalAmendmentNotFound,
        /// reaffirm_amendment called before ProvisioningPeriodBlocks have elapsed.
        ProvisioningPeriodNotElapsed,
        /// reaffirm_amendment requires at least one election since the proposal block.
        LegislatureNotFresh,
        /// reaffirm_amendment already called for this amendment.
        AlreadyReaffirmed,
        /// reaffirm_amendment requires Provisional stage.
        AmendmentNotProvisional,
        /// advance_to_entrenched requires Confirmed stage.
        AmendmentNotConfirmed,
        /// advance_to_entrenched called before ProvisioningPeriodBlocks + ConfirmationPeriodBlocks.
        ConfirmationPeriodNotElapsed,
        /// Amendment is already Entrenched.
        AmendmentAlreadyEntrenched,
        // Petitions
        PetitionNotFound,
        AlreadySigned,
        CitizenNotActive,
    }

    // ── Calls ────────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Enact a new law. Requires LegislatureOrigin.
        /// For Structural/Foundational laws the runtime should wire a higher-threshold legislature origin.
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
            NextLawId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::LawEnacted { law_id: id, tier: tier.clone(), content_hash });
            if tier == LawTier::Structural || tier == LawTier::Foundational {
                let _ = T::AutoChallengeHook::auto_challenge_law(id);
            }
            Ok(())
        }

        /// Pause a law on court invalidation ruling.
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

        /// Propose an amendment to an Ordinary law. Starts the deliberation clock.
        /// For Structural/Foundational laws use propose_constitutional_amendment.
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
            ensure!(law.0 == LawTier::Ordinary, Error::<T>::UseConstitutionalAmendmentCall);
            ensure!(
                !PendingAmendments::<T>::contains_key(law_id),
                Error::<T>::AmendmentAlreadyPending
            );
            let proposed_at = frame_system::Pallet::<T>::block_number();
            PendingAmendments::<T>::insert(law_id, (proposed_hash, proposed_at));
            Self::deposit_event(Event::AmendmentProposed { law_id, proposed_hash });
            Ok(())
        }

        /// Ratify an Ordinary law amendment after its deliberation period expires.
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn ratify_amendment(origin: OriginFor<T>, law_id: u32) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(origin)?;
            let (new_hash, proposed_at) =
                PendingAmendments::<T>::take(law_id).ok_or(Error::<T>::AmendmentNotFound)?;
            let deliberation =
                BlockNumberFor::<T>::from(T::OrdinaryAmendmentDeliberationBlocks::get());
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(now >= proposed_at + deliberation, Error::<T>::DeliberationPeriodActive);
            Laws::<T>::try_mutate(law_id, |maybe_law| {
                let law = maybe_law.as_mut().ok_or(Error::<T>::LawNotFound)?;
                ensure!(law.0 == LawTier::Ordinary, Error::<T>::UseConstitutionalAmendmentCall);
                law.2 = law.2.saturating_add(1);
                law.3 = new_hash;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::AmendmentRatified { law_id, new_hash });
            Ok(())
        }

        /// Submit a new petition. topic_hash is the IPFS CID of the petition text.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn submit_petition(origin: OriginFor<T>, topic_hash: [u8; 32]) -> DispatchResult {
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
        /// Crossing PetitionThreshold auto-creates an Ordinary referendum.
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn sign_petition(origin: OriginFor<T>, petition_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive);
            ensure!(
                !PetitionSignatures::<T>::get((petition_id, &who)),
                Error::<T>::AlreadySigned
            );
            let mut petition =
                Petitions::<T>::get(petition_id).ok_or(Error::<T>::PetitionNotFound)?;
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
                let _ = T::PetitionApprover::create_referendum(petition_id, petition.1);
            }
            Ok(())
        }

        /// Repeal a law entirely. Terminal — cannot be re-enacted under the same id.
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

        /// Propose an amendment to a Structural or Foundational law.
        ///
        /// The new hash is applied immediately and the amendment enters the Provisional stage.
        /// It can be revoked at any stage via RevocationOrigin; the required revocation threshold
        /// grows as the amendment matures (30% Provisional / 35% Confirmed / 40% Entrenched),
        /// enforced externally by the RevocationOrigin's collective configuration.
        #[pallet::call_index(8)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn propose_constitutional_amendment(
            origin: OriginFor<T>,
            law_id: u32,
            new_hash: [u8; 32],
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(origin)?;
            let law = Laws::<T>::get(law_id).ok_or(Error::<T>::LawNotFound)?;
            ensure!(law.1 == LawStatus::Active, Error::<T>::LawNotActive);
            ensure!(
                law.0 == LawTier::Structural || law.0 == LawTier::Foundational,
                Error::<T>::UseOrdinaryAmendmentCall
            );
            ensure!(
                !ConstitutionalAmendments::<T>::contains_key(law_id),
                Error::<T>::ConstitutionalAmendmentAlreadyPending
            );

            let previous_hash = law.3;
            let now = frame_system::Pallet::<T>::block_number();

            // Apply the amendment immediately; the Provisional stage allows revocation.
            Laws::<T>::try_mutate(law_id, |maybe_law| {
                let l = maybe_law.as_mut().ok_or(Error::<T>::LawNotFound)?;
                l.2 = l.2.saturating_add(1);
                l.3 = new_hash;
                Ok::<(), DispatchError>(())
            })?;

            ConstitutionalAmendments::<T>::insert(
                law_id,
                ConstitutionalAmendmentRecord::<T> {
                    previous_hash,
                    new_hash,
                    proposed_at: now,
                    stage: MaturityStage::Provisional,
                    legislature_reaffirmed: false,
                },
            );

            Self::deposit_event(Event::ConstitutionalAmendmentProposed {
                law_id,
                new_hash,
                tier: law.0,
            });
            Ok(())
        }

        /// Reaffirm a Structural/Foundational amendment after ProvisioningPeriodBlocks.
        ///
        /// Must be called by a legislature that held at least one election after the proposal block.
        /// Advances the amendment from Provisional → Confirmed.
        #[pallet::call_index(9)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn reaffirm_amendment(origin: OriginFor<T>, law_id: u32) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(origin)?;
            ConstitutionalAmendments::<T>::try_mutate(law_id, |maybe_record| {
                let record =
                    maybe_record.as_mut().ok_or(Error::<T>::ConstitutionalAmendmentNotFound)?;
                ensure!(
                    record.stage == MaturityStage::Provisional,
                    Error::<T>::AmendmentNotProvisional
                );
                ensure!(!record.legislature_reaffirmed, Error::<T>::AlreadyReaffirmed);

                let now = frame_system::Pallet::<T>::block_number();
                let provisioning =
                    BlockNumberFor::<T>::from(T::ProvisioningPeriodBlocks::get());
                ensure!(
                    now >= record.proposed_at + provisioning,
                    Error::<T>::ProvisioningPeriodNotElapsed
                );
                ensure!(
                    T::FreshLegislatureChecker::has_election_occurred_since(record.proposed_at),
                    Error::<T>::LegislatureNotFresh
                );

                record.legislature_reaffirmed = true;
                record.stage = MaturityStage::Confirmed;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::AmendmentReaffirmed { law_id });
            Ok(())
        }

        /// Advance a Confirmed amendment to Entrenched once the full pipeline has elapsed.
        ///
        /// Permissionless — anyone may call once
        /// now >= proposed_at + ProvisioningPeriodBlocks + ConfirmationPeriodBlocks.
        #[pallet::call_index(10)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn advance_to_entrenched(origin: OriginFor<T>, law_id: u32) -> DispatchResult {
            let _who = ensure_signed(origin)?;
            ConstitutionalAmendments::<T>::try_mutate(law_id, |maybe_record| {
                let record =
                    maybe_record.as_mut().ok_or(Error::<T>::ConstitutionalAmendmentNotFound)?;
                ensure!(
                    record.stage == MaturityStage::Confirmed,
                    Error::<T>::AmendmentNotConfirmed
                );

                let now = frame_system::Pallet::<T>::block_number();
                let total = BlockNumberFor::<T>::from(
                    T::ProvisioningPeriodBlocks::get()
                        .saturating_add(T::ConfirmationPeriodBlocks::get()),
                );
                ensure!(
                    now >= record.proposed_at + total,
                    Error::<T>::ConfirmationPeriodNotElapsed
                );

                record.stage = MaturityStage::Entrenched;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::AmendmentAdvancedToEntrenched { law_id });
            Ok(())
        }

        /// Revoke a constitutional amendment at any stage, restoring the previous law hash.
        ///
        /// Requires RevocationOrigin. The threshold enforced by that collective should be:
        ///   Provisional → 30% of legislature
        ///   Confirmed   → 35% of legislature
        ///   Entrenched  → 40% of legislature + citizen referendum
        #[pallet::call_index(11)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn revoke_amendment(origin: OriginFor<T>, law_id: u32) -> DispatchResult {
            T::RevocationOrigin::ensure_origin(origin)?;
            let record = ConstitutionalAmendments::<T>::take(law_id)
                .ok_or(Error::<T>::ConstitutionalAmendmentNotFound)?;

            let restored_hash = record.previous_hash;
            Laws::<T>::try_mutate(law_id, |maybe_law| {
                let law = maybe_law.as_mut().ok_or(Error::<T>::LawNotFound)?;
                law.2 = law.2.saturating_add(1);
                law.3 = restored_hash;
                Ok::<(), DispatchError>(())
            })?;

            Self::deposit_event(Event::AmendmentRevoked { law_id, restored_hash });
            Ok(())
        }
    }

    // ── Internal helpers (called by other pallets via runtime trait impls) ────────

    impl<T: Config> Pallet<T> {
        /// Enact a law from a passed referendum. Called by pallet-voting via LawEnactor trait.
        pub fn enact_law_internal(tier: LawTier, content_hash: [u8; 32]) -> DispatchResult {
            let id = NextLawId::<T>::get();
            Laws::<T>::insert(id, (tier.clone(), LawStatus::Active, 1u32, content_hash));
            NextLawId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::LawEnacted { law_id: id, tier: tier.clone(), content_hash });
            if tier == LawTier::Structural || tier == LawTier::Foundational {
                let _ = T::AutoChallengeHook::auto_challenge_law(id);
            }
            Ok(())
        }

        /// Pause a law on a court ruling. Called by pallet-courts via LawEnforcer trait.
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
