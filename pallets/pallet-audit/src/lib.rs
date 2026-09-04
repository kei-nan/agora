//! # Audit Office Pallet
//!
//! Implements the Audit Office component of Agora's separation-of-powers framework.
//!
//! Every expenditure recorded by `pallet-treasury-ledger` triggers `AuditHook::on_expenditure`,
//! which inserts a new `AuditEntry` with status `Pending`. Designated auditors can then clear,
//! flag, or escalate entries to `Disputed`. Periodic audit reports are submitted on-chain as
//! IPFS hashes.
//!
//! ## Audit lifecycle
//! ```text
//! Pending → Cleared             (auditor finds it compliant)
//! Pending → Flagged             (auditor finds it irregular; reason stored)
//! Flagged → Disputed            (escalated to formal dispute)
//! Flagged/Disputed → Cleared    (resolved in the department's favor, via resolve_entry)
//! ```
//! Only these transitions are valid. Notably:
//! - `Disputed` cannot be downgraded back to `Flagged`.
//! - `Pending` cannot jump directly to `Disputed` (must go through `Flagged`).
//!
//! ## Treasury enforcement
//! Flagging or disputing an entry (`flag_entry` / a Flagged entry escalated by
//! `dispute_entry`) freezes that expenditure's department in `pallet-treasury-ledger` via
//! `T::TreasuryFreezer` — further `record_expenditure` calls for that department fail while
//! frozen. Each department tracks its own open-flag count (`OpenFlags`): the department stays
//! frozen as long as at least one Flagged/Disputed entry against it remains unresolved, and is
//! only unfrozen once `resolve_entry` clears the last one.
#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {
    use codec::{Decode, DecodeWithMemTracking, Encode, MaxEncodedLen};
    use frame_support::pallet_prelude::*;
    use frame_support::traits::EnsureOriginWithArg;
    use frame_system::pallet_prelude::*;
    use scale_info::TypeInfo;

    // ── Types ──────────────────────────────────────────────────────────────────

    /// Audit lifecycle for a single treasury expenditure.
    #[derive(Clone, Encode, Decode, MaxEncodedLen, TypeInfo, DecodeWithMemTracking, PartialEq)]
    pub enum AuditStatus {
        /// Newly recorded; awaiting auditor review.
        Pending,
        /// Reviewed and found compliant.
        Cleared,
        /// Flagged as potentially irregular; reason document stored on IPFS.
        Flagged,
        /// Escalated to formal dispute. Cannot be downgraded back to Flagged.
        Disputed,
    }

    /// A single audit record mirroring one treasury expenditure.
    #[derive(Clone, Encode, Decode, MaxEncodedLen, TypeInfo, DecodeWithMemTracking)]
    pub struct AuditEntry<AccountId> {
        pub dept_id: u32,
        pub amount: u128,
        pub ipfs_hash: [u8; 32],
        pub status: AuditStatus,
        /// IPFS hash of the flag/dispute reason document (set when Flagged or Disputed).
        pub flag_reason: Option<[u8; 32]>,
        /// Account that flagged the entry.
        pub flagged_by: Option<AccountId>,
    }

    // ── Cross-pallet enforcement trait ────────────────────────────────────────────

    /// Implemented by the runtime to call pallet-treasury-ledger's
    /// `freeze_department_internal` / `unfreeze_department_internal`. Mirrors the
    /// `TreasuryEnforcer` trait pallet-courts uses for the same purpose.
    pub trait TreasuryFreezer {
        fn freeze_department(department_id: u32) -> DispatchResult;
        fn unfreeze_department(department_id: u32) -> DispatchResult;
    }

    // ── Pallet ─────────────────────────────────────────────────────────────────

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Maximum number of registered auditors.
        #[pallet::constant]
        type MaxAuditors: Get<u32>;
        /// Freezes/unfreezes a department in pallet-treasury-ledger when this pallet opens
        /// or fully resolves flags/disputes against it.
        type TreasuryFreezer: TreasuryFreezer;
        /// Origin required to add/remove auditors. `add_auditor`/`remove_auditor` used to be
        /// bare `ensure_root`, which routed appointment through whoever holds `Root` with no
        /// dedicated oversight body at all — see
        /// `pallet_accountability_council`'s module doc comment for why that's a problem
        /// (self-oversight: the branch that controls the treasury must not also pick its own
        /// auditors) and why a separate, independent Accountability Council exists to fix it.
        /// Wire this to `pallet_accountability_council::EnsureAccountabilityCouncilApproved`
        /// in production (requires that Council's genuine 2/3 supermajority for the exact
        /// call — see `add_auditor`/`remove_auditor`'s use of
        /// `pallet_accountability_council::accountability_call_hash` below); kept generic
        /// over `EnsureOriginWithArg<Self::RuntimeOrigin, [u8; 32]>` rather than depending on
        /// that concrete type, the same way `pallet_constitution::Config::CourtOrigin` is
        /// generic over `pallet_courts::EnsureOracleCouncilApproved` — call-hash binding is
        /// what's required here, not the specific pallet.
        type AppointmentOrigin: EnsureOriginWithArg<Self::RuntimeOrigin, [u8; 32]>;
    }

    // ── Storage ────────────────────────────────────────────────────────────────

    /// Audit entries indexed by the treasury expenditure index (u64 matching the ledger counter).
    #[pallet::storage]
    pub type AuditLog<T: Config> =
        StorageMap<_, Blake2_128Concat, u64, AuditEntry<T::AccountId>>;

    /// Secondary index over `AuditLog`, keyed `(dept_id, expenditure_index) -> ()`. Mirrors
    /// `pallet_treasury_ledger::DepartmentExpenditures` exactly — same rationale (`AuditLog` is
    /// keyed by the global expenditure index, not by department, so a department-scoped read
    /// needs its own index rather than a full-log scan), same "value is `()`, only the keys
    /// matter" shape. Populated in `on_expenditure` below, the only place `AuditLog` entries are
    /// ever created, so the two storage items can never drift apart.
    #[pallet::storage]
    pub type DepartmentAuditEntries<T: Config> =
        StorageDoubleMap<_, Blake2_128Concat, u32, Blake2_128Concat, u64, (), OptionQuery>;

    /// The current set of registered auditors.
    #[pallet::storage]
    pub type Auditors<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxAuditors>, ValueQuery>;

    /// Count of currently-open (Flagged or Disputed) entries per department. Drives the
    /// `TreasuryFreezer` calls: a department is frozen while this is nonzero, and unfrozen
    /// only when it drops back to zero — so one department with multiple open flags/disputes
    /// stays frozen until every one of them is resolved.
    #[pallet::storage]
    pub type OpenFlags<T: Config> = StorageMap<_, Blake2_128Concat, u32, u32, ValueQuery>;

    // ── Events ─────────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// A new auditor was added.
        AuditorAdded { who: T::AccountId },
        /// An auditor was removed.
        AuditorRemoved { who: T::AccountId },
        /// An expenditure entry was cleared as compliant.
        EntryCleared { index: u64 },
        /// An expenditure entry was flagged as irregular.
        EntryFlagged { index: u64, by: T::AccountId, reason: [u8; 32] },
        /// An expenditure entry was escalated to disputed status.
        EntryDisputed { index: u64 },
        /// A previously Flagged or Disputed entry was resolved (in the department's favor)
        /// and marked Cleared.
        EntryResolved { index: u64, by: T::AccountId },
        /// An audit report covering a time period was submitted on-chain.
        AuditReportSubmitted { period_hash: [u8; 32] },
    }

    // ── Errors ─────────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        /// Caller is not a registered auditor.
        NotAuditor,
        /// No audit entry exists for the given expenditure index.
        EntryNotFound,
        /// Entry has already been certified (Cleared) and cannot be modified.
        AlreadyCertified,
        /// Auditor list is full.
        TooManyAuditors,
        /// Account is already a registered auditor.
        AlreadyAuditor,
        /// dispute_entry requires the entry to be Flagged first.
        MustBeFlaggedFirst,
        /// flag_entry cannot re-flag an already-flagged entry.
        EntryAlreadyFlagged,
        /// flag_entry cannot downgrade a Disputed entry.
        EntryAlreadyDisputed,
        /// resolve_entry requires the entry to currently be Flagged or Disputed.
        EntryNotOpen,
        /// resolve_entry cannot be called by the same auditor who flagged the entry —
        /// resolution requires a second, different auditor.
        CannotResolveOwnFlag,
    }

    // ── Calls ──────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Add an auditor to the registry. Requires `AppointmentOrigin` — in production, the
        /// Accountability Council's own 2/3 supermajority approval for this exact call (see
        /// `Config::AppointmentOrigin`'s doc comment), not bare `Root`.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn add_auditor(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            T::AppointmentOrigin::ensure_origin(
                origin,
                &pallet_accountability_council::accountability_call_hash(
                    b"pallet-audit::add_auditor",
                    &account,
                ),
            )?;
            Auditors::<T>::try_mutate(|auditors| {
                ensure!(!auditors.contains(&account), Error::<T>::AlreadyAuditor);
                auditors.try_push(account.clone()).map_err(|_| Error::<T>::TooManyAuditors)
            })?;
            Self::deposit_event(Event::AuditorAdded { who: account });
            Ok(())
        }

        /// Remove an auditor from the registry. Same `AppointmentOrigin` gate as `add_auditor`.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn remove_auditor(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            T::AppointmentOrigin::ensure_origin(
                origin,
                &pallet_accountability_council::accountability_call_hash(
                    b"pallet-audit::remove_auditor",
                    &account,
                ),
            )?;
            Auditors::<T>::try_mutate(|auditors| {
                let pos = auditors
                    .iter()
                    .position(|a| a == &account)
                    .ok_or(Error::<T>::NotAuditor)?;
                auditors.remove(pos);
                Ok::<_, DispatchError>(())
            })?;
            Self::deposit_event(Event::AuditorRemoved { who: account });
            Ok(())
        }

        /// Mark an expenditure entry as Cleared. Auditor only.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn clear_entry(origin: OriginFor<T>, expenditure_index: u64) -> DispatchResult {
            let _who = Self::ensure_auditor(origin)?;
            AuditLog::<T>::try_mutate(expenditure_index, |maybe_entry| {
                let entry = maybe_entry.as_mut().ok_or(Error::<T>::EntryNotFound)?;
                // Only Pending→Cleared is a valid transition. Flagged and Disputed entries
                // are in active review and must not be silently cleared.
                ensure!(entry.status == AuditStatus::Pending, Error::<T>::AlreadyCertified);
                entry.status = AuditStatus::Cleared;
                Ok::<_, DispatchError>(())
            })?;
            Self::deposit_event(Event::EntryCleared { index: expenditure_index });
            Ok(())
        }

        /// Flag an expenditure entry with a reason document (IPFS hash). Auditor only.
        /// Only Pending entries may be flagged — Disputed entries cannot be downgraded.
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn flag_entry(
            origin: OriginFor<T>,
            expenditure_index: u64,
            reason_hash: [u8; 32],
        ) -> DispatchResult {
            let who = Self::ensure_auditor(origin)?;
            let dept_id = AuditLog::<T>::try_mutate(expenditure_index, |maybe_entry| {
                let entry = maybe_entry.as_mut().ok_or(Error::<T>::EntryNotFound)?;
                match entry.status {
                    AuditStatus::Pending => {}
                    AuditStatus::Cleared => return Err(Error::<T>::AlreadyCertified.into()),
                    AuditStatus::Flagged => return Err(Error::<T>::EntryAlreadyFlagged.into()),
                    AuditStatus::Disputed => return Err(Error::<T>::EntryAlreadyDisputed.into()),
                }
                entry.status = AuditStatus::Flagged;
                entry.flag_reason = Some(reason_hash);
                entry.flagged_by = Some(who.clone());
                Ok::<_, DispatchError>(entry.dept_id)
            })?;
            // First open flag/dispute against this department freezes it in
            // pallet-treasury-ledger. Further flags on the same department (or other
            // entries in the same department) just add to the open count.
            let open = OpenFlags::<T>::mutate(dept_id, |count| {
                *count = count.saturating_add(1);
                *count
            });
            if open == 1 {
                T::TreasuryFreezer::freeze_department(dept_id)?;
            }
            Self::deposit_event(Event::EntryFlagged {
                index: expenditure_index,
                by: who,
                reason: reason_hash,
            });
            Ok(())
        }

        /// Escalate a Flagged entry to Disputed status. Auditor only.
        /// Requires the entry to be in Flagged state — cannot skip from Pending to Disputed.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn dispute_entry(origin: OriginFor<T>, expenditure_index: u64) -> DispatchResult {
            let _who = Self::ensure_auditor(origin)?;
            AuditLog::<T>::try_mutate(expenditure_index, |maybe_entry| {
                let entry = maybe_entry.as_mut().ok_or(Error::<T>::EntryNotFound)?;
                ensure!(entry.status != AuditStatus::Cleared, Error::<T>::AlreadyCertified);
                ensure!(entry.status == AuditStatus::Flagged, Error::<T>::MustBeFlaggedFirst);
                entry.status = AuditStatus::Disputed;
                Ok::<_, DispatchError>(())
            })?;
            Self::deposit_event(Event::EntryDisputed { index: expenditure_index });
            Ok(())
        }

        /// Resolve a Flagged or Disputed entry in the department's favor, marking it
        /// Cleared. Auditor only. Decrements the department's open-flag count; once that
        /// count reaches zero (no other open flags/disputes remain against the department),
        /// the department is unfrozen in pallet-treasury-ledger. If other flags/disputes
        /// are still open against the same department, it stays frozen.
        ///
        /// The resolving auditor must differ from the auditor who flagged the entry
        /// (`flagged_by`) — otherwise a single auditor could flag then immediately clear
        /// their own flag, defeating the point of the check. Mirrors the
        /// `SameInvestigator` check in `pallet_anticorruption::approve_report_action`.
        #[pallet::call_index(6)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn resolve_entry(origin: OriginFor<T>, expenditure_index: u64) -> DispatchResult {
            let who = Self::ensure_auditor(origin)?;
            let dept_id = AuditLog::<T>::try_mutate(expenditure_index, |maybe_entry| {
                let entry = maybe_entry.as_mut().ok_or(Error::<T>::EntryNotFound)?;
                ensure!(
                    entry.status == AuditStatus::Flagged || entry.status == AuditStatus::Disputed,
                    Error::<T>::EntryNotOpen
                );
                if let Some(flagged_by) = entry.flagged_by.as_ref() {
                    ensure!(flagged_by != &who, Error::<T>::CannotResolveOwnFlag);
                }
                entry.status = AuditStatus::Cleared;
                Ok::<_, DispatchError>(entry.dept_id)
            })?;
            let open = OpenFlags::<T>::mutate(dept_id, |count| {
                *count = count.saturating_sub(1);
                *count
            });
            if open == 0 {
                T::TreasuryFreezer::unfreeze_department(dept_id)?;
            }
            Self::deposit_event(Event::EntryResolved { index: expenditure_index, by: who });
            Ok(())
        }

        /// Submit a periodic audit report (IPFS hash of the full report). Auditor only.
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn submit_audit_report(
            origin: OriginFor<T>,
            period_hash: [u8; 32],
        ) -> DispatchResult {
            let _who = Self::ensure_auditor(origin)?;
            Self::deposit_event(Event::AuditReportSubmitted { period_hash });
            Ok(())
        }
    }

    // ── Internal helpers ───────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
        /// Verify the caller is a registered auditor; return their AccountId.
        fn ensure_auditor(origin: OriginFor<T>) -> Result<T::AccountId, DispatchError> {
            let who = ensure_signed(origin)?;
            let auditors = Auditors::<T>::get();
            ensure!(auditors.contains(&who), Error::<T>::NotAuditor);
            Ok(who)
        }
    }
}

