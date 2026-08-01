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

`CaseSubject` enum:
- `General` — no auto-enforcement
- `LawChallenge { law_id }` — Overturned → `invalidate_law_internal(law_id)` → law paused
- `TreasuryDispute { department_id }` — Overturned → `freeze_department_internal(department_id)`
- `CitizenConduct { nullifier, suspension_blocks }` — Overturned (guilty) → `suspend_citizen_internal`

Jury size routing (enforced in `select_jury`):
- `LawChallenge` → 21 jurors (Level 2 constitutional)
- All other subjects → 7 jurors (Level 1)

Calls:
- `file_case(subject)` — any active citizen
- `submit_ai_ruling(case_id, verdict, ipfs_hash)` — `OracleOrigin`
- `appeal_ruling(case_id)` — within 7-day window; triggers `select_jury`
- `select_jury(case_id, jury_size)` — filer, oracle, or (for system-filed cases) any active citizen; size validated against case subject; only callable once `JurySeedDelayBlocks` blocks have elapsed since `appeal_ruling`
- `finalize_ruling(case_id)` — `OracleOrigin`; for un-appealed Level 0 cases
- `cast_jury_vote(case_id, verdict)` — seated juror only; auto-finalizes on majority
- `set_oracle_account(account)` — root; rotatable without runtime upgrade

TODOs:
- Real VRF-based jury randomness. Current scheme (see log #52) is a commit-then-delayed-reveal
  built inside the pallet: `appeal_ruling` timestamps the case (`JuryRequestBlock`), and
  `select_jury` derives its seed only from the fixed block-hash window starting right after
  that point, once the window has fully elapsed. This closes the old "grind by delaying
  submission across already-mined blocks" hole, but a validator scheduled to author a block
  inside the window can still nudge that block's hash — genuine VRF needs BABE/SASSAFRAS
  (full consensus swap away from Aura, not attempted) or real multi-party commit-reveal.

