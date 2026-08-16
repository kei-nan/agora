# pallet-constitution

### pallet-constitution (crate: pallet-constitution) — runtime index 12

Three-tier law system — no HRC (removed; opposition uses court challenges instead):

| Tier | Description | Amendment pipeline | Legislature-motion passage threshold |
|---|---|---|---|
| `Ordinary` | Legislature simple-majority; standard laws | Propose + ratify after `OrdinaryAmendmentDeliberationBlocks` | 51% (`OrdinaryPassageThreshold`) |
| `Structural` | High-threshold; separation-of-powers, electoral rules | Provisional (0–2yr) → Confirmed (2–6yr, fresh legislature reaffirmation required) → Entrenched (6yr+) | 67% (`ConstitutionalPassageThreshold`) |
| `Foundational` | Highest protection; basic rights, democratic principles | Same pipeline as Structural | 75% (`FoundationalPassageThreshold`) |

These are the exact same three percentages the referendum path in pallet-voting uses
(`PassageThreshold`/`ConstitutionalPassageThreshold`/`FoundationalPassageThreshold` there) — a
direct legislature motion needs the same supermajority as a citizen referendum to enact, amend,
or repeal a law of a given tier. **Fixed 2026-08-16**: until then, `close_motion` enforced a
single flat ~50% threshold for every motion regardless of subject, so a Foundational-tier law
could in principle be enacted via the legislature-motion path on bare-majority support even
though the referendum path already required 75% for the same tier. See
`docs/project/pallets/legislature.md` for the mechanism (`EnsureLegislatureMotion`'s
`([u8; 32], u8)` origin overload) and the module doc comment in
`pallets/pallet-constitution/src/lib.rs` for the full non-gameability argument.

**How the right threshold gets picked, per call** (`required_threshold` in `lib.rs`):
- `enact_law` — from its own `tier` argument, which is part of the hash `LegislatureOrigin`
  checks against the approved motion, so it can't be swapped at execution time without
  invalidating that hash.
- `propose_constitutional_amendment` / `reaffirm_amendment` / `repeal_law` — from the law's
  *current* tier read directly out of `Laws` storage (never from a caller-supplied value).
- `propose_amendment` / `ratify_amendment` — always `OrdinaryPassageThreshold` (51%), since
  both calls are unconditionally rejected downstream (`UseConstitutionalAmendmentCall`) if the
  law isn't actually Ordinary.

Law statuses: `Active`, `Paused` (court-invalidated), `Repealed`

Storage:
- `Laws`: `law_id` → `(LawTier, LawStatus, version: u32, content_hash [u8;32])`
- `PendingAmendments`: `law_id` → `(proposed_hash, proposed_at_block)` (Ordinary tier)
- `ConstitutionalAmendments`: `law_id` → `ConstitutionalAmendmentRecord { previous_hash, new_hash, proposed_at, stage, legislature_reaffirmed }` (Structural/Foundational)
- `Petitions`: `petition_id` → `(AccountId, topic_hash [u8;32], sig_count, submitted_at)`
- `PetitionSignatures`: `(petition_id, AccountId)` → `bool`
- `NextLawId`, `NextPetitionId`

Config constants: `ProvisioningPeriodBlocks = 2 * 365 * DAYS`, `ConfirmationPeriodBlocks = 4 * 365 * DAYS`,
`OrdinaryPassageThreshold = 51`, `ConstitutionalPassageThreshold = 67`, `FoundationalPassageThreshold = 75`

Calls:
- `enact_law(tier, content_hash)` — `LegislatureOrigin`; Structural/Foundational auto-opens a court case via `AutoChallengeHook`
- `invalidate_law(law_id)` — `CourtOrigin` (wired to `pallet_courts::EnsureOracle`)
- `repeal_law(law_id)` — `LegislatureOrigin`; terminal (cannot be re-enacted under the same id);
  cleans up any pending Ordinary/Constitutional amendment records for the law
- `propose_amendment(law_id, hash)` — `LegislatureOrigin`; Ordinary tier only
- `ratify_amendment(law_id)` — `LegislatureOrigin`; Ordinary tier only; enforces deliberation window
- `propose_constitutional_amendment(law_id, new_hash)` — `LegislatureOrigin`; Structural/Foundational; enters Provisional stage
- `reaffirm_amendment(law_id)` — `LegislatureOrigin`; advances Provisional → Confirmed; requires fresh electoral mandate (FreshLegislatureChecker)
- `advance_to_entrenched(law_id)` — anyone; advances Confirmed → Entrenched once ConfirmationPeriod elapsed
- `revoke_amendment(law_id)` — `RevocationOrigin` (EnsureRoot placeholder); 30–40% growing threshold by stage
- `submit_petition(topic_hash)` — any signed
- `sign_petition(petition_id)` — any signed; at 1 000 threshold calls `PetitionApprover::create_referendum`

Internal:
- `enact_law_internal(tier, content_hash)` — called by pallet-voting on referendum pass
- `invalidate_law_internal(law_id)` — called by pallet-courts on Overturned ruling

Auto-challenge: when `enact_law` or `enact_law_internal` enacts a Structural or Foundational law,
`AutoChallengeHook::auto_challenge_law(law_id)` fires → `pallet-courts` opens a `LawChallenge` case
filed by the zero account (`AccountId32::new([0u8; 32])`) → AI judge immediately reviews it.


