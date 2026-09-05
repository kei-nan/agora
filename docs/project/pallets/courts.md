# pallet-courts

### pallet-courts (crate: pallet-courts) — runtime index 11

Case flow: `Filed → AIRulingIssued → InJuryAppeal → JurySeated → FinalRuling`

Storage:
- `Cases`: `case_id` → `(AccountId, CaseStatus, Option<[u8;32]>, CaseSubject)`
- `Rulings`: `case_id` → `Verdict`
- `AIRulingBlock`: `case_id` → `BlockNumber`  (enforces 7-day appeal window)
- `JuryPool`: `case_id` → `BoundedVec<AccountId, 21>`
- `JuryVotes`: `(case_id, AccountId)` → `Verdict`
- `JuryTally`: `case_id` → `(upheld, overturned)`
- `OracleMembers`: `BoundedVec<AccountId, MaxOracleMembers>` — the AI Oracle Council roster
  (bounded to 7, matching this pallet's own Level-1 jury size), root-managed via
  `add_oracle_member`/`remove_oracle_member`. Replaces the earlier single settable
  `OracleAccount` (fixed 2026-08-20, project-review #091 finding 3: a single compromised or
  lost key fully controlled Level-0 rulings chain-wide with no secondary approval).
- `PendingOracleProposal`: `case_id` → the oracle action (ruling submission or finalization)
  awaiting a strict-majority (>1/2) approval, if any
- `OracleApprovals`: `case_id` → `BoundedVec<AccountId, MaxOracleMembers>` — Council members who
  have approved the current `PendingOracleProposal`; single-use per member per action, cleared
  alongside it once the action resolves
- `CaseBonds`: `case_id` → `Balance` — the `CaseFilingBond` reserved by `file_case`, released on finalization
- `AIGovernanceCouncil`: `BoundedVec<AccountId, MaxAIGovernanceCouncilSize>` — root-managed, mirrors `pallet-emergency-council`'s `Council`
- `CurrentAIModelVersion`: `u32` — the governance-approved model version `submit_ai_ruling` checks against
- `AIModelVersions`: `u32` → `model_hash` — approved model history
- `AIRulingModelVersion`: `case_id` → `u32` — which model version ruled on this case
- `PendingAIModelProposal` / `AIModelApprovalVotes`: in-progress `vote_approve_ai_model` round state

`CaseSubject` enum:
- `General` — no auto-enforcement
- `LawChallenge { law_id }` — Overturned → `invalidate_law_internal(law_id)` → law paused
- `TreasuryDispute { department_id }` — Overturned → `freeze_department_internal(department_id)`
- `CitizenConduct { nullifier, suspension_blocks }` — Overturned (guilty) → `suspend_citizen_internal`

Jury size routing (enforced in `select_jury`):
- `LawChallenge` → 21 jurors (Level 2 constitutional)
- All other subjects → 7 jurors (Level 1)

Calls:
- `file_case(subject)` — any active citizen; reserves `CaseFilingBond` (released in full once
  the case reaches a final status) to price the spam risk free Level-0 rulings would otherwise
  create; `auto_file_case(subject)` (system-initiated, e.g. `AutoChallengeHook`) is the
  bond-free internal equivalent, not directly callable
- `submit_ai_ruling(case_id, ruling_hash, model_version, verdict)` — `OracleOrigin` (any current
  `OracleMembers` council member); `ruling_hash` is the IPFS CID of the full reasoning document;
  `model_version` must match `CurrentAIModelVersion` (rejected with
  `NoApprovedAIModel`/`UnapprovedAIModel` otherwise) — the actual on-chain enforcement of "AI
  model updates require on-chain governance vote"; `verdict` is committed on-chain here, in
  `AIRulingVerdict`, at submission time — this is the binding between the published reasoning
  and the verdict `finalize_ruling`'s no-appeal path will later apply, closing the hole where a
  compromised oracle credential could publish reasoning saying one thing and finalize with the
  opposite verdict. This call (like `finalize_ruling`) *proposes* rather than acts immediately —
  it records the caller's own approval in `OracleApprovals`, and the action only takes effect
  once a strict majority (>1/2) of `OracleMembers` has approved via `approve_ai_ruling`
  (auto-finalizes immediately for a 1-member council)
- `approve_ai_ruling(case_id)` — `OracleOrigin`; co-signs the case's current
  `PendingOracleProposal`. Rejects a repeat approval from the same member
  (`Error::AlreadyApprovedOracleAction`) and non-members; applies the action once the strict
  majority is reached
- `appeal_ruling(case_id)` — within 7-day window; triggers `select_jury`. Restricted to the
  case's filer, any Oracle Council member, or (for system-filed cases with no natural filer) any
  active citizen (`is_filer_or_oracle`, the same rule `select_jury` uses) — plus, separately,
  the verified losing party of a `CaseSubject::CitizenConduct` case, matched by registered
  identity nullifier (`is_ruled_against_party`): a genuine appeal right for the person actually
  ruled against, not just whoever filed. Before this check existed, any signed account could
  force a case into `InJuryAppeal` and hijack it into permanent limbo. `LawChallenge`/
  `TreasuryDispute` cases don't get an equivalent ruled-against-party right yet — they don't
  identify a specific ruled-against citizen the way `CitizenConduct` does.
- `select_jury(case_id, jury_size)` — filer, Oracle Council member, or (for system-filed cases) any active citizen; size validated against case subject; only callable once `JurySeedDelayBlocks` blocks have elapsed since `appeal_ruling`
- `finalize_ruling(case_id)` — `OracleOrigin`; for un-appealed Level 0 cases. Takes no verdict
  argument of its own — applies whatever `submit_ai_ruling` already committed in
  `AIRulingVerdict`. Proposes/requires strict-majority approval the same way
  `submit_ai_ruling` does (see above)
- `cast_jury_vote(case_id, verdict)` — seated juror only; auto-finalizes on majority
- `add_oracle_member(account)` / `remove_oracle_member(account)` — root; manages the
  `OracleMembers` roster, bounded to `MaxOracleMembers` (7). Replaces the earlier
  `set_oracle_account`. `remove_oracle_member` also purges the removed member's already-cast
  approvals from every in-flight `PendingOracleProposal` (fixed 2026-08-20: previously the
  removed member's stale approval kept counting toward quorum on any proposal they'd already
  approved, so removing a compromised member — the exact incident-response path this council
  exists to survive — didn't actually shrink the approvals a malicious action needed)
- `add_ai_governance_member(account)` / `remove_ai_governance_member(account)` — root; manages
  the `AIGovernanceCouncil` roster (mirrors pallet-emergency-council's council management)
- `vote_approve_ai_model(model_hash)` — AI Model Governance Council member only; resolves
  immediately once `AIModelSupermajorityNumerator`/`Denominator` of the council has voted for
  the same hash, bumping `CurrentAIModelVersion` — the supermajority gate `submit_ai_ruling`
  checks against

### Stale-proposal recovery: admin actions vs. case-based oracle actions

Two parallel M-of-N proposal mechanisms exist, and each has its own stuck-proposal recovery path.

**Administrative actions** (`propose_admin_action(call_hash)` / `approve_admin_action(call_hash)`,
any `OracleMembers` member) let the Oracle Council authorize a manual-override call in *another*
pallet (currently `invalidate_law`/`suspend_citizen`) via `EnsureOracleCouncilApproved` — a
call-hash-bound proposal accumulates approvals in `PendingAdminAction` until it crosses the
Council's M-of-N threshold, at which point it moves to `ApprovedAdminAction: call_hash →
(proposer, block_approved)`, a resolved-but-unconsumed token that `EnsureOracleCouncilApproved`
lets any current Council member (not only the original proposer) spend once against the matching
call. If nobody ever consumes it, `clear_stale_admin_action(call_hash)` lets any current member
discard it once `AdminActionExpiryBlocks` (14 days in the runtime) have passed unconsumed,
freeing the `call_hash` for a fresh proposal (`Error::ApprovalNotYetStale` if called too early,
`Error::NoApprovedAdminAction` if there's nothing to clear).

**Case-based oracle actions** (`submit_ai_ruling`/`finalize_ruling`, both via `OracleOrigin`) work
differently: they apply themselves immediately on crossing the strict-majority threshold rather
than sitting in an approved-but-unconsumed state, so there is no case-based equivalent of
`ApprovedAdminAction` to go stale. What *can* get stuck is the **pre-threshold** proposal itself —
if Council members never cast enough approvals (e.g. because they're offline),
`Error::OracleActionAlreadyProposed` blocks any fresh `submit_ai_ruling`/`finalize_ruling`
proposal for that `case_id` forever, with no other way to withdraw one. Fixed `c7bd2e2`
(2026-09-04): a new `OracleProposalProposedAt: case_id → BlockNumber` map records when
`PendingOracleProposal` was first inserted for a case, and `clear_stale_oracle_proposal(case_id)`
— open to any current `OracleMembers` member, mirroring `clear_stale_admin_action`'s
authorization/staleness model — discards `PendingOracleProposal`/`OracleApprovals`/
`OracleProposalProposedAt` for that case once `OracleProposalExpiryBlocks` (14 days in the
runtime, same as `AdminActionExpiryBlocks`) have passed unconsumed (`Error::
OracleProposalNotYetStale` if called too early), emitting `Event::OracleProposalCleared { case_id
}` and freeing the case for a fresh proposal. `remove_oracle_member`'s existing
approval-purging/re-resolution logic (see above) already covered this path correctly with no
changes needed.

TODOs:
- Real VRF-based jury randomness. Current scheme (see log #52) is a commit-then-delayed-reveal
  built inside the pallet: `appeal_ruling` timestamps the case (`JuryRequestBlock`), and
  `select_jury` derives its seed only from the fixed block-hash window starting right after
  that point, once the window has fully elapsed. This closes the old "grind by delaying
  submission across already-mined blocks" hole, but a validator scheduled to author a block
  inside the window can still nudge that block's hash — genuine VRF needs BABE/SASSAFRAS
  (full consensus swap away from Aura, not attempted) or real multi-party commit-reveal.

