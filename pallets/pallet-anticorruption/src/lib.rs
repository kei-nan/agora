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
//! Investigators (appointed via `Config::AppointmentOrigin`) move reports through a workflow:
//! Pending → Flagged → UnderInvestigation → Cleared | ReferredToCourts
//! When a report is referred to courts, the investigator then files a case in pallet-courts.
//!
//! ## Recusal: a structural 2-of-N safeguard on `clear_report`/`refer_report_to_courts`
//! Report *content* is off-chain and encrypted to the investigator's key precisely so the chain
//! cannot see who or what a report concerns — which also means the chain can never check "is
//! this investigator recusing from a report about themselves" without breaking that privacy
//! model. Any such on-chain check would be a fabricated no-op. Instead, closing a report
//! (`clear_report` or `refer_report_to_courts`) is structural: one investigator's call only
//! *proposes* the transition (recorded in `PendingReportAction`, keyed by `report_id`); the
//! report's `status` does not change until a **different** investigator calls
//! `approve_report_action` to co-sign it — mirrors the propose/approve pattern used for the
//! Oracle Council (`pallet-courts`) and the Accountability Council
//! (`pallet-accountability-council`), scoped down to a 2-of-N (any two distinct investigators)
//! rather than a supermajority, since this is a peer-recusal safeguard, not a governance vote.
//! A lone investigator — including one clearing/referring a report that happens to be about
//! themselves — can never unilaterally close a report. See `clear_report`/
//! `refer_report_to_courts`/`approve_report_action`'s doc comments below.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {

    use codec::DecodeWithMemTracking;
    use frame_support::pallet_prelude::*;
    use frame_support::traits::EnsureOriginWithArg;
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::Saturating;

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

    /// Which terminal transition a `PendingReportAction` entry is awaiting a second, different
    /// investigator's approval for. See the module doc comment's "Recusal" section.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum ReportAction {
        /// Awaiting approval to move the report to `ReportStatus::Cleared`.
        Clear,
        /// Awaiting approval to move the report to `ReportStatus::ReferredToCourts`.
        ReferToCourts,
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
        /// Origin required to add/remove investigators. `add_investigator`/
        /// `remove_investigator` used to be bare `ensure_root`, which routed appointment
        /// through whoever holds `Root` with no dedicated oversight body at all — see
        /// `pallet_accountability_council`'s module doc comment for why that's a problem
        /// (self-oversight: the branch that controls the treasury must not also pick its own
        /// investigators) and why a separate, independent Accountability Council exists to
        /// fix it. Wire this to
        /// `pallet_accountability_council::EnsureAccountabilityCouncilApproved` in production
        /// (requires that Council's genuine 2/3 supermajority for the exact call — see
        /// `add_investigator`/`remove_investigator`'s use of
        /// `pallet_accountability_council::accountability_call_hash` below); kept generic
        /// over `EnsureOriginWithArg<Self::RuntimeOrigin, [u8; 32]>` rather than depending on
        /// that concrete type, the same way `pallet_constitution::Config::CourtOrigin` is
        /// generic over `pallet_courts::EnsureOracleCouncilApproved` — call-hash binding is
        /// what's required here, not the specific pallet.
        type AppointmentOrigin: EnsureOriginWithArg<Self::RuntimeOrigin, [u8; 32]>;
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

    /// `report_id -> (action, proposer)` for a `clear_report`/`refer_report_to_courts` call
    /// awaiting a second, different investigator's `approve_report_action` sign-off — see the
    /// module doc comment's "Recusal" section. At most one pending action per report at a time:
    /// a new proposal for the same report is rejected while one is outstanding, so a report can
    /// never have conflicting Clear/ReferToCourts proposals in flight simultaneously.
    #[pallet::storage]
    pub type PendingReportAction<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, (ReportAction, T::AccountId)>;

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
        /// An investigator proposed closing a report (`Clear` or `ReferToCourts`) — awaiting a
        /// second, different investigator's `approve_report_action` before it takes effect. See
        /// the module doc comment's "Recusal" section.
        ReportActionProposed { report_id: u32, action: ReportAction, proposer: T::AccountId },
        /// A second, different investigator approved a pending `Clear` action — the report is
        /// now `Cleared`. No violation found.
        ReportCleared { report_id: u32, proposer: T::AccountId, approver: T::AccountId },
        /// A second, different investigator approved a pending `ReferToCourts` action — the
        /// report is now `ReferredToCourts`.
        ReportReferredToCourts { report_id: u32, proposer: T::AccountId, approver: T::AccountId },
        /// A new investigator was appointed.
        InvestigatorAdded { who: T::AccountId },
        /// An investigator was removed.
        InvestigatorRemoved { who: T::AccountId },
        /// An investigator rejected another investigator's pending `Clear`/`ReferToCourts`
        /// proposal without approving it — the report returns to `UnderInvestigation` with no
        /// pending action, open for a fresh proposal. See `reject_report_action`.
        ReportActionRejected {
            report_id: u32,
            action: ReportAction,
            proposer: T::AccountId,
            rejecter: T::AccountId,
        },
        /// `remove_investigator` cleared a `PendingReportAction` entry because the removed
        /// investigator was its sole proposer — otherwise the entry would have been
        /// permanently stuck (no one else could approve their own proposal, and a new
        /// proposal cannot be raised while one is outstanding). See `remove_investigator`.
        PendingReportActionClearedOnRemoval { report_id: u32, removed_investigator: T::AccountId },
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
        /// This report already has a `Clear`/`ReferToCourts` proposal pending a second
        /// investigator's approval — a new proposal cannot be raised until it resolves.
        ReportActionAlreadyPending,
        /// `approve_report_action` was called for a report with no pending action.
        NoPendingReportAction,
        /// The caller is the same investigator who proposed this pending action — a second,
        /// *different* investigator is required. See the module doc comment's "Recusal"
        /// section.
        SameInvestigator,
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
            let update_due_at = now.saturating_add(BlockNumberFor::<T>::from(T::AssetDisclosureRenewalBlocks::get()));
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
        // This call performs a real BN254/UltraHonk pairing check via `T::ZkVerifier::verify`,
        // not just storage writes — the flat `Weight::from_parts(60_000, 0)` this used to carry
        // was the same order of magnitude as the trivial storage-only calls elsewhere in this
        // file (5,000-10,000), pricing a genuine cryptographic pairing check as if it were a
        // single storage read. Checked this codebase for how other real ZK-verification calls
        // are weighted before picking a number:
        //   - `pallet-identity`'s `register_citizen`/`reverify_citizen`/`migrate_oprf_scheme`/
        //     `recover_account` also call `T::ZkVerifier::verify` for a real outer-proof
        //     pairing check, but are themselves only flat `Weight::from_parts` literals in the
        //     20,000-50,000 range — no more benchmarked or crypto-aware than this file's own
        //     placeholder was, so not a genuine precedent to match here.
        //   - `pallet-elections`'s `register_as_delegate`/`back_delegate`
        //     (`pallets/pallet-elections/src/weights.rs`) are the one place in this codebase
        //     that actually costs a real standalone UltraHonk pairing check
        //     (`T::BackingProofVerifier::verify`) as a `WeightInfo`-benchmarked weight rather
        //     than a bare guess: 14,000,000-22,000,000 ref-time units plus DbWeight reads/
        //     writes, with their own doc comment noting even that likely underestimates the
        //     true pairing-check cost.
        // Matching the latter's order of magnitude (same class of operation: a real UltraHonk
        // proof verification gating a citizen-signed extrinsic) rather than the former's
        // unbenchmarked flat constants, which share this call's original underpricing problem.
        // Still a hand-picked placeholder, not a real `frame-benchmarking` number — pending
        // that, sanity-check this choice against pallet-identity's ZK calls being fixed too.
        #[pallet::weight(Weight::from_parts(20_000_000, 0))]
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

        /// Propose clearing a report under investigation — no violation found. Investigator
        /// only. **Does not clear the report by itself**: this records the caller as the
        /// report's pending-`Clear` proposer and leaves `status` at `UnderInvestigation` until
        /// a second, *different* investigator calls `approve_report_action` — see the module
        /// doc comment's "Recusal" section for why a single investigator can never unilaterally
        /// close a report, including one about themselves.
        #[pallet::call_index(6)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn clear_report(origin: OriginFor<T>, report_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_investigator(&who), Error::<T>::NotInvestigator);
            Self::propose_report_action(report_id, ReportAction::Clear, who)
        }

        /// Propose referring an investigated report to pallet-courts for formal proceedings.
        /// Investigator only. Same propose/approve safeguard as `clear_report`: does not
        /// transition the report by itself — a second, different investigator must call
        /// `approve_report_action`. Once approved, the (first) investigator to have proposed or
        /// approved the referral is expected to file the actual case in pallet-courts.
        #[pallet::call_index(7)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn refer_report_to_courts(origin: OriginFor<T>, report_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_investigator(&who), Error::<T>::NotInvestigator);
            Self::propose_report_action(report_id, ReportAction::ReferToCourts, who)
        }

        /// Appoint a new investigator. Requires `AppointmentOrigin` — in production, the
        /// Accountability Council's own 2/3 supermajority approval for this exact call (see
        /// `Config::AppointmentOrigin`'s doc comment), not bare `Root`.
        #[pallet::call_index(8)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn add_investigator(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
            T::AppointmentOrigin::ensure_origin(
                origin,
                &pallet_accountability_council::accountability_call_hash(
                    b"pallet-anticorruption::add_investigator",
                    &who,
                ),
            )?;
            Investigators::<T>::try_mutate(|list| {
                ensure!(!list.contains(&who), Error::<T>::AlreadyInvestigator);
                list.try_push(who.clone()).map_err(|_| Error::<T>::TooManyInvestigators)
            })?;
            Self::deposit_event(Event::InvestigatorAdded { who });
            Ok(())
        }

        /// Remove an investigator. Same `AppointmentOrigin` gate as `add_investigator`.
        ///
        /// Also clears any `PendingReportAction` entry where the removed investigator was the
        /// sole proposer — otherwise ejecting a bad-faith investigator would not actually
        /// unstick the report they'd wedged: no one else can consume their proposal (a
        /// proposer can never approve their own action — `SameInvestigator` in
        /// `approve_report_action` — and the removed investigator can no longer call
        /// `reject_report_action` or anything else, `is_investigator` now excludes them), and
        /// no fresh proposal can be raised for that report while one is outstanding
        /// (`ReportActionAlreadyPending`). Clearing it here re-opens the report for a new
        /// proposal from a remaining investigator, exactly like `reject_report_action` would.
        #[pallet::call_index(9)]
        // `PendingReportAction::<T>::iter()` below is a full, unbounded scan over every report
        // with an outstanding Clear/ReferToCourts proposal, not the single `Investigators`
        // write the pre-existing flat `Weight::from_parts(5_000, 0)` was sized for (see this
        // function's doc comment on the scan's addition). `PendingReportAction` has no `Max*`
        // bound to cost against — unlike, say, `pallet_emergency_council`'s
        // `vote_declare_emergency`/`vote_end_emergency` (`pallets/pallet-emergency-council/src/
        // weights.rs`), which scale their own O(n) scan's DbWeight reads by `T::MaxCouncilSize`,
        // there is no config constant here capping how many reports can simultaneously be
        // `UnderInvestigation` with a pending action — it's driven indirectly by
        // `WhistleblowerReports`/`NextReportId`, themselves unbounded.
        //
        // Considered bounding the scan itself (`.take(N)`) instead of pricing it, mirroring
        // `pallet_elections`'s `MaxDelegateSweepPerBlock`-capped delegate sweep — rejected: a
        // plain iterator `.take(N)` only bounds how many *matches* get removed, not how many map
        // entries get *visited* before finding them (worst case still touches the whole map if
        // this investigator's entries are sparse or absent), so it wouldn't actually cap the
        // read-side cost this weight needs to cover. A real bound would need a resumable,
        // key-ordered cursor like `pallet_elections::DelegateSweepCursor` persisted across
        // separate `remove_investigator` calls — a materially bigger change than this fix's
        // scope, and not obviously worth it: even without this scan, any *other* current
        // investigator can already unstick an individual report proposed by the removed one via
        // `reject_report_action`, which (unlike `approve_report_action`) doesn't require the
        // caller to differ from the proposer — see that call's doc comment. So this scan is a
        // convenience auto-clear, not the only path to recovery.
        //
        // Priced instead for a generous assumed ceiling on realistic concurrent
        // `PendingReportAction` size — 500 entries, well above `pallet_elections`'s
        // `MaxDelegateSweepPerBlock` (100) and `pallet_emergency_council`'s `MaxCouncilSize`-
        // scale figure (35, see that pallet's `weights.rs`) for the same class of per-call
        // bounded/O(n) scan elsewhere in this codebase — at the same per-entry cost this file
        // already uses for a single storage op (matching `approve_report_action`/
        // `reject_report_action`'s own `8_000`): 5_000 (the original `Investigators` write) +
        // 500 * 8_000 = 4_005_000. Like every other weight in this pallet, this is a manually
        // reasoned placeholder, not a `frame-benchmarking` number — if `PendingReportAction` is
        // ever observed approaching this ceiling in practice, that's a signal this call needs
        // real pagination (a persisted cursor), not just a bigger constant.
        #[pallet::weight(Weight::from_parts(4_005_000, 0))]
        pub fn remove_investigator(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
            T::AppointmentOrigin::ensure_origin(
                origin,
                &pallet_accountability_council::accountability_call_hash(
                    b"pallet-anticorruption::remove_investigator",
                    &who,
                ),
            )?;
            Investigators::<T>::mutate(|list| list.retain(|x| x != &who));
            let stuck: alloc::vec::Vec<u32> = PendingReportAction::<T>::iter()
                .filter(|(_, (_, proposer))| proposer == &who)
                .map(|(report_id, _)| report_id)
                .collect();
            for report_id in stuck {
                PendingReportAction::<T>::remove(report_id);
                Self::deposit_event(Event::PendingReportActionClearedOnRemoval {
                    report_id,
                    removed_investigator: who.clone(),
                });
            }
            Self::deposit_event(Event::InvestigatorRemoved { who });
            Ok(())
        }

        /// Approve `report_id`'s pending `clear_report`/`refer_report_to_courts` proposal,
        /// applying whichever transition it recorded (`Clear` → `ReportStatus::Cleared`,
        /// `ReferToCourts` → `ReportStatus::ReferredToCourts`). Must be a current investigator
        /// **different** from the one who proposed it — see the module doc comment's "Recusal"
        /// section. Consumes the `PendingReportAction` entry so it cannot be approved twice.
        #[pallet::call_index(10)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn approve_report_action(origin: OriginFor<T>, report_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_investigator(&who), Error::<T>::NotInvestigator);
            let (action, proposer) = PendingReportAction::<T>::get(report_id)
                .ok_or(Error::<T>::NoPendingReportAction)?;
            ensure!(who != proposer, Error::<T>::SameInvestigator);
            WhistleblowerReports::<T>::try_mutate(report_id, |maybe| {
                let entry = maybe.as_mut().ok_or(Error::<T>::ReportNotFound)?;
                ensure!(
                    entry.status == ReportStatus::UnderInvestigation,
                    Error::<T>::InvalidReportState
                );
                entry.status = match action {
                    ReportAction::Clear => ReportStatus::Cleared,
                    ReportAction::ReferToCourts => ReportStatus::ReferredToCourts,
                };
                Ok::<_, DispatchError>(())
            })?;
            PendingReportAction::<T>::remove(report_id);
            match action {
                ReportAction::Clear => Self::deposit_event(Event::ReportCleared {
                    report_id,
                    proposer,
                    approver: who,
                }),
                ReportAction::ReferToCourts => Self::deposit_event(Event::ReportReferredToCourts {
                    report_id,
                    proposer,
                    approver: who,
                }),
            }
            Ok(())
        }

        /// Reject `report_id`'s pending `clear_report`/`refer_report_to_courts` proposal
        /// without approving it, clearing `PendingReportAction` so the report (still
        /// `UnderInvestigation`) is open for a fresh proposal.
        ///
        /// Fixes a stuck-report griefing path: with only `approve_report_action` available,
        /// an investigator who proposed the *wrong* action (e.g. `Clear` when the report
        /// should be `refer_report_to_courts`'d) permanently blocked anyone from proposing the
        /// right one — `propose_report_action` rejects a second proposal while one is
        /// outstanding (`ReportActionAlreadyPending`), and nothing short of approving the
        /// wrong action could clear the entry.
        ///
        /// Same authorization tier as `approve_report_action` — any current investigator.
        /// Unlike `approve_report_action`, the caller *may* be the original proposer:
        /// rejecting doesn't apply any transition or grant anyone new power, it only returns
        /// the report to its pre-proposal state, so the "different investigator" recusal rule
        /// (`SameInvestigator`, which exists to stop a single investigator from unilaterally
        /// *finalizing* an action) doesn't apply here — self-correcting a bad proposal is fine.
        #[pallet::call_index(11)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn reject_report_action(origin: OriginFor<T>, report_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_investigator(&who), Error::<T>::NotInvestigator);
            let (action, proposer) = PendingReportAction::<T>::get(report_id)
                .ok_or(Error::<T>::NoPendingReportAction)?;
            PendingReportAction::<T>::remove(report_id);
            Self::deposit_event(Event::ReportActionRejected {
                report_id,
                action,
                proposer,
                rejecter: who,
            });
            Ok(())
        }
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
        fn is_investigator(who: &T::AccountId) -> bool {
            Investigators::<T>::get().contains(who)
        }

        /// Shared body for `clear_report`/`refer_report_to_courts`: records `who` as
        /// `report_id`'s pending-action proposer without transitioning `status` — see
        /// `approve_report_action` for the second-investigator step that actually applies it.
        fn propose_report_action(
            report_id: u32,
            action: ReportAction,
            who: T::AccountId,
        ) -> DispatchResult {
            let entry = WhistleblowerReports::<T>::get(report_id).ok_or(Error::<T>::ReportNotFound)?;
            ensure!(entry.status == ReportStatus::UnderInvestigation, Error::<T>::InvalidReportState);
            ensure!(
                PendingReportAction::<T>::get(report_id).is_none(),
                Error::<T>::ReportActionAlreadyPending
            );
            PendingReportAction::<T>::insert(report_id, (action.clone(), who.clone()));
            Self::deposit_event(Event::ReportActionProposed { report_id, action, proposer: who });
            Ok(())
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
