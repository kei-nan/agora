# pallet-voting

### pallet-voting (crate: pallet-voting) — runtime index 9

#### System 1 — MACI 1p1v (proposals and elections)

Storage:
- `Proposals`: `proposal_id` → `(end_block, topic_hash [u8;32], ReferendumTier)`
- `VoteCommitments`: `(proposal_id, nullifier)` → `commitment` (MACI-encrypted)
- `ProposalResults`: `proposal_id` → `(yes_votes, no_votes, commitment_root)`
- `Delegations`: `topic_id` → `AccountId` → `DelegationRecord { delegate, expires_at, resolved_weight }`
  (a `StorageDoubleMap` keyed topic-first so `apply_delegated_weight` can scan just one topic's
  delegators via `iter_prefix(topic_id)` instead of the whole table — see that function's doc
  comment)
- `DelegatorCount`: `(topic_id, AccountId)` → `u32` (direct fan-in only, feeds the absolute cap)
- `DelegatedWeight`: `(topic_id, AccountId)` → `u32` — transitively-resolved weight currently
  delegated *to* this account (not counting the account's own vote), feeds the percentage cap

Calls: `submit_proposal`, `commit_vote`, `submit_maci_tally`, `delegate_vote(delegate, topic_id)`, `revoke_delegation(topic_id)`

Delegation guards:
- Cycle detection: walks chain up to `MaxDelegationDepth` (10) hops; treats depth-exhaustion as cycle
- Absolute cap: max 1 000 direct delegators per delegate per topic
- Percentage cap: bounds *transitively resolved* weight, not just direct fan-in (fixed — see
  changelog). At `delegate_vote` time, the pallet walks forward from the new `delegate` (bounded
  by `MaxDelegationDepth`) to find the terminal delegate the chain would resolve to, and checks
  `DelegatedWeight[terminal] + who's own contribution` (the delegator's own vote, plus whatever
  was already delegated to them if they were themselves a terminal) against
  `DelegationCap (33) × total_citizens`. `DelegatedWeight` is maintained incrementally by
  `delegate_vote`, `revoke_delegation`, and the lazy expired-delegation cleanup inside
  `has_delegation_cycle` — but it is only used for this cap check; `apply_delegated_weight`
  always re-resolves the real `Delegations` graph fresh, so tally correctness never depends on
  it. Known gap: if an *intermediate* delegate later re-delegates to a different target without
  the delegator whose chain runs through them ever touching their own edge, the weight that was
  upstream of that intermediate isn't re-walked onto the new terminal — this can only make a
  later check on that stale terminal too permissive, never let the cap-violating edge itself go
  undetected. See `DelegatedWeight`'s doc comment in `pallets/pallet-voting/src/lib.rs` for the
  full reasoning.
  - Re-targeting an existing delegation (`who` already has an outgoing `Delegations` record for
    the topic, to a *different* delegate than before) uses the old record's own snapshotted
    `resolved_weight` — not `1 + DelegatedWeight[who]` — as `who`'s real contribution: per
    `DelegatedWeight`'s documented invariant, `DelegatedWeight[who]` is always 0 while `who` has
    an active outgoing delegation, so it does not reflect `who`'s real transitively resolved
    weight in the re-target case the way it does for a first-time delegator. That snapshotted
    `resolved_weight` is used both to decrement the old hub's `DelegatedWeight` when the old edge
    is torn down and to increment the new hub's `DelegatedWeight` (and is itself re-snapshotted
    into the new `Delegations` record for next time). This was fixed in commit `65c326a`: the
    previous code used `1 + DelegatedWeight[who]` for the re-target case too, which — since
    `DelegatedWeight[who]` is always 0 mid-delegation — silently collapsed a re-targeting
    delegator's real weight down to 1, undercounting delegation concentration and letting the
    cap check for the new edge pass too easily.

`Delegations` is only resolved into an actual tally for System 3 (Referenda) below, via
`finalize_referendum`/`apply_delegated_weight` — a non-voting delegator's weight counts toward
whichever side their (transitively resolved) delegate voted. It is deliberately **not**
resolved for MACI (System 1): cross-referencing the plaintext `Delegations` graph against
opaque MACI commitments would leak exactly the linkage MACI exists to hide. Real
delegation-aware MACI tallying would have to happen off-chain, inside a MACI coordinator
service that does not exist yet — see `MACITallyVerifier` below.

`MACITallyVerifier` (checked in `submit_maci_tally`) is `PassthroughMACIVerifier` in
`dev-mode` (accepts any tally unconditionally — not a security boundary) and
`FailClosedMACIVerifier` outside it (rejects every tally, since no real MACI circuit verifier
exists yet) — `submit_maci_tally` is effectively unusable in non-dev builds until a real
verifier replaces it.

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
- `LegislatureOrigin = EnsureLegislatureMotion<Runtime>` (for `start_fiscal_year`, `submit_maci_tally`, `open_voting_epoch`, `create_constitutional_referendum`, `create_foundational_referendum`) — `EnsureOriginWithArg<_, [u8; 32]>`; each of those calls passes a hash of its own parameters, checked against the specific motion that authorized it, so one passed motion can't be replayed to execute a different call

Calls:
- `vote_referendum(referendum_id, in_favor: bool)` — one vote per active citizen; requires active epoch
- `finalize_referendum(referendum_id)` — anyone, after `end_block`; enacts law if passed. In the
  common case this never needs to be called at all: `on_initialize` auto-finalizes any referendum
  scheduled (via `PendingFinalization`, populated when the referendum is created) at the first
  block after its `end_block`, running the same `do_finalize_referendum` logic. This call remains
  as a permissionless backstop for anything the hook misses — e.g. a block whose
  `PendingFinalization` list was already full when the referendum was created (bounded by
  `MaxReferendaPerBlock`) — so a referendum can never get permanently stuck in `Voting` if the
  automatic path doesn't reach it.
- `create_constitutional_referendum(topic_hash)` — `LegislatureOrigin`; Constitutional-tier (67%); no petition path
- `create_foundational_referendum(topic_hash)` — `LegislatureOrigin`; Foundational-tier (75%); no petition path
- `open_voting_epoch(duration_blocks)` — `LegislatureOrigin`; opens a Swiss-model voting window
- `close_voting_epoch()` — anyone, after epoch end; manual fallback (auto-close via `on_initialize`)

Internal:
- `create_referendum_internal(petition_id, topic_hash, tier)` — called by PetitionApprover;
  always sets `end_block = now + ReferendumDurationBlocks` (the full fixed window,
  regardless of whether a voting epoch happens to be active), so a referendum created near
  the end of an epoch still gets adequate voting time — citizens may vote in any overlapping
  future epoch within that window. There is no epoch-conditional branch in the actual code.

#### System 4 — Swiss-model voting epochs

Storage:
- `ActiveEpoch`: `Option<(start_block, end_block)>` — None = no epoch open
- `EpochNumber`: `u32` — monotonically increasing epoch counter

Citizens may only cast referendum votes while `ActiveEpoch` is `Some` and `now` is in `[start, end]`.
`on_initialize` auto-closes the epoch on the first block past `end_block`.
Legislature (via motion) opens epochs with `open_voting_epoch(duration_blocks)`.
`MinEpochDurationBlocks = 7 * DAYS`, `MaxEpochDurationBlocks = 30 * DAYS`.

