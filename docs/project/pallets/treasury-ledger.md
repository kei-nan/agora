# pallet-treasury-ledger

### pallet-treasury-ledger (crate: pallet-treasury-ledger) — runtime index 10

Storage:
- `DepartmentBudgets`: `department_id` → `Balance` (`StorageMap`, `ValueQuery`) — allocating twice for the same department *replaces* the prior allocation, it does not add to it
- `DepartmentSpent`: `department_id` → `Balance` (`StorageMap`, `ValueQuery`) — accumulated spend; NOT auto-reset on re-allocation, see `reset_department_spent` below
- `DepartmentSpenders`: `department_id` → `AccountId` (`StorageMap`, `OptionQuery`) — only this account may call `record_expenditure` for that department
- `ExpenditureLog`: `index: u64` → `(department_id, amount, ipfs_metadata_hash [u8;32])` (`StorageMap`, `OptionQuery`)
- `CourtFrozenDepartments`: `department_id` → `bool` (`StorageMap`, `ValueQuery`) — set by pallet-courts via `TreasuryEnforcer`
- `AuditFrozenDepartments`: `department_id` → `bool` (`StorageMap`, `ValueQuery`) — set/cleared by pallet-audit via `TreasuryFreezer`, driven by its own `OpenFlags` counter
- `NextExpenditureIndex`: `u64` (`StorageValue`, `ValueQuery`) — not `u32`

Two independent freeze authorities, two independent storage items: `record_expenditure` rejects
if *either* is set (`Pallet::is_frozen`). This is deliberate — a single shared flag previously let
one authority's unfreeze silently clear the other's still-open freeze.

Calls:
- `allocate_budget(department_id, amount)` — `LegislatureOrigin` (a passed legislature motion, not root — enforces the executive/legislature separation: the legislature approves the budget, the executive spends it)
- `reset_department_spent(department_id)` — `LegislatureOrigin`; zeroes `DepartmentSpent` for a new fiscal period after re-allocating
- `register_department_spender(department_id, spender)` — `LegislatureOrigin`; registers or replaces the authorized spender for a department
- `remove_department_spender(department_id)` — `LegislatureOrigin`

`LegislatureOrigin` is `EnsureOriginWithArg<_, [u8; 32]>` — all four calls above pass a hash of
their own parameters, checked against the specific motion that authorized them, so a motion
passed to allocate one department's budget can't be replayed to reset another's spend counter
(or to register a different department's spender, etc.).

`register_department_spender`/`remove_department_spender` were previously bare `ensure_root`
with no configurable origin type — fixed by wiring them to the pallet's existing
`LegislatureOrigin`, the same origin `allocate_budget`/`reset_department_spent` already use, so
designating who may spend a department's budget requires a passed legislature motion rather than
a single Root/sudo key. This is deliberately *not* routed through
`pallet-accountability-council` (wired 735d876 for auditor/investigator appointment): department-
spender designation is an operational/Executive-branch-like power, distinct from the independent
oversight appointments that Council governs — routing it there would dilute that Council's
independence. See `pallet_accountability_council`'s module doc comment for the same rationale
from that pallet's side.
- `record_expenditure(department_id, amount, metadata_hash)` — must be the account registered in `DepartmentSpenders` for that department; enforces the budget cap and rejects if frozen
- `unfreeze_department(department_id)` — `CourtOrigin` (wired to
  `pallet_courts::EnsureOracleCouncilApproved`: a manual override requiring the Oracle Council's
  M-of-N approval of this exact call, not bare root — fixed after a project review found the
  prior `EnsureRoot` wiring let a single Root/sudo key silently reverse an already-adjudicated
  court-ordered freeze with no council or jury involvement, mirroring the same fix already
  applied to `pallet_constitution::invalidate_law` and `pallet_identity_zk::suspend_citizen`/
  `restore_citizen_rights`). Clears BOTH `CourtFrozenDepartments` and `AuditFrozenDepartments`
  unconditionally (a deliberate full override, not a per-axis clear — see the doc comment on this
  call for why). Used after an appeal overturns a treasury ruling, after remediation, or to clear
  a stuck freeze on either axis.

After every `record_expenditure`, calls `T::AuditHook::on_expenditure(...)` → pallet-audit inserts a `Pending` audit entry.

Internal (no origin check; not dispatchable, only reachable via runtime-wired traits):
- `freeze_department_internal(department_id)` — called by pallet-courts (`TreasuryEnforcer`) on
  an illegal-treasury-activity ruling; sets `CourtFrozenDepartments` only. pallet-courts has no
  corresponding unfreeze call — a court-ordered freeze stays in place until the
  Oracle-Council-gated `unfreeze_department` dispatchable clears it.
- `audit_freeze_department_internal(department_id)` / `audit_unfreeze_department_internal(department_id)`
  — called by pallet-audit (`TreasuryFreezer`) when its `OpenFlags` counter for a department goes
  to/from zero; set/clear `AuditFrozenDepartments` only, independent of the court axis. Idempotent.

