# Agora — Claude Handoff Context

Read this file at the start of every session. It captures the full project state.
Also read `CLAUDE.md` in this same directory for architecture decisions and references.

---

## Environment

- Ubuntu 24.04 WSL2, Rust 1.96 (via rustup)
- Project root: `~/democracy-chain`  (directory name unchanged; project renamed to **Agora**)
- Runtime crate: `agora-runtime`
- Node binary: `agora-node`

### Critical build command

Always use this — without WASM_BUILD_RUSTFLAGS the WASM build fails on Rust 1.84+:

```bash
WASM_BUILD_RUSTFLAGS="-C link-arg=--allow-undefined" cargo build --release
```

Fast check (no WASM binary):
```bash
WASM_BUILD_RUSTFLAGS="-C link-arg=--allow-undefined" cargo check
```

Dev node:
```bash
./target/release/agora-node --dev --tmp
```

---

## Monorepo structure

```
democracy-chain/
├── node/                          # chain binary (agora-node)
├── runtime/                       # WASM runtime (agora-runtime) — all 11 pallets wired in
│   ├── assets/
│   │   ├── vk_sha256.bin          # REAL VK — Rarimo registerIdentity_11_256 (424 bytes)
│   │   └── vk_sha1.bin            # REAL VK — Rarimo registerIdentity_20_160 (424 bytes)
│   └── src/
│       ├── configs/mod.rs         # all pallet Config impls + cross-pallet trait wiring
│       ├── lib.rs                 # runtime construction (construct_runtime!)
│       └── verifier.rs            # RarimoGroth16Verifier (gated behind !dev-mode)
├── pallets/
│   ├── pallet-identity/           # crate: pallet-identity-zk        (index 8)
│   ├── pallet-voting/             # crate: pallet-voting              (index 9)
│   ├── pallet-treasury-ledger/    # crate: pallet-treasury-ledger     (index 10)
│   ├── pallet-courts/             # crate: pallet-courts              (index 11)
│   ├── pallet-constitution/       # crate: pallet-constitution        (index 12)
│   ├── pallet-legislature/        # crate: pallet-legislature         (index 13)
│   ├── pallet-elections/          # crate: pallet-elections           (index 14)
│   ├── pallet-emergency-council/  # crate: pallet-emergency-council   (index 15)
│   ├── pallet-audit/              # crate: pallet-audit               (index 16)
│   ├── pallet-anticorruption/     # crate: pallet-anticorruption      (index 17)
│   └── pallet-executive/          # crate: pallet-executive  Cabinet  (index 18)
├── scripts/
│   └── convert_vk.py              # converts Rarimo snarkjs JSON VK → ark-serialize binary
├── mobile/                        # React Native + Android native project (android/ generated)
├── desktop/                       # Tauri 2 app — wired to real chain RPC + Claude AI agent
├── CLAUDE.md
└── HANDOFF.md
```

Build is clean. Next available pallet index: **19**.

---

## Runtime features

- `default = ["std", "dev-mode"]`
- `dev-mode` enables `PassthroughZkVerifier` (accepts all ZK proofs). Strip this feature
  for any testnet/mainnet build. Without it, `runtime/src/verifier.rs` uses the real
  `RarimoGroth16Verifier`, which rejects all proofs until VK assets are populated.

---

## Cross-pallet trait wiring (runtime/src/configs/mod.rs)

| Trait | Implemented by | Calls |
|---|---|---|
| `ZkProofVerifier` | `PassthroughZkVerifier` (dev) / `RarimoGroth16Verifier` (prod) | ark-groth16 BN254 verify |
| `CitizenChecker<AccountId>` | `Runtime` | `pallet_identity_zk::is_active_citizen` + `TotalCitizens` |
| `CitizenSelector<AccountId>` | `Runtime` | `pallet_identity_zk::CitizenIndex` + `TotalCitizens` |
| `LawEnforcer` | `Runtime` | `pallet_constitution::invalidate_law_internal` |
| `TreasuryEnforcer` | `Runtime` | `pallet_treasury_ledger::freeze_department_internal` |
| `PetitionApprover` | `Runtime` | `pallet_voting::create_referendum_internal` |
| `LawEnactor` | `Runtime` | `pallet_constitution::enact_law_internal(tier, hash)` |
| `CitizenSuspender` | `Runtime` | `pallet_identity_zk::suspend_citizen_internal` |
| `AuditHook` | `pallet_audit::Pallet<Runtime>` | `AuditLog::insert(index, Pending entry)` |
| `pallet_elections::CitizenChecker<AccountId>` | `Runtime` | `pallet_identity_zk::is_active_citizen` |
| `MinisterChecker<AccountId>` | `Cabinet` (`pallet_executive::Pallet<Runtime>`) | `MinisterPortfolio::contains_key` + `PrimeMinister` check |
| `FreshLegislatureChecker<BlockNumber>` | `Runtime` | reads `pallet_elections::LastElectionBlock` |
| `AutoChallengeHook` | `Runtime` | `pallet_courts::Pallet::<Runtime>::auto_file_case(LawChallenge)` |

