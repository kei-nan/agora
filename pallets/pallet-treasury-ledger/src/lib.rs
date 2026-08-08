//! # Treasury Ledger Pallet
//!
//! Real-time public budget ledger adapted from the Polkadot OpenGov treasury pattern.
//! Per-department spend caps enforced on every transaction; every expenditure is tagged
//! with source metadata and triggers audit hooks.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// Called after every recorded expenditure. Implement to maintain an audit trail.
/// `index` is the u64 expenditure counter — using u64 avoids truncation for chains
/// that record billions of expenditures over their lifetime.
pub trait AuditHook {
    fn on_expenditure(index: u64, dept_id: u32, amount: u128, ipfs_hash: [u8; 32]);
}

/// No-op implementation for tests or when audit is disabled.
pub struct NoopAuditHook;
impl AuditHook for NoopAuditHook {
    fn on_expenditure(_index: u64, _dept_id: u32, _amount: u128, _ipfs_hash: [u8; 32]) {}
}

#[frame_support::pallet]
pub mod pallet {

    use crate::AuditHook as _;
    use frame_support::pallet_prelude::*;
    use frame_support::traits::EnsureOriginWithArg;
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::CheckedAdd;

    /// Computes the domain-separated hash a legislature motion's `call_hash` must equal for
    /// `LegislatureOrigin` to authorize `tag`'s call with `params`. See
    /// `pallet_constitution::pallet::legislature_call_hash` for the full rationale — this is
    /// the same scheme, duplicated per-pallet so each pallet's Config stays decoupled from
    /// any one other pallet's crate.
    pub(crate) fn legislature_call_hash(tag: &'static [u8], params: impl Encode) -> [u8; 32] {
        let mut preimage = alloc::vec::Vec::from(tag);
        preimage.extend(params.encode());
        frame_support::Hashable::blake2_256(&preimage)
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type Balance: Member
            + Parameter
            + Default
            + Copy
            + MaxEncodedLen
            + PartialOrd
            + CheckedAdd
            + codec::HasCompact
            + scale_info::TypeInfo
            + Into<u128>;
        /// Hook called after every expenditure is recorded.
        type AuditHook: crate::AuditHook;
        /// Origin permitted to allocate department budgets and reset spend counters.
        /// Must be wired to the legislature motion origin so that budget grants require
        /// a passed legislature vote — enforcing the executive/legislature separation.
        /// `EnsureOriginWithArg` so each call site must pass the domain-separated hash of
        /// its own parameters (see `legislature_call_hash`); the origin then verifies that
        /// hash against the motion's approved `call_hash`, so a motion passed to authorize
        /// one call can never be replayed to execute another.
        type LegislatureOrigin: frame_support::traits::EnsureOriginWithArg<Self::RuntimeOrigin, [u8; 32]>;
    }

    /// Department id -> allocated budget (in base units).
    #[pallet::storage]
    pub type DepartmentBudgets<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, T::Balance, ValueQuery>;

    /// Department id -> amount spent this period.
    #[pallet::storage]
    pub type DepartmentSpent<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, T::Balance, ValueQuery>;

    /// Expenditure log: monotonic index -> (department, amount, ipfs_metadata_hash).
    #[pallet::storage]
    pub type ExpenditureLog<T: Config> =
        StorageMap<_, Blake2_128Concat, u64, (u32, T::Balance, [u8; 32])>;

    #[pallet::storage]
    pub type NextExpenditureIndex<T: Config> = StorageValue<_, u64, ValueQuery>;

    /// Departments frozen by court ruling — no expenditures allowed while frozen.
    #[pallet::storage]
    pub type FrozenDepartments<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, bool, ValueQuery>;

