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
- `clear_entry(expenditure_index)` — auditor only; → `Cleared`
- `flag_entry(expenditure_index, reason_hash)` — auditor only; → `Flagged` with IPFS reason doc
- `dispute_entry(expenditure_index)` — auditor only; → `Disputed`
- `submit_audit_report(period_hash)` — auditor only; emits `AuditReportSubmitted`

