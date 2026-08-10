# pallet-constitution

### pallet-constitution (crate: pallet-constitution) — runtime index 12

Three-tier law system — no HRC (removed; opposition uses court challenges instead):

| Tier | Description | Amendment pipeline |
|---|---|---|
| `Ordinary` | Legislature simple-majority; standard laws | Propose + ratify after `OrdinaryAmendmentDeliberationBlocks` |
| `Structural` | High-threshold; separation-of-powers, electoral rules | Provisional (0–2yr) → Confirmed (2–6yr, fresh legislature reaffirmation required) → Entrenched (6yr+) |
| `Foundational` | Highest protection; basic rights, democratic principles | Same pipeline as Structural; higher passage threshold enforced by referendum |

Law statuses: `Active`, `Paused` (court-invalidated), `Repealed`

Storage:
- `Laws`: `law_id` → `(LawTier, LawStatus, version: u32, content_hash [u8;32])`
- `PendingAmendments`: `law_id` → `(proposed_hash, proposed_at_block)` (Ordinary tier)
- `ConstitutionalAmendments`: `law_id` → `ConstitutionalAmendmentRecord { previous_hash, new_hash, proposed_at, stage, legislature_reaffirmed }` (Structural/Foundational)
- `Petitions`: `petition_id` → `(AccountId, topic_hash [u8;32], sig_count, submitted_at)`
- `PetitionSignatures`: `(petition_id, AccountId)` → `bool`
- `NextLawId`, `NextPetitionId`

Config constants: `ProvisioningPeriodBlocks = 2 * 365 * DAYS`, `ConfirmationPeriodBlocks = 4 * 365 * DAYS`

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


