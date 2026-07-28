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
//! Pending → Cleared   (auditor finds it compliant)
//! Pending → Flagged   (auditor finds it irregular; reason stored)
//! Flagged → Disputed  (escalated to formal dispute)
//! ```
//! Only these transitions are valid. Notably:
//! - `Disputed` cannot be downgraded back to `Flagged`.
//! - `Pending` cannot jump directly to `Disputed` (must go through `Flagged`).
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

    // ── Pallet ─────────────────────────────────────────────────────────────────

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Maximum number of registered auditors.
        #[pallet::constant]
        type MaxAuditors: Get<u32>;
    }

    // ── Storage ────────────────────────────────────────────────────────────────

    /// Audit entries indexed by the treasury expenditure index (u64 matching the ledger counter).
    #[pallet::storage]
    pub type AuditLog<T: Config> =
        StorageMap<_, Blake2_128Concat, u64, AuditEntry<T::AccountId>>;

    /// The current set of registered auditors.
    #[pallet::storage]
    pub type Auditors<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxAuditors>, ValueQuery>;

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
    }

    // ── Calls ──────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Add an auditor to the registry. Root only.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn add_auditor(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            Auditors::<T>::try_mutate(|auditors| {
                ensure!(!auditors.contains(&account), Error::<T>::AlreadyAuditor);
                auditors.try_push(account.clone()).map_err(|_| Error::<T>::TooManyAuditors)
            })?;
            Self::deposit_event(Event::AuditorAdded { who: account });
            Ok(())
        }

        /// Remove an auditor from the registry. Root only.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn remove_auditor(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
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
            AuditLog::<T>::try_mutate(expenditure_index, |maybe_entry| {
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
                Ok::<_, DispatchError>(())
            })?;
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
        let entry = pallet::AuditEntry {
            dept_id,
            amount,
            ipfs_hash,
            status: pallet::AuditStatus::Pending,
            flag_reason: None,
            flagged_by: None,
        };
        pallet::AuditLog::<T>::insert(index, entry);
    }
}
