//! # Treasury Ledger Pallet
//!
//! Real-time public budget ledger adapted from the Polkadot OpenGov treasury pattern.
//! Per-department spend caps enforced on every transaction; every expenditure is tagged
//! with source metadata and triggers audit hooks.
#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {

    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::CheckedAdd;

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
            + scale_info::TypeInfo;
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

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        BudgetAllocated { department_id: u32, amount: T::Balance },
        FundsSpent { department_id: u32, amount: T::Balance, metadata_hash: [u8; 32] },
        DepartmentFrozen { department_id: u32 },
        DepartmentUnfrozen { department_id: u32 },
    }

    #[pallet::error]
    pub enum Error<T> {
        InsufficientBudget,
        DepartmentNotFound,
        DepartmentFrozen,
        Overflow,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Allocate a budget to a department (legislature-approved).
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn allocate_budget(
            origin: OriginFor<T>,
            department_id: u32,
            amount: T::Balance,
        ) -> DispatchResult {
            ensure_root(origin)?;
            DepartmentBudgets::<T>::insert(department_id, amount);
            Self::deposit_event(Event::BudgetAllocated { department_id, amount });
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
            let _who = ensure_signed(origin)?;
            // TODO: enforce authorized spender per department
            ensure!(!FrozenDepartments::<T>::get(department_id), Error::<T>::DepartmentFrozen);
            let budget = DepartmentBudgets::<T>::get(department_id);
            let spent = DepartmentSpent::<T>::get(department_id);
            let new_spent = spent.checked_add(&amount).ok_or(Error::<T>::Overflow)?;
            ensure!(new_spent <= budget, Error::<T>::InsufficientBudget);
            DepartmentSpent::<T>::insert(department_id, new_spent);
            let idx = NextExpenditureIndex::<T>::get();
            ExpenditureLog::<T>::insert(idx, (department_id, amount, metadata_hash));
            NextExpenditureIndex::<T>::put(idx.saturating_add(1));
            Self::deposit_event(Event::FundsSpent { department_id, amount, metadata_hash });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Called by pallet-courts when a ruling finds illegal treasury activity.
        pub fn freeze_department_internal(department_id: u32) -> DispatchResult {
            FrozenDepartments::<T>::insert(department_id, true);
            Self::deposit_event(Event::DepartmentFrozen { department_id });
            Ok(())
        }
    }
}