---

## Pallet status

### pallet-identity (crate: pallet-identity-zk) — runtime index 8

Storage:
- `NullifierRegistry`: `[u8;32]` → `AccountId`
- `CitizenNullifier`: `AccountId` → `[u8;32]`
- `CitizenIndex`: `u32` → `AccountId`  (dense, swap-and-pop on revoke)
- `CitizenPosition`: `AccountId` → `u32`  (reverse index for O(1) removal)
- `TotalCitizens`: `u32`
- `SuspendedNullifiers`: `[u8;32]` → `Option<BlockNumber>`
  - Key absent = not suspended; `None` = indefinite; `Some(block)` = suspended until that block
- `AllowedMerkleRoots`: set of valid Rarimo passport Merkle roots

Calls:
- `register_citizen(nullifier, zk_proof [≤4096 bytes], public_inputs [≤16 × [u8;32]])`
  - Verifies ZK proof via `ZkVerifier` trait
  - Checks passport expiry via `public_inputs[2]` (expirationDate vs current timestamp)
  - Checks country allowlist via `public_inputs[5/6]` (country_code_hash)
- `revoke_citizen()` — swap-and-pop, clears suspension
- `suspend_citizen(nullifier, until)` — `SuspensionOrigin` (EnsureRoot placeholder)
- `restore_citizen_rights(nullifier)` — `SuspensionOrigin`
- `add_allowed_merkle_root(root)` / `remove_allowed_merkle_root(root)` — `AdminOrigin` (EnsureRoot placeholder)

Public helpers:
- `is_active_citizen(who)` — registered AND no active suspension
- `is_citizen(who)` — registered regardless of suspension
- `citizen_at(index)` / `total_citizens()` — for jury selection
- `suspend_citizen_internal(nullifier, until)` — called by pallet-courts on guilty verdict

ZK proof byte format (129 bytes total):
```
[0..32]   A  G1 compressed (ark-serialize LE, flags in byte 31)
[32..96]  B  G2 compressed (ark-serialize LE, flags in byte 63)
[96..128] C  G1 compressed
[128]     variant: 0=SHA-256 circuit, 1=SHA-1 circuit
```

