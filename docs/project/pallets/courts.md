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
- `OracleAccount`: `Option<AccountId>` — the designated AI oracle account (set by root)
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
- `submit_ai_ruling(case_id, ruling_hash, model_version, verdict)` — `OracleOrigin`; `ruling_hash`
  is the IPFS CID of the full reasoning document; `model_version` must match
  `CurrentAIModelVersion` (rejected with `NoApprovedAIModel`/`UnapprovedAIModel` otherwise) —
  the actual on-chain enforcement of "AI model updates require on-chain governance vote";
  `verdict` is committed on-chain here, in `AIRulingVerdict`, at submission time — this is the
  binding between the published reasoning and the verdict `finalize_ruling`'s no-appeal path
  will later apply, closing the hole where a compromised oracle key could publish reasoning
  saying one thing and finalize with the opposite verdict
- `appeal_ruling(case_id)` — within 7-day window; triggers `select_jury`. Restricted to the
  case's filer, the designated oracle, or (for system-filed cases with no natural filer) any
  active citizen (`is_filer_or_oracle`, the same rule `select_jury` uses) — plus, separately,
  the verified losing party of a `CaseSubject::CitizenConduct` case, matched by registered
  identity nullifier (`is_ruled_against_party`): a genuine appeal right for the person actually
  ruled against, not just whoever filed. Before this check existed, any signed account could
  force a case into `InJuryAppeal` and hijack it into permanent limbo. `LawChallenge`/
  `TreasuryDispute` cases don't get an equivalent ruled-against-party right yet — they don't
  identify a specific ruled-against citizen the way `CitizenConduct` does.
- `select_jury(case_id, jury_size)` — filer, oracle, or (for system-filed cases) any active citizen; size validated against case subject; only callable once `JurySeedDelayBlocks` blocks have elapsed since `appeal_ruling`
- `finalize_ruling(case_id)` — `OracleOrigin`; for un-appealed Level 0 cases. Takes no verdict
  argument of its own — applies whatever `submit_ai_ruling` already committed in
  `AIRulingVerdict`
- `cast_jury_vote(case_id, verdict)` — seated juror only; auto-finalizes on majority
- `set_oracle_account(account)` — root; rotatable without runtime upgrade
- `add_ai_governance_member(account)` / `remove_ai_governance_member(account)` — root; manages
  the `AIGovernanceCouncil` roster (mirrors pallet-emergency-council's council management)
- `vote_approve_ai_model(model_hash)` — AI Model Governance Council member only; resolves
  immediately once `AIModelSupermajorityNumerator`/`Denominator` of the council has voted for
  the same hash, bumping `CurrentAIModelVersion` — the supermajority gate `submit_ai_ruling`
  checks against

TODOs:
- Real VRF-based jury randomness. Current scheme (see log #52) is a commit-then-delayed-reveal
  built inside the pallet: `appeal_ruling` timestamps the case (`JuryRequestBlock`), and
  `select_jury` derives its seed only from the fixed block-hash window starting right after
  that point, once the window has fully elapsed. This closes the old "grind by delaying
  submission across already-mined blocks" hole, but a validator scheduled to author a block
  inside the window can still nudge that block's hash — genuine VRF needs BABE/SASSAFRAS
  (full consensus swap away from Aura, not attempted) or real multi-party commit-reveal.

