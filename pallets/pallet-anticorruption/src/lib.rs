//! # Anti-Corruption Pallet
//!
//! Transparency and accountability module for elected officials and public servants.
//!
//! Three pillars:
//! 1. **Asset disclosure** — officials submit an IPFS-hashed declaration of assets;
//!    renewals are due every `AssetDisclosureRenewalBlocks`.
//! 2. **Conflict-of-interest registry** — officials self-declare relationships with entities
//!    they vote on (financial interest, family, former employer, business partner).
//! 3. **Anonymous ZK whistleblower reports** — citizens submit anonymous reports backed by
//!    a ZK proof of passport registration. A per-report nullifier prevents duplicate filings.
//!
//! Investigators (appointed by root) move reports through a workflow:
//! Pending → Flagged → UnderInvestigation → Cleared | ReferredToCourts
//! When a report is referred to courts, the investigator then files a case in pallet-courts.
#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {

    use codec::DecodeWithMemTracking;
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    // ── Cross-pallet traits ──────────────────────────────────────────────────

    /// Verifies a ZK citizenship proof. Used to gate anonymous whistleblower submissions.
    /// The proof must attest to a registered, active passport without revealing the identity.
    /// Implemented in the runtime by delegating to pallet-identity's verifier.
    pub trait ZkProofVerifier {
        fn verify(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> bool;
    }

    // ── Data types ───────────────────────────────────────────────────────────

    /// The nature of a declared conflict of interest.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum ConflictType {
        FinancialInterest,
        FamilyRelation,
        FormerEmployer,
        BusinessPartner,
    }

    /// Workflow status of a whistleblower report.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum ReportStatus {
        /// Submitted but not yet reviewed.
        Pending,
        /// Flagged by an investigator as requiring follow-up.
        Flagged,
        /// Actively under investigation.
        UnderInvestigation,
        /// Investigation concluded — no violation found.
        Cleared,
        /// Referred to pallet-courts for formal proceedings.
        ReferredToCourts,
    }

    /// On-chain record of an official's asset disclosure.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct AssetDeclaration<BlockNumber> {
        /// IPFS content hash of the full signed asset declaration document.
        pub ipfs_hash: [u8; 32],
        /// Block at which this disclosure was submitted.
        pub disclosed_at: BlockNumber,
        /// Block by which the next renewal must be submitted.
        pub update_due_at: BlockNumber,
    }

    /// A declared conflict of interest for an official/entity pair.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct ConflictEntry<BlockNumber> {
        pub conflict_type: ConflictType,
        pub registered_at: BlockNumber,
    }

    /// An anonymous whistleblower report backed by a ZK citizenship proof.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct WhistleblowerReport<BlockNumber> {
        /// IPFS hash of the report content (encrypted to investigator key off-chain).
        pub content_hash: [u8; 32],
        pub submitted_at: BlockNumber,
        pub status: ReportStatus,
        /// Privacy-preserving citizen nullifier from the ZK proof.
        /// Stored for linkage detection; not the raw national-ID hash.
        pub nullifier: [u8; 32],
    }

    // ── Config ───────────────────────────────────────────────────────────────

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Verifier for the ZK citizenship proof supplied by whistleblowers.
        type ZkVerifier: ZkProofVerifier;
        /// Maximum number of appointed investigators.
        #[pallet::constant]
        type MaxInvestigators: Get<u32>;
        /// How many blocks between mandatory asset disclosure renewals (e.g., 1 year).
        #[pallet::constant]
        type AssetDisclosureRenewalBlocks: Get<u32>;
    }

    // ── Storage ──────────────────────────────────────────────────────────────

    /// Per-account asset declarations. Any account (official) may submit one.
    #[pallet::storage]
    pub type AssetDisclosures<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, AssetDeclaration<BlockNumberFor<T>>>;

    /// Conflict-of-interest registry: (official, entity_id) → conflict entry.
    #[pallet::storage]
    pub type ConflictRegistry<T: Config> =
        StorageMap<_, Blake2_128Concat, (T::AccountId, u32), ConflictEntry<BlockNumberFor<T>>>;

    /// Whistleblower reports keyed by auto-incrementing report id.
    #[pallet::storage]
    pub type WhistleblowerReports<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, WhistleblowerReport<BlockNumberFor<T>>>;

    /// Prevents the same citizen from filing the same report twice.
    /// Key = (nullifier [u8;32], content_hash [u8;32]) → exists.
    #[pallet::storage]
    pub type ReportNullifiers<T: Config> =
        StorageMap<_, Blake2_128Concat, ([u8; 32], [u8; 32]), bool, ValueQuery>;

    /// Auto-incrementing report id counter.
    #[pallet::storage]
    pub type NextReportId<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Appointed investigators who may advance report workflow state.
    #[pallet::storage]
    pub type Investigators<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxInvestigators>, ValueQuery>;

    // ── Events ───────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// An official submitted or renewed their asset declaration.
        AssetDisclosed {
            who: T::AccountId,
            ipfs_hash: [u8; 32],
            update_due_at: BlockNumberFor<T>,
        },
        /// An official registered a conflict of interest.
        ConflictRegistered {
            who: T::AccountId,
            entity_id: u32,
            conflict_type: ConflictType,
        },
        /// An official removed a conflict of interest declaration.
        ConflictCleared { who: T::AccountId, entity_id: u32 },
        /// An anonymous whistleblower report was submitted.
        ReportSubmitted { report_id: u32, content_hash: [u8; 32] },
        /// An investigator flagged a report for follow-up.
        ReportFlagged { report_id: u32, investigator: T::AccountId },
        /// An investigator opened a formal investigation on a report.
        InvestigationOpened { report_id: u32, investigator: T::AccountId },
        /// An investigator cleared a report — no violation found.
        ReportCleared { report_id: u32, investigator: T::AccountId },
        /// An investigator referred a report to pallet-courts for formal proceedings.
        ReportReferredToCourts { report_id: u32, investigator: T::AccountId },
        /// A new investigator was appointed.
        InvestigatorAdded { who: T::AccountId },
        /// An investigator was removed.
        InvestigatorRemoved { who: T::AccountId },
    }

    // ── Errors ───────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        /// The ZK citizenship proof is invalid — caller is not a registered citizen.
        InvalidZkProof,
        /// This citizen has already filed an identical report (same nullifier + content hash).
        DuplicateReport,
        /// public_inputs is empty — at minimum public_inputs[0] (the nullifier) must be present.
        MissingNullifierInput,
        /// Report id does not exist.
        ReportNotFound,
        /// The report is not in the expected state for this transition.
        InvalidReportState,
        /// Caller is not a designated investigator.
        NotInvestigator,
        /// Conflict-of-interest entry not found for this (account, entity_id) pair.
        ConflictNotFound,
        /// Investigator list is at capacity (MaxInvestigators).
        TooManyInvestigators,
        /// Account is already a registered investigator.
        AlreadyInvestigator,
    }

    // ── Calls ────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Submit or renew an asset declaration. Any account may disclose their assets.
        /// The IPFS hash points to a signed declaration document stored off-chain.
        /// Sets `update_due_at` to `now + AssetDisclosureRenewalBlocks`.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn submit_asset_disclosure(
            origin: OriginFor<T>,
            ipfs_hash: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let now = frame_system::Pallet::<T>::block_number();
            let update_due_at = now + BlockNumberFor::<T>::from(T::AssetDisclosureRenewalBlocks::get());
            AssetDisclosures::<T>::insert(
                &who,
                AssetDeclaration { ipfs_hash, disclosed_at: now, update_due_at },
            );
            Self::deposit_event(Event::AssetDisclosed { who, ipfs_hash, update_due_at });
            Ok(())
        }

        /// Register a conflict of interest between the caller and a given entity.
        /// `entity_id` is an arbitrary identifier (e.g., department id, company registry id).
        /// Overwrites any existing entry for the same (caller, entity_id) pair.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn register_conflict(
            origin: OriginFor<T>,
            entity_id: u32,
            conflict_type: ConflictType,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let now = frame_system::Pallet::<T>::block_number();
            ConflictRegistry::<T>::insert(
                (who.clone(), entity_id),
                ConflictEntry { conflict_type: conflict_type.clone(), registered_at: now },
            );
            Self::deposit_event(Event::ConflictRegistered { who, entity_id, conflict_type });
            Ok(())
        }

        /// Remove a previously declared conflict of interest.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn clear_conflict(origin: OriginFor<T>, entity_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(
                ConflictRegistry::<T>::contains_key((who.clone(), entity_id)),
                Error::<T>::ConflictNotFound
            );
            ConflictRegistry::<T>::remove((who.clone(), entity_id));
            Self::deposit_event(Event::ConflictCleared { who, entity_id });
            Ok(())
        }

        /// Submit an anonymous whistleblower report.
        ///
        /// Requires a valid ZK proof of passport registration so that spam is gated behind
        /// real citizenship while preserving anonymity. The nullifier in `public_inputs[0]`
        /// is stored (not the plaintext identity) to detect duplicate filings.
        ///
        /// A (nullifier, content_hash) pair may only be used once — a citizen cannot file
        /// the same report twice, but can file different reports.
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(60_000, 0))]
        pub fn submit_whistleblower_report(
            origin: OriginFor<T>,
            content_hash: [u8; 32],
            zk_proof: BoundedVec<u8, ConstU32<4096>>,
            public_inputs: BoundedVec<[u8; 32], ConstU32<16>>,
        ) -> DispatchResult {
            let _who = ensure_signed(origin)?;
            // public_inputs[0] is the citizen nullifier (Poseidon2(national_id || country_code)).
            // Must check length before calling the verifier so dev-mode passthrough can't be
            // exploited with empty inputs to produce an all-zero sentinel nullifier.
            ensure!(!public_inputs.is_empty(), Error::<T>::MissingNullifierInput);
            ensure!(
                T::ZkVerifier::verify(zk_proof.as_slice(), public_inputs.as_slice()),
                Error::<T>::InvalidZkProof
            );
            let nullifier = public_inputs[0];
            ensure!(
                !ReportNullifiers::<T>::get((nullifier, content_hash)),
                Error::<T>::DuplicateReport
            );
            ReportNullifiers::<T>::insert((nullifier, content_hash), true);
            let id = NextReportId::<T>::get();
            let now = frame_system::Pallet::<T>::block_number();
            WhistleblowerReports::<T>::insert(
                id,
                WhistleblowerReport {
                    content_hash,
                    submitted_at: now,
                    status: ReportStatus::Pending,
                    nullifier,
                },
            );
            NextReportId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::ReportSubmitted { report_id: id, content_hash });
            Ok(())
        }

        /// Flag a pending report for investigator follow-up. Investigator only.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn flag_report(origin: OriginFor<T>, report_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_investigator(&who), Error::<T>::NotInvestigator);
            WhistleblowerReports::<T>::try_mutate(report_id, |maybe| {
                let entry = maybe.as_mut().ok_or(Error::<T>::ReportNotFound)?;
                ensure!(entry.status == ReportStatus::Pending, Error::<T>::InvalidReportState);
                entry.status = ReportStatus::Flagged;
                Ok::<_, DispatchError>(())
            })?;
            Self::deposit_event(Event::ReportFlagged { report_id, investigator: who });
            Ok(())
        }

        /// Open a formal investigation on a flagged report. Investigator only.
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn open_investigation(origin: OriginFor<T>, report_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_investigator(&who), Error::<T>::NotInvestigator);
            WhistleblowerReports::<T>::try_mutate(report_id, |maybe| {
                let entry = maybe.as_mut().ok_or(Error::<T>::ReportNotFound)?;
                ensure!(entry.status == ReportStatus::Flagged, Error::<T>::InvalidReportState);
                entry.status = ReportStatus::UnderInvestigation;
                Ok::<_, DispatchError>(())
            })?;
            Self::deposit_event(Event::InvestigationOpened { report_id, investigator: who });
            Ok(())
        }

        /// Clear a report under investigation — no violation found. Investigator only.
        #[pallet::call_index(6)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn clear_report(origin: OriginFor<T>, report_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_investigator(&who), Error::<T>::NotInvestigator);
            WhistleblowerReports::<T>::try_mutate(report_id, |maybe| {
                let entry = maybe.as_mut().ok_or(Error::<T>::ReportNotFound)?;
                ensure!(
                    entry.status == ReportStatus::UnderInvestigation,
                    Error::<T>::InvalidReportState
                );
                entry.status = ReportStatus::Cleared;
                Ok::<_, DispatchError>(())
            })?;
            Self::deposit_event(Event::ReportCleared { report_id, investigator: who });
            Ok(())
        }

        /// Refer an investigated report to pallet-courts for formal proceedings.
        /// Emits ReportReferredToCourts; the investigator then files a case in pallet-courts.
        /// Investigator only.
        #[pallet::call_index(7)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn refer_report_to_courts(origin: OriginFor<T>, report_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_investigator(&who), Error::<T>::NotInvestigator);
            WhistleblowerReports::<T>::try_mutate(report_id, |maybe| {
                let entry = maybe.as_mut().ok_or(Error::<T>::ReportNotFound)?;
                ensure!(
                    entry.status == ReportStatus::UnderInvestigation,
                    Error::<T>::InvalidReportState
                );
                entry.status = ReportStatus::ReferredToCourts;
                Ok::<_, DispatchError>(())
            })?;
            Self::deposit_event(Event::ReportReferredToCourts { report_id, investigator: who });
            Ok(())
        }

        /// Appoint a new investigator. Root only.
        #[pallet::call_index(8)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn add_investigator(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            Investigators::<T>::try_mutate(|list| {
                ensure!(!list.contains(&who), Error::<T>::AlreadyInvestigator);
                list.try_push(who.clone()).map_err(|_| Error::<T>::TooManyInvestigators)
            })?;
            Self::deposit_event(Event::InvestigatorAdded { who });
            Ok(())
        }

        /// Remove an investigator. Root only.
        #[pallet::call_index(9)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn remove_investigator(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            Investigators::<T>::mutate(|list| list.retain(|x| x != &who));
            Self::deposit_event(Event::InvestigatorRemoved { who });
            Ok(())
        }
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
        fn is_investigator(who: &T::AccountId) -> bool {
            Investigators::<T>::get().contains(who)
        }
    }
}