TODOs:
- Populate `runtime/assets/vk_sha256.bin` and `vk_sha1.bin` using `scripts/convert_vk.py`
  (download VKs from https://github.com/rarimo/passport-zk-circuits)

---

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

---

### pallet-treasury-ledger (crate: pallet-treasury-ledger) — runtime index 10

Storage:
- `DepartmentBudgets`: `department_id` → `Balance`
- `DepartmentSpent`: `department_id` → `Balance`
- `DepartmentSpenders`: `department_id` → `AccountId`  (only this account may spend)
- `ExpenditureLog`: `index` → `(department_id, amount, ipfs_metadata_hash [u8;32])`
- `FrozenDepartments`: `department_id` → `bool`
- `NextExpenditureIndex`: `u32`

Calls:
- `allocate_budget(department_id, amount)` — root
- `set_department_spender(department_id, spender)` — root
- `record_expenditure(department_id, amount, metadata_hash)` — designated spender only

After every `record_expenditure`, calls `T::AuditHook::on_expenditure(...)` → pallet-audit inserts a `Pending` audit entry.

Internal: `freeze_department_internal(department_id)` — called by courts enforcement

---

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
- `select_jury(case_id, jury_size)` — anyone; size validated against case subject
- `finalize_ruling(case_id)` — `OracleOrigin`; for un-appealed Level 0 cases
- `cast_jury_vote(case_id, verdict)` — seated juror only; auto-finalizes on majority
- `set_oracle_account(account)` — root; rotatable without runtime upgrade

TODOs:
- VRF-based jury randomness (current: block hash — manipulable by block authors)

---

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


---

### pallet-legislature (crate: pallet-legislature) — runtime index 13

Storage:
- `Members`: `BoundedVec<AccountId, 500>`
- `Motions`: `motion_id` → `Motion { call_hash, proposer, ayes, nays, end_block, executed }`
- `MotionVotes`: `(motion_id, AccountId)` → `bool`
- `NextMotionId`

Calls:
- `add_member(account)` / `remove_member(account)` — root
- `propose_motion(call_hash)` — member only; proposer's aye recorded immediately
- `vote_motion(motion_id, approve: bool)` — member only; **active ministers blocked** (incompatibility rule via `MinisterChecker`)
- `close_motion(motion_id)` — anyone, after `end_block`; passes if ayes * 100 >= 50 * total_members

`EnsureLegislatureMotion<Runtime>` origin — gates law enactment, budget epochs, minister appointments.
`MinisterChecker` trait — implemented by `Cabinet` (pallet-executive); blocks PM + portfolio ministers from voting.

---

### pallet-elections (crate: pallet-elections) — runtime index 14

Elections Commission pallet: manages candidate registration, commissioner certification, and result certification.

Storage:
- `Commissioners`: `BoundedVec<AccountId, 20>`
- `Elections`: `election_id` → `ElectionInfo { office, start_block, end_block, status, winner, results_ipfs_hash }`
- `Candidates`: `(election_id, AccountId)` → `CandidateInfo { profile_ipfs_hash, status, deposit }`
- `NextElectionId`

Calls:
- `add_commissioner(account)` / `remove_commissioner(account)` — root
- `create_election(office, start_block, end_block)` — root or commissioner
- `register_candidate(election_id, profile_ipfs_hash)` — active citizen; reserves 1 AGR deposit
- `certify_candidate(election_id, candidate)` — commissioner only
- `submit_results(election_id, winner, results_ipfs_hash)` — commissioner only
- `certify_results(election_id)` — commissioner only; unreserves all candidate deposits

---

### pallet-emergency-council (crate: pallet-emergency-council) — runtime index 15

Time-limited emergency powers with a hard-coded constitutional sunset clause.

Storage:
- `Council`: `BoundedVec<AccountId, 15>`
- `ActiveEmergency`: `Option<EmergencyInfo { declared_at, expires_at, reason_hash, votes_to_declare, votes_to_end }>`
- `DeclareVotes`: `AccountId` → `bool` (reset each new emergency)
- `EndVotes`: `AccountId` → `bool`

Config:
- `MaxEmergencyBlocks = 432_000` (30 days at 6s/block — constitutional ceiling)
- `SupermajorityNumerator / Denominator = 2/3`

Calls:
- `add_council_member(account)` / `remove_council_member(account)` — root
- `vote_declare_emergency(reason_hash, duration_blocks)` — council member; duration clamped to max; activates on 2/3 supermajority
- `vote_end_emergency()` — council member; lifts on 2/3 supermajority

`on_initialize` hook: auto-expires `ActiveEmergency` when `expires_at <= current_block`, emits `EmergencyExpired`.

---

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

---

### pallet-anticorruption (crate: pallet-anticorruption) — runtime index 17

Three accountability pillars for elected officials and public servants.

Storage:
- `AssetDisclosures`: `AccountId` → `AssetDeclaration { ipfs_hash, disclosed_at, update_due_at }`
- `ConflictRegistry`: `(AccountId, entity_id: u32)` → `ConflictEntry { conflict_type, registered_at }`
- `WhistleblowerReports`: `report_id` → `WhistleblowerReport { content_hash, submitted_at, status, nullifier }`
- `ReportNullifiers`: `(nullifier [u8;32], content_hash [u8;32])` → `bool` (dedup guard)
- `NextReportId`: `u32`
- `Investigators`: `BoundedVec<AccountId, 20>`

`ConflictType` enum: `FinancialInterest` | `FamilyRelation` | `FormerEmployer` | `BusinessPartner`

`ReportStatus` enum: `Pending` → `Flagged` → `UnderInvestigation` → `Cleared` | `ReferredToCourts`

Calls:
- `submit_asset_disclosure(ipfs_hash)` — any signed; mandatory annual renewal
- `register_conflict(entity_id, conflict_type)` — any signed
- `clear_conflict(entity_id)` — any signed (self-removal)
- `submit_whistleblower_report(content_hash, zk_proof, public_inputs)` — gated by ZK citizenship proof;
  stores `public_inputs[0]` as nullifier; `(nullifier, content_hash)` unique per citizen per report
- `flag_report(report_id)` — investigator: Pending → Flagged
- `open_investigation(report_id)` — investigator: Flagged → UnderInvestigation
- `clear_report(report_id)` — investigator: UnderInvestigation → Cleared
- `refer_report_to_courts(report_id)` — investigator: UnderInvestigation → ReferredToCourts;
  emits `ReportReferredToCourts`; investigator then files a case in pallet-courts
- `add_investigator(account)` / `remove_investigator(account)` — root

ZK verifier: `PassthroughAntiCorruptionZkVerifier` (dev-mode) / `RarimoAntiCorruptionZkVerifier` (prod).
Production impl reuses the same Rarimo Groth16 BN254 circuit as pallet-identity.

Config: `MaxInvestigators = 20`, `AssetDisclosureRenewalBlocks = 5_256_000` (~1 year at 6s/block).

---

### pallet-executive (crate: pallet-executive) — runtime index 18, alias `Cabinet`

Parliamentary executive. The legislature appoints ministers to named portfolios via passed motions.
Active ministers are **blocked from casting legislature votes** (incompatibility rule — separation of
executive and legislative power). One account holds at most one portfolio at a time.

Storage:
- `PrimeMinister`: `Option<AccountId>`
- `Portfolios`: `portfolio_id` → `Portfolio { name_hash: [u8;32] }` (name_hash = IPFS CID of terms of reference)
- `PortfolioMinister`: `portfolio_id` → `AccountId`
- `MinisterPortfolio`: `AccountId` → `portfolio_id` (enables O(1) is_active_minister)
- `NextPortfolioId`: `u32`

Config: `LegislatureOrigin = EnsureLegislatureMotion<Runtime>`, `MaxPortfolios = 20`

Calls (all `LegislatureOrigin` except `resign`):
- `define_portfolio(name_hash)` — creates a new named cabinet portfolio
- `appoint_prime_minister(who)` — installs PM; auto-dismisses old PM if any
- `dismiss_prime_minister()` — removes current PM
- `appoint_minister(portfolio_id, who)` — installs minister; auto-vacates old holder + old portfolio of incoming
- `dismiss_minister(portfolio_id)` — removes minister from a portfolio
- `resign()` — any active minister may self-vacate

`EnsureExecutiveMinister<T>` origin — passes if signer is PM or holds a portfolio; returns `AccountId`.

Implements `MinisterChecker<AccountId>` from pallet-legislature: `is_active_minister(who)` returns true
if the account holds a portfolio OR is the PM. This is the cross-pallet trait that enforces the
incompatibility rule without circular dependencies.

---

## Full citizen → law pipeline

**Ordinary law via citizen petition:**
```
submit_petition(topic_hash)
  → sign_petition(petition_id)  [× 1 000 citizens]
    → PetitionApprover::create_referendum  [auto, same tx]
      → Ordinary referendum, 14-day window (or epoch end if epoch active)
        → vote_referendum(referendum_id, in_favor)  [any active citizen, during active epoch]
        → finalize_referendum(referendum_id)  [after end_block, anyone]
          → if yes*100 >= 51*total: LawEnactor::enact_law(Ordinary, topic_hash)
            → Laws storage: Ordinary law, Active
```

**Structural law via legislature:**
```
propose_motion(create_constitutional_referendum call)  [legislature member]
  → vote_motion / close_motion  [passes at >50%]
    → create_constitutional_referendum(topic_hash) → Constitutional referendum (67% threshold)
      → finalize_referendum → enact_law(Structural, hash)
        → Law enters Provisional stage + auto court review (AI judge Level 2)
```

**Foundational law via legislature:**
```
propose_motion(create_foundational_referendum call)  [legislature member]
  → vote_motion / close_motion  [passes at >50%]
    → create_foundational_referendum(topic_hash) → Foundational referendum (75% threshold)
      → finalize_referendum → enact_law(Foundational, hash)
        → Law enters Provisional stage + auto court review (AI judge Level 2)
```

**Ordinary law enacted directly by legislature:**
```
propose_motion(encoded enact_law call)  [legislature member]
  → vote_motion / close_motion  [passes at >50%]
    → enact_law(Ordinary, content_hash) executes
```

---

## ZK verifier status

Infrastructure: complete. `runtime/src/verifier.rs` implements `RarimoGroth16Verifier` using `ark-groth16 0.4` + `ark-bn254 0.4`. Proof format is 129 bytes (ark-serialize compressed A+B+C + 1-byte variant tag).

**Operational step remaining:**
```bash
# 1. Download VKs from https://github.com/rarimo/passport-zk-circuits
# 2. Convert to binary:
python3 scripts/convert_vk.py sha256_verification_key.json runtime/assets/vk_sha256.bin
python3 scripts/convert_vk.py sha1_verification_key.json  runtime/assets/vk_sha1.bin
# 3. Rebuild without dev-mode:
cargo build --release --no-default-features --features std
```

Mobile: `mobile/src/chain/identity.ts` exports `encodeProofForChain(snarkjsProof, variant)` and `encodePublicInputs(signals)` to convert snarkjs output to chain binary format.

---

## Desktop app (Tauri 2 — functional)

Location: `desktop/`

- **Tauri backend** (`src-tauri/src/`): JSON-RPC client talks directly to the running node at `127.0.0.1:9944`
- **Chain commands** (`commands/chain.rs`): `chain_status`, `fetch_proposals`, `fetch_laws`, `fetch_treasury`, `fetch_rulings` — all read from real chain storage via `state_getKeysPaged` + `state_queryStorageAt`
- **AI agent** (`commands/agent.rs`): `agent_ask(question, item_context, history)` — calls Claude API (`claude-sonnet-4-6`); reads `CLAUDE_API_KEY` env var; degrades gracefully offline
- **Auth** (`commands/auth.rs`): `auth_generate_challenge` generates UUID + embeds callback port in deep-link; `auth_poll_session` returns signed session; `auth_start_callback_server` spawns a local HTTP listener; `auth_verify_nullifier` scans `Identity.NullifierRegistry` keys to confirm the mobile-reported nullifier exists on-chain before accepting the session
- **Chain reads** (`commands/chain.rs`): `fetch_proposals` decodes `Voting.Referenda` (42-byte SCALE: petition_id(4) + topic_hash(32) + end_block(4) + state(1) + tier(1)) + `Voting.ReferendumTally` (8 bytes: yes(4) + no(4)); `fetch_rulings` cross-references `Courts.Cases` for IPFS ruling hashes; `fetch_department_budgets` decodes `DepartmentBudgets`/`DepartmentSpent` as u128 LE (16 bytes, 12 decimal places = 1 AGR); `format_agr(planck)` helper; `fetch_treasury` decodes `ExpenditureLog` as 52-byte SCALE (dept_id(4) + amount(16) + hash(32))

Frontend pages: Proposals (with tier chip for constitutional referenda), Laws, Courts, Treasury (department budget table + IPFS audit fetching), auth QR page, Claude AI sidebar panel.

Browser dev mode uses `desktop/src/lib/mocks.ts` stub data; the real Tauri commands fire when running as a native app.

TODOs:
- Mobile side of QR auth: phone NFC + ZK proof; `mobile/src/screens/AuthScreen.tsx` scaffolded (parses deep-link, signs challenge, POSTs to callback)

IPFS content fetching: **implemented** on all detail pages.
- `fetch_ipfs_content(hash_hex: String) -> String` Tauri command in `commands/chain.rs`
- Converts on-chain 32-byte SHA-256 digest → CIDv0 via bs58 multihash (0x1220 prefix)
- Fetches from `https://ipfs.io/ipfs/{cid}` with 30-second timeout
- `LawsPage`, `ProposalsPage`, `CourtsPage`, `TreasuryPage` all fetch content on selection and pass full text to AI agent

---

## Mobile app (needs native project init — JS complete)

All TypeScript/JS logic is done. Missing only the native Android/iOS projects (generated by `react-native init`).

### To make runnable

Run in WSL2 (Node.js is already installed):
```bash
cd mobile
npm install
# Generate native projects — one-time, adds android/ and ios/:
npx react-native@0.74.0 init Agora --template react-native-template-typescript --skip-install --directory .
# Then rebuild JS deps:
npm install
```

Or on Windows PowerShell:
```powershell
cd C:\Users\<you>\democracy-chain\mobile
npx react-native@0.74.0 init Agora --template react-native-template-typescript --skip-install --directory .
npm install
```

After init, register the deep link scheme:
- **Android**: add `<data android:scheme="democracychain" android:host="auth" />` to `AndroidManifest.xml` intent filter
- **iOS**: add `democracychain` as a URL scheme in `Info.plist`

### Files

Chain reads:
- `src/chain/api.ts` — WsProvider + ApiPromise singleton
- `src/chain/identity.ts` — `registerCitizen` (5 public inputs, Rarimo registerIdentity circuit), `isCitizen`, `encodeProofForChain`, `encodePublicInputs`, `getSigningKeypair`, `getNullifier`
- `src/chain/governance.ts` — `fetchProposals`, `fetchLaws`, `fetchPetitions`, `voteOnReferendum`, `signPetition`, `getDelegation`, `delegateVote`, `revokeDelegation`
- `src/chain/voting.ts` — MACI proposal submission, budget allocation
- `src/chain/constitution.ts` — petition submission and amendment
- `src/chain/courts.ts` — case filing, appeal, jury vote

Screens:
- `src/screens/HomeScreen.tsx` — citizen status, chain stats, quick nav
- `src/screens/ProposalsScreen.tsx` — referendum list with For/Against vote buttons
- `src/screens/LawsScreen.tsx` — active laws with tier + status chips
- `src/screens/PetitionScreen.tsx` — petition list with progress bar + sign button
- `src/screens/DelegateScreen.tsx` — per-topic delegation: set delegate, revoke, current status
- `src/screens/AuthScreen.tsx` — desktop QR deep-link handler (auto-activates on `democracychain://auth?...`)
- `src/screens/RegisterScreen.tsx` — NFC passport registration flow stub

`src/App.tsx`:
- Bottom tab navigator (Home / Proposals / Laws / Petitions / Delegate)
- Stack routes for Register + Auth (modal)
- `Linking` listener for `democracychain://auth?...` deep links → auto-navigates to AuthScreen

### Rarimo SDK integration (future)
```
npm install @rarimo/react-native-passport-reader
```
Replace the TODO stubs in `RegisterScreen.tsx` with real Rarimo SDK calls.

---

## Next steps (remaining work)

1. [DONE] **VK assets** — real 424-byte Rarimo Groth16 BN254 VKs in `runtime/assets/`
2. [DONE] **Mobile app native init** — `android/` generated; JS/TS complete; iOS deferred (WSL2)
3. [DONE] **QR auth — chain verification** — `auth_verify_nullifier` scans NullifierRegistry on-chain
4. [DONE] **pallet-executive (Cabinet)** — parliamentary executive, incompatibility rule, `EnsureExecutiveMinister`
5. [DONE] **ReferendumTier::Foundational** — 75% threshold, `create_foundational_referendum` call, maps to `LawTier::Foundational` in pallet-constitution
6. [ ] **VRF jury randomness** — replace 81-block XOR hash with BABE/SASSAFRAS VRF before mainnet; blocked by sp-io 38 vs 40 version conflict while on Aura consensus
7. [ ] **Rarimo SDK (mobile)** — replace `RegisterScreen.tsx` TODO stubs with real `@rarimo/react-native-passport-reader` SDK calls
8. [ ] **Stablecoin bridge** — Phase 2; treasury currently uses native AGR token

---

## Completed next steps log

1. [DONE] Create monorepo + stub all pallets
2. [DONE] Fix treasury accounting bug
3. [DONE] Wire all pallets into runtime
4. [DONE] Delegation cycle detection + cap enforcement
5. [DONE] Jury selection + cross-pallet auto-enforcement
6. [DONE] ZkVerifier trait + PassthroughZkVerifier placeholder
7. [DONE] React Native mobile scaffold (TypeScript skeleton)
8. [DONE] `is_active_citizen` suspension guard in pallet-voting
9. [DONE] 10-finding code review — all bugs fixed
10. [DONE] Real Rarimo Groth16 verifier infrastructure (ark-groth16, needs VK assets)
11. [DONE] Referendum pipeline: petition → referendum → vote → law enacted
12. [DONE] Populate `runtime/assets/vk_sha256.bin` + `vk_sha1.bin` (convert_vk.py script created)
13. [DONE] Per-department authorized spenders in pallet-treasury-ledger
14. [DONE] Legislature collective origin — pallet-legislature (index 13) with EnsureLegislatureMotion
15. [DONE] AI oracle origin (`OracleOrigin` + `OracleAccount` storage) in pallet-courts
16. [DONE] Human Rights Commission veto hook in pallet-constitution (14-day window)
17. [DONE] CitizenConduct case subject + CitizenSuspender trait + suspend_citizen_internal
18. [DONE] Passport expiry + country allowlist checks in `register_citizen`
19. [DONE] Off-chain MACI tally submission with ZK proof (submit_maci_tally + PassthroughMACIVerifier)
20. [DONE] Desktop Tauri 2 app — chain RPC wired, Claude AI agent integrated
21. [DONE] Configurable origins: LegislatureOrigin (pallet-voting), AdminOrigin (pallet-identity), CourtOrigin (pallet-constitution)
22. [DONE] Level 2 jury routing: LawChallenge cases auto-route to 21-person jury
23. [DONE] pallet-elections (index 14) — Elections Commission, candidate registration + deposit lifecycle
24. [DONE] pallet-emergency-council (index 15) — 2/3 supermajority, 30-day constitutional max, auto-sunset
25. [DONE] pallet-audit (index 16) + AuditHook in pallet-treasury-ledger — every expenditure creates Pending audit entry
26. [DONE] Per-referendum passage threshold — ReferendumTier enum (Ordinary/Constitutional) in pallet-voting; constitutional referenda require 67% supermajority; tier forwarded through LawEnactor to pallet-constitution
27. [DONE] Swiss-model batched voting epochs — ActiveEpoch storage, open_voting_epoch (LegislatureOrigin), auto-close via on_initialize; vote_referendum now requires active epoch
28. [DONE] IPFS content fetching (desktop) — fetch_ipfs_content Tauri command converts 32-byte SHA-256 digest → CIDv0 via bs58; LawsPage.tsx fetches and displays content inline
29. [DONE] pallet-anticorruption (index 17) — asset disclosure, conflict-of-interest registry, anonymous ZK whistleblower reports with Pending→Flagged→UnderInvestigation→Cleared/ReferredToCourts workflow
30. [DONE] IPFS fetching for Proposals and Rulings (desktop) — ProposalsPage and CourtsPage now fetch and display IPFS content on selection, update AI agent context with full text
31. [DONE] Constitutional referendum path — `create_constitutional_referendum(topic_hash)` call in pallet-voting (call_index 11), gated by LegislatureOrigin; uses u32::MAX as sentinel petition_id; emits ReferendumCreated with Constitutional tier
32. [DONE] Replace EnsureRoot origin placeholders — CourtOrigin (pallet-constitution) and SuspensionOrigin (pallet-identity) now wire to `pallet_courts::EnsureOracle<Runtime>`; AdminOrigin (pallet-identity merkle roots) wires to `pallet_legislature::EnsureLegislatureMotion<Runtime>`
33. [DONE] BlockHashRandomness upgraded to 81-block XOR mixing — jury selection uses 81 historical block hashes XOR'd with subject; `pallet_insecure_randomness_collective_flip` blocked by sp-io v38 vs v40 version conflict
34. [DONE] Anticorruption ZK bounds fix — `ensure!(!public_inputs.is_empty(), MissingNullifierInput)` before indexing public_inputs[0], prevents panic on malformed submissions
35. [DONE] QR auth full flow (desktop) — `PendingSessions(Arc<Mutex<...>>)` with Clone; `auth_start_callback_server` spawns tokio TCP listener on random port 12000–12999; `auth_generate_challenge` embeds port in deep-link URL; `AuthContext` starts server on mount, polls for session after QR scan
36. [DONE] Mobile AuthScreen scaffold — `mobile/src/screens/AuthScreen.tsx` parses deep-link challenge + callback URL, signs with dev keypair, POSTs JSON `{challenge, nullifierHash, signature, expiresAt}` to desktop callback; `mobile/src/chain/identity.ts` exports `getSigningKeypair()` + `getNullifier()`
37. [DONE] Desktop chain reads — real SCALE decoding: `fetch_proposals` reads Referenda (42 bytes) + ReferendumTally (8 bytes) with vote counts and tier; `fetch_rulings` cross-references Courts.Cases (SCALE: filer(32)+status(1)+Option<hash>(33)+subject) for IPFS hashes; `fetch_treasury` reads ExpenditureLog (52 bytes: dept_id(4)+amount(16)+hash(32))
38. [DONE] `fetch_department_budgets` — reads DepartmentBudgets + DepartmentSpent as u128 LE (16 bytes each); `format_agr(planck)` helper (1 AGR = 1_000_000_000_000 Planck); returns `DepartmentBudget { department_id, budget, spent, remaining }` structs
39. [DONE] `auth_verify_nullifier` — scans all `Identity.NullifierRegistry` storage keys using Blake2_128Concat layout; extracts last 32 bytes of each key as nullifier; returns bool; wired into AuthContext chain verification step
40. [DONE] TreasuryPage rewrite — parallel fetch of treasury entries + department budgets; department budget table (dept/budget/spent/remaining); IPFS audit record fetching on expenditure selection; budget table CSS in Page.css
41. [DONE] ProposalsPage tier field — Proposal interface has `tier: "ordinary" | "constitutional"`; constitutional proposals show a purple "const." chip in the list; IPFS content fetches on proposal selection
42. [DONE] VK assets populated — downloaded real Rarimo Groth16 BN254 VKs (`registerIdentity_11_256` SHA-256, `registerIdentity_20_160` SHA-1) from GitHub releases; converted to ark-serialize binary via `convert_vk.py`; both files are 424 bytes with 5-IC-point real circuit data
43. [DONE] verifier.rs + identity.ts corrected for real Rarimo circuit layout — public inputs are 5 signals (dg15PubKeyHash, passportHash, dg1Commitment, pkIdentityHash, slaveMerkleRoot); nullifier = public_inputs[2] (dg1Commitment); `registerCitizen` no longer takes a separate nullifier arg; both files updated to match
44. [DONE] Mobile app JS complete — all screens implemented: HomeScreen (citizen status, chain stats), ProposalsScreen (vote for/against), LawsScreen (tier + status chips), PetitionScreen (progress bar + sign), DelegateScreen (per-topic delegation UI); governance.ts chain reads; App.tsx with bottom tabs + Linking deep link handler for `democracychain://auth?...`; boilerplate files added (index.js, babel.config.js, metro.config.js); `@react-navigation/bottom-tabs` added to package.json
45. [DONE] Three-tier constitutional law system — `LawTier::Ordinary/Structural/Foundational` in pallet-constitution; Structural/Foundational enter Provisional→Confirmed→Entrenched maturing pipeline (2yr + 4yr stages); FreshLegislatureChecker trait enforces Belgian-model fresh electoral mandate before Confirmed; AutoChallengeHook auto-opens court review for Structural/Foundational laws; HRC removed (replaced by court challenges)
46. [DONE] pallet-executive (`Cabinet`, index 18) — parliamentary executive with PM + named portfolios; legislature appoints/dismisses; `MinisterChecker` cross-pallet trait blocks active ministers from legislature votes (incompatibility rule); `EnsureExecutiveMinister` origin for future executive-gated calls
47. [DONE] `ReferendumTier::Foundational` — 75% supermajority threshold; `create_foundational_referendum` call (call_index 13) in pallet-voting; `FoundationalPassageThreshold = 75` in runtime; `LawEnactor` maps `Foundational → LawTier::Foundational`; desktop Proposal tier chip handles foundational display

---

## Key references

- Rarimo Freedom Tool: https://docs.rarimo.com/freedom-tool/
- Rarimo passport-zk-circuits: https://github.com/rarimo/passport-zk-circuits
- MACI: https://maci.pse.dev/
- Polkadot OpenGov treasury: https://wiki.polkadot.com/learn/learn-polkadot-opengov-treasury/
- Kleros Court V2 (court architecture reference): https://kleros.io/
- Semaphore v4: https://docs.semaphore.pse.dev/
- polkadot-sdk-solochain-template: https://github.com/paritytech/polkadot-sdk-solochain-template
