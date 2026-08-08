//! # Courts Pallet
//!
//! AI-first court system (Level 0: AI judge, Level 1: 7-person jury, Level 2: 21-person jury).
//! Rulings are auto-enforced: invalidated law -> pallet-constitution pauses it;
//! illegal treasury tx -> pallet-treasury-ledger freezes department.
#![cfg_attr(not(feature = "std"), no_std)]
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
    }

    /// Implemented by the runtime to call pallet-constitution's invalidate_law_internal.
    pub trait LawEnforcer {
        fn invalidate_law(law_id: u32) -> DispatchResult;
    }

    /// Implemented by the runtime to call pallet-treasury-ledger's freeze_department_internal.
    pub trait TreasuryEnforcer {
        fn freeze_department(department_id: u32) -> DispatchResult;
    }

    /// Implemented by the runtime to call pallet-identity's suspend_citizen_internal.
    /// `suspension_until` is an **absolute block number** when the suspension lifts
    /// (None = indefinite). The courts pallet computes this from `suspension_blocks` +
    /// `now` before calling this trait — the implementor just passes it through.
    pub trait CitizenSuspender<BlockNumber> {
        fn suspend_citizen(
            nullifier: [u8; 32],
            suspension_until: Option<BlockNumber>,
        ) -> DispatchResult;
    }

    // ── Oracle origin ────────────────────────────────────────────────────────────

    /// Accepts a `Signed` origin only if the signer matches the stored `OracleAccount`.
    /// Returns `Err(origin)` if no oracle account is set or the signer doesn't match.
    /// Governance can rotate the oracle via `set_oracle_account` without a runtime upgrade.
    pub struct EnsureOracle<T>(core::marker::PhantomData<T>);

    impl<T: Config> frame_support::traits::EnsureOrigin<T::RuntimeOrigin> for EnsureOracle<T> {
        type Success = T::AccountId;

        fn try_origin(o: T::RuntimeOrigin) -> Result<Self::Success, T::RuntimeOrigin> {
            use frame_system::RawOrigin;
            let oracle = match OracleAccount::<T>::get() {
                Some(a) => a,
                None => return Err(o),
            };
            match o.clone().into() {
                Ok(RawOrigin::Signed(who)) if who == oracle => Ok(who),
                _ => Err(o),
            }
        }

        #[cfg(feature = "runtime-benchmarks")]
        fn try_successful_origin() -> Result<T::RuntimeOrigin, ()> {
            let oracle = OracleAccount::<T>::get().ok_or(())?;
            Ok(frame_system::RawOrigin::Signed(oracle).into())
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
        /// The origin permitted to submit AI rulings and finalize un-appealed cases.
        /// Configure as EnsureRoot for dev; wire to a dedicated oracle account in production.
        type OracleOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;
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
        /// AccountId used as the filer for system-initiated cases (e.g. auto law challenges).
        /// Wire to a well-known zero account or a dedicated pallet account in the runtime.
        type AutoChallengeAccount: Get<Self::AccountId>;
        /// Currency used to reserve the citizen-filed case bond (`CaseFilingBond`).
        type Currency: ReservableCurrency<Self::AccountId>;
        /// Bond reserved from a citizen's account when they call `file_case`, released in
        /// full once the case reaches a final status (see `auto_finalize`). Mirrors
        /// pallet-elections' `CandidateDeposit` spam-prevention pattern: instant, free
        /// Level-0 AI rulings make `file_case` an attractive DoS/spam vector without a cost
        /// to filing, and this bond gives it one. Defaults to the same 1 AGR pallet-elections
        /// uses for `CandidateDeposit`, absent any documented reason this pallet should differ.
        /// System-initiated filings via `auto_file_case` never reserve this bond — see that
        /// function's doc comment for why.
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
    }

    // ── Storage ─────────────────────────────────────────────────────────────────

    /// case_id -> (filer, status, ruling_ipfs_hash, subject).
    #[pallet::storage]
    pub type Cases<T: Config> =
        StorageMap<_, Blake2_128Concat, u32,
            (T::AccountId, CaseStatus, Option<[u8; 32]>, CaseSubject)>;

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

    /// The designated AI oracle account. Only this account may call `submit_ai_ruling`
    /// and `finalize_ruling`. Set by root via `set_oracle_account`; rotatable without
    /// a runtime upgrade.
    #[pallet::storage]
    pub type OracleAccount<T: Config> = StorageValue<_, T::AccountId, OptionQuery>;

    /// case_id -> bond reserved from the filer by `file_case`. Only ever populated for
    /// citizen-filed cases — `auto_file_case` (system-initiated) never inserts an entry here,
    /// since there's no spam risk to price against for a filing the runtime itself triggers.
    /// Taken (removed) and unreserved in full by `auto_finalize` once the case resolves.
    #[pallet::storage]
    pub type CaseBonds<T: Config> = StorageMap<_, Blake2_128Concat, u32, BalanceOf<T>>;
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

    // ── Events ──────────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        CaseFiled { case_id: u32, filer: T::AccountId, subject: CaseSubject },
        AIRulingIssued { case_id: u32, ruling_hash: [u8; 32], model_version: u32 },
        JurySelected { case_id: u32, jurors: BoundedVec<T::AccountId, ConstU32<21>> },
        AppealFiled { case_id: u32, appellant: T::AccountId },
        RulingFinalized { case_id: u32, verdict: Verdict },
        /// Emitted only when an Overturned verdict triggers actual on-chain enforcement
        /// (law paused, department frozen, or citizen suspended).
        RulingEnforced { case_id: u32 },
        JuryVoteCast { case_id: u32, juror: T::AccountId, verdict: Verdict },
        OracleAccountSet { account: T::AccountId },
        /// A new member was added to the AI Model Governance Council.
        AIGovernanceMemberAdded { who: T::AccountId },
        /// A member was removed from the AI Model Governance Council.
        AIGovernanceMemberRemoved { who: T::AccountId },
        /// A new AI model version was approved by supermajority vote.
        AIModelApproved { version: u32, model_hash: [u8; 32] },
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
    }

    // ── Calls ───────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// File a new case. Only active (non-suspended) citizens may file.
        /// subject determines what gets auto-enforced on ruling.
        ///
        /// Reserves `CaseFilingBond` from the filer as a spam-prevention deposit — instant,
        /// free Level-0 AI rulings would otherwise make this call a cheap DoS vector against
        /// a court system meant to carry real judicial weight. The bond is released in full
        /// once the case reaches a final status; see `auto_finalize`.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn file_case(origin: OriginFor<T>, subject: CaseSubject) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(
                T::CitizenChecker::is_active_citizen(&who),
                Error::<T>::NotActiveCitizen
            );
            let bond = T::CaseFilingBond::get();
            T::Currency::reserve(&who, bond).map_err(|_| Error::<T>::InsufficientBalance)?;
            let id = NextCaseId::<T>::get();
            Cases::<T>::insert(id, (who.clone(), CaseStatus::Filed, None::<[u8; 32]>, subject.clone()));
            CaseBonds::<T>::insert(id, bond);
            NextCaseId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::CaseFiled { case_id: id, filer: who, subject });
            Ok(())
        }

        /// Submit an AI ruling. ruling_hash is the IPFS CID of the full reasoning document.
        ///
        /// `model_version` must match `CurrentAIModelVersion` — this is the actual on-chain
        /// enforcement of CLAUDE.md's "AI model updates require on-chain governance vote
        /// (supermajority)" claim: a ruling can only be attributed to a model version that
        /// has actually been approved via `vote_approve_ai_model`. Rejected with
        /// `Error::NoApprovedAIModel` if no model has ever been approved, or
        /// `Error::UnapprovedAIModel` if `model_version` doesn't match the current one
        /// (including a stale, previously-approved-but-since-superseded version).
        ///
        /// Only callable by the designated AI oracle account (root for now).
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn submit_ai_ruling(
            origin: OriginFor<T>,
            case_id: u32,
            ruling_hash: [u8; 32],
            model_version: u32,
        ) -> DispatchResult {
            T::OracleOrigin::ensure_origin(origin)?;
            let current_model_version = CurrentAIModelVersion::<T>::get();
            ensure!(current_model_version != 0, Error::<T>::NoApprovedAIModel);
            ensure!(model_version == current_model_version, Error::<T>::UnapprovedAIModel);
            Cases::<T>::try_mutate(case_id, |maybe_case| {
                let case = maybe_case.as_mut().ok_or(Error::<T>::CaseNotFound)?;
                ensure!(case.1 == CaseStatus::Filed, Error::<T>::InvalidStatus);
                case.1 = CaseStatus::AIRulingIssued;
                case.2 = Some(ruling_hash);
                Ok::<(), DispatchError>(())
            })?;
            AIRulingBlock::<T>::insert(case_id, frame_system::Pallet::<T>::block_number());
            AIRulingModelVersion::<T>::insert(case_id, model_version);
            Self::deposit_event(Event::AIRulingIssued { case_id, ruling_hash, model_version });
            Ok(())
        }

        /// Appeal an AI ruling within the appeal window. Triggers jury selection.
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
            Cases::<T>::try_mutate(case_id, |maybe_case| {
                let case = maybe_case.as_mut().ok_or(Error::<T>::CaseNotFound)?;
                ensure!(case.1 == CaseStatus::AIRulingIssued, Error::<T>::InvalidStatus);
                case.1 = CaseStatus::InJuryAppeal;
                Ok::<(), DispatchError>(())
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
            let oracle_ok = OracleAccount::<T>::get().map_or(false, |o| o == who);
            let system_case = case.0 == T::AutoChallengeAccount::get();
            let authorized = who == case.0
                || oracle_ok
                || (system_case && T::CitizenChecker::is_active_citizen(&who));
            ensure!(authorized, Error::<T>::NotAuthorized);
            // Derive the required jury size from the case subject so that the
            // routing logic (Level 1 vs Level 2) is enforced on-chain rather than
            // relying on the caller to pass the correct value.
            let required_size: u8 = match &case.3 {
                CaseSubject::LawChallenge { .. } => 21,
                _ => 7,
            };
            ensure!(jury_size == required_size, Error::<T>::InvalidJurySize);
            let total = T::CitizenSelector::total_citizens();
            ensure!(total >= required_size as u32, Error::<T>::NotEnoughCitizens);
            // Delayed-reveal seed window: must be fully elapsed (all its block hashes fixed
            // in history) before we can derive the jury seed from it. See
            // `JurySeedDelayBlocks` doc comment for why this window is anchored to the
            // appeal block rather than "now".
            let request_block =
                JuryRequestBlock::<T>::get(case_id).ok_or(Error::<T>::JurySeedNotReady)?;
            let window_start = request_block.saturating_add(BlockNumberFor::<T>::from(1u32));
            let delay = T::JurySeedDelayBlocks::get();
            let window_end = request_block.saturating_add(BlockNumberFor::<T>::from(delay));
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(now > window_end, Error::<T>::JurySeedNotReady);
            let jurors = Self::pick_random_jurors(case_id, required_size, total, window_start, delay)?;
            Self::deposit_event(Event::JurySelected { case_id, jurors: jurors.clone() });
            JuryPool::<T>::insert(case_id, jurors);
            // Advance status so a second select_jury call is rejected.
            Cases::<T>::try_mutate(case_id, |maybe_case| {
                let c = maybe_case.as_mut().ok_or(Error::<T>::CaseNotFound)?;
                c.1 = CaseStatus::JurySeated;
                Ok::<(), DispatchError>(())
            })?;
            Ok(())
        }

        /// Finalize a ruling for the no-appeal path (AI ruling expires without appeal).
        /// Only callable when status is AIRulingIssued (InJuryAppeal cases auto-finalize via jury).
        /// Automatically enforces: pauses laws, freezes treasury departments.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn finalize_ruling(
            origin: OriginFor<T>,
            case_id: u32,
            verdict: Verdict,
        ) -> DispatchResult {
            T::OracleOrigin::ensure_origin(origin)?;
            let case = Cases::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            ensure!(case.1 == CaseStatus::AIRulingIssued, Error::<T>::InvalidStatus);
            let ruling_block = AIRulingBlock::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            let appeal_deadline = ruling_block
                .saturating_add(BlockNumberFor::<T>::from(T::AppealWindowBlocks::get()));
            ensure!(
                frame_system::Pallet::<T>::block_number() > appeal_deadline,
                Error::<T>::AppealWindowClosed
            );
            Self::auto_finalize(case_id, verdict)?;
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

        /// Set the designated AI oracle account. Only root may call this.
        /// After this call, `submit_ai_ruling` and `finalize_ruling` require the oracle's signature.
        #[pallet::call_index(6)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn set_oracle_account(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            OracleAccount::<T>::put(account.clone());
            Self::deposit_event(Event::OracleAccountSet { account });
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
    }

    // ── Helpers ─────────────────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
        /// Shared finalization logic used by both `finalize_ruling` (root / AI path)
        /// and `cast_jury_vote` (automatic majority path).
        fn auto_finalize(case_id: u32, verdict: Verdict) -> DispatchResult {
            // Fetch the subject before mutating status, so we can do enforcement after.
            let case = Cases::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            ensure!(case.1 != CaseStatus::FinalRuling, Error::<T>::MajorityAlreadyReached);
            Cases::<T>::try_mutate(case_id, |maybe_case| {
                let c = maybe_case.as_mut().ok_or(Error::<T>::CaseNotFound)?;
                c.1 = CaseStatus::FinalRuling;
                Ok::<(), DispatchError>(())
            })?;
            Rulings::<T>::insert(case_id, verdict.clone());
            // Release the filing bond, if any, now that the case has reached a final status.
            // Always released in full regardless of verdict — there's no precedent elsewhere
            // in this pallet for slashing on a "bad-faith" outcome, and inventing one here
            // would be guessing at policy rather than following an established pattern (see
            // the bond-and-release design note on `Config::CaseFilingBond`). Absent for
            // system-filed cases (`auto_file_case` never inserts a `CaseBonds` entry), so this
            // is a no-op for those — `take` just returns `None`.
            if let Some(bond) = CaseBonds::<T>::take(case_id) {
                T::Currency::unreserve(&case.0, bond);
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
                        T::CitizenSuspender::suspend_citizen(*nullifier, until)?;
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
            let filer = T::AutoChallengeAccount::get();
            let id = NextCaseId::<T>::get();
            Cases::<T>::insert(id, (filer.clone(), CaseStatus::Filed, None::<[u8; 32]>, subject.clone()));
            NextCaseId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::CaseFiled { case_id: id, filer, subject });
            Ok(())
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

        /// Pick `jury_size` unique citizens at random, using the delayed-reveal seed
        /// derived from `[window_start, window_start + window_len)`.
        fn pick_random_jurors(
            case_id: u32,
            jury_size: u8,
            total: u32,
            window_start: BlockNumberFor<T>,
            window_len: u32,
        ) -> Result<BoundedVec<T::AccountId, ConstU32<21>>, DispatchError> {
            let raw = Self::anchored_entropy(case_id, window_start, window_len);
            let mut jurors: BoundedVec<T::AccountId, ConstU32<21>> = BoundedVec::new();
            let mut nonce: u32 = 0;
            let max_attempts = total.saturating_add(jury_size as u32).saturating_mul(3);
            while (jurors.len() as u8) < jury_size && nonce < max_attempts {
                let seed_input = (raw, case_id, nonce).encode();
                let hash = T::Hashing::hash(&seed_input);
                let bytes = hash.as_ref();
                let idx = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % total;
                if let Some(citizen) = T::CitizenSelector::citizen_at(idx) {
                    if !jurors.contains(&citizen) {
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
    }
}
