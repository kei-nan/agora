//! # Courts Pallet
//!
//! AI-first court system (Level 0: AI judge, Level 1: 7-person jury, Level 2: 21-person jury).
//! Rulings are auto-enforced: invalidated law -> pallet-constitution pauses it;
//! illegal treasury tx -> pallet-treasury-ledger freezes department.
#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {

    use codec::{Decode, DecodeWithMemTracking, Encode};
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::Hash as HashT;

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    // ── Cross-pallet enforcement traits ─────────────────────────────────────────

    /// Implemented by the runtime to call pallet-identity's citizen index.
    pub trait CitizenSelector<AccountId> {
        fn citizen_at(index: u32) -> Option<AccountId>;
        fn total_citizens() -> u32;
    }

    /// Implemented by the runtime to call pallet-constitution's invalidate_law_internal.
    pub trait LawEnforcer {
        fn invalidate_law(law_id: u32) -> DispatchResult;
    }

    /// Implemented by the runtime to call pallet-treasury-ledger's freeze_department_internal.
    pub trait TreasuryEnforcer {
        fn freeze_department(department_id: u32) -> DispatchResult;
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
        /// Hook called to pause a law when an Overturned verdict is issued.
        type LawEnforcer: LawEnforcer;
        /// Hook called to freeze a department when an Overturned treasury verdict is issued.
        type TreasuryEnforcer: TreasuryEnforcer;
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

    /// Each juror's vote for a case. Only accounts in JuryPool[case_id] may vote.
    #[pallet::storage]
    pub type JuryVotes<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, T::AccountId), Verdict>;

    /// Running tally: case_id -> (upheld_count, overturned_count).
    #[pallet::storage]
    pub type JuryTally<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, (u32, u32), ValueQuery>;

    // ── Events ──────────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        CaseFiled { case_id: u32, filer: T::AccountId, subject: CaseSubject },
        AIRulingIssued { case_id: u32, ruling_hash: [u8; 32] },
        JurySelected { case_id: u32, jurors: BoundedVec<T::AccountId, ConstU32<21>> },
        AppealFiled { case_id: u32, appellant: T::AccountId },
        RulingFinalized { case_id: u32, verdict: Verdict },
        RulingEnforced { case_id: u32 },
        JuryVoteCast { case_id: u32, juror: T::AccountId, verdict: Verdict },
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
    }

    // ── Calls ───────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// File a new case. subject determines what gets auto-enforced on ruling.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn file_case(origin: OriginFor<T>, subject: CaseSubject) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let id = NextCaseId::<T>::get();
            Cases::<T>::insert(id, (who.clone(), CaseStatus::Filed, None::<[u8; 32]>, subject.clone()));
            NextCaseId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::CaseFiled { case_id: id, filer: who, subject });
            Ok(())
        }

        /// Submit an AI ruling. ruling_hash is the IPFS CID of the full reasoning document.
        /// Only callable by the designated AI oracle account (root for now).
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn submit_ai_ruling(
            origin: OriginFor<T>,
            case_id: u32,
            ruling_hash: [u8; 32],
        ) -> DispatchResult {
            ensure_root(origin)?; // TODO: replace with AI oracle origin
            Cases::<T>::try_mutate(case_id, |maybe_case| {
                let case = maybe_case.as_mut().ok_or(Error::<T>::CaseNotFound)?;
                ensure!(case.1 == CaseStatus::Filed, Error::<T>::InvalidStatus);
                case.1 = CaseStatus::AIRulingIssued;
                case.2 = Some(ruling_hash);
                Ok::<(), DispatchError>(())
            })?;
            AIRulingBlock::<T>::insert(case_id, frame_system::Pallet::<T>::block_number());
            Self::deposit_event(Event::AIRulingIssued { case_id, ruling_hash });
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
            let deadline = ruling_block + BlockNumberFor::<T>::from(T::AppealWindowBlocks::get());
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(now <= deadline, Error::<T>::AppealWindowClosed);
            Cases::<T>::try_mutate(case_id, |maybe_case| {
                let case = maybe_case.as_mut().ok_or(Error::<T>::CaseNotFound)?;
                ensure!(case.1 == CaseStatus::AIRulingIssued, Error::<T>::InvalidStatus);
                case.1 = CaseStatus::InJuryAppeal;
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::AppealFiled { case_id, appellant: who });
            Ok(())
        }

        /// Select a jury from the citizen registry using block randomness.
        /// jury_size: 7 for Level 1 appeal, 21 for Level 2 constitutional questions.
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(100_000, 0))]
        pub fn select_jury(origin: OriginFor<T>, case_id: u32, jury_size: u8) -> DispatchResult {
            let _who = ensure_signed(origin)?;
            ensure!(jury_size <= 21, Error::<T>::InvalidJurySize);
            let case = Cases::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            ensure!(case.1 == CaseStatus::InJuryAppeal, Error::<T>::InvalidStatus);
            let total = T::CitizenSelector::total_citizens();
            ensure!(total >= jury_size as u32, Error::<T>::NotEnoughCitizens);
            let jurors = Self::pick_random_jurors(case_id, jury_size, total)?;
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
            ensure_root(origin)?;
            let case = Cases::<T>::get(case_id).ok_or(Error::<T>::CaseNotFound)?;
            ensure!(case.1 == CaseStatus::AIRulingIssued, Error::<T>::InvalidStatus);
            Self::auto_finalize(case_id, verdict)?;
            Ok(())
        }

        /// Cast a jury vote for a case in InJuryAppeal status.
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
            Self::deposit_event(Event::RulingFinalized { case_id, verdict: verdict.clone() });
            // Auto-enforce on Overturned verdicts.
            if verdict == Verdict::Overturned {
                match &case.3 {
                    CaseSubject::LawChallenge { law_id } => {
                        T::LawEnforcer::invalidate_law(*law_id)?;
                    }
                    CaseSubject::TreasuryDispute { department_id } => {
                        T::TreasuryEnforcer::freeze_department(*department_id)?;
                    }
                    CaseSubject::General => {}
                }
            }
            Self::deposit_event(Event::RulingEnforced { case_id });
            Ok(())
        }

        /// Pick `jury_size` unique citizens at random using the parent block hash as entropy.
        /// NOTE: Block hash randomness is manipulable by block authors — production should use
        /// VRF (Babe randomness) or a commit-reveal scheme.
        fn pick_random_jurors(
            case_id: u32,
            jury_size: u8,
            total: u32,
        ) -> Result<BoundedVec<T::AccountId, ConstU32<21>>, DispatchError> {
            let block_hash = frame_system::Pallet::<T>::parent_hash();
            let mut jurors: BoundedVec<T::AccountId, ConstU32<21>> = BoundedVec::new();
            let mut nonce: u32 = 0;
            let max_attempts = total.saturating_add(jury_size as u32).saturating_mul(3);
            while (jurors.len() as u8) < jury_size && nonce < max_attempts {
                let seed_input = (block_hash, case_id, nonce).encode();
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
    }
}
