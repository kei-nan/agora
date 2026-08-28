# pallet-accountability-council

### pallet-accountability-council (crate: pallet-accountability-council) — runtime index 19

An independent oversight body that governs appointment of `pallet-audit` auditors and
`pallet-anticorruption` investigators. Routing appointment through the legislature would
reproduce the exact self-oversight failure real Supreme Audit Institutions are designed to
prevent: the legislature already controls the treasury budget via `LegislatureOrigin` (see
`pallet_treasury_ledger::Config::LegislatureOrigin`), so letting it also pick who audits/
investigates that same spending puts the auditors under the thumb of the body they audit.
(Concrete cautionary precedent cited in the design discussion: Indonesia's KPK was weakened when
its legislature inserted itself into the anti-corruption commission's appointment structure.)

**Wired as `AppointmentOrigin` on both consuming pallets.** `pallet-audit` and
`pallet-anticorruption` each gained a new `AppointmentOrigin: EnsureOriginWithArg<Self::
RuntimeOrigin, [u8; 32]>` associated type (replacing their previous bare `ensure_root` calls with
no configurable `EnsureOrigin` at all), wired in `runtime/src/configs/mod.rs` to
`pallet_accountability_council::EnsureAccountabilityCouncilApproved<Runtime>`. `add_auditor`/
`remove_auditor`/`add_investigator`/`remove_investigator` compute their call hash via this
pallet's own `accountability_call_hash(tag, params)` (imported directly — both consuming crates
now depend on pallet-accountability-council for this one function, rather than each
reimplementing the domain-separation algorithm locally the way `pallet_constitution::
legislature_call_hash` mirrors `pallet-courts`' hashing) with tags `pallet-audit::add_auditor` /
`::remove_auditor` and `pallet-anticorruption::add_investigator` / `::remove_investigator`. See
`docs/project/pallets/audit.md` and `docs/project/pallets/anticorruption.md` for the per-pallet
detail, including each pallet's permissive `AsEnsureOriginWithArg<EnsureRoot<u64>>` mock
convention (mirroring `pallet_treasury_ledger`'s mock `LegislatureOrigin`) and the
lone-signature-rejected tests it added.

Department-spender designation (`pallet_treasury_ledger::register_department_spender`) is
deliberately **not** routed through this Council — it's an operational, Executive-branch-like
power, not an oversight one, and stays on `LegislatureOrigin` (already present in that pallet's
`Config`).

Storage:
- `Members`: `BoundedVec<AccountId, MaxCouncilSize>` — the Council roster (sized 7-9 in the
  runtime, `MaxCouncilSize = 9`), matching the Oracle Council's size range
  (`pallet_courts::Config::MaxOracleMembers`)
- `Bootstrapped`: `bool` — see "Self-perpetuating membership" below
- `PendingAction`: `call_hash` → `(proposer, BoundedVec<AccountId, MaxCouncilSize>)` — an
  in-flight action (external, or this pallet's own post-bootstrap `add_member`/`remove_member`)
  awaiting supermajority approval
- `ApprovedAction`: `call_hash` → `(proposer, block_approved)` — resolved, awaiting consumption
  via `EnsureAccountabilityCouncilApproved`

Mechanics mirror `pallet-courts`' Oracle Council admin-action pattern
(`propose_admin_action`/`approve_admin_action`/`EnsureOracleCouncilApproved`) — call-hash-bound
proposals, any current member may consume an approved token (not only the original proposer, to
avoid a permanent deadlock if the proposer goes offline or is removed), and a stale unconsumed
token can be discarded after `ApprovalExpiryBlocks` (14 days in the runtime). The threshold here
is a genuine supermajority, evaluated with `>=` (mirrors `pallet-emergency-council`'s
`supermajority_reached`), not the Oracle Council's strict-`>` plain majority:
`approvals * SupermajorityDenominator >= council_size * SupermajorityNumerator`, wired to 2/3 in
the runtime (`SupermajorityNumerator = 2`, `SupermajorityDenominator = 3`).

**`EnsureAccountabilityCouncilApproved<T>`** — the type other pallets depend on — implements
`frame_support::traits::EnsureOriginWithArg<T::RuntimeOrigin, [u8; 32]>` with `Success = ()`.
Succeeds only for a `Signed` origin that is a current `Members` entry, and only when
`ApprovedAction` already holds a token for the exact `call_hash` argument passed; consumes
(removes) that token on success.

Cross-pallet incompatibility checks (`Config::LegislatureChecker`/`ExecutiveChecker`):
- `LegislatureChecker::is_legislature_member` — mirrors the shape of
  `pallet_legislature::pallet::MinisterChecker`; the runtime implements it by reading
  `pallet_legislature::Members` directly
- `ExecutiveChecker::is_active_minister` — same question `MinisterChecker::is_active_minister`
  answers for pallet-legislature, defined as its own local trait here (rather than this crate
  depending on `pallet-legislature` for the trait) for consistency with how every other
  cross-pallet "Checker" in this codebase is defined locally per consumer (`CitizenChecker` is
  independently redefined in `pallet-voting`, `pallet-courts`, `pallet-constitution`, and
  `pallet-elections`). The runtime implements it by reading `pallet-executive`'s
  `MinisterPortfolio`/`PrimeMinister` storage directly.

Both checks are enforced in `add_member` itself — at the point the member is actually seated, on
both the bootstrap (root) path and the post-bootstrap (council-approved) path — not only at
proposal time, so a state change between `propose_action` and consumption can't smuggle in an
account that became ineligible (or was never eligible) in between; an `ApprovedAction` token
whose consumption fails this check is *not* consumed (the whole call reverts, tokens included —
FRAME wraps dispatch in a storage transaction), so the Council can retry once the account becomes
eligible without re-proposing from scratch.

**The legislature/executive-overlap bar is now a maintained invariant, not just a join-time
gate.** `add_member` above only ever blocked one direction — a current legislature member or
executive minister/PM joining the Council. Until now nothing stopped the reverse: a sitting
Council member could later be automatically seated into the legislature by `pallet-elections`'
top-N-by-backing seating, or appointed a minister/invested PM by `pallet-executive`, with no
on-chain check in either direction. Both are now closed, mirroring `pallet_elections::
DisclosureChecker`'s existing shape:
- `pallet_elections::AccountabilityCouncilChecker<AccountId>` (defined in pallet-elections, the
  consumer) is checked in `run_election` alongside `DisclosureChecker` — a would-be-seated
  delegate who currently sits on the Council is skipped rather than seated, the next-highest-
  backed eligible delegate fills the seat instead, and `Event::
  SeatingSkippedAccountabilityCouncilMember` is emitted per skip.
- `pallet_executive::AccountabilityCouncilChecker<AccountId>` (defined in pallet-executive, the
  consumer) is checked at the three points that actually install someone into executive power —
  `nominate_pm` (candidate), `remove_and_replace_prime_minister` (successor), and
  `appoint_minister` (who) — rejecting the call outright with `Error::AccountabilityCouncilMember`
  (no ranked fallback exists there the way seating has, so this is a hard rejection rather than a
  skip-and-fall-through).

Both traits are implemented directly on this pallet's own `Pallet<T>` (reading `Members`), the
same consumer-defines/provider-implements idiom `DisclosureChecker` already uses, and wired in
`runtime/src/configs/mod.rs` as `pallet_elections::Config::AccountabilityCouncilChecker =
AccountabilityCouncil` / `pallet_executive::Config::AccountabilityCouncilChecker =
AccountabilityCouncil`.

Self-perpetuating membership: unlike every other council in this codebase (Oracle Council, AI
Model Governance Council, Emergency Council, the legislature itself), which stay permanently
`Root`-managed, this Council's `Root` access is deliberately one-time.
- While `Bootstrapped == false`, `Root` may freely call `add_member`/`remove_member` — same shape
  every other council's bootstrap uses.
- `close_bootstrap()` — `Root`-only, one-time, requires at least one member already seated. Sets
  `Bootstrapped = true` permanently; no call ever sets it back to `false`.
- Once `Bootstrapped == true`, `add_member`/`remove_member` reject a bare `Root` origin
  (`DispatchError::BadOrigin`) and instead require an `EnsureAccountabilityCouncilApproved` token
  already approved for that exact call — computed as
  `accountability_call_hash(b"pallet-accountability-council::add_member", &who)` (or
  `::remove_member`) via this pallet's own `propose_action`/`approve_action` flow. The Council
  governs its own future composition by its own supermajority vote; `Root` can never again
  unilaterally change it.

Calls:
- `add_member(who)` — `Root` pre-bootstrap; council-approved (2/3 supermajority) post-bootstrap.
  Rejects a current legislature member or executive minister/PM
  (`Error::LegislatureOrExecutiveOverlap`), an already-seated member, or capacity overflow
- `remove_member(who)` — same dual gate as `add_member`. Also purges the removed member's
  already-cast approvals from every in-flight `PendingAction`, mirroring
  `pallet_courts::remove_oracle_member`'s identical rationale: a departing/compromised member's
  vote shouldn't keep counting toward quorum on still-open proposals
- `close_bootstrap()` — `Root`; one-time; see above
- `propose_action(call_hash)` — current Council member only; proposes an action identified by its
  domain-separated call hash and casts the proposer's own approval immediately (resolves
  immediately for a council small enough that one vote already clears the supermajority)
- `approve_action(call_hash)` — current Council member only; co-signs a pending action, rejecting
  a repeat approval (`Error::AlreadyApproved`) or a non-member; applies once the supermajority is
  reached
- `clear_stale_action(call_hash)` — current Council member only; discards an `ApprovedAction`
  token nobody consumed once `ApprovalExpiryBlocks` have passed

TODOs:
- ~~`pallet-anticorruption` still needs a per-report recusal/multi-sign-off mechanism independent
  of the appointment-origin fix above: today any current investigator can unilaterally clear or
  refer *any* report, including one filed about themselves.~~ **Fixed (commit `0529508`).**
  `clear_report`/`refer_report_to_courts` now only *propose* a transition (recorded in new
  `PendingReportAction` storage, keyed by `report_id`) and leave the report at
  `UnderInvestigation`; a second, different investigator must call the new
  `approve_report_action` to actually apply it — the proposer is rejected from approving their
  own proposal (`Error::SameInvestigator`). This is a per-report 2-of-N peer sign-off, not a
  supermajority vote, mirroring the propose/approve pattern this pallet and pallet-courts'
  Oracle Council already use elsewhere. Report content is still deliberately encrypted to the
  investigator's key, so the chain still can't check on-chain whether a report concerns the
  caller specifically — the fix is structural (a second investigator's sign-off) rather than
  content-based.