    /// Per-department authorized spender. Only this account may call record_expenditure for that department.
    #[pallet::storage]
    pub type DepartmentSpenders<T: Config> =
        StorageMap<_, Twox64Concat, u32, T::AccountId, OptionQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        BudgetAllocated { department_id: u32, amount: T::Balance },
        FundsSpent { department_id: u32, amount: T::Balance, metadata_hash: [u8; 32] },
        DepartmentFrozen { department_id: u32 },
        DepartmentUnfrozen { department_id: u32 },
        SpenderRegistered { department_id: u32, spender: T::AccountId },
        SpenderRemoved { department_id: u32 },
    }

    #[pallet::error]
    pub enum Error<T> {
        InsufficientBudget,
        DepartmentNotFound,
        DepartmentFrozen,
        DepartmentNotFrozen,
        Overflow,
        NotAuthorizedSpender,
        DepartmentHasNoSpender,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Allocate a budget to a department (legislature-approved).
        /// Note: calling this twice for the same department replaces the prior allocation.
        /// A supplemental appropriation should pass the new total, not an addend.
        /// `DepartmentSpent` is NOT reset on re-allocation — call reset_department_spent
        /// explicitly at the start of a new fiscal period.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn allocate_budget(
            origin: OriginFor<T>,
            department_id: u32,
            amount: T::Balance,
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-treasury-ledger::allocate_budget", (department_id, amount)),
            )?;
            DepartmentBudgets::<T>::insert(department_id, amount);
            Self::deposit_event(Event::BudgetAllocated { department_id, amount });
            Ok(())
        }

        /// Reset the accumulated spend counter for a department to zero.
        /// Call at the start of each fiscal period after allocating the new budget.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn reset_department_spent(
            origin: OriginFor<T>,
            department_id: u32,
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-treasury-ledger::reset_department_spent", department_id),
            )?;
            DepartmentSpent::<T>::remove(department_id);
            Ok(())
        }

        /// Record an expenditure. Enforces the department spend cap.
        /// metadata_hash is the IPFS CID of the spending justification document.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn record_expenditure(
            origin: OriginFor<T>,
            department_id: u32,
            amount: T::Balance,
            metadata_hash: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let authorized = DepartmentSpenders::<T>::get(department_id)
                .ok_or(Error::<T>::DepartmentHasNoSpender)?;
            ensure!(authorized == who, Error::<T>::NotAuthorizedSpender);
            ensure!(!FrozenDepartments::<T>::get(department_id), Error::<T>::DepartmentFrozen);
            let budget = DepartmentBudgets::<T>::get(department_id);
            let spent = DepartmentSpent::<T>::get(department_id);
            let new_spent = spent.checked_add(&amount).ok_or(Error::<T>::Overflow)?;
            ensure!(new_spent <= budget, Error::<T>::InsufficientBudget);
            DepartmentSpent::<T>::insert(department_id, new_spent);
            let idx = NextExpenditureIndex::<T>::get();
            ExpenditureLog::<T>::insert(idx, (department_id, amount, metadata_hash));
            NextExpenditureIndex::<T>::put(idx.saturating_add(1));
            // Notify the audit pallet (or no-op if AuditHook = NoopAuditHook).
            // idx is u64 — the AuditHook trait accepts u64 to avoid truncation.
            T::AuditHook::on_expenditure(
                idx,
                department_id,
                amount.into(),
                metadata_hash,
            );
            Self::deposit_event(Event::FundsSpent { department_id, amount, metadata_hash });
            Ok(())
        }

        /// Register (or replace) the authorized spender for a department.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn register_department_spender(
            origin: OriginFor<T>,
            department_id: u32,
            spender: T::AccountId,
        ) -> DispatchResult {
            ensure_root(origin)?;
            DepartmentSpenders::<T>::insert(department_id, &spender);
            Self::deposit_event(Event::SpenderRegistered { department_id, spender });
            Ok(())
        }

        /// Remove the authorized spender for a department.
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn remove_department_spender(
            origin: OriginFor<T>,
            department_id: u32,
        ) -> DispatchResult {
            ensure_root(origin)?;
            ensure!(
                DepartmentSpenders::<T>::contains_key(department_id),
                Error::<T>::DepartmentHasNoSpender
            );
            DepartmentSpenders::<T>::remove(department_id);
            Self::deposit_event(Event::SpenderRemoved { department_id });
            Ok(())
        }

        /// Unfreeze a previously frozen department. Root only.
        /// Called after an appeal overturns a treasury court ruling, or after remediation.
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn unfreeze_department(
            origin: OriginFor<T>,
            department_id: u32,
        ) -> DispatchResult {
            ensure_root(origin)?;
            ensure!(
                FrozenDepartments::<T>::get(department_id),
                Error::<T>::DepartmentNotFrozen
            );
            FrozenDepartments::<T>::remove(department_id);
            Self::deposit_event(Event::DepartmentUnfrozen { department_id });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Called by pallet-courts when a ruling finds illegal treasury activity.
        /// Cross-pallet internal call — no origin check needed here; courts are pre-authorized.
        pub fn freeze_department_internal(department_id: u32) -> DispatchResult {
            FrozenDepartments::<T>::insert(department_id, true);
            Self::deposit_event(Event::DepartmentFrozen { department_id });
            Ok(())
        }
    }
}
