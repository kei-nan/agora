# pallet-treasury-ledger

### pallet-treasury-ledger (crate: pallet-treasury-ledger) — runtime index 10

Storage:
- `DepartmentBudgets`: `department_id` → `Balance` (`StorageMap`, `ValueQuery`) — allocating twice for the same department *replaces* the prior allocation, it does not add to it
- `DepartmentSpent`: `department_id` → `Balance` (`StorageMap`, `ValueQuery`) — accumulated spend; NOT auto-reset on re-allocation, see `reset_department_spent` below
- `DepartmentSpenders`: `department_id` → `AccountId` (`StorageMap`, `OptionQuery`) — only this account may call `record_expenditure` for that department
- `ExpenditureLog`: `index: u64` → `(department_id, amount, ipfs_metadata_hash [u8;32])` (`StorageMap`, `OptionQuery`)
- `FrozenDepartments`: `department_id` → `bool` (`StorageMap`, `ValueQuery`)
- `NextExpenditureIndex`: `u64` (`StorageValue`, `ValueQuery`) — not `u32`

Calls:
- `allocate_budget(department_id, amount)` — `LegislatureOrigin` (a passed legislature motion, not root — enforces the executive/legislature separation: the legislature approves the budget, the executive spends it)
- `reset_department_spent(department_id)` — `LegislatureOrigin`; zeroes `DepartmentSpent` for a new fiscal period after re-allocating

`LegislatureOrigin` is `EnsureOriginWithArg<_, [u8; 32]>` — both calls above pass a hash of
their own parameters, checked against the specific motion that authorized them, so a motion
passed to allocate one department's budget can't be replayed to reset another's spend counter.
- `record_expenditure(department_id, amount, metadata_hash)` — must be the account registered in `DepartmentSpenders` for that department; enforces the budget cap and rejects if frozen
- `register_department_spender(department_id, spender)` — root; registers or replaces the authorized spender
- `remove_department_spender(department_id)` — root
- `unfreeze_department(department_id)` — root; reverses `freeze_department_internal` after an appeal overturns a treasury ruling or after remediation

After every `record_expenditure`, calls `T::AuditHook::on_expenditure(...)` → pallet-audit inserts a `Pending` audit entry.

Internal: `freeze_department_internal(department_id)` — called by courts enforcement (no origin check, courts are pre-authorized)

