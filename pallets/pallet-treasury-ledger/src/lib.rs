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
        /// Origin permitted to manually clear a freeze via `unfreeze_department`. Must be
        /// wired to `pallet_courts::EnsureOracleCouncilApproved` in the runtime, exactly like
        /// `pallet_constitution::Config::CourtOrigin` / `pallet_identity_zk::Config::
        /// SuspensionOrigin` — succeeds only once the Oracle Council's M-of-N threshold has
        /// approved this exact call via `propose_admin_action`/`approve_admin_action`. This
        /// closes the gap where bare `ensure_root` let a single Root/sudo key silently reverse
        /// an already-adjudicated court-ordered freeze (`CourtFrozenDepartments`, set only via
        /// the M-of-N-gated `TreasuryEnforcer`/`freeze_department_internal` path) with no
        /// council or jury involvement — the same class of bug `EnsureOracleCouncilApproved`
        /// was introduced to fix for `invalidate_law`/`suspend_citizen`/`restore_citizen_rights`.
        /// `EnsureOriginWithArg` so the call binds to the domain-separated hash of its own
        /// parameters (see `legislature_call_hash`), the same binding every other privileged
        /// call in this pallet uses — a token approved for one call can never be replayed
        /// against another.
        type CourtOrigin: frame_support::traits::EnsureOriginWithArg<Self::RuntimeOrigin, [u8; 32]>;
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

    /// Secondary index over `ExpenditureLog`, keyed `(department_id, expenditure_index) -> ()`.
    /// `ExpenditureLog` itself is keyed by a global monotonic counter, not by department, so
    /// "give me this department's expenditures" has no way to scope a chain read without this
    /// index — the alternative is scanning the entire log and filtering client-side, which is
    /// exactly what `court-oracle`'s `fetch_expenditures_for_department` used to do (documented
    /// there as a real scaling problem it never fixed). Populated in the same
    /// `record_expenditure` call that writes the primary log entry, so the two can never drift
    /// apart. The value is `()`: this map exists purely so its *keys* can be enumerated via a
    /// department-scoped `state_getKeysPaged` prefix — nothing is ever read out of the value
    /// itself, so `Blake2_128Concat` on both keys (rather than a cheaper unkeyed hasher) is
    /// fine, and no `T::Balance`/amount duplication is needed here.
    #[pallet::storage]
    pub type DepartmentExpenditures<T: Config> =
        StorageDoubleMap<_, Blake2_128Concat, u32, Blake2_128Concat, u64, (), OptionQuery>;

    #[pallet::storage]
    pub type NextExpenditureIndex<T: Config> = StorageValue<_, u64, ValueQuery>;

    /// Departments frozen by a court ruling (via `T::TreasuryEnforcer`, called from
    /// pallet-courts) — no expenditures allowed while frozen.
    ///
    /// Kept independent of `AuditFrozenDepartments` below: these are two independent freeze
    /// authorities sharing one department namespace, and each authority must only ever be able
    /// to clear its own freeze — not the other's. This is a deliberate split (not the original
    /// shape): a single shared `FrozenDepartments` bool used to let pallet-audit's
    /// `resolve_entry` silently lift a still-unresolved court-ordered freeze (or vice versa)
    /// whenever its own open-flag count happened to return to zero. See `is_frozen` for the
    /// combined check `record_expenditure` actually uses.
    ///
    /// pallet-courts' `TreasuryEnforcer` trait exposes only `freeze_department` — no unfreeze —
    /// so a court-ordered freeze stays in place until `unfreeze_department` clears it.
    /// `unfreeze_department` is gated by `T::CourtOrigin` (Oracle Council M-of-N approval, not
    /// bare root — see that call's doc comment), so lifting a court-ordered freeze always
    /// requires the same collective authorization that court rulings themselves do.
    #[pallet::storage]
    pub type CourtFrozenDepartments<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, bool, ValueQuery>;

    /// Departments frozen by pallet-audit (via `T::TreasuryFreezer`), driven by pallet-audit's
    /// own `OpenFlags` counter: set on the first open flag/dispute against a department,
    /// cleared once `resolve_entry` brings that counter back to zero. Independent of
    /// `CourtFrozenDepartments` for the same reason described there.
    #[pallet::storage]
    pub type AuditFrozenDepartments<T: Config> =
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
        /// `record_expenditure` was called with `amount == 0`. A zero-amount expenditure is
        /// never legitimate spending, but `checked_add` with 0 always succeeds and the budget
        /// check trivially holds even at `budget == 0`, so without this check any authorized
        /// `DepartmentSpenders` key could loop free calls that each write a new `ExpenditureLog`
        /// entry and a new `DepartmentExpenditures` map entry at no real cost — an unbounded
        /// state-growth DoS.
        ZeroExpenditureAmount,
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
            ensure!(!Self::is_frozen(department_id), Error::<T>::DepartmentFrozen);
            ensure!(amount > T::Balance::default(), Error::<T>::ZeroExpenditureAmount);
            let budget = DepartmentBudgets::<T>::get(department_id);
            let spent = DepartmentSpent::<T>::get(department_id);
            let new_spent = spent.checked_add(&amount).ok_or(Error::<T>::Overflow)?;
            ensure!(new_spent <= budget, Error::<T>::InsufficientBudget);
            DepartmentSpent::<T>::insert(department_id, new_spent);
            let idx = NextExpenditureIndex::<T>::get();
            ExpenditureLog::<T>::insert(idx, (department_id, amount, metadata_hash));
            DepartmentExpenditures::<T>::insert(department_id, idx, ());
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

        /// Register (or replace) the authorized spender for a department. Gated by
        /// `T::LegislatureOrigin` — the same origin `allocate_budget`/`reset_department_spent`
        /// use — rather than bare `ensure_root`: department-spender designation is an
        /// operational/Executive-branch-like power, so it requires a passed legislature motion
        /// like this pallet's other privileged calls, not a single Root/sudo key. Deliberately
        /// NOT routed through `pallet-accountability-council`: that Council governs independent
        /// oversight appointments (auditors, investigators), and routing an operational power
        /// like this through it would dilute that independence. `EnsureOriginWithArg` so the
        /// call binds to the domain-separated hash of its own parameters (see
        /// `legislature_call_hash`) — a motion approved for one call/department/spender can
        /// never be replayed to authorize another.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn register_department_spender(
            origin: OriginFor<T>,
            department_id: u32,
            spender: T::AccountId,
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(
                    b"pallet-treasury-ledger::register_department_spender",
                    (department_id, spender.clone()),
                ),
            )?;
            DepartmentSpenders::<T>::insert(department_id, &spender);
            Self::deposit_event(Event::SpenderRegistered { department_id, spender });
            Ok(())
        }

        /// Remove the authorized spender for a department. Gated by `T::LegislatureOrigin` —
        /// see `register_department_spender`'s doc comment for the rationale (same origin, same
        /// reason it's not routed through the Accountability Council).
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn remove_department_spender(
            origin: OriginFor<T>,
            department_id: u32,
        ) -> DispatchResult {
            T::LegislatureOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-treasury-ledger::remove_department_spender", department_id),
            )?;
            ensure!(
                DepartmentSpenders::<T>::contains_key(department_id),
                Error::<T>::DepartmentHasNoSpender
            );
            DepartmentSpenders::<T>::remove(department_id);
            Self::deposit_event(Event::SpenderRemoved { department_id });
            Ok(())
        }

        /// Unfreeze a previously frozen department. Gated by `T::CourtOrigin` — in production
        /// `pallet_courts::EnsureOracleCouncilApproved`, i.e. this only succeeds once the
        /// Oracle Council's M-of-N threshold has approved this exact call via
        /// `propose_admin_action`/`approve_admin_action`. Deliberately NOT bare `ensure_root`:
        /// a court-ordered freeze (`CourtFrozenDepartments`) is only ever set via the M-of-N-
        /// gated `TreasuryEnforcer` path (a court ruling), so a single Root/sudo key must not
        /// be able to unilaterally reverse it either — that would let one compromised or
        /// rogue key silently undo an already-adjudicated ruling with no council or jury
        /// involvement.
        ///
        /// Clears BOTH freeze axes (`CourtFrozenDepartments` and `AuditFrozenDepartments`)
        /// unconditionally, rather than picking one. This is a deliberate design choice, not
        /// an oversight: a partial clear here would recreate exactly the silent-desync failure
        /// mode the two-axis split exists to prevent — a caller expecting "the department can
        /// spend again" but leaving a leftover flag on the axis they didn't know about, with
        /// `record_expenditure` still failing for no visible reason. A caller invoking this is
        /// presumed to have already confirmed remediation on whichever side(s) actually needed
        /// it (e.g. via governance record or off-chain process) before reaching for the one
        /// manual override that exists. It is not meant to replace either authority's own
        /// everyday path: pallet-courts has no separate unfreeze path of its own (this
        /// dispatchable, M-of-N-gated, *is* its unfreeze path), and pallet-audit clears its own
        /// axis automatically via `audit_unfreeze_department_internal` as `resolve_entry`
        /// brings `OpenFlags` back to zero — this dispatchable is only the manual escape hatch
        /// for cases that path doesn't cover, now raised from single-key root to the same
        /// M-of-N collective every other manual court-adjacent override in this codebase uses.
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn unfreeze_department(
            origin: OriginFor<T>,
            department_id: u32,
        ) -> DispatchResult {
            T::CourtOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-treasury-ledger::unfreeze_department", department_id),
            )?;
            ensure!(Self::is_frozen(department_id), Error::<T>::DepartmentNotFrozen);
            CourtFrozenDepartments::<T>::remove(department_id);
            AuditFrozenDepartments::<T>::remove(department_id);
            Self::deposit_event(Event::DepartmentUnfrozen { department_id });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// True if `department_id` is frozen under either axis — a court-ordered freeze
        /// (`CourtFrozenDepartments`) or an audit-driven freeze (`AuditFrozenDepartments`).
        /// `record_expenditure` must check this combined state, not either flag alone, so that
        /// one authority resolving its own freeze can never silently lift the other's.
        pub fn is_frozen(department_id: u32) -> bool {
            CourtFrozenDepartments::<T>::get(department_id)
                || AuditFrozenDepartments::<T>::get(department_id)
        }

        /// Called by pallet-courts (via `T::TreasuryEnforcer`) when a ruling finds illegal
        /// treasury activity. Sets the court-freeze axis only (`CourtFrozenDepartments`) —
        /// independent of pallet-audit's flag/dispute axis, so pallet-audit later resolving
        /// its own open flags (`audit_unfreeze_department_internal`) can never silently lift a
        /// court-ordered freeze. Cross-pallet internal call — no origin check needed here;
        /// callers are pre-authorized via runtime wiring, not exposed as a dispatchable.
        pub fn freeze_department_internal(department_id: u32) -> DispatchResult {
            CourtFrozenDepartments::<T>::insert(department_id, true);
            Self::deposit_event(Event::DepartmentFrozen { department_id });
            Ok(())
        }

        /// Called by pallet-audit (via `T::TreasuryFreezer`) when its own `OpenFlags` counter
        /// for a department goes from 0 to nonzero (first open flag/dispute against it). Sets
        /// the audit-freeze axis only (`AuditFrozenDepartments`) — independent of
        /// `CourtFrozenDepartments`, for the same reason described on that storage item.
        /// Cross-pallet internal call — no origin check needed; callers are pre-authorized via
        /// runtime wiring, not exposed as a dispatchable.
        pub fn audit_freeze_department_internal(department_id: u32) -> DispatchResult {
            AuditFrozenDepartments::<T>::insert(department_id, true);
            Self::deposit_event(Event::DepartmentFrozen { department_id });
            Ok(())
        }

        /// Called by pallet-audit (via `T::TreasuryFreezer`) once its own `OpenFlags` counter
        /// for a department returns to zero — every open flag/dispute against it resolved via
        /// `resolve_entry`. Clears the audit-freeze axis only. If the department is still
        /// frozen under `CourtFrozenDepartments` it remains frozen overall —
        /// `record_expenditure` checks the combined state via `is_frozen`, and the
        /// `DepartmentUnfrozen` event below is only emitted once the department is actually
        /// spendable again, so observers never see a misleading "unfrozen" event while the
        /// court axis still blocks spending. Idempotent: does nothing if the audit axis wasn't
        /// set.
        pub fn audit_unfreeze_department_internal(department_id: u32) -> DispatchResult {
            if AuditFrozenDepartments::<T>::take(department_id) {
                if !CourtFrozenDepartments::<T>::get(department_id) {
                    Self::deposit_event(Event::DepartmentUnfrozen { department_id });
                }
            }
            Ok(())
        }
    }
}
