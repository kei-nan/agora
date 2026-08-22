//! # Anti-Corruption Pallet
//!
//! Transparency and accountability module for elected officials and public servants.
//!
//! Three pillars:
//! 1. **Asset disclosure** — officials submit an IPFS-hashed declaration of assets;
//!    renewals are due every `AssetDisclosureRenewalBlocks`.
//! 2. **Conflict-of-interest registry** — officials self-declare relationships with entities
//!    they vote on (financial interest, family, former employer, business partner).
//! 3. **ZK-gated whistleblower reports** — citizens submit reports backed by a ZK proof of
//!    passport registration, which gates spam behind real citizenship and (via a per-report
//!    nullifier) prevents duplicate filings. The report *content* is off-chain and encrypted
//!    to the investigator's key, so only its IPFS hash lands on-chain. This is **not**
//!    anonymous submission, though: the call is a normal signed extrinsic, so the reporter's
//!    `AccountId` is public block data, and pallet-identity's `CitizenNullifier` map
//!    (`AccountId -> nullifier`) lets anyone watching the chain tie a report back to the
//!    citizen who filed it. See `submit_whistleblower_report`'s doc comment below for the
//!    full writeup — this has the same structural gap as `pallet-voting::commit_vote`.
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

    /// Verifies a ZK citizenship proof. Used to gate whistleblower submissions on real
    /// citizenship. The proof itself attests to a registered, active passport without
    /// revealing the identity behind it — but note that this proof-level property does not
    /// by itself make report *submission* anonymous, since `submit_whistleblower_report` is
    /// still a signed extrinsic; see that call's doc comment for the full gap.
    /// Implemented in the runtime by delegating to pallet-identity's verifier.
    pub trait ZkProofVerifier {
        fn verify(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> bool;
    }

    // ── ZKPassport public-input layout ──────────────────────────────────────
    //
    // `submit_whistleblower_report`'s `public_inputs` is a ZKPassport `main/outer/count_N`
    // outer-circuit public-input array. See `runtime/src/verifier.rs`'s module doc for the
    // authoritative, source-confirmed field-by-field table; summarized here:
    //
    //   0            certificate_registry_root   (shared by every citizen at a given
    //                                              registry state — NOT per-citizen)
    //   1            circuit_registry_root
    //   2            current_date
    //   3            service_scope
    //   4            service_subscope
    //   5 .. 5+D     param_commitments[D]         D = outer_count - 3
    //   5+D          nullifier_type
    //   6+D          scoped_nullifier              (= len - 2; the real per-citizen value)
    //   7+D          oprf_pk_hash                   (= len - 1)
    //
    // So `public_inputs.len() == D + 8`, minimized at `D = 1` (the smallest allowlisted
    // `count_4` variant) to exactly 9. `MIN_PUBLIC_INPUTS` mirrors
    // `pallet_identity::Pallet::verify_outer_proof`'s identical `public_inputs.len() >= 9`
    // guard, for the same reason: with this floor, the fixed-offset fields below
    // (`service_scope`/`service_subscope` at 3/4, `scoped_nullifier` at `len - 2`) are
    // always in bounds no matter how many disclosure subproofs the proof actually carries —
    // extra disclosures only widen the `param_commitments` slice in the middle, they never
    // move these fields out of range.

    /// Minimum length a structurally valid `public_inputs` array can have (the `count_4`
    /// variant, `D = 1`). Checked before indexing anything below it, and before invoking
    /// `T::ZkVerifier::verify` at all — so a dev-mode passthrough verifier (which does not
    /// itself check shape) can't be exploited with a too-short array to smuggle an
    /// attacker-chosen sentinel into `scoped_nullifier`/`service_scope`/`service_subscope`.
    const MIN_PUBLIC_INPUTS: usize = 9;

    /// Index of `service_scope` in `public_inputs` — fixed regardless of disclosure count.
    const SERVICE_SCOPE_INDEX: usize = 3;
    /// Index of `service_subscope` in `public_inputs` — fixed regardless of disclosure count.
    const SERVICE_SUBSCOPE_INDEX: usize = 4;

    /// Zero-pads a 31-byte ASCII tag into a canonical 32-byte big-endian BN254 `Fr` element:
    /// leading byte `0x00`, tag in the remaining 31 bytes. The field modulus's own leading
    /// byte is `0x30`, so any value whose leading byte is `0x00` is unconditionally below it
    /// regardless of what follows — this makes the result canonical by construction, without
    /// needing to hash anything or reason about the tag's specific bytes. The `&[u8; 31]`
    /// parameter type is itself a compile-time length check: a tag literal that isn't exactly
    /// 31 bytes fails to typecheck at the call site below.
    const fn zero_padded_scope_tag(tag: &[u8; 31]) -> [u8; 32] {
        let mut out = [0u8; 32];
        let mut i = 0;
        while i < 31 {
            out[i + 1] = tag[i];
            i += 1;
        }
        out
    }

    /// Domain-separation constant this pallet requires as the proof's `service_scope`
    /// (`public_inputs[SERVICE_SCOPE_INDEX]`). A ZKPassport outer-circuit proof only
    /// satisfies `submit_whistleblower_report` if it was generated with exactly this value
    /// (and [`WHISTLEBLOWER_REPORT_SERVICE_SUBSCOPE`] below) as its ZKPassport
    /// `scope`/`subscope` request parameters — this is what stops a proof produced for a
    /// different purpose (e.g. `pallet-identity::register_citizen`'s citizen-registration
    /// flow, which would carry a different scope) from being replayed here even though the
    /// underlying passport proof is perfectly valid citizenship. This pallet is the source
    /// of truth for the expected value; whatever constructs the real proof request
    /// (mobile/desktop ZKPassport integration) must be configured to match it exactly, the
    /// same way `mobile/src/chain/proofEncoding.ts` and `runtime/src/verifier.rs` already
    /// have to agree on the proof envelope.
    pub const WHISTLEBLOWER_REPORT_SERVICE_SCOPE: [u8; 32] =
        zero_padded_scope_tag(b"AGORA_ANTICORRUPTION_WHISTLE_V1");

    /// Domain-separation constant this pallet requires as the proof's `service_subscope`
    /// (`public_inputs[SERVICE_SUBSCOPE_INDEX]`). See
    /// [`WHISTLEBLOWER_REPORT_SERVICE_SCOPE`] above for the full rationale; `scope` and
    /// `subscope` are checked independently so a proof cannot pass by matching only one of
    /// the two.
    pub const WHISTLEBLOWER_REPORT_SERVICE_SUBSCOPE: [u8; 32] =
        zero_padded_scope_tag(b"AGORA_ANTICORRUPTION_WB_REPORT1");

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

    /// A whistleblower report backed by a ZK citizenship proof. Despite the name, the
    /// *submission* is not sender-anonymous — see `submit_whistleblower_report`'s doc comment.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct WhistleblowerReport<BlockNumber> {
        /// IPFS hash of the report content (encrypted to investigator key off-chain).
        pub content_hash: [u8; 32],
        pub submitted_at: BlockNumber,
        pub status: ReportStatus,
        /// The ZKPassport outer circuit's own `scoped_nullifier` public output
        /// (`public_inputs[public_inputs.len() - 2]`) — a real per-citizen value, not the
        /// shared `certificate_registry_root` at index 0. Stored for linkage detection; not
        /// the raw national-ID hash.
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
        /// `public_inputs` is shorter than `MIN_PUBLIC_INPUTS` — too short to contain a real
        /// ZKPassport outer-circuit layout (`service_scope`/`service_subscope`/
        /// `scoped_nullifier` at their expected fixed offsets).
        MissingNullifierInput,
        /// The proof's `service_scope`/`service_subscope` public inputs don't match this
        /// call's required domain-separation constants — the proof was generated for a
        /// different purpose (e.g. citizen registration) and cannot be replayed here.
        InvalidProofScope,
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

        /// Submit a whistleblower report.
        ///
        /// Requires a valid ZK proof of passport registration so that spam is gated behind
        /// real citizenship. The real per-citizen nullifier — ZKPassport's own
        /// `scoped_nullifier` output, at `public_inputs[public_inputs.len() - 2]`, **not**
        /// `public_inputs[0]` (which is `certificate_registry_root`, shared by every citizen
        /// at a given registry state) — is stored (not the plaintext identity) to detect
        /// duplicate filings: a (nullifier, content_hash) pair may only be used once, so a
        /// citizen cannot file the same report twice, but can file different reports. The
        /// proof must also carry this call's own `service_scope`/`service_subscope`
        /// (see [`WHISTLEBLOWER_REPORT_SERVICE_SCOPE`]/[`WHISTLEBLOWER_REPORT_SERVICE_SUBSCOPE`]),
        /// so a valid proof generated for a different purpose (e.g.
        /// `pallet-identity::register_citizen`) cannot be replayed into this call.
        ///
        /// ## Known sender-anonymity gap
        /// This call requires `ensure_signed`, so the extrinsic's signing `AccountId` (`_who`
        /// below — deliberately discarded, not written to storage) is publicly visible
        /// on-chain, in the block/extrinsic itself, independent of anything this pallet
        /// stores. The report's *content* is not exposed this way: only `content_hash`, the
        /// IPFS hash of a document that is itself encrypted to the investigator's key
        /// off-chain (see `WhistleblowerReport::content_hash`), ever lands in pallet storage,
        /// and both `WhistleblowerReports` and `Event::ReportSubmitted` carry only the
        /// nullifier/content hash, never `_who`. But pallet-identity's `CitizenNullifier`
        /// storage is a public, permanent map from `AccountId -> nullifier`. So anyone
        /// watching the chain can join "this signed extrinsic came from `AccountId` X" with
        /// "X's nullifier is N" (a public lookup) and learn "the citizen behind nullifier N
        /// filed a whistleblower report at block B" — i.e. *who* filed a report, and *when*,
        /// is linkable to their real registered identity, even though the report's *content*
        /// stays hidden. This is weaker than the "anonymous"/"ZK whistleblower" framing this
        /// pallet's module doc and `CLAUDE.md` use, which implies sender anonymity that does
        /// not exist — this call is pseudonymous against a casual reader, not anonymous
        /// against a chain observer willing to cross-reference `CitizenNullifier`.
        ///
        /// This is the same structural gap as `pallet-voting::commit_vote` (see that call's
        /// doc comment for the fuller writeup) and the same fix would apply: an unsigned
        /// extrinsic validated via a custom `ValidateUnsigned`/`SignedExtension` that checks
        /// ZK group-membership instead of a signature, or a relayer/mixnet that decouples
        /// submission from the signing key. No such infrastructure exists anywhere in this
        /// repo yet; building one is a genuine architectural addition, not a local fix to
        /// this call — left as a tracked gap rather than force a partial change here.
        ///
        /// **Note (2026-08-22): the `CitizenNullifier`-map cross-reference above is only one
        /// sub-case of a broader submission-metadata gap.** Even a signer account never
        /// registered as a citizen — so absent from `CitizenNullifier` — can still be
        /// deanonymized by ordinary chain analysis of its funding source (a direct on-chain
        /// transfer from the whistleblower's known account) or its submission timing relative
        /// to other citizen-linked activity, regardless of `content_hash` staying hidden. See
        /// `CLAUDE.md`'s Voting System section for the general writeup.
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(60_000, 0))]
        pub fn submit_whistleblower_report(
            origin: OriginFor<T>,
            content_hash: [u8; 32],
            zk_proof: BoundedVec<u8, ConstU32<4096>>,
            public_inputs: BoundedVec<[u8; 32], ConstU32<16>>,
        ) -> DispatchResult {
            let _who = ensure_signed(origin)?;
            // Must check length before indexing anything, and before calling the verifier, so
            // a dev-mode passthrough verifier (which does not itself check shape) can't be
            // exploited with a too-short array to smuggle an attacker-chosen sentinel into
            // scoped_nullifier/service_scope/service_subscope.
            ensure!(public_inputs.len() >= MIN_PUBLIC_INPUTS, Error::<T>::MissingNullifierInput);
            // Domain-separation: this proof must have been generated specifically for this
            // call (service_scope/service_subscope), not replayed from a different-purpose
            // proof (e.g. pallet-identity::register_citizen) that happens to be a valid,
            // observable-on-chain ZK proof of citizenship. Checked before the expensive
            // pairing check for the same cheap-first reasoning as the length check above.
            ensure!(
                public_inputs[SERVICE_SCOPE_INDEX] == WHISTLEBLOWER_REPORT_SERVICE_SCOPE
                    && public_inputs[SERVICE_SUBSCOPE_INDEX]
                        == WHISTLEBLOWER_REPORT_SERVICE_SUBSCOPE,
                Error::<T>::InvalidProofScope
            );
            ensure!(
                T::ZkVerifier::verify(zk_proof.as_slice(), public_inputs.as_slice()),
                Error::<T>::InvalidZkProof
            );
            // The real per-citizen value is scoped_nullifier, at `len - 2` (== `6 + D`, where
            // `D` is the disclosure-subproof count derived from the array's own length) — see
            // the "ZKPassport public-input layout" section above and
            // `pallet_identity::Pallet::register_citizen`'s identical extraction. NOT
            // `public_inputs[0]` (certificate_registry_root), which is shared by every
            // citizen at a given registry state and would collapse dedup to essentially just
            // content_hash.
            let nullifier = public_inputs[public_inputs.len() - 2];
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

        /// True if `who` has an asset declaration on file whose `update_due_at` has not yet
        /// passed. Used by pallet-elections (via a trait impl in the runtime) to gate candidacy
        /// on a current disclosure — an account with no disclosure, or one that's gone overdue
        /// since it was filed, is not current.
        pub fn has_current_disclosure(who: &T::AccountId) -> bool {
            match AssetDisclosures::<T>::get(who) {
                Some(declaration) => {
                    frame_system::Pallet::<T>::block_number() <= declaration.update_due_at
                }
                None => false,
            }
        }
    }
}

// ── DisclosureChecker implementation (pallet-elections seating gate) ──────────
//
// pallet-elections defines `DisclosureChecker<AccountId>` (the consumer) and this pallet (the
// provider) implements it directly on its own `Pallet<T>`, wrapping `has_current_disclosure`
// unchanged — the same idiom already used for `pallet_treasury_ledger::AuditHook` (implemented
// on `pallet_audit::Pallet<T>`) and `pallet_elections::SeatLegislature` (implemented on
// `pallet_legislature::Pallet<T>`). The runtime just wires `pallet_elections::Config::
// DisclosureChecker` to this pallet's type alias — no `Runtime`-level delegating impl needed.
impl<T: Config> pallet_elections::DisclosureChecker<T::AccountId> for Pallet<T> {
    fn has_current_disclosure(who: &T::AccountId) -> bool {
        Pallet::<T>::has_current_disclosure(who)
    }
}
