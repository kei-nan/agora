//! # Courts Pallet
//!
//! AI-first court system (Level 0: AI judge, Level 1: 7-person jury, Level 2: 21-person jury).
//! Rulings are auto-enforced: invalidated law -> pallet-constitution pauses it;
//! illegal treasury tx -> pallet-treasury-ledger freezes department.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {

    use codec::{Decode, DecodeWithMemTracking, Encode};
    use frame_support::{
        pallet_prelude::*,
        traits::{Currency, ReservableCurrency},
    };
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::{Hash as HashT, Saturating};

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    // ── Balance type alias ─────────────────────────────────────────────────────

    pub type BalanceOf<T> = <<T as Config>::Currency as Currency<
        <T as frame_system::Config>::AccountId,
    >>::Balance;

    // ── Cross-pallet enforcement traits ─────────────────────────────────────────

    /// Implemented by the runtime to call pallet-identity's citizen index.
    pub trait CitizenSelector<AccountId> {
        fn citizen_at(index: u32) -> Option<AccountId>;
        fn total_citizens() -> u32;
    }

    /// Implemented by the runtime to check whether an account is an active citizen.
    pub trait CitizenChecker<AccountId> {
        fn is_active_citizen(who: &AccountId) -> bool;
        /// Returns `who`'s registered identity nullifier, if any. Used by `appeal_ruling` to
        /// verify that a signer claiming to be the ruled-against party of a
        /// `CaseSubject::CitizenConduct` case actually is — see `is_ruled_against_party`.
        fn citizen_nullifier(who: &AccountId) -> Option<[u8; 32]>;
    }

    /// Implemented by the runtime to call pallet-constitution's invalidate_law_internal.
    pub trait LawEnforcer {
        fn invalidate_law(law_id: u32) -> DispatchResult;
    }

    /// Implemented by the runtime to call pallet-treasury-ledger's freeze_department_internal.
    pub trait TreasuryEnforcer {
        fn freeze_department(department_id: u32) -> DispatchResult;
    }

    /// Verifies a ZK citizenship proof for anonymized case filing (see `file_case`'s doc
    /// comment and the `CaseFiler` type). Same shape as `pallet_anticorruption::
    /// ZkProofVerifier`/`pallet_identity_zk::ZkProofVerifier` — implemented in the runtime by
    /// delegating to the same ZKPassport outer-circuit verifier those pallets use.
    pub trait ZkProofVerifier {
        fn verify(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> bool;
    }

    // ── ZKPassport public-input layout (case-filing anonymization) ─────────────
    //
    // Mirrors `pallet_anticorruption`'s identical section verbatim — see that pallet's module
    // doc comment / "ZKPassport public-input layout" section for the authoritative field-by-
    // field table (sourced from `runtime/src/verifier.rs`). Duplicated locally (rather than
    // imported from that pallet) the same way `legislature_call_hash`-style small helpers are
    // duplicated per-pallet elsewhere in this codebase, to avoid a needless crate dependency
    // just for a handful of consts and a const fn.

    /// Minimum length a structurally valid `public_inputs` array can have (the `count_4`
    /// variant, `D = 1`). See `pallet_anticorruption::MIN_PUBLIC_INPUTS`'s doc comment for the
    /// full "checked before indexing / before the verifier runs" rationale.
    const MIN_PUBLIC_INPUTS: usize = 9;
    /// Index of `service_scope` in `public_inputs` — fixed regardless of disclosure count.
    const SERVICE_SCOPE_INDEX: usize = 3;
    /// Index of `service_subscope` in `public_inputs` — fixed regardless of disclosure count.
    const SERVICE_SUBSCOPE_INDEX: usize = 4;

    /// Zero-pads a 31-byte ASCII tag into a canonical 32-byte big-endian BN254 `Fr` element.
    /// See `pallet_anticorruption::zero_padded_scope_tag`'s doc comment for why this
    /// construction is canonical by inspection.
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
    /// (`public_inputs[SERVICE_SCOPE_INDEX]`) for anonymized case filing (`file_case`'s
    /// `LawChallenge`/`TreasuryDispute`/`TierConflict` branch, and `file_case_for`). Stops a
    /// proof generated for a different purpose (citizen registration, whistleblower reports,
    /// etc.) from being replayed here even though the underlying passport proof is valid
    /// citizenship — see `pallet_anticorruption::WHISTLEBLOWER_REPORT_SERVICE_SCOPE`'s doc
    /// comment for the general rationale.
    pub const CASE_FILING_SERVICE_SCOPE: [u8; 32] =
        zero_padded_scope_tag(b"AGORA_COURTS_CASE_FILING_SCOPE1");

    /// Domain-separation constant this pallet requires as the proof's `service_subscope`
    /// (`public_inputs[SERVICE_SUBSCOPE_INDEX]`). See [`CASE_FILING_SERVICE_SCOPE`] above.
    pub const CASE_FILING_SERVICE_SUBSCOPE: [u8; 32] =
        zero_padded_scope_tag(b"AGORA_COURTS_CASE_FILING_SUBSC1");

    /// Implemented by the runtime to call pallet-identity's suspend_citizen_internal.
    /// `suspension_until` is an **absolute block number** when the suspension lifts
    /// (None = indefinite). The courts pallet computes this from `suspension_blocks` +
    /// `now` before calling this trait — the implementor just passes it through.
    /// `jury_reviewed` is true only when this case was decided by `cast_jury_vote`'s majority
    /// (i.e. reached `CaseStatus::JurySeated`), false when it was finalized via `finalize_ruling`
    /// on an unappealed `AIRulingIssued` case — see `auto_finalize`'s doc comment for how this
    /// is computed, and pallet-identity's `SuspendedByJuryReview` for what it's used for.
    pub trait CitizenSuspender<BlockNumber> {
        fn suspend_citizen(
            nullifier: [u8; 32],
            suspension_until: Option<BlockNumber>,
            jury_reviewed: bool,
        ) -> DispatchResult;
    }

    // ── Oracle origin ────────────────────────────────────────────────────────────

    /// Accepts a `Signed` origin only if the signer is a current member of `OracleMembers`.
    /// Returns `Err(origin)` if the account isn't (or no longer is) a member.
    ///
    /// This is the M-of-N Oracle Council replacement for the earlier single-`OracleAccount`
    /// design: previously this origin accepted exactly one designated account, so a single
    /// compromised or lost oracle signing key fully controlled Level-0 AI court rulings and
    /// their finalization for the entire court system, with no secondary approval. `EnsureOracle`
    /// itself now only answers "is this signer allowed to participate in oracle actions at
    /// all" — it does *not* by itself authorize a ruling to take effect. The actual M-of-N
    /// gate lives in `submit_ai_ruling`/`approve_ai_ruling`/`finalize_ruling`: any single
    /// member can *propose* a ruling submission or finalization, but it only takes effect once
    /// `OracleApprovalNumerator`/`Denominator` of the council has approved the same pending
    /// action (see `PendingOracleProposal`, `OracleApprovals`, `try_resolve_oracle_action`).
    /// `Success = T::AccountId` (not `()`) because the M-of-N flow needs to know *which*
    /// member is acting, to record their approval and reject a repeat approval from the same
    /// member. Governance manages membership via `add_oracle_member`/`remove_oracle_member`
    /// (root-gated, same as the `set_oracle_account` this replaces) without a runtime upgrade.
    pub struct EnsureOracle<T>(core::marker::PhantomData<T>);

    impl<T: Config> frame_support::traits::EnsureOrigin<T::RuntimeOrigin> for EnsureOracle<T> {
        type Success = T::AccountId;

        fn try_origin(o: T::RuntimeOrigin) -> Result<Self::Success, T::RuntimeOrigin> {
            use frame_system::RawOrigin;
            match o.clone().into() {
                Ok(RawOrigin::Signed(who)) if OracleMembers::<T>::get().contains(&who) => Ok(who),
                _ => Err(o),
            }
        }

        // NOTE: this cfg can never currently activate. `pallet-courts/Cargo.toml` declares no
        // `runtime-benchmarks` feature of its own, and `runtime/Cargo.toml`'s
        // `runtime-benchmarks` feature list doesn't include `pallet-courts/runtime-benchmarks`
        // either — so this is dead code, not a working benchmark hook. This pallet also has no
        // `benchmarking.rs` (`#[benchmarks]` module) yet. See the note in
        // `pallets/pallet-legislature/src/weights.rs` for the full accounting; making this
        // real requires adding the feature to this crate's Cargo.toml, wiring it into
        // `runtime/Cargo.toml`, and writing an actual `#[benchmarks]` module.
        #[cfg(feature = "runtime-benchmarks")]
        fn try_successful_origin() -> Result<T::RuntimeOrigin, ()> {
            let member = OracleMembers::<T>::get().first().cloned().ok_or(())?;
            Ok(frame_system::RawOrigin::Signed(member).into())
        }
    }

    /// `EnsureOriginWithArg<RuntimeOrigin, [u8; 32]>` that succeeds only when
    /// `propose_admin_action`/`approve_admin_action` have already driven the given call hash's
    /// approvals past the Oracle Council's M-of-N threshold (see `ApprovedAdminAction`). This is
    /// the fix for the gap where `pallet_constitution::invalidate_law` and
    /// `pallet_identity_zk::suspend_citizen`/`restore_citizen_rights` were gated only by the bare
    /// `EnsureOracle` membership check — any single Oracle Council member could call those
    /// "manual override" extrinsics directly and take effect immediately, with none of the M-of-N
    /// approval `submit_ai_ruling`/`approve_ai_ruling`/`finalize_ruling` require. Wire this as
    /// `CourtOrigin`/`SuspensionOrigin` in the runtime instead of `EnsureOracle<Runtime>` directly.
    ///
    /// Mirrors `pallet_legislature::EnsureLegislatureMotion`'s `call_hash`-binding design: the
    /// consuming pallet computes a domain-separated hash of its own call's parameters (so
    /// byte-identical parameters passed to a different call can never collide) and only that
    /// exact hash's approved token authorizes the origin — a token approved for one call can
    /// never be replayed against a different one.
    ///
    /// Any *current* Oracle Council member may consume an approved token, not only the original
    /// `propose_admin_action` caller — mirrors `df8fff7`'s identical fix in
    /// `pallet_legislature::EnsureLegislatureMotion`, ported here after the same permanent-
    /// deadlock risk was found unfixed in this pallet: the approval vote is what legitimizes the
    /// action, not the proposer's continued availability, and requiring the exact proposer meant
    /// a proposer who went offline, lost their key, or was removed via `remove_oracle_member`
    /// before consuming it would permanently block every future admin action gated on this
    /// origin (`propose_admin_action` refuses to re-propose an already-approved `call_hash`, see
    /// `Error::OracleActionAlreadyProposed`). `ApprovedAdminAction` now also stores the block the
    /// token was approved at; `clear_stale_admin_action` lets any current council member discard
    /// it once `AdminActionExpiryBlocks` have passed unconsumed, unblocking a fresh proposal for
    /// the same `call_hash`.
    pub struct EnsureOracleCouncilApproved<T>(core::marker::PhantomData<T>);

    impl<T: Config> frame_support::traits::EnsureOriginWithArg<T::RuntimeOrigin, [u8; 32]>
        for EnsureOracleCouncilApproved<T>
    {
        type Success = ();

        fn try_origin(
            o: T::RuntimeOrigin,
            call_hash: &[u8; 32],
        ) -> Result<Self::Success, T::RuntimeOrigin> {
            use frame_system::RawOrigin;
            match o.clone().into() {
                Ok(RawOrigin::Signed(who)) if OracleMembers::<T>::get().contains(&who) => {
                    if ApprovedAdminAction::<T>::get(call_hash).is_some() {
                        ApprovedAdminAction::<T>::remove(call_hash);
                        Ok(())
                    } else {
                        Err(o)
                    }
                }
                _ => Err(o),
            }
        }

        #[cfg(feature = "runtime-benchmarks")]
        fn try_successful_origin(call_hash: &[u8; 32]) -> Result<T::RuntimeOrigin, ()> {
            let member = OracleMembers::<T>::get().first().cloned().ok_or(())?;
            ApprovedAdminAction::<T>::insert(
                *call_hash,
                (member.clone(), frame_system::Pallet::<T>::block_number()),
            );
            Ok(frame_system::RawOrigin::Signed(member).into())
        }
    }

    // ── Enums ───────────────────────────────────────────────────────────────────

    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum CaseStatus {
        Filed,
        AIRulingIssued,
        InJuryAppeal,
        /// Jury selected and seated; votes are being collected.
        JurySeated,
        FinalRuling,
        Enforced,
    }

    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum Verdict {
        Upheld,
        Overturned,
    }

    /// What the case is about — drives auto-enforcement on ruling.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum CaseSubject {
        /// General dispute with no automatic on-chain enforcement.
        General,
        /// Challenges a specific law; Overturned ruling pauses that law.
        LawChallenge { law_id: u32 },
        /// Alleges illegal treasury activity; Overturned ruling freezes the department.
        TreasuryDispute { department_id: u32 },
        /// Criminal/conduct case against a citizen identified by their nullifier.
        /// Overturned verdict (i.e. guilty) suspends them; suspension_blocks = None means indefinite.
        /// suspension_blocks is a DURATION in blocks; auto_finalize converts to an absolute block.
        CitizenConduct { nullifier: [u8; 32], suspension_blocks: Option<u32> },
        /// Alleges `law_id` was enacted at the wrong `LawTier` (e.g. a bare Ordinary-majority
        /// vote enacting constitutionally-weighted law while declaring it Ordinary, skipping
        /// both the entrenchment pipeline and the mandatory auto-challenge review that only
        /// fires for Structural/Foundational tiers — see `pallet_constitution::challenge_law_tier`,
        /// which is the only intended way to open one of these). Overturned ruling applies the
        /// same remedy `LawChallenge` does — `T::LawEnforcer::invalidate_law` — deliberately not
        /// a "re-tier and re-enter entrenchment" flow: if the legislature wants the law back,
        /// they re-enact it, and it draws scrutiny at the tier it actually deserves that time.
        TierConflict { law_id: u32 },
    }

    /// A fieldless twin of the law-targeting `CaseSubject` variants, used only as the recorded
    /// value in `OpenCaseByLaw` — never as part of its key. `CaseSubject` itself isn't reused
    /// here because it carries payload (`law_id`, `department_id`, `nullifier`+
    /// `suspension_blocks`) that has no business inside a dedup-guard's stored value.
    #[derive(Clone, Copy, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum CaseSubjectKind {
        LawChallenge,
        TierConflict,
    }

    /// Identifies who filed a case (`Cases`' first tuple element). For case types that are
    /// "citizen vs. institutional power" — `LawChallenge`, `TreasuryDispute`, `TierConflict` —
    /// this is `Nullifier`, the filer's ZKPassport outer-proof `scoped_nullifier`, never an
    /// `AccountId`: the filer risks retaliation from the very power they're challenging, so
    /// their identity is not permanently, publicly bound to the case the way a plain signed
    /// extrinsic would (see `file_case`'s doc comment for the full writeup, including the
    /// residual submission-metadata gap this does *not* close). For citizen-vs-citizen/general
    /// cases — `CitizenConduct`, `General` — this remains `Account`, unchanged from before:
    /// the accused has a legitimate interest in knowing their accuser.
    ///
    /// A case's bond is *always* reserved from (and released back to) the real signing account
    /// regardless of which variant is stored here — see `CaseBondAccount`, which is deliberately
    /// decoupled from this field for exactly that reason.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum CaseFiler<AccountId> {
        Account(AccountId),
        Nullifier([u8; 32]),
    }

    /// What a pending Oracle Council proposal for a case will do once approvals reach the
    /// configured M-of-N threshold — either commit a fresh Level-0 AI ruling (mirrors the old
    /// single-signer `submit_ai_ruling`'s effect) or apply the verdict already committed in
    /// `AIRulingVerdict` (mirrors the old single-signer `finalize_ruling`'s effect). A case can
    /// have at most one pending oracle action at a time: a `Submission` only makes sense while
    /// status is `Filed`, a `Finalization` only while status is `AIRulingIssued` past the
    /// appeal window — so both variants share one map keyed by `case_id`
    /// (`PendingOracleProposal`) rather than needing two separate maps.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum PendingOracleAction {
        /// Committing a fresh Level-0 AI ruling for a `Filed` case — the frozen arguments
        /// `submit_ai_ruling`'s proposer originally supplied.
        Submission { ruling_hash: [u8; 32], model_version: u32, verdict: Verdict },
        /// Applying the verdict already bound in `AIRulingVerdict` for an unappealed
        /// `AIRulingIssued` case whose appeal window has closed.
        Finalization,
    }

    /// A governance-approved AI model version, referenced by `submit_ai_ruling`'s
    /// `model_version` parameter (see `CurrentAIModelVersion` / `AIModelVersions`).
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct ModelInfo<BlockNumber> {
        /// Hash identifying the approved model — e.g. a hash of the model card, weights
        /// manifest, or a version string. This pallet is agnostic to what's hashed; it only
        /// needs a stable, compact, on-chain-comparable identifier for the approved model.
        pub model_hash: [u8; 32],
        /// Block at which this version was approved (i.e. reached supermajority).
        pub approved_at: BlockNumber,
    }

    /// Maximum number of pending oracle-case actions — and, separately, up to that many
    /// pending admin actions — that `remove_oracle_member` will attempt to re-resolve
    /// (`try_resolve_oracle_action`/`try_resolve_admin_action`) in a single call, after purging
    /// the removed member's approvals. See `remove_oracle_member`'s doc comment for the fuller
    /// weight-pricing rationale; in short: the purge itself (walking every
    /// `OracleApprovals`/`PendingAdminAction` entry to strip the removed member's vote) must
    /// stay unbounded regardless of this cap — skipping an entry there would leave a removed
    /// (e.g. compromised) member's vote silently still counting toward some pending action's
    /// threshold forever, defeating the whole point of removal — but that purge is cheap,
    /// bounded per-item work (`BoundedVec` position/remove, capped at `MaxOracleMembers`
    /// entries). What genuinely needs bounding is the optional *re-resolution* step:
    /// `try_resolve_oracle_action` can trigger real cross-pallet enforcement
    /// (`T::LawEnforcer`/`T::TreasuryEnforcer`/`T::CitizenSuspender` via `auto_finalize`), whose
    /// cost this pallet cannot know statically, so leaving it fully unbounded would make this
    /// call's real weight scale with however many cases happen to be pending removal-time — an
    /// admin action a citizen's transaction never even sees, but one that still occupies a
    /// block's weight budget. Any action left un-resolved past this cap simply stays pending
    /// exactly as it was before this fix (already purged of the removed member's vote) and
    /// resolves itself the moment any other Oracle Council member's next
    /// `approve_ai_ruling`/`approve_admin_action`/`submit_ai_ruling`/`finalize_ruling`/
    /// `propose_admin_action` call touches it — not stuck, just not resolved *within this
    /// specific call*.
    const MAX_ORACLE_ACTIONS_RERESOLVED_PER_REMOVAL: usize = 20;

    // ── Config ──────────────────────────────────────────────────────────────────

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Blocks available to appeal an AI ruling (Level 0 -> Level 1).
        #[pallet::constant]
        type AppealWindowBlocks: Get<u32>;
        /// Source of citizen accounts for jury selection.
        type CitizenSelector: CitizenSelector<Self::AccountId>;
        /// Gate: checks whether an account is an active (non-suspended) citizen.
        /// Used to require active citizenship before filing a case.
        type CitizenChecker: CitizenChecker<Self::AccountId>;
        /// Hook called to pause a law when an Overturned verdict is issued.
        type LawEnforcer: LawEnforcer;
        /// Hook called to freeze a department when an Overturned treasury verdict is issued.
        type TreasuryEnforcer: TreasuryEnforcer;
        /// The origin permitted to propose or approve AI ruling submission/finalization — any
        /// Oracle Council member (`OracleMembers`). `Success = Self::AccountId` (not `()`)
        /// because the M-of-N flow needs to know *which* member is acting: it's recorded in
        /// `OracleApprovals` and checked to reject a repeat approval from the same member.
        /// Configure as `EnsureOracle<Runtime>` in production; a mock can use anything that
        /// yields a distinct `AccountId` per caller.
        type OracleOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin, Success = Self::AccountId>;
        /// Maximum size of the Oracle Council — the M-of-N body that jointly authorizes Level-0
        /// AI ruling submission and finalization (see `OracleMembers`, `submit_ai_ruling`,
        /// `approve_ai_ruling`, `finalize_ruling`). Replaces the earlier single `OracleAccount`
        /// design, which a project review flagged as a single point of failure: one compromised
        /// or lost oracle key fully controlled every Level-0 ruling and its finalization, with
        /// no secondary approval. Sized at 7 in the runtime — the same as a Level-1 appeal jury
        /// (`select_jury`'s non-`LawChallenge` branch); small enough that each member is a real,
        /// individually-run signing key/Claude-API instance to be tractable (see
        /// `court-oracle/README.md`'s multi-instance deployment note), large enough that losing
        /// any one member's key doesn't stall the court system.
        #[pallet::constant]
        type MaxOracleMembers: Get<u32>;
        /// Numerator of the fraction of Oracle Council members whose approval a pending action
        /// (`PendingOracleProposal`) needs before it takes effect. The check is
        /// `approvals * OracleApprovalDenominator > council_size * OracleApprovalNumerator` —
        /// **strict** inequality, because "majority" means *more than* half, not "at least
        /// half" (contrast this pallet's own `ai_model_supermajority_reached` and
        /// pallet-emergency-council's `supermajority_reached`, which both use `>=` because they
        /// gate genuine supermajorities, e.g. 2/3, rather than a plain majority). Set to 1/2 in
        /// the runtime — a simple majority is what closes the reviewed single-point-of-failure
        /// gap; nothing here stops a future change to a higher bar (e.g. 2/3) by adjusting
        /// these two constants, the same way `AIModelSupermajorityNumerator`/`Denominator` are
        /// independently configurable for the separate AI-model-approval council below.
        #[pallet::constant]
        type OracleApprovalNumerator: Get<u32>;
        /// Denominator of the oracle-approval fraction (see `OracleApprovalNumerator`).
        #[pallet::constant]
        type OracleApprovalDenominator: Get<u32>;
        /// How many blocks an `ApprovedAdminAction` token may sit unconsumed before any current
        /// Oracle Council member can discard it via `clear_stale_admin_action`. Mirrors
        /// `pallet_legislature::Config::PendingApprovalExpiryBlocks` — see
        /// `EnsureOracleCouncilApproved`'s doc comment for the deadlock this closes.
        #[pallet::constant]
        type AdminActionExpiryBlocks: Get<u32>;
        /// How many blocks a `PendingOracleProposal` may sit without reaching its
        /// `OracleApprovalNumerator`/`Denominator` threshold before any current Oracle Council
        /// member can discard it via `clear_stale_oracle_proposal`. The case-based counterpart
        /// of `AdminActionExpiryBlocks`: unlike an admin action (which has a distinct
        /// post-threshold "approved but unconsumed" state, `ApprovedAdminAction`, that's what
        /// actually goes stale), a case-based action applies itself the instant it crosses
        /// threshold (see `try_resolve_oracle_action`) — so what can go stale here is the
        /// *pre-threshold* proposal itself, if Oracle Council members never cast enough
        /// approvals (e.g. because they're offline). Without this, a case whose proposer's
        /// lone approval never reaches quorum is stuck forever: `submit_ai_ruling`/
        /// `finalize_ruling` both refuse to re-propose while a `PendingOracleProposal` already
        /// exists for that `case_id` (`Error::OracleActionAlreadyProposed`), and there is no
        /// other way to withdraw one. See `clear_stale_oracle_proposal`.
        #[pallet::constant]
        type OracleProposalExpiryBlocks: Get<u32>;
        /// How many blocks a jury may sit in `CaseStatus::JurySeated` without a strict majority
        /// ever being reached (no-shows, or votes split in a way that never crosses the
        /// threshold for either side) before `clear_stale_jury_deadlock` may discard it and
        /// re-schedule a fresh jury selection. The jury-voting counterpart of
        /// `OracleProposalExpiryBlocks` — mirrors its naming/wiring pattern exactly, just keyed
        /// off `JurySeatedBlock` instead of `OracleProposalProposedAt`. Without this, a
        /// deadlocked jury has no deadline, no re-selection mechanism, and no admin override:
        /// `cast_jury_vote` only ever moves the case forward on a majority, and nothing else
        /// touches `CaseStatus::JurySeated`. See `clear_stale_jury_deadlock`.
        #[pallet::constant]
        type JuryVotingExpiryBlocks: Get<u32>;
        /// Hook called to suspend a citizen when a CitizenConduct verdict is Overturned (guilty).
        type CitizenSuspender: CitizenSuspender<BlockNumberFor<Self>>;
        /// Number of blocks, starting the block *after* a case enters jury appeal, whose
        /// hashes are mixed into the jury-selection seed. Jury selection is blocked until
        /// this whole window has elapsed.
        ///
        /// This is a commit-then-delayed-reveal scheme: `appeal_ruling` is the implicit
        /// "commit" (it timestamps the case via `JuryRequestBlock`), and the "reveal" is
        /// the fixed window of `JurySeedDelayBlocks` blocks immediately following it. None
        /// of those blocks exist yet — and their hashes are therefore unknowable to anyone,
        /// including the appellant, the oracle, or whoever ends up calling `select_jury` —
        /// at the moment the appeal is filed.
        ///
        /// This closes the dominant, cheap attack in a naive "mix the last N blocks as of
        /// call time" scheme (what this pallet used before): since that scheme's output is
        /// fully computable from already-mined history, *any* authorized caller could grind
        /// for a favorable jury simply by delaying submission of `select_jury` block by
        /// block until the (already-known) result looked good, with no need to author blocks
        /// or hold any special role.
        ///
        /// It does **not** eliminate all manipulation risk. A validator who happens to be
        /// scheduled (Aura round-robin, publicly known in advance) to author one of the
        /// blocks inside the seed window can still nudge that block's hash — by choosing
        /// which transactions to include and in what order — within the bounded space of
        /// valid blocks they could produce, and someone author-ing the *last* block in the
        /// window has a slight edge since they see the accumulated entropy from the earlier
        /// blocks before finalizing their own. This is the same residual "last revealer"
        /// class of risk inherent to RANDAO-style schemes generally, and is materially
        /// narrower than the pre-existing hole (requires being a scheduled block author,
        /// not just any authorized caller). Closing it fully requires either genuine
        /// multi-party commit-reveal or consensus-native VRF (BABE/SASSAFRAS) — neither is
        /// implemented here; see HANDOFF.md item 7.
        #[pallet::constant]
        type JurySeedDelayBlocks: Get<u32>;
        /// Maximum number of cases whose jury-seed capture can be scheduled for the same
        /// block (i.e. whose `JurySeedDelayBlocks` window closes at the same block number).
        /// Bounds `on_initialize`'s worst-case per-block weight for `SeedCaptureDue`. In
        /// practice this only needs to be as large as "how many `appeal_ruling` calls could
        /// plausibly land in the same block."
        #[pallet::constant]
        type MaxCasesPerBlock: Get<u32>;
        /// AccountId used as the filer for system-initiated cases (e.g. auto law challenges).
        /// Wire to a well-known zero account or a dedicated pallet account in the runtime.
        type AutoChallengeAccount: Get<Self::AccountId>;
        /// Currency used to reserve the citizen-filed case bond (`CaseFilingBond`).
        type Currency: ReservableCurrency<Self::AccountId>;
        /// Bond reserved from a citizen's account when they call `file_case`, released in
        /// full once the case reaches a final status (see `auto_finalize`). A plain
        /// spam-prevention deposit: instant, free Level-0 AI rulings make `file_case` an
        /// attractive DoS/spam vector without a cost to filing, and this bond gives it one.
        /// Defaults to 1 AGR, the same size as this runtime's other single-AGR bonds/deposits,
        /// absent any documented reason this pallet should differ. (Formerly described as
        /// matching pallet-elections' `CandidateDeposit`; that deposit no longer exists —
        /// pallet-elections' Elections Commission subsystem was removed, see
        /// `docs/project/pallets/elections.md`.) System-initiated filings via `auto_file_case`
        /// never reserve this bond — see that function's doc comment for why.
        #[pallet::constant]
        type CaseFilingBond: Get<BalanceOf<Self>>;

        // ── AI model governance (supermajority vote) ────────────────────────────
        /// Maximum size of the AI Model Governance Council (see `AIGovernanceCouncil`).
        #[pallet::constant]
        type MaxAIGovernanceCouncilSize: Get<u32>;
        /// Numerator of the supermajority fraction required to approve a new AI model
        /// version via `vote_approve_ai_model` (e.g. 2 for 2/3). Mirrors
        /// pallet-emergency-council's identical `SupermajorityNumerator`/`Denominator`
        /// pair — see the doc comment on `vote_approve_ai_model` for why this pallet
        /// reimplements that pattern locally instead of depending on
        /// pallet-emergency-council or pallet-legislature directly.
        #[pallet::constant]
        type AIModelSupermajorityNumerator: Get<u32>;
        /// Denominator of the supermajority fraction (e.g. 3 for 2/3).
        #[pallet::constant]
        type AIModelSupermajorityDenominator: Get<u32>;

        /// Verifier for the ZK citizenship proof `file_case`/`file_case_for` require when
        /// filing an anonymized case (`LawChallenge`/`TreasuryDispute`/`TierConflict`) — see
        /// `CaseFiler` and `ZkProofVerifier`.
        type ZkVerifier: ZkProofVerifier;
    }

    // ── Storage ─────────────────────────────────────────────────────────────────

    /// (filer, status, ruling_ipfs_hash, subject) — the shape of a `Cases` entry, named so
    /// `is_filer_or_oracle`/`is_ruled_against_party` don't need to repeat the tuple type.
    /// `filer` is `CaseFiler<AccountId>`, not a bare `AccountId` — see that type's doc comment.
    pub type CaseOf<T> =
        (CaseFiler<<T as frame_system::Config>::AccountId>, CaseStatus, Option<[u8; 32]>, CaseSubject);

    /// case_id -> (filer, status, ruling_ipfs_hash, subject).
    #[pallet::storage]
    pub type Cases<T: Config> = StorageMap<_, Blake2_128Concat, u32, CaseOf<T>>;

    /// case_id -> verdict (set after jury or AI ruling is final).
    #[pallet::storage]
    pub type Rulings<T: Config> = StorageMap<_, Blake2_128Concat, u32, Verdict>;

    /// case_id -> list of selected juror AccountIds (max 21 for Level 2).
    #[pallet::storage]
    pub type JuryPool<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, BoundedVec<T::AccountId, ConstU32<21>>>;

    #[pallet::storage]
    pub type NextCaseId<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Block number when the AI ruling was issued. Used to enforce the appeal window.
    #[pallet::storage]
    pub type AIRulingBlock<T: Config> = StorageMap<_, Blake2_128Concat, u32, BlockNumberFor<T>>;

    /// Block number when the case entered jury appeal (set by `appeal_ruling`). This is the
    /// "commit" point for the delayed-reveal jury seed: `select_jury` may only be called once
    /// `JurySeedDelayBlocks` blocks have elapsed after this point, and the seed is derived
    /// solely from the hashes of blocks in that window — see `JurySeedDelayBlocks` doc comment.
    #[pallet::storage]
    pub type JuryRequestBlock<T: Config> = StorageMap<_, Blake2_128Concat, u32, BlockNumberFor<T>>;

    /// Each juror's vote for a case. Only accounts in JuryPool[case_id] may vote.
    #[pallet::storage]
    pub type JuryVotes<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, T::AccountId), Verdict>;

    /// Running tally: case_id -> (upheld_count, overturned_count).
    #[pallet::storage]
    pub type JuryTally<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, (u32, u32), ValueQuery>;

    /// Block at which `select_jury` most recently seated a jury for a case (i.e. the block
    /// `CaseStatus` moved to `JurySeated`). The jury-voting counterpart of
    /// `OracleProposalProposedAt`: used by `clear_stale_jury_deadlock` to detect a jury that has
    /// sat in `JurySeated` for `JuryVotingExpiryBlocks` without a strict majority ever being
    /// reached — no-shows, or votes split in a way that can never cross the threshold for either
    /// side (see `cast_jury_vote`'s majority check). Without this, such a case has no deadline,
    /// no re-selection path, and no admin override — it sits in `JurySeated` forever. Cleared
    /// whenever the case leaves `JurySeated`, whether by `auto_finalize` reaching a majority or
    /// by `clear_stale_jury_deadlock` re-scheduling a fresh jury, and re-populated the next time
    /// `select_jury` seats a jury for the case.
    #[pallet::storage]
    pub type JurySeatedBlock<T: Config> = StorageMap<_, Blake2_128Concat, u32, BlockNumberFor<T>>;

    /// The Oracle Council — the M-of-N body whose members may propose/approve AI ruling
    /// submission (`submit_ai_ruling`) and finalization (`finalize_ruling`). Replaces the
    /// earlier single `OracleAccount`. Managed by root via `add_oracle_member`/
    /// `remove_oracle_member`, mirroring `AIGovernanceCouncil`'s management calls in this same
    /// pallet (see those calls' doc comments).
    #[pallet::storage]
    pub type OracleMembers<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxOracleMembers>, ValueQuery>;

    /// case_id -> the oracle action awaiting `OracleApprovalNumerator`/`Denominator` approval,
    /// if any. Cleared (alongside `OracleApprovals`) the moment the action resolves — either by
    /// taking effect once the threshold is reached, or by being dropped as stale if the case's
    /// status no longer matches what the action expects (e.g. a `Finalization` proposal whose
    /// case got appealed by someone else in the interim) — see `try_resolve_oracle_action`.
    #[pallet::storage]
    pub type PendingOracleProposal<T: Config> = StorageMap<_, Blake2_128Concat, u32, PendingOracleAction>;

    /// case_id -> Oracle Council members who have approved the current `PendingOracleProposal`
    /// for that case. Single-use per member per action: `approve_ai_ruling` rejects a repeat
    /// approval from the same member (`Error::AlreadyApprovedOracleAction`), and both this map
    /// and `PendingOracleProposal` are cleared together once the action resolves, so a member's
    /// past approval never silently counts toward a *later*, different proposal for the same
    /// case_id.
    #[pallet::storage]
    pub type OracleApprovals<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, BoundedVec<T::AccountId, T::MaxOracleMembers>>;

    /// case_id -> the block `PendingOracleProposal` was first inserted for it (by
    /// `submit_ai_ruling` or `finalize_ruling`). Used by `clear_stale_oracle_proposal` to
    /// discard a proposal that has sat for `OracleProposalExpiryBlocks` without reaching the
    /// approval threshold — the case-based counterpart of `ApprovedAdminAction`'s stored
    /// approval block (see `clear_stale_admin_action`). Cleared alongside
    /// `PendingOracleProposal`/`OracleApprovals` whenever the action resolves, expires, or is
    /// cleared, so a later, unrelated proposal for the same `case_id` never inherits a stale
    /// timestamp.
    #[pallet::storage]
    pub type OracleProposalProposedAt<T: Config> = StorageMap<_, Blake2_128Concat, u32, BlockNumberFor<T>>;

    /// call_hash -> (proposer, Oracle Council members who have approved) for an in-flight
    /// "administrative action" proposal — the case-less counterpart of `PendingOracleProposal`/
    /// `OracleApprovals`, used to gate manual-override extrinsics in other pallets (currently
    /// `pallet_constitution::invalidate_law` and `pallet_identity_zk::suspend_citizen`/
    /// `restore_citizen_rights`) behind the same M-of-N threshold instead of the bare
    /// single-member `EnsureOracle` check those extrinsics used before. `call_hash` is the
    /// domain-separated hash of the exact call being authorized (mirrors
    /// `pallet_legislature::EnsureLegislatureMotion`'s `call_hash`-binding design), computed by
    /// the consuming pallet the same way it computes a `LegislatureOrigin` call hash. See
    /// `propose_admin_action`/`approve_admin_action` and `EnsureOracleCouncilApproved`.
    #[pallet::storage]
    pub type PendingAdminAction<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        [u8; 32],
        (T::AccountId, BoundedVec<T::AccountId, T::MaxOracleMembers>),
    >;

    /// call_hash -> (proposer, block approved) once `PendingAdminAction`'s approvals reached the
    /// M-of-N threshold. Any *current* Oracle Council member may consume it (via
    /// `EnsureOracleCouncilApproved`) to actually execute the authorized call — not only the
    /// original proposer; see `EnsureOracleCouncilApproved`'s doc comment for why (ports
    /// `pallet_legislature`'s `df8fff7` deadlock fix, which restricting consumption to the
    /// proposer alone would otherwise reintroduce here). `proposer` is retained only as an audit
    /// trail, not for authorization. The approval block is used by `clear_stale_admin_action` to
    /// discard a token nobody ever consumes once `AdminActionExpiryBlocks` elapses. Removed the
    /// moment it is consumed (or expired); each resolved proposal authorizes exactly one call.
    #[pallet::storage]
    pub type ApprovedAdminAction<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], (T::AccountId, BlockNumberFor<T>)>;

    /// case_id -> bond reserved from the filer by `file_case`. Only ever populated for
    /// citizen-filed cases — `auto_file_case` (system-initiated) never inserts an entry here,
    /// since there's no spam risk to price against for a filing the runtime itself triggers.
    /// Taken (removed) and unreserved in full by `auto_finalize` once the case resolves.
    #[pallet::storage]
    pub type CaseBonds<T: Config> = StorageMap<_, Blake2_128Concat, u32, BalanceOf<T>>;

    /// law_id -> (the kind of case currently open against it, its case_id). At most one
    /// law-targeting case (`LawChallenge` or `TierConflict`) may be open per law_id at a time,
    /// regardless of kind. Keying on law_id ALONE (not `(CaseSubjectKind, law_id)`) is what
    /// makes a `LawChallenge` and a `TierConflict` against the same law mutually exclusive with
    /// each other, not just each with itself — replaces the earlier
    /// `OpenLawChallengeCase`/`OpenTierConflictCase` maps, which only guarded within one
    /// case-subject type and let the two types coexist, reintroducing the revert-and-strand bug
    /// they were built to prevent.
    ///
    /// Enforced by `do_file_case` and `auto_file_case` (`Error::DuplicateLawChallenge`/
    /// `Error::DuplicateTierConflict` — selected by the *incoming* subject's own kind — rejects
    /// a second filing while an entry exists) and cleared by `auto_finalize` once that case
    /// reaches `FinalRuling`.
    ///
    /// Exists to close a griefing/stranding bug: without this, two law-targeting cases (of
    /// either kind, in any combination) could be pending against the same `law_id`
    /// simultaneously. If the first resolves Overturned, `auto_finalize` calls
    /// `T::LawEnforcer::invalidate_law`, which pauses the law (moves it out of `Active`). When
    /// the second case later also resolves, its own `T::LawEnforcer::invalidate_law` call fails
    /// because the law is no longer `Active` — and because `auto_finalize` runs inside the same
    /// transactional extrinsic as its caller (`cast_jury_vote` or `finalize_ruling`), that whole
    /// call reverts, permanently stranding the second case (and its filing bond, for
    /// citizen-filed cases) in a state nothing can ever move forward. A side index keyed by
    /// `law_id` -> open case lets both filing calls reject a duplicate in O(1), rather than an
    /// unbounded scan of `Cases` for an existing pending challenge against the same law.
    #[pallet::storage]
    pub type OpenCaseByLaw<T: Config> = StorageMap<_, Blake2_128Concat, u32, (CaseSubjectKind, u32)>;

    /// case_id -> the account that actually reserved `CaseBonds` via `T::Currency::reserve`,
    /// for `T::Currency::unreserve` at `auto_finalize` time. Deliberately decoupled from
    /// `Cases`' filer field (`CaseFiler`): for an anonymized case type
    /// (`LawChallenge`/`TreasuryDispute`/`TierConflict`) that field stores a ZK nullifier, not
    /// an `AccountId`, but the bond is always reserved from — and must be returned to — the
    /// real signing account regardless of case type. Populated (and later taken) alongside
    /// `CaseBonds`; absent exactly when `CaseBonds` is (never populated for `auto_file_case`'s
    /// system-initiated filings, which reserve no bond at all).
    #[pallet::storage]
    pub type CaseBondAccount<T: Config> = StorageMap<_, Blake2_128Concat, u32, T::AccountId>;
    // ── AI model governance (supermajority vote) ─────────────────────────────────

    /// Council authorized to vote on which AI model version `submit_ai_ruling` may cite.
    /// A small, dedicated, root-managed body — mirrors pallet-emergency-council's `Council`
    /// rather than reusing pallet-legislature's membership or an `EnsureOrigin` from either
    /// pallet (see the design note on `vote_approve_ai_model`).
    #[pallet::storage]
    pub type AIGovernanceCouncil<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxAIGovernanceCouncilSize>, ValueQuery>;

    /// Which council members have voted to approve the current `PendingAIModelProposal`.
    /// Reset once a proposal resolves (approved) — mirrors pallet-emergency-council's
    /// `DeclareVotes`.
    #[pallet::storage]
    pub type AIModelApprovalVotes<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, bool, ValueQuery>;

    /// The model_hash locked in by the first vote of the current round, if any. Mirrors
    /// pallet-emergency-council's `PendingEmergencyProposal`: the first voter's hash is
    /// authoritative for the round, so a later voter can't switch what's being approved.
    #[pallet::storage]
    pub type PendingAIModelProposal<T: Config> = StorageValue<_, [u8; 32], OptionQuery>;

    /// The currently approved AI model version. 0 means no model has ever been approved —
    /// `submit_ai_ruling` always rejects `model_version == 0` (see `Error::NoApprovedAIModel`).
    #[pallet::storage]
    pub type CurrentAIModelVersion<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Full history of approved model versions, keyed by version — kept rather than only
    /// storing the current one, because `submit_ai_ruling` permanently records which version
    /// produced each ruling (`AIRulingModelVersion`); without this map that record becomes
    /// unrecoverable the moment governance approves a newer version. Mirrors the shape of
    /// pallet-identity's `OprfCommitteeKeys` (versioned map) + `OprfSchemeVersion` (current
    /// pointer) pattern.
    #[pallet::storage]
    pub type AIModelVersions<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, ModelInfo<BlockNumberFor<T>>>;

    /// case_id -> the AI model version that produced its `submit_ai_ruling` call. Populated
    /// only for cases that actually went through a Level-0 AI ruling.
    #[pallet::storage]
    pub type AIRulingModelVersion<T: Config> = StorageMap<_, Blake2_128Concat, u32, u32>;

    /// case_id -> the verdict committed by `submit_ai_ruling` at Level-0 ruling time.
    ///
    /// This is what closes the "verdict never bound to the published ruling" hole:
    /// `finalize_ruling`'s no-appeal path applies exactly this value — it no longer takes a
    /// free-form `verdict` argument of its own — so a compromised oracle key can no longer
    /// publish reasoning saying one thing (`ruling_hash`) and finalize with the opposite
    /// verdict. The jury-appeal path (`cast_jury_vote`'s majority tally) does NOT read this
    /// map; its verdict is derived independently from real jury votes, which is a strictly
    /// stronger binding than anything recorded here.
    #[pallet::storage]
    pub type AIRulingVerdict<T: Config> = StorageMap<_, Blake2_128Concat, u32, Verdict>;

    /// Captured jury-selection entropy per case, written by `on_initialize` at the block the
    /// case's delayed-reveal seed window closes -- i.e. while the window's block hashes are
    /// still live in `frame_system::BlockHash`, before `BlockHashCount`-based pruning can erase
    /// them. Once captured, `select_jury` can be called arbitrarily far in the future (long
    /// after the live block hashes it was derived from have been pruned) and still get the
    /// same, real, unpredictable-at-commit-time seed. See `SeedCaptureDue` and
    /// `JurySeedDelayBlocks`.
    #[pallet::storage]
    pub type CapturedJurySeed<T: Config> = StorageMap<_, Blake2_128Concat, u32, [u8; 32]>;

    /// Schedule of pending seed captures: block number -> case_ids whose seed window closes
    /// (becomes fully elapsed) at that block. Populated by `appeal_ruling` at the same time it
    /// sets `JuryRequestBlock`, using the fixed `JurySeedDelayBlocks` window length to compute
    /// the exact future capture block. Consumed (and cleared) by `on_initialize` once that block
    /// is reached.
    #[pallet::storage]
    pub type SeedCaptureDue<T: Config> =
        StorageMap<_, Blake2_128Concat, BlockNumberFor<T>, BoundedVec<u32, T::MaxCasesPerBlock>>;

    // ── Hooks ───────────────────────────────────────────────────────────────────

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// Capture each due case's jury-selection entropy from still-live block hashes at
        /// exactly the block its seed window closes, before `frame_system`'s separate
        /// `BlockHashCount`-based pruning can erase them. See `SeedCaptureDue` /
        /// `CapturedJurySeed` and the `JurySeedDelayBlocks` doc comment for the full design
        /// this closes a hole in.
        fn on_initialize(now: BlockNumberFor<T>) -> Weight {
            let due = SeedCaptureDue::<T>::take(now).unwrap_or_default();
            if due.is_empty() {
                return T::DbWeight::get().reads(1);
            }
            let delay = T::JurySeedDelayBlocks::get();
            let mut weight = T::DbWeight::get().reads_writes(1, 1);
            for case_id in due.iter().copied() {
                weight = weight.saturating_add(T::DbWeight::get().reads(1));
                if let Some(request_block) = JuryRequestBlock::<T>::get(case_id) {
                    let window_start = request_block.saturating_add(BlockNumberFor::<T>::from(1u32));
                    let seed = Self::anchored_entropy(case_id, window_start, delay);
                    CapturedJurySeed::<T>::insert(case_id, seed);
                    weight = weight.saturating_add(T::DbWeight::get().writes(1));
                }
            }
            weight
        }
    }

    // ── Events ──────────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// `filer` is `CaseFiler::Nullifier` for `LawChallenge`/`TreasuryDispute`/`TierConflict`
        /// cases and `CaseFiler::Account` for `CitizenConduct`/`General` ones — see `CaseFiler`'s
        /// doc comment. The event never carries a real `AccountId` for the anonymized types.
        CaseFiled { case_id: u32, filer: CaseFiler<T::AccountId>, subject: CaseSubject },
        AIRulingIssued { case_id: u32, ruling_hash: [u8; 32], model_version: u32 },
        JurySelected { case_id: u32, jurors: BoundedVec<T::AccountId, ConstU32<21>> },
        AppealFiled { case_id: u32, appellant: T::AccountId },
        RulingFinalized { case_id: u32, verdict: Verdict },
        /// Emitted only when an Overturned verdict triggers actual on-chain enforcement
        /// (law paused, department frozen, or citizen suspended).
        RulingEnforced { case_id: u32 },
        JuryVoteCast { case_id: u32, juror: T::AccountId, verdict: Verdict },
        /// A new member was added to the Oracle Council.
        OracleMemberAdded { account: T::AccountId },
        /// A member was removed from the Oracle Council.
        OracleMemberRemoved { account: T::AccountId },
        /// An Oracle Council member proposed committing a fresh Level-0 AI ruling for a case
        /// (their own approval is recorded immediately, same as a legislature motion's proposer).
        AIRulingProposed { case_id: u32, proposer: T::AccountId },
        /// An Oracle Council member proposed finalizing an unappealed case.
        RulingFinalizationProposed { case_id: u32, proposer: T::AccountId },
        /// An Oracle Council member cast an approval on a case's pending oracle action.
        OracleApprovalCast { case_id: u32, member: T::AccountId },
        /// A pending oracle action reached its approval threshold but could not be applied
        /// because the case's status had moved on in the interim (e.g. a `Finalization`
        /// proposal whose case was appealed by someone else before the last approval landed).
        /// The stale proposal was dropped rather than applied or left dangling forever.
        OracleActionExpired { case_id: u32 },
        /// An Oracle Council member proposed a case-less administrative action (e.g. a manual
        /// `invalidate_law`/`suspend_citizen`/`restore_citizen_rights` override) identified by
        /// the domain-separated hash of the call it authorizes.
        AdminActionProposed { call_hash: [u8; 32], proposer: T::AccountId },
        /// An Oracle Council member cast an approval on a pending administrative action.
        AdminActionApprovalCast { call_hash: [u8; 32], member: T::AccountId },
        /// A pending administrative action reached its approval threshold; any current Oracle
        /// Council member may now consume it once via `EnsureOracleCouncilApproved`.
        AdminActionApproved { call_hash: [u8; 32] },
        /// An `ApprovedAdminAction` token expired unconsumed and was discarded via
        /// `clear_stale_admin_action`, freeing the call_hash for a new proposal.
        AdminActionExpired { call_hash: [u8; 32] },
        /// A `PendingOracleProposal` sat for `OracleProposalExpiryBlocks` without reaching its
        /// approval threshold and was discarded via `clear_stale_oracle_proposal`, freeing the
        /// case_id for a fresh `submit_ai_ruling`/`finalize_ruling` proposal.
        OracleProposalCleared { case_id: u32 },
        /// A new member was added to the AI Model Governance Council.
        AIGovernanceMemberAdded { who: T::AccountId },
        /// A member was removed from the AI Model Governance Council.
        AIGovernanceMemberRemoved { who: T::AccountId },
        /// A new AI model version was approved by supermajority vote.
        AIModelApproved { version: u32, model_hash: [u8; 32] },
        /// A jury sat in `JurySeated` for `JuryVotingExpiryBlocks` without ever reaching a
        /// strict majority and was cleared via `clear_stale_jury_deadlock`. The case is moved
        /// back to `InJuryAppeal` and a fresh delayed-reveal jury-selection window is scheduled
        /// (the same mechanism `appeal_ruling` uses) — `select_jury` may be called again for
        /// `case_id` once that window elapses.
        JuryDeadlockCleared { case_id: u32 },
    }

    // ── Errors ──────────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        CaseNotFound,
        NotEligibleJuror,
        AppealWindowClosed,
        AlreadyRuled,
        AlreadyVoted,
        InvalidStatus,
        NotEnoughCitizens,
        InvalidJurySize,
        MajorityAlreadyReached,
        /// Only active (non-suspended) citizens may file cases.
        NotActiveCitizen,
        /// Caller is not authorized to perform this action.
        NotAuthorized,
        /// The jury seed window hasn't fully elapsed yet (or was never requested), so
        /// jury selection can't happen — see `JurySeedDelayBlocks`.
        JurySeedNotReady,
        /// Filer's free balance is too low to reserve `CaseFilingBond`.
        InsufficientBalance,
        /// The caller is not a member of the AI Model Governance Council.
        NotAIGovernanceCouncilMember,
        /// This council member has already voted to approve the current proposal.
        AlreadyVotedForAIModel,
        /// Cannot add member: AI Model Governance Council is at maximum capacity.
        AIGovernanceCouncilAtCapacity,
        /// The account is not in the AI Model Governance Council list.
        AIGovernanceMemberNotFound,
        /// The account is already an AI Model Governance Council member.
        AlreadyAIGovernanceCouncilMember,
        /// No AI model has ever been approved — `submit_ai_ruling` cannot be called yet.
        NoApprovedAIModel,
        /// `model_version` does not match the currently approved AI model version.
        UnapprovedAIModel,
        /// `finalize_ruling` was called on a case with no `AIRulingVerdict` recorded. Should
        /// never happen in practice — `submit_ai_ruling` always sets it in the same call that
        /// moves a case to `AIRulingIssued` — but is handled explicitly rather than panicking.
        NoRulingVerdict,
        /// Too many cases have their jury-seed capture scheduled for the same block; this
        /// `appeal_ruling` call would exceed `MaxCasesPerBlock` for its capture block.
        TooManyCasesInBlock,
        /// This case (or, for `propose_admin_action`, this `call_hash`) already has a pending
        /// oracle action awaiting approval; it must resolve before another can be proposed.
        OracleActionAlreadyProposed,
        /// `approve_ai_ruling`/`approve_admin_action` was called for a case/call_hash with no
        /// pending oracle action.
        NoPendingOracleAction,
        /// This Oracle Council member has already approved the current pending action.
        AlreadyApprovedOracleAction,
        /// Cannot add member/approval: Oracle Council (or its per-case/per-call_hash approval
        /// list) is at maximum capacity.
        OracleCouncilAtCapacity,
        /// The account is already an Oracle Council member.
        AlreadyOracleMember,
        /// The account is not in the Oracle Council member list.
        OracleMemberNotFound,
        /// There is no `ApprovedAdminAction` token for this call_hash to clear.
        NoApprovedAdminAction,
        /// The `ApprovedAdminAction` token has not yet reached `AdminActionExpiryBlocks`.
        ApprovalNotYetStale,
        /// The `PendingOracleProposal` for this case_id has not yet reached
        /// `OracleProposalExpiryBlocks`, so `clear_stale_oracle_proposal` refuses to discard it.
        OracleProposalNotYetStale,
        /// The jury for this case_id has not yet sat in `JurySeated` for
        /// `JuryVotingExpiryBlocks`, so `clear_stale_jury_deadlock` refuses to discard it.
        JuryNotYetStale,
        /// A `LawChallenge` case was filed against a `law_id` that already has a law-targeting
        /// case open (filed but not yet `FinalRuling`/`Enforced`) — which may itself be either
        /// a `LawChallenge` or a `TierConflict`. See `OpenCaseByLaw`'s doc comment for the
        /// revert-and-strand bug this prevents.
        DuplicateLawChallenge,
        /// A `TierConflict` case was filed against a `law_id` that already has a law-targeting
        /// case open — which may itself be either a `TierConflict` or a `LawChallenge`. See
        /// `OpenCaseByLaw`'s doc comment.
        DuplicateTierConflict,
        /// `LawChallenge`/`TreasuryDispute`/`TierConflict` filing requires both `zk_proof` and
        /// `public_inputs` — see `CaseFiler`'s doc comment for why these case types must file
        /// anonymously.
        MissingZkProof,
        /// A `zk_proof`/`public_inputs` pair was supplied for a `CitizenConduct`/`General`
        /// filing, which files under the filer's plain `AccountId` and has no use for one.
        UnexpectedZkProof,
        /// `public_inputs` is shorter than the real ZKPassport outer-circuit layout's 9-element
        /// floor — too short to contain `service_scope`/`service_subscope`/`scoped_nullifier`
        /// at their expected fixed offsets. See `MIN_PUBLIC_INPUTS`.
        MissingNullifierInput,
        /// The proof's `service_scope`/`service_subscope` don't match this pallet's required
        /// case-filing domain-separation constants — the proof was generated for a different
        /// purpose and cannot be replayed here. See `CASE_FILING_SERVICE_SCOPE`.
        InvalidProofScope,
        /// The ZK citizenship proof failed verification.
        InvalidZkProof,
    }

    // ── Calls ───────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// File a new case. Only active (non-suspended) citizens may file.
        /// subject determines what gets auto-enforced on ruling.
        ///
        /// Reserves `CaseFilingBond` from the signer as a spam-prevention deposit — instant,
        /// free Level-0 AI rulings would otherwise make this call a cheap DoS vector against
        /// a court system meant to carry real judicial weight. The bond is released in full
        /// once the case reaches a final status; see `auto_finalize`.
        ///
        /// `zk_proof`/`public_inputs` are required for `LawChallenge`/`TreasuryDispute`/
        /// `TierConflict` (cases against institutional power — see `CaseFiler`'s doc comment)
        /// and rejected for `CitizenConduct`/`General` (which still file under the signer's
        /// plain `AccountId`, unchanged). This is a signed extrinsic regardless of case type —
        /// **the signer's `AccountId` (used to reserve the bond) is still ordinary public
        /// transaction data**, the same residual submission-metadata-linkability gap this
        /// codebase already documents for `commit_vote`/`submit_whistleblower_report`/delegate-
        /// persona filings (see CLAUDE.md's Voting System section). What this call fixes is that
        /// the *stored case record and its `CaseFiled` event* no longer name the filer for the
        /// anonymized types — it does not, and cannot by itself, achieve full transaction-level
        /// anonymity (that would need a relayer/mixnet or unsigned ZK-gated submission, neither
        /// of which exists in this codebase — out of scope here, same as everywhere else this
        /// gap is documented).
        #[pallet::call_index(0)]
        // Worst case (an anonymized filing) performs a real BN254/UltraHonk pairing check via
        // `T::ZkVerifier::verify`, not just storage writes — priced the same order of magnitude
        // as `pallet_anticorruption::submit_whistleblower_report`'s identical real ZK-verification
        // call (see that call's own weight-choice doc comment for the reasoning). A
        // `CitizenConduct`/`General` filing is much cheaper in practice (no proof to check) but
        // this flat weight doesn't vary by argument content, so it prices the worst case.
        #[pallet::weight(Weight::from_parts(20_000_000, 0))]
        pub fn file_case(
            origin: OriginFor<T>,
            subject: CaseSubject,
            zk_proof: Option<BoundedVec<u8, ConstU32<4096>>>,
            public_inputs: Option<BoundedVec<[u8; 32], ConstU32<16>>>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::do_file_case(who, subject, zk_proof, public_inputs)
        }

        /// Propose an AI ruling. ruling_hash is the IPFS CID of the full reasoning document.
        /// `verdict` is frozen into the proposal here, at proposal time — this is the actual
        /// on-chain binding between the published reasoning and the verdict that will later
        /// be applied: once the proposal resolves, `AIRulingVerdict` holds exactly this value,
        /// and `finalize_ruling`'s no-appeal path applies exactly that stored value with no
        /// free-form verdict argument of its own — closing the hole where a compromised oracle
        /// key could publish reasoning saying one thing and finalize with the opposite verdict.
        /// See `AIRulingVerdict`.
        ///
        /// `model_version` must match `CurrentAIModelVersion` at proposal time — this is the
        /// actual on-chain enforcement of CLAUDE.md's "AI model updates require on-chain
        /// governance vote (supermajority)" claim: a ruling can only be attributed to a model
        /// version that has actually been approved via `vote_approve_ai_model`. Rejected with
        /// `Error::NoApprovedAIModel` if no model has ever been approved, or
        /// `Error::UnapprovedAIModel` if `model_version` doesn't match the current one
        /// (including a stale, previously-approved-but-since-superseded version). Not
        /// re-checked at resolution time — like the verdict, the model version is frozen into
        /// the proposal the instant it's created.
        ///
        /// Only callable by an Oracle Council member (`OracleMembers`). This call both
        /// *proposes* the ruling (creating `PendingOracleProposal`) and casts the proposer's
        /// own approval, exactly like `pallet_legislature::propose_motion` counting the
        /// proposer's aye immediately. The case moves to `AIRulingIssued` (and `AIRulingIssued`
        /// fires) only once `OracleApprovalNumerator`/`Denominator` of the council has approved
        /// — via this call alone if the council has just one member (trivially satisfies any
        /// majority, preserving the old single-oracle behavior for a 1-member council), or via
        /// subsequent `approve_ai_ruling` calls from other members otherwise.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn submit_ai_ruling(
            origin: OriginFor<T>,
            case_id: u32,
            ruling_hash: [u8; 32],
            model_version: u32,
            verdict: Verdict,
        ) -> DispatchResult {
            let who = T::OracleOrigin::ensure_origin(origin)?;
            let current_model_version = CurrentAIModelVersion::<T>::get();
            ensure!(current_model_version != 0, Error::<T>::NoApprovedAIModel);
            ensure!(model_version == current_model_version, Error::<T>::UnapprovedAIModel);
            let case = Cases::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            ensure!(case.1 == CaseStatus::Filed, Error::<T>::InvalidStatus);
            ensure!(
                PendingOracleProposal::<T>::get(case_id).is_none(),
                Error::<T>::OracleActionAlreadyProposed
            );
            PendingOracleProposal::<T>::insert(
                case_id,
                PendingOracleAction::Submission { ruling_hash, model_version, verdict },
            );
            OracleProposalProposedAt::<T>::insert(case_id, frame_system::Pallet::<T>::block_number());
            let mut approvals: BoundedVec<T::AccountId, T::MaxOracleMembers> = BoundedVec::default();
            approvals.try_push(who.clone()).map_err(|_| Error::<T>::OracleCouncilAtCapacity)?;
            OracleApprovals::<T>::insert(case_id, approvals);
            Self::deposit_event(Event::AIRulingProposed { case_id, proposer: who });
            Self::try_resolve_oracle_action(case_id)?;
            Ok(())
        }

        /// Cast an Oracle Council approval on `case_id`'s current pending action (a
        /// `submit_ai_ruling` submission or a `finalize_ruling` finalization — whichever is
        /// pending; see `PendingOracleAction`). Any member may call once per pending action;
        /// a repeat approval from the same member is rejected (`Error::AlreadyApprovedOracleAction`).
        /// Once approvals reach the configured threshold, the action is applied immediately in
        /// this same call — see `try_resolve_oracle_action`.
        #[pallet::call_index(11)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn approve_ai_ruling(origin: OriginFor<T>, case_id: u32) -> DispatchResult {
            let who = T::OracleOrigin::ensure_origin(origin)?;
            ensure!(
                PendingOracleProposal::<T>::contains_key(case_id),
                Error::<T>::NoPendingOracleAction
            );
            OracleApprovals::<T>::try_mutate(case_id, |maybe_approvals| {
                let approvals = maybe_approvals.get_or_insert_with(BoundedVec::default);
                ensure!(!approvals.contains(&who), Error::<T>::AlreadyApprovedOracleAction);
                approvals.try_push(who.clone()).map_err(|_| Error::<T>::OracleCouncilAtCapacity)
            })?;
            Self::deposit_event(Event::OracleApprovalCast { case_id, member: who });
            Self::try_resolve_oracle_action(case_id)?;
            Ok(())
        }

        /// Appeal an AI ruling within the appeal window. Triggers jury selection.
        ///
        /// Restricted to the case's filer, the designated oracle, or (for system-initiated
        /// cases with no natural filer to restrict to) any active citizen — the same
        /// filer-or-oracle rule `select_jury` already applies, via `is_filer_or_oracle`.
        /// Additionally open to the verified losing party of a `CaseSubject::CitizenConduct`
        /// case (via `is_ruled_against_party`, matched by registered identity nullifier) — a
        /// genuine appeal right for the person actually ruled against, not just whoever filed.
        ///
        /// Before this check existed, *any* signed account (not necessarily the filer, not
        /// even necessarily a citizen) could force a case into `InJuryAppeal`. Combined with
        /// `select_jury` being restricted to the filer/oracle and no off-chain service ever
        /// automating jury selection for appealed cases, that let a party who'd rather not
        /// face the enforced ruling hijack their own case into permanent limbo via any
        /// throwaway account, evading enforcement indefinitely. Extending appeal rights to a
        /// verified losing party for `LawChallenge`/`TreasuryDispute` cases (which don't
        /// identify a specific ruled-against citizen the way `CitizenConduct` does) is a
        /// documented follow-up, not attempted here — no clean existing mechanism identifies
        /// "the losing party" for those subjects the way `CitizenNullifier` does for a citizen.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(6_000, 0))]
        pub fn appeal_ruling(origin: OriginFor<T>, case_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            // Enforce the appeal window before changing any state.
            let ruling_block = AIRulingBlock::<T>::get(case_id)
                .ok_or(Error::<T>::CaseNotFound)?;
            let deadline = ruling_block
                .saturating_add(BlockNumberFor::<T>::from(T::AppealWindowBlocks::get()));
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(now <= deadline, Error::<T>::AppealWindowClosed);
            let case = Cases::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            ensure!(
                Self::is_filer_or_oracle(&who, &case) || Self::is_ruled_against_party(&who, &case),
                Error::<T>::NotAuthorized
            );
            Cases::<T>::try_mutate(case_id, |maybe_case| {
                let case = maybe_case.as_mut().ok_or(Error::<T>::CaseNotFound)?;
                ensure!(case.1 == CaseStatus::AIRulingIssued, Error::<T>::InvalidStatus);
                case.1 = CaseStatus::InJuryAppeal;
                Ok::<(), DispatchError>(())
            })?;
            // Schedule the delayed-reveal jury seed to be captured (from still-live block
            // hashes) at the exact block the seed window closes -- see `SeedCaptureDue` /
            // `CapturedJurySeed`. Must succeed before we commit to InJuryAppeal state, so a
            // full schedule slot fails this call atomically rather than leaving the case
            // stuck (dispatch failure reverts everything in this call, including the Cases
            // mutation above).
            let delay = T::JurySeedDelayBlocks::get();
            let capture_block = now
                .saturating_add(BlockNumberFor::<T>::from(delay))
                .saturating_add(BlockNumberFor::<T>::from(1u32));
            SeedCaptureDue::<T>::try_mutate(capture_block, |maybe_due| {
                maybe_due
                    .get_or_insert_with(BoundedVec::default)
                    .try_push(case_id)
                    .map_err(|_| Error::<T>::TooManyCasesInBlock)
            })?;
            // Commit point for the delayed-reveal jury seed: jury selection can't use any
            // block hash from at or before `now`, only ones produced after it.
            JuryRequestBlock::<T>::insert(case_id, now);
            Self::deposit_event(Event::AppealFiled { case_id, appellant: who });
            Ok(())
        }

        /// Select a jury from the citizen registry using the delayed-reveal seed derived
        /// from `JuryRequestBlock`. Only callable once `JurySeedDelayBlocks` blocks have
        /// elapsed since the appeal — see that constant's doc comment.
        /// The jury size is determined by the case subject:
        ///   - LawChallenge: 21 jurors (Level 2 constitutional review).
        ///   - General / TreasuryDispute / CitizenConduct: 7 jurors (Level 1 appeal).
        /// The caller-supplied `jury_size` is validated against this required size and
        /// rejected with `InvalidJurySize` if it doesn't match, preventing callers from
        /// accidentally (or maliciously) under- or over-seating a jury.
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(100_000, 0))]
        pub fn select_jury(origin: OriginFor<T>, case_id: u32, jury_size: u8) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let case = Cases::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            ensure!(case.1 == CaseStatus::InJuryAppeal, Error::<T>::InvalidStatus);
            // For system-filed cases (filer == AutoChallengeAccount, an unsignable zero account),
            // allow any active citizen to trigger jury selection — otherwise the case is permanently
            // stuck when no oracle is configured. For citizen-filed cases, only the filer or the
            // designated oracle may call. Note this authorization check no longer needs to defend
            // against "timing" the randomness the way it once did: the seed is anchored to the
            // fixed post-appeal window (see `JurySeedDelayBlocks`), not to whichever block the
            // caller of `select_jury` happens to submit in, so delaying this call doesn't let the
            // caller pick a favorable outcome the way it could under the old scheme.
            ensure!(Self::is_filer_or_oracle(&who, &case), Error::<T>::NotAuthorized);
            // Derive the required jury size from the case subject so that the
            // routing logic (Level 1 vs Level 2) is enforced on-chain rather than
            // relying on the caller to pass the correct value.
            let required_size: u8 = match &case.3 {
                // Both are Level-2 constitutional-review questions: LawChallenge disputes a
                // law's validity outright, TierConflict disputes whether it was enacted at the
                // correct tier — the same weight of question, so the same 21-juror bar.
                CaseSubject::LawChallenge { .. } | CaseSubject::TierConflict { .. } => 21,
                _ => 7,
            };
            ensure!(jury_size == required_size, Error::<T>::InvalidJurySize);
            let defendant_nullifier = match &case.3 {
                CaseSubject::CitizenConduct { nullifier, .. } => Some(*nullifier),
                _ => None,
            };
            let total = T::CitizenSelector::total_citizens();
            // `pick_random_jurors` below always excludes the case's own filer, and (for
            // `CaseSubject::CitizenConduct`) also the defendant — see that function's doc
            // comment. Checking the raw citizen `total` here (as this used to) can pass
            // eligibility while the pool is too small to actually seat a jury once those
            // exclusions are applied: at `total == required_size` with an excluded party
            // inside the pool, `pick_random_jurors` can never fill the last seat, and since
            // eligibility already passed here, nothing ever rejects the case again — it's
            // permanently stranded in `InJuryAppeal`. Subtracting the exclusions before
            // comparing closes that gap.
            //
            // `excluded` is a conservative approximation for `CitizenConduct`: if the filer and
            // defendant happen to be the same citizen, this overcounts by one and could
            // over-reject a pool that would technically still work. Not worth detecting
            // precisely for a vanishingly rare case — erring toward over-rejection here is the
            // safe side.
            let excluded = 1u32 + defendant_nullifier.is_some() as u32;
            let eligible = total.saturating_sub(excluded);
            ensure!(eligible >= required_size as u32, Error::<T>::NotEnoughCitizens);
            // Read the pre-captured delayed-reveal seed rather than recomputing it from live
            // block-hash storage. `CapturedJurySeed` is written by `on_initialize` at exactly
            // the block the seed window closes (see `SeedCaptureDue`), so its mere presence
            // here already proves the window has fully elapsed -- no separate block-number
            // check is needed, and (unlike recomputing from `frame_system::block_hash`) this
            // is immune to `BlockHashCount`-based pruning no matter how late `select_jury` is
            // actually called.
            let seed = CapturedJurySeed::<T>::get(case_id).ok_or(Error::<T>::JurySeedNotReady)?;
            let jurors = Self::pick_random_jurors(
                case_id,
                required_size,
                total,
                seed,
                case.0.clone(),
                defendant_nullifier,
            )?;
            Self::deposit_event(Event::JurySelected { case_id, jurors: jurors.clone() });
            JuryPool::<T>::insert(case_id, jurors);
            // Advance status so a second select_jury call is rejected.
            Cases::<T>::try_mutate(case_id, |maybe_case| {
                let c = maybe_case.as_mut().ok_or(Error::<T>::CaseNotFound)?;
                c.1 = CaseStatus::JurySeated;
                Ok::<(), DispatchError>(())
            })?;
            // Timestamp the seating so `clear_stale_jury_deadlock` can detect a jury that never
            // reaches a majority — see `JurySeatedBlock`'s doc comment.
            JurySeatedBlock::<T>::insert(case_id, frame_system::Pallet::<T>::block_number());
            Ok(())
        }

        /// Propose finalizing a ruling for the no-appeal path (AI ruling expires without
        /// appeal). Only callable when status is AIRulingIssued (InJuryAppeal cases
        /// auto-finalize via jury). Once the proposal resolves, finalization automatically
        /// enforces: pauses laws, freezes treasury departments.
        ///
        /// Applies the verdict `submit_ai_ruling` committed on-chain (`AIRulingVerdict`) —
        /// this call deliberately takes no `verdict` argument of its own. That's the fix for
        /// the "compromised oracle key finalizes with a different verdict than the reasoning
        /// it published" hole: there is nothing left for any caller to choose here, only
        /// whether to apply the already-bound verdict.
        ///
        /// Only callable by an Oracle Council member. Like `submit_ai_ruling`, this both
        /// proposes the finalization and casts the proposer's own approval; it resolves
        /// immediately for a 1-member council, or once enough `approve_ai_ruling` calls land
        /// otherwise. See `PendingOracleAction::Finalization` and `try_resolve_oracle_action`.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn finalize_ruling(origin: OriginFor<T>, case_id: u32) -> DispatchResult {
            let who = T::OracleOrigin::ensure_origin(origin)?;
            let case = Cases::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            ensure!(case.1 == CaseStatus::AIRulingIssued, Error::<T>::InvalidStatus);
            let ruling_block = AIRulingBlock::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            let appeal_deadline = ruling_block
                .saturating_add(BlockNumberFor::<T>::from(T::AppealWindowBlocks::get()));
            ensure!(
                frame_system::Pallet::<T>::block_number() > appeal_deadline,
                Error::<T>::AppealWindowClosed
            );
            ensure!(
                PendingOracleProposal::<T>::get(case_id).is_none(),
                Error::<T>::OracleActionAlreadyProposed
            );
            PendingOracleProposal::<T>::insert(case_id, PendingOracleAction::Finalization);
            OracleProposalProposedAt::<T>::insert(case_id, frame_system::Pallet::<T>::block_number());
            let mut approvals: BoundedVec<T::AccountId, T::MaxOracleMembers> = BoundedVec::default();
            approvals.try_push(who.clone()).map_err(|_| Error::<T>::OracleCouncilAtCapacity)?;
            OracleApprovals::<T>::insert(case_id, approvals);
            Self::deposit_event(Event::RulingFinalizationProposed { case_id, proposer: who });
            Self::try_resolve_oracle_action(case_id)?;
            Ok(())
        }

        /// Cast a jury vote for a case in JurySeated status.
        /// Auto-finalizes the case when a strict majority is reached.
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn cast_jury_vote(
            origin: OriginFor<T>,
            case_id: u32,
            verdict: Verdict,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            // Case must exist and have a seated jury.
            let case = Cases::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            ensure!(case.1 == CaseStatus::JurySeated, Error::<T>::InvalidStatus);
            // Voter must be in JuryPool.
            let jury = JuryPool::<T>::get(case_id).ok_or(Error::<T>::NotEligibleJuror)?;
            ensure!(jury.contains(&who), Error::<T>::NotEligibleJuror);
            // Voter must not have already voted.
            ensure!(
                !JuryVotes::<T>::contains_key((case_id, who.clone())),
                Error::<T>::AlreadyVoted
            );
            // Record the vote.
            JuryVotes::<T>::insert((case_id, who.clone()), verdict.clone());
            // Update the tally.
            let (mut upheld, mut overturned) = JuryTally::<T>::get(case_id);
            match verdict {
                Verdict::Upheld => upheld = upheld.saturating_add(1),
                Verdict::Overturned => overturned = overturned.saturating_add(1),
            }
            JuryTally::<T>::insert(case_id, (upheld, overturned));
            Self::deposit_event(Event::JuryVoteCast { case_id, juror: who, verdict: verdict.clone() });
            // Check for strict majority.
            let jury_size = jury.len() as u32;
            let majority_threshold = jury_size / 2;
            if upheld > majority_threshold {
                Self::auto_finalize(case_id, Verdict::Upheld)?;
            } else if overturned > majority_threshold {
                Self::auto_finalize(case_id, Verdict::Overturned)?;
            }
            Ok(())
        }

        /// Add a member to the Oracle Council. Only root may call this — the same gating level
        /// `set_oracle_account` (the single-account design this replaces) used, preserved
        /// deliberately rather than silently loosened by this fix. Mirrors
        /// `add_ai_governance_member`'s shape.
        #[pallet::call_index(6)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn add_oracle_member(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            OracleMembers::<T>::try_mutate(|members| {
                ensure!(!members.contains(&account), Error::<T>::AlreadyOracleMember);
                members.try_push(account.clone()).map_err(|_| Error::<T>::OracleCouncilAtCapacity)
            })?;
            Self::deposit_event(Event::OracleMemberAdded { account });
            Ok(())
        }

        /// Remove a member from the Oracle Council. Only root may call this.
        ///
        /// Also purges this member's already-cast approvals from every in-flight
        /// `PendingOracleProposal` and `PendingAdminAction`, unlike `pallet_legislature`'s
        /// `remove_member` (which deliberately leaves a removed member's votes on still-open
        /// motions in place). That tradeoff doesn't hold here: this council exists specifically
        /// to survive a compromised member, and the expected incident-response path is "member's
        /// key is compromised, so root removes them" — if their already-cast approval kept
        /// counting toward quorum on a proposal they (or an attacker using their key) approved,
        /// removal wouldn't actually shrink the set of approvals a malicious submission/
        /// finalization/admin action needs, undercutting the whole point of the M-of-N design.
        /// Neither map has a per-member index, so this walks every in-flight proposal —
        /// acceptable here because the call is root-gated and root-initiated removals are rare,
        /// incident-driven events, not routine traffic.
        ///
        /// After purging `PendingAdminAction`, `try_resolve_admin_action` is re-run for every
        /// entry the removed member had approved: removing a member also shrinks the council
        /// size the approval threshold is computed against, so a purge can newly cross the
        /// threshold for an action that was previously still short (e.g. a 2-of-3 proposal with
        /// the removed member as one of the two approvers becomes a 1-of-2 proposal, which the
        /// remaining approval alone already satisfies). `PendingOracleProposal` gets the same
        /// re-check, for the same reason (fixed after the fact — see
        /// `remove_oracle_member_reresolves_case_action_crossing_threshold_after_shrink` in
        /// tests.rs for the failure this closes: a 2-member council where the non-proposer is
        /// removed left the proposer's own approval already meeting the shrunk 1-of-1 threshold,
        /// but nothing re-triggered resolution, permanently stranding the proposal — the
        /// proposer couldn't re-propose (`OracleActionAlreadyProposed`) and no one else could
        /// approve it (`AlreadyApprovedOracleAction`, and there was no one else)).
        ///
        /// Two things make this re-check different from the admin-action one:
        ///
        /// - `try_resolve_oracle_action` returns a `DispatchResult`, not `()` —
        ///   a case-based action has extra preconditions (case status, appeal window) that can
        ///   genuinely fail even once the approval threshold is met, and per that function's own
        ///   doc comment, callers are expected to propagate that error so Substrate's
        ///   transactional dispatch reverts the whole call. That's the right behavior for
        ///   `submit_ai_ruling`/`finalize_ruling`/`approve_ai_ruling`, but not here: this call's
        ///   primary job (removing a compromised member) must succeed regardless of the state of
        ///   some unrelated in-flight case, so an error from the re-check must not abort the
        ///   member removal or the purges already performed above it. Each re-check therefore
        ///   runs inside its own nested `with_transaction`, committed on success and rolled back
        ///   on error — so a failed attempt (e.g. `auto_finalize` partially mutating before an
        ///   enforcement hook errors) can't leave partial state behind, while the member-removal
        ///   and purge writes made earlier in this call are unaffected either way. The
        ///   proposal simply stays pending on error, exactly the pre-purge status quo, resolvable
        ///   later via the surviving members' own calls.
        /// - Unlike `PendingAdminAction`, which stores `(proposer, approvals)` and preserves
        ///   `proposer` through the purge (it's needed later to gate who may consume
        ///   `ApprovedAdminAction`), `PendingOracleProposal`/`OracleApprovals` never track a
        ///   distinct "proposer" role at all — `submit_ai_ruling`/`finalize_ruling` just push the
        ///   caller into the same `OracleApprovals` list any `approve_ai_ruling` call appends to.
        ///   So removing the proposer needs no special case: their approval is purged exactly
        ///   like any other member's, the proposal survives (it's still valid, just down one
        ///   vote), and it can still resolve immediately in this call if the remaining approvals
        ///   already meet the shrunk threshold, or later via other members' approvals otherwise.
        /// - The re-check is run for *every* case_id with an in-flight `OracleApprovals` entry,
        ///   not only the ones `account` itself had approved (`PendingAdminAction`'s `affected`
        ///   list is narrower — gated on the removed member having approved that specific
        ///   action). That narrower gate is actually wrong for the failure this fix targets: in
        ///   the report's 2-member scenario, the removed member never approved the lone pending
        ///   proposal at all (only the proposer did) — the council shrinking is what crosses the
        ///   threshold, independent of whether the removed member happened to have voted on any
        ///   given case. Gating the oracle re-check the same way `PendingAdminAction` does would
        ///   silently fail to close this exact bug. Left `PendingAdminAction` itself unchanged
        ///   (out of scope here) even though the same broader argument likely applies to it too.
        #[pallet::call_index(10)]
        // Was a flat 5_000 (sized for a single write) even after the fix above gave this call
        // an unbounded loop of `try_resolve_oracle_action`/`try_resolve_admin_action` calls —
        // each of which can trigger real cross-pallet enforcement via `auto_finalize`. Rather
        // than leave that loop genuinely unbounded (an admin action whose real cost would then
        // scale with however many cases happen to be pending at removal time) or build a full
        // multi-block resumable scan for what's realistically a rare, root-gated,
        // incident-response action, the loop itself is capped at
        // `MAX_ORACLE_ACTIONS_RERESOLVED_PER_REMOVAL` re-resolutions each for cases and admin
        // actions (see that constant's doc comment for why a resumable scan — the pattern
        // `pallet_elections::run_election` uses for its own unbounded-scan fix — is
        // disproportionate here: a member removal's own purge step must still apply to *every*
        // pending entry immediately and atomically, for the removal to actually be a complete,
        // immediate incident response; only the strictly-optional re-resolution step is
        // deferrable). Priced here as a base cost (member removal + the two unavoidably
        // full-map `translate` purges, each item's work bounded by `MaxOracleMembers`) plus up
        // to `MAX_ORACLE_ACTIONS_RERESOLVED_PER_REMOVAL * 2` re-resolutions (cases and admin
        // actions), each costed the same as `finalize_ruling`'s own flat weight (20_000) since
        // `try_resolve_oracle_action` performs equivalent work to what `finalize_ruling` itself
        // triggers.
        #[pallet::weight(
            Weight::from_parts(5_000, 0).saturating_add(
                Weight::from_parts(20_000, 0)
                    .saturating_mul(MAX_ORACLE_ACTIONS_RERESOLVED_PER_REMOVAL as u64 * 2)
            )
        )]
        pub fn remove_oracle_member(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            OracleMembers::<T>::try_mutate(|members| {
                let pos = members
                    .iter()
                    .position(|m| m == &account)
                    .ok_or(Error::<T>::OracleMemberNotFound)?;
                members.remove(pos);
                Ok::<(), DispatchError>(())
            })?;
            // Unlike the `PendingAdminAction` purge below (which only re-checks entries the
            // *removed* member had actually approved), every case_id with an in-flight
            // `OracleApprovals` entry is collected here, whether or not `account` had approved
            // it — removing a member shrinks the threshold's denominator for *every* pending
            // case, not just the ones they personally voted on (that's exactly the bug: in the
            // report's 2-member scenario the removed member never approved the pending proposal
            // at all, only the proposer did, yet removing them still crosses the new 1-of-1
            // threshold and must resolve it).
            let mut affected_cases: alloc::vec::Vec<u32> = alloc::vec::Vec::new();
            OracleApprovals::<T>::translate(
                |case_id, mut approvers: BoundedVec<T::AccountId, T::MaxOracleMembers>| {
                    if let Some(pos) = approvers.iter().position(|m| m == &account) {
                        approvers.remove(pos);
                    }
                    affected_cases.push(case_id);
                    Some(approvers)
                },
            );
            // Bounded to `MAX_ORACLE_ACTIONS_RERESOLVED_PER_REMOVAL` — see that constant's doc
            // comment for why this loop, unlike the purge above, is capped rather than left to
            // scale with however many cases are pending. Every entry was already purged of the
            // removed member's vote above regardless of whether it gets re-resolved here.
            for case_id in affected_cases.into_iter().take(MAX_ORACLE_ACTIONS_RERESOLVED_PER_REMOVAL) {
                // See the doc comment above this function for why errors here are swallowed
                // (via a nested, independently-rolled-back transaction) rather than propagated:
                // this call's own success must not depend on the state of an unrelated case.
                let _ = frame_support::storage::with_transaction(|| {
                    match Self::try_resolve_oracle_action(case_id) {
                        Ok(()) => sp_runtime::TransactionOutcome::Commit(Ok(())),
                        Err(e) => sp_runtime::TransactionOutcome::Rollback(Err(e)),
                    }
                });
            }
            let mut affected: alloc::vec::Vec<[u8; 32]> = alloc::vec::Vec::new();
            PendingAdminAction::<T>::translate(
                |call_hash,
                 (proposer, mut approvers): (
                    T::AccountId,
                    BoundedVec<T::AccountId, T::MaxOracleMembers>,
                )| {
                    if let Some(pos) = approvers.iter().position(|m| m == &account) {
                        approvers.remove(pos);
                        affected.push(call_hash);
                    }
                    Some((proposer, approvers))
                },
            );
            // Same bound as the case-action loop above, for the same reason.
            for call_hash in affected.into_iter().take(MAX_ORACLE_ACTIONS_RERESOLVED_PER_REMOVAL) {
                Self::try_resolve_admin_action(call_hash);
            }
            Self::deposit_event(Event::OracleMemberRemoved { account });
            Ok(())
        }

        /// Add a member to the AI Model Governance Council. Only root may call this.
        /// Mirrors pallet-emergency-council's `add_council_member`.
        #[pallet::call_index(7)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn add_ai_governance_member(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            AIGovernanceCouncil::<T>::try_mutate(|members| {
                ensure!(!members.contains(&account), Error::<T>::AlreadyAIGovernanceCouncilMember);
                members.try_push(account.clone()).map_err(|_| Error::<T>::AIGovernanceCouncilAtCapacity)
            })?;
            Self::deposit_event(Event::AIGovernanceMemberAdded { who: account });
            Ok(())
        }

        /// Remove a member from the AI Model Governance Council. Only root may call this.
        /// Mirrors pallet-emergency-council's `remove_council_member`.
        #[pallet::call_index(8)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn remove_ai_governance_member(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            AIGovernanceCouncil::<T>::try_mutate(|members| {
                let pos = members
                    .iter()
                    .position(|m| m == &account)
                    .ok_or(Error::<T>::AIGovernanceMemberNotFound)?;
                members.remove(pos);
                Ok::<(), DispatchError>(())
            })?;
            // Clear any pending vote from this member to keep state clean, mirroring
            // pallet-emergency-council's remove_council_member.
            AIModelApprovalVotes::<T>::remove(&account);
            Self::deposit_event(Event::AIGovernanceMemberRemoved { who: account });
            Ok(())
        }

        /// Vote to approve `model_hash` as the next AI model version. Any AI Model
        /// Governance Council member may call. When a supermajority of the council has
        /// voted for the same hash, the vote resolves immediately: `CurrentAIModelVersion`
        /// is bumped, the approval is recorded in `AIModelVersions`, and the vote state
        /// resets for the next round.
        ///
        /// ## Design note: why a dedicated in-pallet supermajority vote
        /// CLAUDE.md requires "AI model updates require on-chain governance vote
        /// (supermajority)". Two existing mechanisms in this codebase were considered and
        /// rejected in favor of a dedicated, self-contained vote here:
        ///   - `pallet_legislature::EnsureLegislatureMotion` — reusable across pallets, but
        ///     its only threshold is `Config::PassageThreshold`, a single pallet-wide
        ///     *simple* majority (wired to 50 in the runtime). There is no per-motion-kind
        ///     higher bar to parameterize; giving AI model changes their own threshold would
        ///     require adding motion-kind-aware thresholds to pallet-legislature itself —
        ///     well beyond this task's scope of adding the missing primitive to
        ///     pallet-courts.
        ///   - `pallet-emergency-council` — has exactly the right shape (small dedicated
        ///     council + `SupermajorityNumerator`/`Denominator` + vote-until-threshold), but
        ///     exposes no reusable `EnsureOrigin`: its calls check council membership and
        ///     tally votes internally rather than through a generic origin type, and it
        ///     isn't even wired into the runtime yet.
        /// So this mirrors pallet-emergency-council's own self-contained mechanism instead:
        /// a dedicated `AIGovernanceCouncil` (root-managed, like `Council`), a per-round
        /// vote map, and the identical `votes * denominator >= size * numerator` tally,
        /// evaluated on every vote and resolving the instant the threshold is crossed. This
        /// is a small, already-proven pattern in this exact codebase — pallet-executive
        /// independently reimplements the identical Numerator/Denominator Config-constant
        /// idiom for its own cabinet supermajority — not a new voting scheme invented for
        /// this feature.
        #[pallet::call_index(9)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn vote_approve_ai_model(origin: OriginFor<T>, model_hash: [u8; 32]) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let council = AIGovernanceCouncil::<T>::get();
            ensure!(council.contains(&who), Error::<T>::NotAIGovernanceCouncilMember);
            ensure!(!AIModelApprovalVotes::<T>::get(&who), Error::<T>::AlreadyVotedForAIModel);

            // Lock in the proposed hash from the first vote of the round; a later voter's
            // differing hash is simply ignored in favor of the agreed-upon one — same
            // rationale as pallet-emergency-council's `PendingEmergencyProposal`.
            if PendingAIModelProposal::<T>::get().is_none() {
                PendingAIModelProposal::<T>::put(model_hash);
            }
            let agreed_hash = PendingAIModelProposal::<T>::get().unwrap_or(model_hash);

            AIModelApprovalVotes::<T>::insert(&who, true);

            // Count votes cast so far (including this one, just inserted above).
            let vote_count = council
                .iter()
                .filter(|m| AIModelApprovalVotes::<T>::get(m))
                .count() as u32;

            if Self::ai_model_supermajority_reached(vote_count, council.len() as u32) {
                let new_version = CurrentAIModelVersion::<T>::get().saturating_add(1);
                let now = frame_system::Pallet::<T>::block_number();
                AIModelVersions::<T>::insert(
                    new_version,
                    ModelInfo { model_hash: agreed_hash, approved_at: now },
                );
                CurrentAIModelVersion::<T>::put(new_version);

                // Proposal consumed; clear it and reset the vote map for the next round.
                PendingAIModelProposal::<T>::kill();
                let _ = AIModelApprovalVotes::<T>::clear(u32::MAX, None);

                Self::deposit_event(Event::AIModelApproved { version: new_version, model_hash: agreed_hash });
            }
            Ok(())
        }

        /// Propose a case-less Oracle Council administrative action, identified by the
        /// domain-separated hash of the exact call it will authorize (e.g. a manual
        /// `invalidate_law`/`suspend_citizen`/`restore_citizen_rights` override in another
        /// pallet — see `EnsureOracleCouncilApproved`). Any Oracle Council member may call; this
        /// both proposes the action and casts the proposer's own approval, exactly like
        /// `submit_ai_ruling`. Resolves immediately for a 1-member council, or once enough
        /// `approve_admin_action` calls land otherwise.
        #[pallet::call_index(12)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn propose_admin_action(origin: OriginFor<T>, call_hash: [u8; 32]) -> DispatchResult {
            let who = T::OracleOrigin::ensure_origin(origin)?;
            ensure!(
                PendingAdminAction::<T>::get(call_hash).is_none()
                    && ApprovedAdminAction::<T>::get(call_hash).is_none(),
                Error::<T>::OracleActionAlreadyProposed
            );
            let mut approvals: BoundedVec<T::AccountId, T::MaxOracleMembers> = BoundedVec::default();
            approvals.try_push(who.clone()).map_err(|_| Error::<T>::OracleCouncilAtCapacity)?;
            PendingAdminAction::<T>::insert(call_hash, (who.clone(), approvals));
            Self::deposit_event(Event::AdminActionProposed { call_hash, proposer: who });
            Self::try_resolve_admin_action(call_hash);
            Ok(())
        }

        /// Cast an Oracle Council approval on `call_hash`'s pending administrative action. See
        /// `propose_admin_action`.
        #[pallet::call_index(13)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn approve_admin_action(origin: OriginFor<T>, call_hash: [u8; 32]) -> DispatchResult {
            let who = T::OracleOrigin::ensure_origin(origin)?;
            PendingAdminAction::<T>::try_mutate(call_hash, |maybe| {
                let (_, approvals) = maybe.as_mut().ok_or(Error::<T>::NoPendingOracleAction)?;
                ensure!(!approvals.contains(&who), Error::<T>::AlreadyApprovedOracleAction);
                approvals.try_push(who.clone()).map_err(|_| Error::<T>::OracleCouncilAtCapacity)
            })?;
            Self::deposit_event(Event::AdminActionApprovalCast { call_hash, member: who });
            Self::try_resolve_admin_action(call_hash);
            Ok(())
        }

        /// Discard an unconsumed `ApprovedAdminAction` token once `AdminActionExpiryBlocks`
        /// have passed since it was approved. Open to any *current* Oracle Council member —
        /// recovers the court system from an approved admin action (invalidate_law/
        /// suspend_citizen/restore_citizen_rights) that nobody ever consumes because the
        /// proposer went offline, lost their key, or was removed. Mirrors
        /// `pallet_legislature::clear_stale_approval` — see `EnsureOracleCouncilApproved`'s doc
        /// comment for the deadlock this closes.
        #[pallet::call_index(14)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn clear_stale_admin_action(origin: OriginFor<T>, call_hash: [u8; 32]) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(
                OracleMembers::<T>::get().contains(&who),
                Error::<T>::OracleMemberNotFound
            );
            let (_, approved_at) = ApprovedAdminAction::<T>::get(call_hash)
                .ok_or(Error::<T>::NoApprovedAdminAction)?;
            let now = frame_system::Pallet::<T>::block_number();
            let expiry = BlockNumberFor::<T>::from(T::AdminActionExpiryBlocks::get());
            ensure!(
                now >= approved_at.saturating_add(expiry),
                Error::<T>::ApprovalNotYetStale
            );
            ApprovedAdminAction::<T>::remove(call_hash);
            Self::deposit_event(Event::AdminActionExpired { call_hash });
            Ok(())
        }

        /// Discard a `PendingOracleProposal` that has sat for `OracleProposalExpiryBlocks`
        /// without reaching the Oracle Council's M-of-N approval threshold. Open to any
        /// *current* Oracle Council member — the case-based counterpart of
        /// `clear_stale_admin_action`, recovering the court system from a `submit_ai_ruling`/
        /// `finalize_ruling` proposal that nobody ever pushes past quorum because members are
        /// offline, non-participating, or were removed. Without this, `case_id` is stuck
        /// forever: neither `submit_ai_ruling` nor `finalize_ruling` will re-propose while a
        /// `PendingOracleProposal` already exists (`Error::OracleActionAlreadyProposed`), and
        /// `approve_ai_ruling` only accepts approvals, never withdrawals. See
        /// `OracleProposalProposedAt` and `Config::OracleProposalExpiryBlocks`.
        ///
        /// Clears `PendingOracleProposal`, `OracleApprovals`, and `OracleProposalProposedAt`
        /// for `case_id`, exactly like a normal resolution or expiry via
        /// `try_resolve_oracle_action` — so a fresh `submit_ai_ruling`/`finalize_ruling` call
        /// afterward starts a clean proposal with no leftover approvals.
        #[pallet::call_index(15)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn clear_stale_oracle_proposal(origin: OriginFor<T>, case_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(
                OracleMembers::<T>::get().contains(&who),
                Error::<T>::OracleMemberNotFound
            );
            let proposed_at = OracleProposalProposedAt::<T>::get(case_id)
                .ok_or(Error::<T>::NoPendingOracleAction)?;
            let now = frame_system::Pallet::<T>::block_number();
            let expiry = BlockNumberFor::<T>::from(T::OracleProposalExpiryBlocks::get());
            ensure!(
                now >= proposed_at.saturating_add(expiry),
                Error::<T>::OracleProposalNotYetStale
            );
            PendingOracleProposal::<T>::remove(case_id);
            OracleApprovals::<T>::remove(case_id);
            OracleProposalProposedAt::<T>::remove(case_id);
            Self::deposit_event(Event::OracleProposalCleared { case_id });
            Ok(())
        }

        /// Recover a jury that has sat in `CaseStatus::JurySeated` for
        /// `JuryVotingExpiryBlocks` without a strict majority ever being reached — no-shows,
        /// or votes split in a way that never crosses the threshold for either side (see
        /// `cast_jury_vote`'s majority check). Without this, such a case has no deadline, no
        /// re-selection mechanism, and no admin override: nothing but a majority vote ever
        /// moves a case out of `JurySeated`, so a jury that never reaches one leaves the case
        /// stuck there forever.
        ///
        /// Rather than merely clearing the stuck jury with no path forward, this re-schedules
        /// a fresh jury selection through exactly the same delayed-reveal commit-then-reveal
        /// pipeline `appeal_ruling` uses (see `Config::JurySeedDelayBlocks`): the case moves
        /// back to `CaseStatus::InJuryAppeal`, the deadlocked jury's pool/votes/tally are
        /// discarded, and a new `JuryRequestBlock`/`SeedCaptureDue` entry is scheduled from the
        /// current block — a follow-up `select_jury` call (same as the original flow) draws the
        /// new jury once that window elapses. This deliberately reuses the existing, already-
        /// audited randomness pipeline instead of drawing a replacement jury inline from some
        /// ad-hoc immediately-available randomness source: the latter would reopen exactly the
        /// grinding hole `JurySeedDelayBlocks` was built to close, since whoever is authorized
        /// to call this could otherwise pick their submission moment once a favorable outcome
        /// was already computable. A bare "clear with no next step" was considered and rejected
        /// as strictly less useful for the same implementation cost, since the re-seed above
        /// needs nothing beyond storage this pallet already has.
        ///
        /// Also clears any `CapturedJurySeed` left over from the deadlocked jury's own window —
        /// without this, `select_jury` would see a stale, already fully-computable seed from
        /// the old window still present and could be called immediately, bypassing the new
        /// delayed-reveal window entirely.
        ///
        /// Authorization mirrors `select_jury`/`appeal_ruling`'s own gate
        /// (`is_filer_or_oracle`): the case's filer, any Oracle Council member, or — for a
        /// system-initiated case with no natural filer — any active citizen. A jury deadlock
        /// isn't specifically an Oracle Council concern the way `clear_stale_admin_action`/
        /// `clear_stale_oracle_proposal` are (those recover a stuck *council approval* flow);
        /// the parties already entitled to drive this case through the rest of the appeal
        /// pipeline are the closer fit for recovering a stuck jury on it too.
        #[pallet::call_index(16)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn clear_stale_jury_deadlock(origin: OriginFor<T>, case_id: u32) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let case = Cases::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            ensure!(case.1 == CaseStatus::JurySeated, Error::<T>::InvalidStatus);
            ensure!(Self::is_filer_or_oracle(&who, &case), Error::<T>::NotAuthorized);
            let seated_at =
                JurySeatedBlock::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            let now = frame_system::Pallet::<T>::block_number();
            let expiry = BlockNumberFor::<T>::from(T::JuryVotingExpiryBlocks::get());
            ensure!(now >= seated_at.saturating_add(expiry), Error::<T>::JuryNotYetStale);

            // Discard the deadlocked jury's pool, votes, and tally.
            if let Some(old_jury) = JuryPool::<T>::take(case_id) {
                for juror in old_jury.iter() {
                    JuryVotes::<T>::remove((case_id, juror.clone()));
                }
            }
            JuryTally::<T>::remove(case_id);
            JurySeatedBlock::<T>::remove(case_id);
            // Stale leftover from the deadlocked jury's own window — must not let a fresh
            // select_jury call reuse it before the new window scheduled below actually closes.
            CapturedJurySeed::<T>::remove(case_id);

            Cases::<T>::try_mutate(case_id, |maybe_case| {
                let c = maybe_case.as_mut().ok_or(Error::<T>::CaseNotFound)?;
                c.1 = CaseStatus::InJuryAppeal;
                Ok::<(), DispatchError>(())
            })?;

            // Re-run the same delayed-reveal scheduling `appeal_ruling` performs.
            let delay = T::JurySeedDelayBlocks::get();
            let capture_block = now
                .saturating_add(BlockNumberFor::<T>::from(delay))
                .saturating_add(BlockNumberFor::<T>::from(1u32));
            SeedCaptureDue::<T>::try_mutate(capture_block, |maybe_due| {
                maybe_due
                    .get_or_insert_with(BoundedVec::default)
                    .try_push(case_id)
                    .map_err(|_| Error::<T>::TooManyCasesInBlock)
            })?;
            JuryRequestBlock::<T>::insert(case_id, now);

            Self::deposit_event(Event::JuryDeadlockCleared { case_id });
            Ok(())
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
        /// True if `who` is authorized to act on `case` in a filer/oracle capacity: the
        /// case's own filer, the designated oracle, or — for system-initiated cases with no
        /// natural filer to restrict to (see `AutoChallengeAccount`) — any active citizen.
        /// Shared by `appeal_ruling` and `select_jury` so both calls that gate on "who may
        /// move this case along" apply exactly the same rule.
        fn is_filer_or_oracle(who: &T::AccountId, case: &CaseOf<T>) -> bool {
            let oracle_ok = OracleMembers::<T>::get().contains(who);
            // `case.0` is `CaseFiler::Account` for `CitizenConduct`/`General` cases (including
            // every system-initiated `auto_file_case` filing, which always uses
            // `CaseFiler::Account(T::AutoChallengeAccount::get())`) and `CaseFiler::Nullifier`
            // for anonymized `LawChallenge`/`TreasuryDispute`/`TierConflict` cases. For the
            // latter, "is `who` the filer" is answered the same way `is_ruled_against_party`
            // answers "is `who` the accused" — by matching `who`'s own registered nullifier
            // against the stored one — since there's no `AccountId` in the record to compare
            // against directly.
            let (is_filer, system_case) = match &case.0 {
                CaseFiler::Account(acct) => (who == acct, acct == &T::AutoChallengeAccount::get()),
                CaseFiler::Nullifier(n) => (T::CitizenChecker::citizen_nullifier(who) == Some(*n), false),
            };
            is_filer || oracle_ok || (system_case && T::CitizenChecker::is_active_citizen(who))
        }

        /// True if `who` is the verified losing party of a `CaseSubject::CitizenConduct` case
        /// -- i.e. their own registered identity nullifier (`CitizenChecker::citizen_nullifier`)
        /// matches the nullifier the case was filed against. Always false for every other
        /// subject (see `appeal_ruling`'s doc comment for why those subjects aren't extended
        /// the same way).
        fn is_ruled_against_party(who: &T::AccountId, case: &CaseOf<T>) -> bool {
            match &case.3 {
                CaseSubject::CitizenConduct { nullifier, .. } => {
                    T::CitizenChecker::citizen_nullifier(who) == Some(*nullifier)
                }
                _ => false,
            }
        }

        /// Returns the `OpenCaseByLaw` kind + law_id for `subject`, if `subject` is one of the
        /// law-targeting variants (`LawChallenge`/`TierConflict`) — `None` for every other
        /// subject. Shared by `do_file_case`, `auto_file_case`, and `auto_finalize` so the
        /// mapping from subject to dedup-guard key/kind is defined exactly once.
        fn law_case_kind(subject: &CaseSubject) -> Option<(CaseSubjectKind, u32)> {
            match subject {
                CaseSubject::LawChallenge { law_id } => Some((CaseSubjectKind::LawChallenge, *law_id)),
                CaseSubject::TierConflict { law_id } => Some((CaseSubjectKind::TierConflict, *law_id)),
                _ => None,
            }
        }

        /// Shared finalization logic used by both `finalize_ruling` (root / AI path)
        /// and `cast_jury_vote` (automatic majority path).
        fn auto_finalize(case_id: u32, verdict: Verdict) -> DispatchResult {
            // Fetch the subject before mutating status, so we can do enforcement after.
            let case = Cases::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            ensure!(case.1 != CaseStatus::FinalRuling, Error::<T>::MajorityAlreadyReached);
            // The status right before this call distinguishes which path led here:
            // `finalize_ruling` only proceeds from `AIRulingIssued` (never appealed, no jury
            // ever involved); `cast_jury_vote` only proceeds from `JurySeated` (a jury actually
            // reviewed it). Captured now, before the mutation below overwrites it.
            let jury_reviewed = case.1 == CaseStatus::JurySeated;
            if jury_reviewed {
                // The case is leaving JurySeated for good (majority reached) — clear the
                // staleness timestamp so it can't be mistaken for a still-pending deadlock.
                JurySeatedBlock::<T>::remove(case_id);
            }
            Cases::<T>::try_mutate(case_id, |maybe_case| {
                let c = maybe_case.as_mut().ok_or(Error::<T>::CaseNotFound)?;
                c.1 = CaseStatus::FinalRuling;
                Ok::<(), DispatchError>(())
            })?;
            // This case has now reached a terminal status (FinalRuling, possibly advanced to
            // Enforced below) — release its `law_id` slot in `OpenCaseByLaw` (if this case is a
            // law-targeting one) so a fresh LawChallenge or TierConflict against the same law
            // can be filed. Guarded on the map still pointing at *this* case (matching both
            // kind and case_id), though in practice it always does: only `do_file_case`/
            // `auto_file_case` ever insert into this map, and the dedup check they both perform
            // first prevents a second case from ever overwriting an existing entry for the same
            // law_id.
            if let Some((kind, law_id)) = Self::law_case_kind(&case.3) {
                if OpenCaseByLaw::<T>::get(law_id) == Some((kind, case_id)) {
                    OpenCaseByLaw::<T>::remove(law_id);
                }
            }
            Rulings::<T>::insert(case_id, verdict.clone());
            // Release the filing bond, if any, now that the case has reached a final status.
            // Always released in full regardless of verdict — there's no precedent elsewhere
            // in this pallet for slashing on a "bad-faith" outcome, and inventing one here
            // would be guessing at policy rather than following an established pattern (see
            // the bond-and-release design note on `Config::CaseFilingBond`). Absent for
            // system-filed cases (`auto_file_case` never inserts a `CaseBonds`/`CaseBondAccount`
            // entry), so this is a no-op for those — `take` just returns `None`.
            //
            // Unreserved from `CaseBondAccount` (the real signing account that reserved it),
            // not `case.0` (the `CaseFiler`, which is a ZK nullifier — not an `AccountId` at
            // all — for anonymized case types) — see `CaseBondAccount`'s doc comment.
            if let Some(bond) = CaseBonds::<T>::take(case_id) {
                if let Some(bond_account) = CaseBondAccount::<T>::take(case_id) {
                    T::Currency::unreserve(&bond_account, bond);
                } else {
                    // Not reachable via any call site today — `do_file_case` always inserts
                    // both `CaseBonds` and `CaseBondAccount` together in the same call, and
                    // `auto_file_case` inserts neither — but nothing *structurally* enforces
                    // that the two maps stay in sync. If they ever did fall out of sync,
                    // silently doing nothing here would permanently lose the reserved bond
                    // (never unreserved, and `CaseBonds::take` above has already removed the
                    // only record of it). A hard error here would be worse: it would abort
                    // finalization for an otherwise-legitimate case over an unrelated
                    // bookkeeping bug, reintroducing exactly the kind of permanently-stuck case
                    // the dedup-guard fix above exists to eliminate. So this stays a debug-only
                    // trip wire — a no-op in production, not a new runtime error path — to
                    // catch the invariant violation immediately in any test/dev build.
                    debug_assert!(
                        false,
                        "CaseBonds entry with no matching CaseBondAccount for case {case_id}"
                    );
                }
            }
            Self::deposit_event(Event::RulingFinalized { case_id, verdict: verdict.clone() });
            // Auto-enforce only on Overturned verdicts — Upheld means "AI was right, no action".
            // For General cases there is no automatic enforcement target by design.
            if verdict == Verdict::Overturned {
                let enforced = match &case.3 {
                    CaseSubject::LawChallenge { law_id } => {
                        T::LawEnforcer::invalidate_law(*law_id)?;
                        true
                    }
                    // Same remedy as LawChallenge — see `CaseSubject::TierConflict`'s doc
                    // comment for why this is deliberately not a "re-tier and re-run through
                    // entrenchment" flow: invalidating the wrongly-tiered law is enough, and if
                    // the legislature wants it back they re-enact it at the correct tier.
                    CaseSubject::TierConflict { law_id } => {
                        T::LawEnforcer::invalidate_law(*law_id)?;
                        true
                    }
                    CaseSubject::TreasuryDispute { department_id } => {
                        T::TreasuryEnforcer::freeze_department(*department_id)?;
                        true
                    }
                    CaseSubject::CitizenConduct { nullifier, suspension_blocks } => {
                        // Convert the duration to an absolute block number here in the courts pallet.
                        // The CitizenSuspender trait receives an absolute block, not a duration,
                        // so the identity pallet can store it directly without knowing "now".
                        let now = frame_system::Pallet::<T>::block_number();
                        let until = suspension_blocks.map(|b| {
                            now.saturating_add(BlockNumberFor::<T>::from(b))
                        });
                        T::CitizenSuspender::suspend_citizen(*nullifier, until, jury_reviewed)?;
                        true
                    }
                    CaseSubject::General => false,
                };
                if enforced {
                    // Advance status to Enforced so observers can distinguish
                    // "finalized but not enforced" from "finalized and enforced".
                    Cases::<T>::try_mutate(case_id, |maybe_case| {
                        let c = maybe_case.as_mut().ok_or(Error::<T>::CaseNotFound)?;
                        c.1 = CaseStatus::Enforced;
                        Ok::<(), DispatchError>(())
                    })?;
                    Self::deposit_event(Event::RulingEnforced { case_id });
                }
            }
            Ok(())
        }

        /// System-initiated case filing — used by pallet-constitution when a Structural or
        /// Foundational law is enacted, guaranteeing automatic AI review without requiring
        /// a citizen to proactively file a challenge.
        ///
        /// Deliberately does NOT reserve `CaseFilingBond` (unlike `file_case`): the bond
        /// exists to price out spam from citizens choosing to open cases, but this path is
        /// triggered by the runtime itself (via `AutoChallengeHook`) as a mandatory
        /// constitutional-review step, not a discretionary citizen action — there's no spam
        /// vector to defend against here, and charging `T::AutoChallengeAccount` (an
        /// unsignable well-known account, see that Config item) would just be a pointless
        /// balance requirement on an account nobody controls.
        pub fn auto_file_case(subject: CaseSubject) -> DispatchResult {
            // Same duplicate law-targeting-case guard as `do_file_case` — see `OpenCaseByLaw`'s
            // doc comment. System-initiated filings can hit the same revert-and-strand bug in
            // `auto_finalize` just as citizen-filed ones can, so this path needs the same
            // check even though it never reserves a bond. Written generically against both
            // law-targeting kinds (via `law_case_kind`), even though today only `LawChallenge`
            // subjects are ever actually routed through this system-initiated entry point (the
            // runtime's `AutoChallengeHook` impl only ever calls this with `LawChallenge`;
            // `TierConflict` only ever reaches the chain via `file_case_for`) — defense in
            // depth, so a future caller passing `TierConflict` here doesn't silently bypass the
            // guard.
            let law_case = Self::law_case_kind(&subject);
            if let Some((kind, law_id)) = &law_case {
                ensure!(
                    OpenCaseByLaw::<T>::get(law_id).is_none(),
                    match kind {
                        CaseSubjectKind::LawChallenge => Error::<T>::DuplicateLawChallenge,
                        CaseSubjectKind::TierConflict => Error::<T>::DuplicateTierConflict,
                    }
                );
            }
            let filer = CaseFiler::Account(T::AutoChallengeAccount::get());
            let id = NextCaseId::<T>::get();
            if let Some((kind, law_id)) = &law_case {
                OpenCaseByLaw::<T>::insert(law_id, (*kind, id));
            }
            Cases::<T>::insert(id, (filer.clone(), CaseStatus::Filed, None::<[u8; 32]>, subject.clone()));
            NextCaseId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::CaseFiled { case_id: id, filer, subject });
            Ok(())
        }

        /// Same filing logic as the `file_case` dispatchable, for callers that already have a
        /// verified signed account in hand — used by `pallet-constitution`'s
        /// `challenge_law_tier` (via its `TierConflictHook` trait) to open a
        /// `CaseSubject::TierConflict` case through this pallet's normal citizen-filing
        /// pipeline (ZK-gated anonymized storage, bond reservation, dedup guard) without
        /// pallet-constitution needing to duplicate any of it. Mirrors how `auto_file_case`
        /// above is the existing non-dispatchable entry point `AutoChallengeHook` calls into —
        /// same cross-pallet-hook pattern, applied to a citizen-initiated (not system-initiated)
        /// filing this time.
        pub fn file_case_for(
            who: T::AccountId,
            subject: CaseSubject,
            zk_proof: BoundedVec<u8, ConstU32<4096>>,
            public_inputs: BoundedVec<[u8; 32], ConstU32<16>>,
        ) -> DispatchResult {
            Self::do_file_case(who, subject, Some(zk_proof), Some(public_inputs))
        }

        /// Shared body for the `file_case` dispatchable and `file_case_for`. See `file_case`'s
        /// doc comment for the full behavior; this just factors out the logic both entry points
        /// need so pallet-constitution's `challenge_law_tier` doesn't have to re-implement any
        /// of it.
        fn do_file_case(
            who: T::AccountId,
            subject: CaseSubject,
            zk_proof: Option<BoundedVec<u8, ConstU32<4096>>>,
            public_inputs: Option<BoundedVec<[u8; 32], ConstU32<16>>>,
        ) -> DispatchResult {
            ensure!(
                T::CitizenChecker::is_active_citizen(&who),
                Error::<T>::NotActiveCitizen
            );
            // Reject a second open law-targeting case (of *either* kind) against the same
            // law_id — see `OpenCaseByLaw`'s doc comment. Checked before verifying any proof or
            // reserving the bond, matching this file's cheap-checks-first pattern elsewhere.
            let law_case = Self::law_case_kind(&subject);
            if let Some((kind, law_id)) = &law_case {
                ensure!(
                    OpenCaseByLaw::<T>::get(law_id).is_none(),
                    match kind {
                        CaseSubjectKind::LawChallenge => Error::<T>::DuplicateLawChallenge,
                        CaseSubjectKind::TierConflict => Error::<T>::DuplicateTierConflict,
                    }
                );
            }

            // Determine the filer identity to store — see `CaseFiler`'s doc comment for which
            // subjects get which treatment.
            let filer = match &subject {
                CaseSubject::LawChallenge { .. }
                | CaseSubject::TreasuryDispute { .. }
                | CaseSubject::TierConflict { .. } => {
                    let proof = zk_proof.ok_or(Error::<T>::MissingZkProof)?;
                    let inputs = public_inputs.ok_or(Error::<T>::MissingZkProof)?;
                    CaseFiler::Nullifier(Self::verify_and_extract_case_filing_nullifier(
                        &proof, &inputs,
                    )?)
                }
                CaseSubject::CitizenConduct { .. } | CaseSubject::General => {
                    ensure!(
                        zk_proof.is_none() && public_inputs.is_none(),
                        Error::<T>::UnexpectedZkProof
                    );
                    CaseFiler::Account(who.clone())
                }
            };

            let bond = T::CaseFilingBond::get();
            T::Currency::reserve(&who, bond).map_err(|_| Error::<T>::InsufficientBalance)?;
            let id = NextCaseId::<T>::get();
            if let Some((kind, law_id)) = &law_case {
                OpenCaseByLaw::<T>::insert(law_id, (*kind, id));
            }
            Cases::<T>::insert(id, (filer.clone(), CaseStatus::Filed, None::<[u8; 32]>, subject.clone()));
            CaseBonds::<T>::insert(id, bond);
            CaseBondAccount::<T>::insert(id, who);
            NextCaseId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::CaseFiled { case_id: id, filer, subject });
            Ok(())
        }

        /// Verifies a case-filing ZK proof and returns its `scoped_nullifier`. See
        /// `pallet_anticorruption::submit_whistleblower_report`'s identical logic for the full
        /// field-by-field rationale (length check before indexing/before the verifier runs,
        /// scope/subscope domain separation, `len - 2` for the real per-citizen value rather
        /// than `[0]`'s shared registry root).
        fn verify_and_extract_case_filing_nullifier(
            zk_proof: &BoundedVec<u8, ConstU32<4096>>,
            public_inputs: &BoundedVec<[u8; 32], ConstU32<16>>,
        ) -> Result<[u8; 32], DispatchError> {
            ensure!(public_inputs.len() >= MIN_PUBLIC_INPUTS, Error::<T>::MissingNullifierInput);
            ensure!(
                public_inputs[SERVICE_SCOPE_INDEX] == CASE_FILING_SERVICE_SCOPE
                    && public_inputs[SERVICE_SUBSCOPE_INDEX] == CASE_FILING_SERVICE_SUBSCOPE,
                Error::<T>::InvalidProofScope
            );
            ensure!(
                T::ZkVerifier::verify(zk_proof.as_slice(), public_inputs.as_slice()),
                Error::<T>::InvalidZkProof
            );
            Ok(public_inputs[public_inputs.len() - 2])
        }

        /// Mix the hashes of `window_len` blocks starting at `window_start` into a single
        /// 32-byte seed, domain-separated by `case_id`. Every block in the window was
        /// produced strictly after the case's `JuryRequestBlock` (the appeal), so none of
        /// their hashes were computable at commit time — see `JurySeedDelayBlocks`.
        fn anchored_entropy(
            case_id: u32,
            window_start: BlockNumberFor<T>,
            window_len: u32,
        ) -> [u8; 32] {
            let mut entropy = [0u8; 32];
            for offset in 0u32..window_len {
                let n = window_start.saturating_add(BlockNumberFor::<T>::from(offset));
                let h = frame_system::Pallet::<T>::block_hash(n);
                for (i, b) in h.as_ref().iter().enumerate() {
                    entropy[i % 32] ^= b;
                }
            }
            for (i, b) in case_id.to_le_bytes().iter().enumerate() {
                entropy[i % 32] ^= b;
            }
            for (i, b) in b"AGORA_JURY_SEED_V2".iter().enumerate() {
                entropy[(i + 7) % 32] ^= b;
            }
            let out_hash = T::Hashing::hash(&entropy);
            let mut out = [0u8; 32];
            out.copy_from_slice(out_hash.as_ref());
            out
        }

        /// Pick `jury_size` unique citizens at random, using an already-captured delayed-reveal
        /// seed (see `CapturedJurySeed`) — this no longer recomputes `anchored_entropy` itself
        /// from live block-hash storage, so it has no dependency on whether the seed window's
        /// block hashes are still live in `frame_system::BlockHash` at call time.
        ///
        /// Excludes `filer` (the case's own filer has an obvious stake in the outcome) and,
        /// when `defendant_nullifier` is `Some` (a `CaseSubject::CitizenConduct` case), any
        /// candidate whose own registered nullifier matches it — the accused sitting on their
        /// own jury. `defendant_nullifier` is a nullifier rather than an `AccountId` because
        /// that's the only identifier `CaseSubject::CitizenConduct` itself carries for the
        /// accused; comparing via `T::CitizenChecker::citizen_nullifier` avoids needing a
        /// nullifier -> AccountId reverse lookup, which doesn't exist anywhere in this
        /// codebase (`CitizenNullifier` in pallet-identity only maps AccountId -> nullifier).
        ///
        /// `filer` is `CaseFiler<T::AccountId>`, not a bare `AccountId`: for an anonymized case
        /// (`LawChallenge`/`TreasuryDispute`/`TierConflict`) the case only carries the filer's
        /// nullifier, so exclusion for those is done the same nullifier-matching way as the
        /// `defendant_nullifier` check above, not a direct `AccountId` comparison.
        ///
        /// Also excludes any current `OracleMembers`/`AIGovernanceCouncil` member from the
        /// eligible pool. Every case that ever reaches jury selection got here via
        /// `appeal_ruling`, which only fires from `CaseStatus::AIRulingIssued` — i.e. every
        /// jury this function ever seats is reviewing a Level-0 AI ruling, so this exclusion is
        /// unconditional rather than gated on case type or history (there is no jury-eligible
        /// case that *didn't* go through an AI ruling first). `OracleMembers` is the direct
        /// conflict: that council is exactly who proposes/approves the `submit_ai_ruling`/
        /// `finalize_ruling` action for the very ruling under appeal (see `submit_ai_ruling`,
        /// `approve_ai_ruling`). `AIGovernanceCouncil` is included too, as a narrower but still
        /// real institutional stake: that body approves which AI model version produces every
        /// Level-0 ruling system-wide (see `vote_approve_ai_model`), so its members have a
        /// standing interest in AI rulings holding up on appeal even though they didn't approve
        /// this specific ruling. Both memberships are small, root-managed councils, so checking
        /// both costs little and closes the conflict named by both bodies rather than only one.
        fn pick_random_jurors(
            case_id: u32,
            jury_size: u8,
            total: u32,
            raw: [u8; 32],
            filer: CaseFiler<T::AccountId>,
            defendant_nullifier: Option<[u8; 32]>,
        ) -> Result<BoundedVec<T::AccountId, ConstU32<21>>, DispatchError> {
            let mut jurors: BoundedVec<T::AccountId, ConstU32<21>> = BoundedVec::new();
            let mut nonce: u32 = 0;
            let max_attempts = total.saturating_add(jury_size as u32).saturating_mul(3);
            let oracle_members = OracleMembers::<T>::get();
            let ai_governance_members = AIGovernanceCouncil::<T>::get();
            while (jurors.len() as u8) < jury_size && nonce < max_attempts {
                let seed_input = (raw, case_id, nonce).encode();
                let hash = T::Hashing::hash(&seed_input);
                let bytes = hash.as_ref();
                let idx = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % total;
                if let Some(citizen) = T::CitizenSelector::citizen_at(idx) {
                    let is_filer = match &filer {
                        CaseFiler::Account(acct) => citizen == *acct,
                        CaseFiler::Nullifier(n) => {
                            T::CitizenChecker::citizen_nullifier(&citizen) == Some(*n)
                        }
                    };
                    let is_defendant = defendant_nullifier.is_some_and(|nullifier| {
                        T::CitizenChecker::citizen_nullifier(&citizen) == Some(nullifier)
                    });
                    let is_conflicted = oracle_members.contains(&citizen)
                        || ai_governance_members.contains(&citizen);
                    if !is_filer && !is_defendant && !is_conflicted && !jurors.contains(&citizen) {
                        jurors.try_push(citizen).map_err(|_| Error::<T>::InvalidJurySize)?;
                    }
                }
                nonce = nonce.saturating_add(1);
            }
            ensure!(jurors.len() as u8 == jury_size, Error::<T>::NotEnoughCitizens);
            Ok(jurors)
        }

        /// Returns true if `votes` meets the configured AI-model supermajority threshold.
        /// Identical formula to pallet-emergency-council's `supermajority_reached`:
        /// `votes * AIModelSupermajorityDenominator >= council_size * AIModelSupermajorityNumerator`.
        fn ai_model_supermajority_reached(votes: u32, council_size: u32) -> bool {
            if council_size == 0 {
                return false;
            }
            votes.saturating_mul(T::AIModelSupermajorityDenominator::get())
                >= council_size.saturating_mul(T::AIModelSupermajorityNumerator::get())
        }

        /// Returns true if `approvals` meets the configured Oracle Council approval threshold.
        /// Unlike `ai_model_supermajority_reached` (and pallet-emergency-council's
        /// `supermajority_reached`), this uses **strict** `>` — a plain majority means *more*
        /// than half, not "at least half" — see `Config::OracleApprovalNumerator`'s doc comment.
        fn oracle_approval_reached(approvals: u32, council_size: u32) -> bool {
            if council_size == 0 {
                return false;
            }
            approvals.saturating_mul(T::OracleApprovalDenominator::get())
                > council_size.saturating_mul(T::OracleApprovalNumerator::get())
        }

        /// If `case_id`'s pending oracle action has reached its approval threshold, apply it
        /// and clear `PendingOracleProposal`/`OracleApprovals`. Otherwise a no-op (the action
        /// stays pending for more approvals). Called at the end of both `submit_ai_ruling`/
        /// `finalize_ruling` (the proposer's own approval may already be enough for a
        /// small/1-member council) and `approve_ai_ruling`.
        ///
        /// If the threshold is reached but the case's status no longer matches what the
        /// pending action expects — the one legitimate race this can hit: a `Finalization`
        /// proposal whose case was independently appealed by someone else after it was
        /// proposed but before the last approval landed — the stale proposal is dropped (not
        /// applied, not left dangling) and `OracleActionExpired` is emitted instead of erroring
        /// out the approver whose call happened to cross the threshold. A `Submission`
        /// proposal's `Filed` precondition can't actually change out from under it this way
        /// (nothing else moves a case out of `Filed`), but is handled identically for defense
        /// in depth. Every *other* failure (missing verdict, appeal window not yet closed,
        /// case vanished) is a genuine error and is propagated normally via `?` — Substrate's
        /// transactional dispatch means that reverts this whole call's storage writes,
        /// including the `PendingOracleProposal`/`OracleApprovals` entries inserted earlier in
        /// the same call, so nothing is left half-applied.
        fn try_resolve_oracle_action(case_id: u32) -> DispatchResult {
            let approvals = OracleApprovals::<T>::get(case_id).unwrap_or_default();
            let council_size = OracleMembers::<T>::get().len() as u32;
            if !Self::oracle_approval_reached(approvals.len() as u32, council_size) {
                return Ok(());
            }
            let action = match PendingOracleProposal::<T>::get(case_id) {
                Some(a) => a,
                None => return Ok(()),
            };
            let applied = match action {
                PendingOracleAction::Submission { ruling_hash, model_version, verdict } => {
                    let case = Cases::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
                    if case.1 != CaseStatus::Filed {
                        false
                    } else {
                        Cases::<T>::try_mutate(case_id, |maybe_case| {
                            let c = maybe_case.as_mut().ok_or(Error::<T>::CaseNotFound)?;
                            c.1 = CaseStatus::AIRulingIssued;
                            c.2 = Some(ruling_hash);
                            Ok::<(), DispatchError>(())
                        })?;
                        AIRulingBlock::<T>::insert(case_id, frame_system::Pallet::<T>::block_number());
                        AIRulingModelVersion::<T>::insert(case_id, model_version);
                        AIRulingVerdict::<T>::insert(case_id, verdict.clone());
                        Self::deposit_event(Event::AIRulingIssued { case_id, ruling_hash, model_version });
                        true
                    }
                }
                PendingOracleAction::Finalization => {
                    let case = Cases::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
                    if case.1 != CaseStatus::AIRulingIssued {
                        false
                    } else {
                        let ruling_block =
                            AIRulingBlock::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
                        let appeal_deadline = ruling_block
                            .saturating_add(BlockNumberFor::<T>::from(T::AppealWindowBlocks::get()));
                        ensure!(
                            frame_system::Pallet::<T>::block_number() > appeal_deadline,
                            Error::<T>::AppealWindowClosed
                        );
                        let verdict =
                            AIRulingVerdict::<T>::get(case_id).ok_or(Error::<T>::NoRulingVerdict)?;
                        Self::auto_finalize(case_id, verdict)?;
                        true
                    }
                }
            };
            if !applied {
                Self::deposit_event(Event::OracleActionExpired { case_id });
            }
            PendingOracleProposal::<T>::remove(case_id);
            OracleApprovals::<T>::remove(case_id);
            OracleProposalProposedAt::<T>::remove(case_id);
            Ok(())
        }

        /// If `call_hash`'s pending administrative action has reached its approval threshold,
        /// move it from `PendingAdminAction` to `ApprovedAdminAction` so its proposer can consume
        /// it via `EnsureOracleCouncilApproved`. Otherwise a no-op (stays pending for more
        /// approvals). Called at the end of both `propose_admin_action` (the proposer's own
        /// approval may already be enough for a small/1-member council) and
        /// `approve_admin_action`. Mirrors `try_resolve_oracle_action`'s shape for the case-based
        /// flow.
        fn try_resolve_admin_action(call_hash: [u8; 32]) {
            let Some((proposer, approvals)) = PendingAdminAction::<T>::get(call_hash) else {
                return;
            };
            let council_size = OracleMembers::<T>::get().len() as u32;
            if Self::oracle_approval_reached(approvals.len() as u32, council_size) {
                PendingAdminAction::<T>::remove(call_hash);
                let now = frame_system::Pallet::<T>::block_number();
                ApprovedAdminAction::<T>::insert(call_hash, (proposer, now));
                Self::deposit_event(Event::AdminActionApproved { call_hash });
            }
        }
    }
}