// ── AuditHook implementation ───────────────────────────────────────────────────

impl<T: Config> pallet_treasury_ledger::AuditHook for Pallet<T> {
    fn on_expenditure(index: u64, dept_id: u32, amount: u128, ipfs_hash: [u8; 32]) {
        // In real chain operation `index` is `pallet-treasury-ledger`'s own monotonic counter,
        // so this hook only ever fires once per index and this branch never runs. It exists so
        // the index stays consistent even in the hypothetical (test-only, see
        // `on_expenditure_overwrites_existing_index`) case of the hook firing twice for the same
        // index under a different department — without this, the stale `(old_dept, index)`
        // entry in `DepartmentAuditEntries` would linger forever, pointing at an index whose
        // `AuditLog` entry no longer belongs to that department.
        if let Some(old) = pallet::AuditLog::<T>::get(index) {
            if old.dept_id != dept_id {
                pallet::DepartmentAuditEntries::<T>::remove(old.dept_id, index);
            }
        }
        let entry = pallet::AuditEntry {
            dept_id,
            amount,
            ipfs_hash,
            status: pallet::AuditStatus::Pending,
            flag_reason: None,
            flagged_by: None,
        };
        pallet::AuditLog::<T>::insert(index, entry);
        pallet::DepartmentAuditEntries::<T>::insert(dept_id, index, ());
    }
}
