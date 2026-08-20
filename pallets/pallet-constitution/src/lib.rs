//! # Constitution Pallet
//!
//! Versioned on-chain law ledger with three tiers:
//!   - Ordinary: legislature simple-majority (`OrdinaryPassageThreshold`, 51%); amendments take
//!     effect after OrdinaryAmendmentDeliberationBlocks.
//!   - Structural: high-threshold tier (`ConstitutionalPassageThreshold`, 67%); amendments enter
//!     the Provisional → Confirmed → Entrenched pipeline.
//!   - Foundational: highest tier (`FoundationalPassageThreshold`, 75%); same pipeline as
//!     Structural.
//!
//! These are the same three percentages the referendum path in pallet-voting uses
//! (`PassageThreshold`/`ConstitutionalPassageThreshold`/`FoundationalPassageThreshold` there),
//! so a citizen referendum and a direct legislature motion need the same supermajority to
//! enact/amend/repeal a law of a given tier — there is no lower-threshold back door through
//! the legislature-motion path.
//!
//! **How the right threshold gets enforced, and why it can't be gamed:** `enact_law`,
//! `propose_constitutional_amendment`, `reaffirm_amendment`, and `repeal_law` each compute the
//! required percentage from the *real* tier of what they're acting on — never from a value a
//! proposer merely asserts — before calling `T::LegislatureOrigin::ensure_origin`:
//!   - `enact_law`'s `tier` argument is itself part of the domain-separated preimage hashed into
//!     `call_hash` (see `legislature_call_hash` below). A motion's `call_hash` is fixed at
//!     `propose_motion` time and is what legislature members actually vote on; if whoever
//!     executes the passed motion supplied a different `tier` than the one that was proposed
//!     (e.g. to claim Ordinary while enacting a Foundational law), the recomputed hash would no
//!     longer match the motion's approved `call_hash` and `EnsureLegislatureMotion` would reject
//!     the origin outright — independent of the threshold check. So the tier used to compute the
//!     required percentage is cryptographically pinned to the tier that was actually voted on.
//!   - `propose_constitutional_amendment`, `reaffirm_amendment`, and `repeal_law` instead read
//!     the tier directly from `Laws` on-chain storage (the law's *current*, already-enacted
//!     tier) — ground truth no caller can influence via a call parameter.
//! `pallet_legislature::EnsureLegislatureMotion`'s `EnsureOriginWithArg<_, ([u8; 32], u8)>`
//! overload then checks that percentage against the real `ayes`/`total_members` tally frozen
//! when the motion closed, so authorization only succeeds if the legislature's *actual* recorded
//! support met the tier-appropriate bar.
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
//! Courts may pause any Active law via CourtOrigin.
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
    use codec::DecodeWithMemTracking;
    use frame_support::pallet_prelude::*;
    use frame_support::traits::EnsureOriginWithArg;
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::Saturating;
    use crate::weights::WeightInfo;

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
        Paused,   // court-invalidated, pending review
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
        /// Law version before this amendment — restored on revocation.
        pub previous_version: u32,
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

    /// Benchmark-only hook: makes an account satisfy `CitizenChecker::is_active_citizen` for
    /// extrinsics gated on citizen status (`submit_petition`, `sign_petition`). Real citizen
    /// registration goes through pallet-identity-zk's full ZK-proof flow, which a generic
    /// pallet-constitution benchmark has no way to drive directly — this hook lets each
    /// runtime (or test mock) short-circuit that for benchmarking purposes only. See
    /// `weights.rs`'s module doc comment for which implementations actually wire this up.
    #[cfg(feature = "runtime-benchmarks")]
    pub trait BenchmarkHelper<AccountId> {
        fn make_active_citizen(who: &AccountId);
        /// Makes `FreshLegislatureChecker::has_election_occurred_since` return `true`, for
        /// benchmarking `reaffirm_amendment` (real state is normally pallet-elections'
        /// `LastElectionBlock`, which a generic pallet-constitution benchmark can't set).
        fn make_legislature_fresh();
    }

    /// Computes the domain-separated hash a legislature motion's `call_hash` must equal for
    /// `EnsureLegislatureMotion` (or any `LegislatureOrigin`) to authorize `tag`'s call with
    /// `params`. `tag` should uniquely identify the pallet + dispatchable (e.g.
    /// `b"pallet-constitution::enact_law"`) so that byte-identical parameters passed to a
    /// different call — in this pallet or another legislature-gated one — never collide.
    /// Off-chain tooling proposing a motion must compute the hash the same way.
    pub(crate) fn legislature_call_hash(tag: &'static [u8], params: impl Encode) -> [u8; 32] {
        let mut preimage = alloc::vec::Vec::from(tag);
        preimage.extend(params.encode());
        frame_support::Hashable::blake2_256(&preimage)
    }

    /// Required legislature passage percentage for `tier`, from this pallet's own
    /// `OrdinaryPassageThreshold`/`ConstitutionalPassageThreshold`/`FoundationalPassageThreshold`
    /// config constants (kept numerically in sync with pallet-voting's referendum thresholds by
    /// the runtime wiring — see that pallet's `Config` doc comments). See the module doc comment
    /// for why the `tier` fed into this can't be spoofed independently of what's actually being
    /// enacted, amended, or repealed.
    pub(crate) fn required_threshold<T: Config>(tier: &LawTier) -> u8 {
        match tier {
            LawTier::Ordinary => T::OrdinaryPassageThreshold::get(),
            LawTier::Structural => T::ConstitutionalPassageThreshold::get(),
            LawTier::Foundational => T::FoundationalPassageThreshold::get(),
        }
    }

    // ── Pallet ───────────────────────────────────────────────────────────────────

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    // ── Config ───────────────────────────────────────────────────────────────────

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        // ── Legislature ──────────────────────────────────────────────────────────
        /// Origin representing a passed legislature motion (law enactment, amendments, repeal).
        /// `EnsureOriginWithArg<_, ([u8; 32], u8)>` so each call site is *required* to pass both
        /// the domain-separated hash of its own parameters (see `legislature_call_hash` below)
        /// and the passage percentage that call actually needs (see `required_threshold` and
        /// the module doc comment for how that percentage is derived and why it can't be
        /// spoofed) — the origin check verifies the hash against the motion's approved
        /// `call_hash` *and* the required percentage against the tally frozen when the motion
        /// closed. A motion passed to authorize one call can never be replayed to execute
        /// another, and a motion that didn't reach the real required threshold can never
        /// authorize a higher-tier action.
        type LegislatureOrigin: frame_support::traits::EnsureOriginWithArg<Self::RuntimeOrigin, ([u8; 32], u8)>;

        /// Legislature-motion passage percentage for Ordinary-tier calls (e.g. 51). Matches
        /// pallet-voting's Ordinary referendum `PassageThreshold`.
        #[pallet::constant]
        type OrdinaryPassageThreshold: Get<u8>;
        /// Legislature-motion passage percentage for Structural-tier calls (e.g. 67). Must be
        /// higher than `OrdinaryPassageThreshold`. Matches pallet-voting's Structural referendum
        /// `ConstitutionalPassageThreshold`.
        #[pallet::constant]
        type ConstitutionalPassageThreshold: Get<u8>;
        /// Legislature-motion passage percentage for Foundational-tier calls (e.g. 75). Must be
        /// higher than `ConstitutionalPassageThreshold`. Matches pallet-voting's Foundational
        /// referendum `FoundationalPassageThreshold`.
        #[pallet::constant]
        type FoundationalPassageThreshold: Get<u8>;

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

        /// Weight functions needed for this pallet's extrinsics.
        type WeightInfo: crate::weights::WeightInfo;

        /// See `BenchmarkHelper`'s doc comment.
        #[cfg(feature = "runtime-benchmarks")]
        type BenchmarkHelper: BenchmarkHelper<Self::AccountId>;
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
        /// A Structural/Foundational law enacted successfully, but `AutoChallengeHook::
        /// auto_challenge_law` returned an error, so the mandatory automatic court review for
        /// that law was never filed. The law is still Active — this event exists so off-chain
        /// monitors/indexers can detect the gap and alert or retry out of band.
        AutoChallengeFilingFailed { law_id: u32 },
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
        /// Enact a new law. Requires LegislatureOrigin at the passage percentage `tier` demands
        /// (`required_threshold`) — see the module doc comment for why `tier` can't be spoofed.
        #[pallet::call_index(0)]
        #[pallet::weight(T::WeightInfo::enact_law())]
        pub fn enact_law(
            origin: OriginFor<T>,
            tier: LawTier,
            content_hash: [u8; 32],
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &(
                    legislature_call_hash(b"pallet-constitution::enact_law", (tier.clone(), content_hash)),
                    required_threshold::<T>(&tier),
                ),
            )?;
            let id = NextLawId::<T>::get();
            Laws::<T>::insert(id, (tier.clone(), LawStatus::Active, 1u32, content_hash));
            NextLawId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::LawEnacted { law_id: id, tier: tier.clone(), content_hash });
            if tier == LawTier::Structural || tier == LawTier::Foundational {
                // Best-effort: a courts pallet error must not prevent law enactment, but a
                // failure must still be visible on-chain — see `AutoChallengeFilingFailed`.
                if T::AutoChallengeHook::auto_challenge_law(id).is_err() {
                    Self::deposit_event(Event::AutoChallengeFilingFailed { law_id: id });
                }
            }
            Ok(())
        }

        /// Pause a law on court invalidation ruling.
        #[pallet::call_index(1)]
        #[pallet::weight(T::WeightInfo::invalidate_law())]
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
        /// Requires `OrdinaryPassageThreshold` — this call is unconditionally Ordinary-only
        /// (enforced by the `UseConstitutionalAmendmentCall` check below), so there is no tier
        /// to derive a higher requirement from, and a mismatched real tier is still rejected
        /// downstream regardless of what threshold authorized the origin.
        #[pallet::call_index(2)]
        #[pallet::weight(T::WeightInfo::propose_amendment())]
        pub fn propose_amendment(
            origin: OriginFor<T>,
            law_id: u32,
            proposed_hash: [u8; 32],
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &(
                    legislature_call_hash(b"pallet-constitution::propose_amendment", (law_id, proposed_hash)),
                    T::OrdinaryPassageThreshold::get(),
                ),
            )?;
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
        /// Requires `OrdinaryPassageThreshold` — see `propose_amendment`'s doc comment; the
        /// same "Ordinary-only, rejected downstream otherwise" reasoning applies here.
        #[pallet::call_index(3)]
        #[pallet::weight(T::WeightInfo::ratify_amendment())]
        pub fn ratify_amendment(origin: OriginFor<T>, law_id: u32) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &(
                    legislature_call_hash(b"pallet-constitution::ratify_amendment", law_id),
                    T::OrdinaryPassageThreshold::get(),
                ),
            )?;
            let (new_hash, proposed_at) =
                PendingAmendments::<T>::take(law_id).ok_or(Error::<T>::AmendmentNotFound)?;
            let deliberation =
                BlockNumberFor::<T>::from(T::OrdinaryAmendmentDeliberationBlocks::get());
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(
                now >= proposed_at.saturating_add(deliberation),
                Error::<T>::DeliberationPeriodActive
            );
            Laws::<T>::try_mutate(law_id, |maybe_law| {
                let law = maybe_law.as_mut().ok_or(Error::<T>::LawNotFound)?;
                ensure!(law.0 == LawTier::Ordinary, Error::<T>::UseConstitutionalAmendmentCall);
                // Guard: law must still be Active. A court ruling may have paused it between
                // propose_amendment and ratify_amendment; ratifying a Paused law would silently
                // update content while the law is suspended, corrupting the law ledger state.
                ensure!(law.1 == LawStatus::Active, Error::<T>::LawNotActive);
                law.2 = law.2.saturating_add(1);
                law.3 = new_hash;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::AmendmentRatified { law_id, new_hash });
            Ok(())
        }

        /// Submit a new petition. topic_hash is the IPFS CID of the petition text.
        /// The proposer is automatically recorded as the first signer (count starts at 1).
        #[pallet::call_index(4)]
        #[pallet::weight(T::WeightInfo::submit_petition())]
        pub fn submit_petition(origin: OriginFor<T>, topic_hash: [u8; 32]) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive);
            let id = NextPetitionId::<T>::get();
            let now = frame_system::Pallet::<T>::block_number();
            // Proposer is counted as the first signature — prevents a petition from existing
            // with 0 signatures and requiring a separate sign_petition call from the same proposer.
            Petitions::<T>::insert(id, (who.clone(), topic_hash, 1u32, now));
            PetitionSignatures::<T>::insert((id, &who), true);
            NextPetitionId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::PetitionSubmitted { petition_id: id, proposer: who.clone(), topic_hash });
            Self::deposit_event(Event::PetitionSigned { petition_id: id, signer: who, signature_count: 1 });
            // When threshold is 1, the proposer's signature already satisfies it.
            if 1u32 == T::PetitionThreshold::get() {
                Self::deposit_event(Event::PetitionThresholdReached { petition_id: id, topic_hash });
                T::PetitionApprover::create_referendum(id, topic_hash)?;
            }
            Ok(())
        }

        /// Sign an existing petition. Each account may sign once.
        /// Crossing PetitionThreshold auto-creates an Ordinary referendum.
        #[pallet::call_index(5)]
        #[pallet::weight(T::WeightInfo::sign_petition())]
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
                T::PetitionApprover::create_referendum(petition_id, petition.1)?;
            }
            Ok(())
        }

        /// Repeal a law entirely. Terminal — cannot be re-enacted under the same id.
        /// Cleans up any pending Ordinary or Constitutional amendments for this law.
        /// Requires LegislatureOrigin at the passage percentage the law's *current, on-chain*
        /// tier demands — repealing a Structural/Foundational law is as significant as enacting
        /// one, so it needs the same supermajority, read from `Laws` storage (not a caller-
        /// supplied value) so it can't be understated. If `law_id` doesn't exist the fallback is
        /// the highest (Foundational) threshold — fail closed; the call rejects with
        /// `LawNotFound` right after regardless of what threshold authorized the origin.
        #[pallet::call_index(6)]
        #[pallet::weight(T::WeightInfo::repeal_law())]
        pub fn repeal_law(origin: OriginFor<T>, law_id: u32) -> DispatchResult {
            let tier = Laws::<T>::get(law_id).map(|l| l.0).unwrap_or(LawTier::Foundational);
            T::LegislatureOrigin::ensure_origin(
                origin,
                &(
                    legislature_call_hash(b"pallet-constitution::repeal_law", law_id),
                    required_threshold::<T>(&tier),
                ),
            )?;
            Laws::<T>::try_mutate(law_id, |maybe_law| {
                let law = maybe_law.as_mut().ok_or(Error::<T>::LawNotFound)?;
                ensure!(law.1 != LawStatus::Repealed, Error::<T>::LawAlreadyRepealed);
                law.1 = LawStatus::Repealed;
                Ok::<(), DispatchError>(())
            })?;
            // Clean up any in-flight amendments so their storage is reclaimed and
            // a future propose_amendment call doesn't see stale pending records.
            PendingAmendments::<T>::remove(law_id);
            ConstitutionalAmendments::<T>::remove(law_id);
            Self::deposit_event(Event::LawRepealed { law_id });
            Ok(())
        }

        /// Propose an amendment to a Structural or Foundational law.
        ///
        /// The new hash is applied immediately and the amendment enters the Provisional stage.
        /// It can be revoked at any stage via RevocationOrigin; the required revocation threshold
        /// grows as the amendment matures (30% Provisional / 35% Confirmed / 40% Entrenched),
        /// enforced externally by the RevocationOrigin's collective configuration.
        ///
        /// Requires LegislatureOrigin at the passage percentage the law's *current, on-chain*
        /// tier demands (read from `Laws` storage, not a caller-supplied value — see the module
        /// doc comment). Fallback if `law_id` doesn't exist is the highest (Foundational)
        /// threshold — fail closed; the call rejects with `LawNotFound` right after regardless.
        #[pallet::call_index(7)]
        #[pallet::weight(T::WeightInfo::propose_constitutional_amendment())]
        pub fn propose_constitutional_amendment(
            origin: OriginFor<T>,
            law_id: u32,
            new_hash: [u8; 32],
        ) -> DispatchResult {
            let tier = Laws::<T>::get(law_id).map(|l| l.0).unwrap_or(LawTier::Foundational);
            T::LegislatureOrigin::ensure_origin(
                origin,
                &(
                    legislature_call_hash(
                        b"pallet-constitution::propose_constitutional_amendment",
                        (law_id, new_hash),
                    ),
                    required_threshold::<T>(&tier),
                ),
            )?;
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
            let previous_version = law.2;
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
                    previous_version,
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
        ///
        /// Requires LegislatureOrigin at the passage percentage the law's *current, on-chain*
        /// tier demands (read from `Laws` storage — same pattern as `propose_constitutional_amendment`
        /// and `repeal_law`; see the module doc comment).
        #[pallet::call_index(8)]
        #[pallet::weight(T::WeightInfo::reaffirm_amendment())]
        pub fn reaffirm_amendment(origin: OriginFor<T>, law_id: u32) -> DispatchResult {
            let tier = Laws::<T>::get(law_id).map(|l| l.0).unwrap_or(LawTier::Foundational);
            T::LegislatureOrigin::ensure_origin(
                origin,
                &(
                    legislature_call_hash(b"pallet-constitution::reaffirm_amendment", law_id),
                    required_threshold::<T>(&tier),
                ),
            )?;
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
                    now >= record.proposed_at.saturating_add(provisioning),
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
        #[pallet::call_index(9)]
        #[pallet::weight(T::WeightInfo::advance_to_entrenched())]
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
                    now >= record.proposed_at.saturating_add(total),
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
        #[pallet::call_index(10)]
        #[pallet::weight(T::WeightInfo::revoke_amendment())]
        pub fn revoke_amendment(origin: OriginFor<T>, law_id: u32) -> DispatchResult {
            T::RevocationOrigin::ensure_origin(origin)?;
            let record = ConstitutionalAmendments::<T>::take(law_id)
                .ok_or(Error::<T>::ConstitutionalAmendmentNotFound)?;

            let restored_hash = record.previous_hash;
            Laws::<T>::try_mutate(law_id, |maybe_law| {
                let law = maybe_law.as_mut().ok_or(Error::<T>::LawNotFound)?;
                // Restore the version from before the amendment was proposed, not increment.
                law.2 = record.previous_version;
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
                // Best-effort: a courts pallet error must not prevent law enactment, but a
                // failure must still be visible on-chain — see `AutoChallengeFilingFailed`.
                if T::AutoChallengeHook::auto_challenge_law(id).is_err() {
                    Self::deposit_event(Event::AutoChallengeFilingFailed { law_id: id });
                }
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
