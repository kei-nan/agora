# pallet-audit

### pallet-audit (crate: pallet-audit) — runtime index 16

Maintains an audit trail of every treasury expenditure. Populated automatically via the `AuditHook` wired into pallet-treasury-ledger.

Storage:
- `AuditLog`: `expenditure_index` → `AuditEntry { dept_id, amount, ipfs_hash, status, flag_reason, flagged_by }`
- `Auditors`: `BoundedVec<AccountId, 10>`

`AuditStatus` enum: `Pending` | `Cleared` | `Flagged` | `Disputed`

Every `record_expenditure` in pallet-treasury-ledger automatically inserts a `Pending` entry here.

Calls:
- `add_auditor(account)` / `remove_auditor(account)` — `T::AppointmentOrigin`, wired to
  `pallet_accountability_council::EnsureAccountabilityCouncilApproved<Runtime>` in the runtime,
  **not** bare root (fixed after a self-oversight gap: pallet-legislature already controls the
  treasury budget via `LegislatureOrigin`, so if it also appointed the auditors overseeing that
  same spending, the auditors would answer to the branch they audit — see
  `docs/project/pallets/accountability-council.md` for the full rationale, including the
  Indonesia KPK precedent this is meant to avoid). `Config::AppointmentOrigin` is generic over
  `EnsureOriginWithArg<Self::RuntimeOrigin, [u8; 32]>` (not hardcoded to the Accountability
  Council's concrete type), the same way `pallet_constitution::Config::CourtOrigin` is generic
  over `pallet_courts::EnsureOracleCouncilApproved`. `add_auditor`/`remove_auditor` compute the
  call hash via `pallet_accountability_council::accountability_call_hash(b"pallet-
  audit::add_auditor"|"::remove_auditor", &account)` — pallet-audit depends on the
  pallet-accountability-council crate for this one shared hash function rather than
  reimplementing the domain-separation algorithm locally, so the two sides can't drift apart.
  The mock test runtime wires `AppointmentOrigin = AsEnsureOriginWithArg<EnsureRoot<u64>>` (a
  permissive stand-in, ignoring the call hash — mirrors `pallet_treasury_ledger`'s mock
  `LegislatureOrigin`), so this pallet's own tests exercise the auditor-registry logic, not the
  Accountability Council's call-hash-binding/supermajority invariant (covered by that pallet's
  own test suite); `add_auditor_requires_appointment_origin`/
  `remove_auditor_requires_appointment_origin` do assert that a lone signed account (including
  the very account being appointed) is rejected with `BadOrigin`.
- `clear_entry(expenditure_index)` — auditor only; → `Cleared` (Pending entries only)
- `flag_entry(expenditure_index, reason_hash)` — auditor only; → `Flagged` with IPFS reason doc
- `dispute_entry(expenditure_index)` — auditor only; → `Disputed`
- `resolve_entry(expenditure_index)` — auditor only; Flagged/Disputed → `Cleared` (resolved in the
  department's favor). Fixed `7e288c5` (2026-09-04): the resolving auditor must now be a
  *different* account from the entry's `flagged_by` auditor (`Error::CannotResolveOwnFlag`
  otherwise) — previously the same auditor who flagged an entry could immediately clear their own
  flag, defeating the point of having a second party check the spend. Mirrors the
  `SameInvestigator`/different-investigator check `pallet_anticorruption::approve_report_action`
  already enforces (commit `0529508`); this pallet's own test suite had until now codified the
  self-clear as intended, passing behavior
- `submit_audit_report(period_hash)` — auditor only; emits `AuditReportSubmitted`

Treasury enforcement: flagging or disputing an entry actually freezes that expenditure's
department in pallet-treasury-ledger (further `record_expenditure` calls for that department
fail) via a `T::TreasuryFreezer` associated type, implemented in `runtime/src/configs/mod.rs` by
calling `pallet_treasury_ledger::Pallet::<Runtime>::audit_freeze_department_internal` /
`audit_unfreeze_department_internal`. A per-department `OpenFlags` counter tracks how many
Flagged/Disputed entries are still open against it, so pallet-audit's own axis
(`AuditFrozenDepartments`) only unfreezes once `resolve_entry` clears the last one — resolving
one of several open flags/disputes does not unfreeze it while others remain.

**Independent from pallet-courts' freeze.** pallet-treasury-ledger tracks pallet-audit's freezes
and pallet-courts' freezes (via its own `TreasuryEnforcer` trait) in two separate storage items —
`AuditFrozenDepartments` and `CourtFrozenDepartments` — not one shared flag. A department is
blocked from spending while *either* is set. This means: if pallet-courts has also frozen a
department for an unresolved ruling, `resolve_entry` clearing pallet-audit's last open flag does
**not** lift the department's freeze overall — the court-ordered freeze remains until cleared by
pallet-treasury-ledger's Oracle-Council-gated `unfreeze_department` dispatchable — `CourtOrigin`,
wired to `pallet_courts::EnsureOracleCouncilApproved` rather than bare root, since a court-ordered
freeze must not be reversible by a single Root/sudo key. That call clears both axes at once. This
storage split was previously a single shared boolean, which let either authority silently clear
the other's freeze; see pallet-treasury-ledger's `CourtFrozenDepartments`/`AuditFrozenDepartments`
doc comments for the full rationale.

