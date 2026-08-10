# pallet-audit

### pallet-audit (crate: pallet-audit) — runtime index 16

Maintains an audit trail of every treasury expenditure. Populated automatically via the `AuditHook` wired into pallet-treasury-ledger.

Storage:
- `AuditLog`: `expenditure_index` → `AuditEntry { dept_id, amount, ipfs_hash, status, flag_reason, flagged_by }`
- `Auditors`: `BoundedVec<AccountId, 10>`

`AuditStatus` enum: `Pending` | `Cleared` | `Flagged` | `Disputed`

Every `record_expenditure` in pallet-treasury-ledger automatically inserts a `Pending` entry here.

Calls:
- `add_auditor(account)` / `remove_auditor(account)` — root
- `clear_entry(expenditure_index)` — auditor only; → `Cleared` (Pending entries only)
- `flag_entry(expenditure_index, reason_hash)` — auditor only; → `Flagged` with IPFS reason doc
- `dispute_entry(expenditure_index)` — auditor only; → `Disputed`
- `resolve_entry(expenditure_index)` — auditor only; Flagged/Disputed → `Cleared` (resolved in the
  department's favor)
- `submit_audit_report(period_hash)` — auditor only; emits `AuditReportSubmitted`

Treasury enforcement: flagging or disputing an entry actually freezes that expenditure's
department in pallet-treasury-ledger (further `record_expenditure` calls for that department
fail) via a `T::TreasuryFreezer` associated type, implemented in `runtime/src/configs/mod.rs` by
calling `pallet_treasury_ledger::Pallet::<Runtime>::freeze_department_internal` /
`unfreeze_department_internal` — the same `FrozenDepartments` storage pallet-courts freezes via
its own `TreasuryEnforcer` trait. A per-department `OpenFlags` counter tracks how many
Flagged/Disputed entries are still open against it, so the department only unfreezes once
`resolve_entry` clears the last one — resolving one of several open flags/disputes does not
unfreeze it while others remain.

