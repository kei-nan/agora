# pallet-treasury-ledger

### pallet-treasury-ledger (crate: pallet-treasury-ledger) — runtime index 10

Storage:
- `DepartmentBudgets`: `department_id` → `Balance`
- `DepartmentSpent`: `department_id` → `Balance`
- `DepartmentSpenders`: `department_id` → `AccountId`  (only this account may spend)
- `ExpenditureLog`: `index` → `(department_id, amount, ipfs_metadata_hash [u8;32])`
- `FrozenDepartments`: `department_id` → `bool`
- `NextExpenditureIndex`: `u32`

Calls:
- `allocate_budget(department_id, amount)` — root
- `set_department_spender(department_id, spender)` — root
- `record_expenditure(department_id, amount, metadata_hash)` — designated spender only

After every `record_expenditure`, calls `T::AuditHook::on_expenditure(...)` → pallet-audit inserts a `Pending` audit entry.

Internal: `freeze_department_internal(department_id)` — called by courts enforcement

