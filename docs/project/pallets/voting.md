# pallet-voting

### pallet-voting (crate: pallet-voting) — runtime index 9

#### System 1 — MACI 1p1v (proposals and elections)

Storage:
- `Proposals`: `proposal_id` → `end_block`
- `VoteCommitments`: `(proposal_id, nullifier)` → `commitment` (MACI-encrypted)
- `MACITallies`: `proposal_id` → `(yes_votes, no_votes, commitment_root)`
- `Delegations`: `(AccountId, topic_id)` → `delegate AccountId`  (per-topic)
- `DelegatorCount`: `(topic_id, AccountId)` → `u32`

Calls: `submit_proposal`, `commit_vote`, `submit_maci_tally`, `delegate_vote(delegate, topic_id)`, `revoke_delegation(topic_id)`

Delegation guards:
- Cycle detection: walks chain up to `MaxDelegationDepth` (10) hops; treats depth-exhaustion as cycle
- Absolute cap: max 1 000 direct delegators per delegate per topic
- Percentage cap: delegate's count × 100 must be ≤ `DelegationCap` (33) × `total_citizens`

#### System 2 — Quadratic budget voting

Storage:
- `FiscalYearEpoch` / `EpochTokenAllocation` / `CitizenClaimedEpoch` / `BudgetBalance` / `CategoryVotes`

Calls: `start_fiscal_year(tokens)` (`LegislatureOrigin`), `claim_fiscal_year_tokens()`, `allocate_budget(category, count)`

Token cost for N votes on a category = N². Refundable by reducing count.

#### System 3 — Referendum pipeline

Storage:
- `Referenda`: `referendum_id` → `(petition_id, topic_hash [u8;32], end_block, ReferendumState, ReferendumTier)`
- `PetitionReferendum`: `petition_id` → `referendum_id`  (prevents duplicate referenda)
- `ReferendumTally`: `referendum_id` → `(yes_count, no_count)`
- `ReferendumHasVoted`: `(referendum_id, AccountId)` → `bool`
- `NextReferendumId`

`ReferendumTier` enum: `Ordinary` (51%) | `Constitutional` (67%) | `Foundational` (75%).
Petitions always produce Ordinary referenda. Constitutional and Foundational referenda require a passed legislature motion.

Config:
- `ReferendumDurationBlocks = 14 * DAYS`
- `PassageThreshold = 51` (ordinary majority)
- `ConstitutionalPassageThreshold = 67` (2/3 supermajority for Structural laws)
- `FoundationalPassageThreshold = 75` (3/4 supermajority for Foundational laws)
- `LawEnactor = Runtime` → calls `pallet_constitution::enact_law_internal` with the correct tier
- `LegislatureOrigin = EnsureLegislatureMotion<Runtime>` (for `start_fiscal_year`, `open_voting_epoch`, `create_constitutional_referendum`, `create_foundational_referendum`)

Calls:
- `vote_referendum(referendum_id, in_favor: bool)` — one vote per active citizen; requires active epoch
- `finalize_referendum(referendum_id)` — anyone, after `end_block`; enacts law if passed
- `create_constitutional_referendum(topic_hash)` — `LegislatureOrigin`; Constitutional-tier (67%); no petition path
- `create_foundational_referendum(topic_hash)` — `LegislatureOrigin`; Foundational-tier (75%); no petition path
- `open_voting_epoch(duration_blocks)` — `LegislatureOrigin`; opens a Swiss-model voting window
- `close_voting_epoch()` — anyone, after epoch end; manual fallback (auto-close via `on_initialize`)

Internal:
- `create_referendum_internal(petition_id, topic_hash, tier)` — called by PetitionApprover;
  sets `end_block` = epoch end if epoch active, else now + ReferendumDurationBlocks

#### System 4 — Swiss-model voting epochs

Storage:
- `ActiveEpoch`: `Option<(start_block, end_block)>` — None = no epoch open
- `EpochNumber`: `u32` — monotonically increasing epoch counter

Citizens may only cast referendum votes while `ActiveEpoch` is `Some` and `now` is in `[start, end]`.
`on_initialize` auto-closes the epoch on the first block past `end_block`.
Legislature (via motion) opens epochs with `open_voting_epoch(duration_blocks)`.
`MinEpochDurationBlocks = 7 * DAYS`, `MaxEpochDurationBlocks = 30 * DAYS`.

