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
│   ├── convert_vk.py              # converts Rarimo snarkjs JSON VK → ark-serialize binary
│   └── certificate-registry/      # builds our own DSC Merkle tree (see log #63) — off-chain only
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

### Rarimo passport integration (future) — researched architecture, see item 8 below

`@rarimo/react-native-passport-reader` (previously referenced here) **does not exist** — verified
via npm/GitHub search, nothing under that name from Rarimo or anyone else. That reference was
wrong; don't install it or search for it again. See item 8 in "Next steps" for the real,
researched plan: on-device Groth16 proving via `@iden3/react-native-rapidsnark` +
`@iden3/react-native-circom-witnesscalc` against `passport-zk-circuits` directly, **not**
`@rarimo/rarime-rn-sdk` (a real but wrong-fit package — Expo-coupled, generates Noir proofs,
submits straight to Rarimo's own EVM identity contracts; doesn't feed our pallet at all).

---

## Next steps (remaining work)

1. [DONE] **VK assets** — real 424-byte Rarimo Groth16 BN254 VKs in `runtime/assets/`
2. [DONE] **Mobile app native init** — `android/` generated; JS/TS complete; iOS deferred (WSL2)
3. [DONE] **QR auth — chain verification** — `auth_verify_nullifier` scans NullifierRegistry on-chain
4. [DONE] **pallet-executive (Cabinet)** — parliamentary executive, incompatibility rule, `EnsureExecutiveMinister`
5. [DONE] **ReferendumTier::Foundational** — 75% threshold, `create_foundational_referendum` call, maps to `LawTier::Foundational` in pallet-constitution
6. [DONE] **Anti-corruption desktop page** — asset disclosures, conflict registry, whistleblower report list
7. [PARTIAL] **VRF jury randomness** — the sp-io v38/v40-class conflict is reconfirmed still blocking `pallet_insecure_randomness_collective_flip` (see log #52 for the re-verification), and it wouldn't have been real VRF anyway. Instead, jury selection now uses a self-contained commit-then-delayed-reveal scheme in pallet-courts (no new deps, stays on Aura) — closes the old scheme's dominant hole (any authorized caller could grind for a favorable jury by delaying `select_jury` across already-mined blocks) but is still not VRF-grade: a validator scheduled to author a block inside the seed window retains bounded influence over that block's hash. Genuine BABE/SASSAFRAS VRF still requires a full consensus swap away from Aura — not attempted, deliberately out of scope (see log #52 for the full writeup and residual-risk detail)
8. **[SUPERSEDED IN PART — see log #65, decided 2026-07-30: dropping Rarimo entirely as the passport-ZK circuit vendor, replatforming to ZKPassport (`github.com/zkpassport/circuits`, Noir/UltraHonk). Everything below in this item describes the now-abandoned Rarimo circom/Groth16 integration — kept as historical record of real, verified engineering work, not deleted, since some of it (NFC chip reading itself, which is circuit-agnostic) remains valid. VK assets, `verifier.rs`'s `RarimoGroth16Verifier`, `sodParser.ts`, `certificateTree.ts`, `poseidon.js`, `asn1.js`, `zkProving.ts`, `proofEncoding.ts` are all Rarimo-circuit-specific and now need rework against ZKPassport's actual circuit shape — none of that rework has started. See log #65 for the full decision record and what's next.]** [PARTIAL] **Real Rarimo passport ZK flow (mobile)** — architecture researched and decided (log #55), on-device proving toolchain + proof-byte-encoding implemented and unit-tested (log #56), NFC reading researched with a concrete library choice (log #57) and the Android native module implemented (log #58). The `.wcd` witness-graph file blocker is now cleared in principle (log #61: fixed all 4 upstream `circom-witnesscalc` bugs found; `build-circuit` produces a complete, structurally-verified `out.wcd` for the real `registerIdentity_11_256_3_2_336_216_NA` circuit — 49MB, 3.78M nodes) but not yet in practice: the fixes only exist on two PR branches on a fork (`github.com/kei-nan/circom-witnesscalc`), not merged upstream, and the actual `out.wcd` this session produced exists only in this environment's ephemeral scratchpad — nobody has published it anywhere the mobile app could fetch it from yet (the decided plan per log #55/#56 is IPFS, with the desktop app pinning it). DG1/DG15/SOD → circuit-inputs assembly (log #59's "not yet started" item) is now built and tested for its self-contained half (log #62: `sodParser.ts`, cross-checked against the real reference implementation on a synthetic-but-real SOD fixture, wired into `RegisterScreen.tsx`). Log #62 also surfaced a blocker — `slaveMerkleRoot`/`slaveMerkleInclusionBranches` need a live inclusion proof from Rarimo's own `CertificatesSMT` registry — which log #63 resolved architecturally, not just technically: depending on Rarimo's hosted registry for citizen registration was a real vendor-lock-in bug (this chain's registration would depend on infrastructure it doesn't govern), not merely an unresolved integration. We now build and host our own equivalent certificate tree instead (`mobile/src/chain/certificateTree.ts` + `scripts/certificate-registry/`), registered via `pallet-identity`'s already-existing `AllowedMerkleRoots` governance. Still blocked on: actually sourcing a meaningful set of trusted DSC certificates (log #63 — this is a governance/PKI problem, not a coding one); a real (not public-data-derived) `skIdentity` generation scheme; publishing the `.wcd` (or a freshly-rebuilt one, once the upstream PRs land) to IPFS; verifying any of the native/NFC code actually compiles or runs (no JDK, Android SDK, or device in this environment — see log #58); and the iOS side (no `ios/` project exists yet to scaffold into). Separately, log #64 found that Rarimo itself is migrating this whole circuit family from circom/Groth16 to Noir/UltraHonk — recommendation there is to keep building on the current path for now (see log #64 for why) but not treat that as settled long-term. Full writeup in logs #55/#56/#57/#58/#61/#62/#63/#64; summary:
   - **`@rarimo/react-native-passport-reader` does not exist** (was a wrong reference in this file — corrected). `@rarimo/rarime-rn-sdk` is real but the wrong tool: Expo-coupled, generates Noir proofs, registers straight to Rarimo's own EVM contracts — doesn't feed `pallet-identity` at all.
   - **Decided path**: stay on `passport-zk-circuits` (circom + Groth16 BN254) directly — confirmed current/actively maintained, and confirmed byte-for-byte compatible with our existing VK assets and `verifier.rs` (downloaded `registerIdentity_11_256_3_2_336_216_NA`'s real verification key from their latest release: `protocol: groth16, curve: bn128, nPublic: 5` — exact match to our 5-signal layout). Prove on-device with `@iden3/react-native-rapidsnark` (Groth16 prover) + `@iden3/react-native-circom-witnesscalc` (witness calc) — both are real, maintained, **plain bare-RN native modules, no Expo migration needed**. This is the same toolchain Rarimo's own production RariMe app ships (confirmed: their iOS build bundles `librapidsnark.a` + `libwitnesscalc_registerIdentity_20_160_3_3_736_200_NA.a`, named after our exact circuit variant).
   - **Decided: use the Full circuit, not "Light."** Light mode (proving key ~15–22MB vs. Full's 515MB) drops the on-device PKI signature-chain check and defers it to a "trusted Rarimo verifier" server — a centralized, unaccountable trust dependency that contradicts this project's whole point and doesn't match `pallet-identity`'s existing design (`AllowedMerkleRoots`, gated by `AdminOrigin` → legislature vote — i.e., trust anchors decided by on-chain governance, not a vendor's server). The Full circuit verifies everything (passport integrity **and** the PKI chain) inside the SNARK, fully self-contained and verifiable from public data alone. This is a deliberate size-for-trustlessness tradeoff, made on purpose — don't "optimize" this back to Light mode without re-litigating the tradeoff.
   - **Decided: distribute the 515MB proving key via IPFS**, not bundle it in the app or serve it from a corporate server. Content-addressing means the file's integrity doesn't depend on trusting whoever served it (re-hash on receipt); this is different from Light mode's problem, which requires trusting a *claim*, not just bytes. Real embedded P2P (libp2p) nodes in React Native are **not practically supported today** (`gomobile-ipfs` archived since 2023) — mobile stays a fetch-only IPFS client. The desktop app (already built, less battery/bandwidth-constrained) is a much better candidate to actually pin/seed this file as part of a genuinely decentralized swarm. Only fetch the one circuit variant matching the user's actual passport signature/hash scheme, not all of them.
   - **Ruled out: peer-assisted proof computation**, even with the Light circuit. Common misconception worth recording since it'll come up again: "Light" only removes the PKI-chain constraints from the circuit — it does **not** remove DG1 (biographic data: name, DOB, nationality, passport number) from the witness, since both Light and Full still need to prove things about DG1. A witness is the complete plaintext assignment of every circuit wire; sending it to any peer (Light or Full) leaks the passport data in the clear, worse than trusting a server since a random peer has zero accountability. The real answer to "peer-assisted proving without leaking data" is collaborative/MPC-based SNARK proving (witness secret-shared across non-colluding parties — e.g. "Collaborative zk-SNARKs," Ozdemir & Boneh) — legitimate but a fundamentally different, much heavier proving stack than `rapidsnark`/`witnesscalc` (single-party, no MPC support). Flagged as a real future direction, explicitly out of scope for now.
   - **Still genuinely open, not yet researched/resolved**: (1) NFC chip reading itself — no confirmed off-the-shelf RN library found; still needs BAC key derivation from the MRZ + low-level APDU exchange with the chip. (2) A witness-calculator format mismatch: `passport-zk-circuits`' release bundle ships the classic circom C++ witness generator (`.cpp`/`.dat`), while `@iden3/react-native-circom-witnesscalc` expects the newer graph format (`.wcd`) from a different iden3 tool — need to either compile a `.wcd` graph ourselves from the `.circom` source, or bridge directly against Rarimo's own precompiled approach. (3) Proof encoding: `groth16Prove` returns snarkjs's standard JSON proof format; converting to the compact ark-serialize byte layout `verifier.rs` expects (129 bytes: A/B/C points + variant byte) is real, bounded work, not yet done.
9. [ ] **Stablecoin bridge** — Phase 2; treasury currently uses native AGR token

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
48. [DONE] Second batch pallet bug fixes — courts: any active citizen can trigger jury selection for system-filed (zero-account) cases; voting: Proposals stores (end_block, topic_hash, tier) so MACI tally can enact laws, referendum end time always uses full ReferendumDurationBlocks; elections: remove active_blocks_this_term counter (derive from term_start_block on demand), guard ElectionCycleBlocks != 0, fix run_election weight; emergency-council + executive: PendingEmergencyProposal locks in first-voter's terms; legislature: PendingLegislatureApproval stores (call_hash, proposer) to prevent token hijacking
49. [DONE] Legislature desktop page — `fetch_legislature_data` Tauri command reads Legislature.Members (StorageValue, compact BoundedVec) + Legislature.Motions (77-byte SCALE); LegislaturePage shows pending/executed motions with aye/nay chips, detail panel with proposer + call hash, scrollable member list
50. [DONE] Elections desktop page — `fetch_elections_data` Tauri command reads PalletElections.Delegates (variable SCALE, compact display_name) + BackingCount + Elections; ElectionsPage shows delegate leaderboard by backing count with status dots, active/past election list, detail panel with consecutive-term counter and IPFS profile link
51. [DONE] Anti-Corruption desktop page — `fetch_anticorruption_data` Tauri command reads PalletAntiCorruption.AssetDisclosures (40-byte SCALE, key suffix = account), ConflictRegistry (5-byte SCALE, tuple key suffix = account+entity_id), WhistleblowerReports (69-byte SCALE, key suffix = report id), and Investigators count; AntiCorruptionPage shows three sections (whistleblower report status chips, asset disclosures with IPFS + AI agent, conflict-of-interest registry) — whistleblower report content is intentionally never IPFS-fetched or shown since it's encrypted to the investigator, only status/metadata is public; added `/anti-corruption` route + sidebar nav item + browser-dev mock
52. [PARTIAL] Jury randomness investigation + commit-reveal upgrade — re-verified empirically (not just from prior notes) that `pallet_insecure_randomness_collective_flip` v37.0.0 still cannot be added: it pulls in `polkadot-sdk-frame` 0.18.0, which drags in a parallel frame-support/frame-system/sp-io 48.0.0 stack alongside our pinned 40.x/40.0.1 one (`cargo tree -i sp-io --duplicates` shows both resolved), and the build hard-fails compiling the transitively-pulled `sp-runtime-interface` v29.0.1 (`assert_eq_size!(usize, u32)` fails on a 64-bit host). Concluded that pallet wouldn't have been real VRF anyway — same "mix N already-known past blocks" shape as what already existed. Concluded a real BABE/SASSAFRAS VRF requires swapping the chain's consensus mechanism away from Aura entirely — out of scope, not attempted, flagged for a deliberate human decision given the blast radius. Implemented instead: a commit-then-delayed-reveal scheme fully inside pallet-courts (no new pallet, no new deps). `appeal_ruling` now stores `JuryRequestBlock` (case_id -> block number of the appeal); `select_jury` requires `JurySeedDelayBlocks` (10 min / ~50 blocks in the runtime, `ConstU32<3>` in the pallet-courts mock) to have fully elapsed since then, and derives the seed *only* from the hashes of that fixed post-appeal block window — none of which existed (or were knowable to anyone, including the appellant/oracle) at appeal time. This removes the old scheme's dominant, cheap attack: mixing "the last 81 blocks as of call time" made the entire outcome computable from already-mined history, so any authorized caller could grind for a favorable jury simply by delaying submission across candidate blocks. It does **not** close everything: a validator scheduled to author a block inside the seed window can still nudge that block's hash (which transactions it includes/orders) within the bounded space of valid blocks it could produce — the same residual "last revealer" risk class inherent to RANDAO-style schemes, just requiring a scheduled-author role rather than being available to any caller. `BlockHashRandomness` and `pallet_courts::Config::Randomness` were removed entirely (replaced by `JurySeedDelayBlocks`); `runtime/src/configs/mod.rs` has the full writeup. Added `pallets/pallet-courts/src/mock.rs` + `tests.rs` (13 tests) — the pallet had zero coverage before; tests specifically verify the delayed-reveal property (jury outcome is identical regardless of when `select_jury` is actually called or what happens outside the seed window, but changes when the window's own hashes change), plus authorization, jury sizing (7 vs 21), insufficient-citizen handling, and the downstream vote/auto-enforce paths (law invalidation, department freeze, citizen suspension). `cargo test -p pallet-courts` and `WASM_BUILD_RUSTFLAGS="-C link-arg=--allow-undefined" cargo build --release` (full workspace) both pass. Item 7 above marked [PARTIAL], not [DONE] — this is a real, bounded improvement, not VRF; do not oversell it as solved
53. [DONE] First pallet unit test coverage — every pallet in the repo had zero tests until now (only the unused `pallet-template` scaffold had any). Added `mock.rs` + `tests.rs` (the `#[frame_support::runtime]` mock-runtime pattern, same style as `pallet-template`) to `pallet-identity` (31 tests), `pallet-voting` (78 tests), `pallet-constitution` (54 tests), `pallet-treasury-ledger` (32 tests), and `pallet-courts` (13 tests, see #52) — 208 tests total, `cargo test --workspace --lib` all green. One real bug surfaced and fixed: `pallet-voting`'s `revoke_delegation` was missing the `CitizenChecker::is_active_citizen` gate every other citizen-facing call in the pallet has — a suspended citizen could still revoke a standing delegation. Fixed by adding the same gate (`Error::CitizenNotActive`); this doesn't strand anyone, since delegations already carry a bounded `expires_at` ceiling (`MaxDelegationDurationBlocks`) that lapses them on its own. Test updated accordingly. `pallet-elections`, `pallet-emergency-council`, `pallet-executive`, `pallet-audit`, `pallet-legislature`, `pallet-anticorruption` still have zero test coverage — good next target.
54. [DONE] Mobile chain connectivity — corrected a stale claim in this file (items 36/43/44 said `mobile/src/chain/api.ts` was a real `WsProvider`/`ApiPromise` singleton and governance reads were real; they were actually stubs that threw/returned mock data). Added RN polyfills for `@polkadot/api` (`react-native-get-random-values`, `fast-text-encoding`, `buffer`, `@polkadot/wasm-crypto-asmjs`, wired into `index.js`/`metro.config.js`); `api.ts` now a real cached `ApiPromise` singleton (`NODE_WS_URL = 'ws://10.0.2.2:9944'`, Android-emulator default); `identity.ts`'s `getSigningKeypair`/`isCitizen`/`registerCitizen` now real (dev-only fixed-mnemonic keypair, clearly marked — no Secure Enclave/Keystore integration yet, that's still real future work); `governance.ts` rewritten to real storage reads/extrinsics for proposals/laws/petitions/delegation plus pallet-elections delegate-registry calls, replacing all mock/local-storage-backed data. Found and fixed two real signature bugs in the pre-existing (unreachable-until-now) `voting.ts`: `commitVote` passed a stray `nullifier` arg `commit_vote` doesn't take, and `delegateVote` was missing the required `duration_blocks` arg. `VoteScreen.tsx` rebuilt as a "Budget" tab (fiscal-year claim + quadratic allocation), added to `App.tsx` nav. `mobile && npx tsc --noEmit` clean. Not attempted (no emulator/device in this environment, and no runtime testing was possible): IPFS content upload for `constitution.ts`'s `enactLaw` TODO; real Rarimo passport NFC/ZK proof flow (`RegisterScreen.tsx` still stubbed, item 8 below).
55. [DONE] Rarimo passport ZK integration — architecture research and decisions (no code changes; see item 8 above for the summary carried into "Next steps"). Started from a wrong reference already in this file (`@rarimo/react-native-passport-reader`, confirmed via npm/GitHub search not to exist) and worked out the real, verifiable architecture instead:
    - Confirmed `passport-zk-circuits` (circom/Groth16, what our VK assets and `verifier.rs` are built on) is current and actively maintained (pushed 2026-07-02, more recently than the alternative SDK below), and downloaded a real verification key from their latest release to confirm the exact match: `protocol: groth16, curve: bn128, nPublic: 5`.
    - Confirmed `@rarimo/rarime-rn-sdk` is real but the wrong tool for this project (Expo-coupled, Noir proofs, submits to Rarimo's own EVM contracts, not chain-agnostic).
    - Found the real on-device proving toolchain: `@iden3/react-native-rapidsnark` + `@iden3/react-native-circom-witnesscalc`, both real, maintained, plain bare-RN native modules (no Expo needed). Verified this is what Rarimo's own production RariMe app actually ships by inspecting their public iOS repo's `Frameworks/` folder directly — found `librapidsnark.a` and, notably, `libwitnesscalc_registerIdentity_20_160_3_3_736_200_NA.a`, named after the exact circuit variant our `vk_sha1.bin` already targets.
    - Downloaded and measured the actual proving-key sizes: Full circuit `circuit_final.zkey` = 514,925,929 bytes (~515MB); Light circuit variants (`registerIdentityLight160/224/256/384/512`) = 15.7–22.7MB, roughly 30x smaller.
    - **Decided against Light mode despite the size win**: it defers the PKI signature-chain check to a "trusted Rarimo verifier" server, a centralized trust dependency inconsistent with this project's on-chain-governed trust model (`pallet-identity`'s `AllowedMerkleRoots` + `AdminOrigin` → legislature vote). Decided to eat the 515MB Full-circuit cost instead, distributed via IPFS (content-addressed — verified by hash regardless of who serves it, unlike Light mode's unverifiable server assertion) with the desktop app as a natural pinning peer; confirmed real embedded P2P/libp2p nodes aren't practically supported in React Native today (`gomobile-ipfs` archived 2023), so mobile stays fetch-only.
    - **Ruled out peer-assisted proof computation** (even with the Light circuit, which doesn't help here): a SNARK witness is a full plaintext assignment of every circuit wire, including DG1 biographic data (name/DOB/nationality/passport number) — "Light" only drops the PKI-chain constraints, not the DG1 constraints, so a Light witness is exactly as sensitive as a Full one. Sending a witness to any peer for proving leaks the passport data outright. Noted collaborative/MPC-based SNARK proving as the real cryptographic answer to this, and as a legitimate but out-of-scope future direction — it needs a fundamentally different proving stack than `rapidsnark`/`witnesscalc` (which are single-party, no MPC support).
    - Genuinely unresolved, not yet researched: NFC chip reading itself (no library found), a witness-calculator file-format mismatch (`passport-zk-circuits` ships classic `.cpp`/`.dat` witness generators; `@iden3/react-native-circom-witnesscalc` wants the newer `.wcd` graph format), and the proof-encoding conversion from snarkjs's JSON format to `verifier.rs`'s compact byte layout.
56. [DONE] Full pallet test coverage (remaining 6) + Rarimo mobile scaffolding — all 11 real pallets now have unit test coverage (only the unused `pallet-template` scaffold predates this effort). `cargo test --workspace --lib` all green.
    - Added `mock.rs`/`tests.rs` to `pallet-elections` (73 tests, needed `pallet-balances` wired into the mock for `ReservableCurrency` — added as a dev-dependency), `pallet-emergency-council` (27 tests), `pallet-executive` (54 tests), `pallet-audit` (29 tests), `pallet-legislature` (33 tests), `pallet-anticorruption` (36 tests) — 252 new tests, same `#[frame_support::runtime]` mock pattern as the earlier batch (#53).
    - **Real bug found and fixed**: `pallet-elections`' `Config::MaxCandidatesPerElection` was declared but never checked anywhere in `register_candidate` — unbounded candidate registration was possible, and `certify_results`' iteration over `Candidates::iter_prefix(election_id)` had no real bound (an unbounded-weight concern). Fixed by adding a `CandidateCount` storage map, checked and incremented in `register_candidate` (`Error::TooManyCandidates`); the flagging agent's test documenting the old behavior was rewritten to assert the correct rejection instead of the bug. Full workspace + release build reverified after the fix.
    - Mobile: wired the real on-device proving toolchain decided in #55. `mobile/src/chain/zkProving.ts` (new) — `fetchZkAsset` (hash → CIDv0 → cached streaming download via RNFS, temp-file-then-rename for crash safety, never buffers the ~515MB key in memory) plus thin wrappers around `@iden3/react-native-circom-witnesscalc`'s `calculateWitness` and `@iden3/react-native-rapidsnark`'s `groth16Prove` — corrected against the packages' actual shipped `.d.ts` types rather than their README prose (both return JSON/base64-encoded *strings*, not parsed objects, contrary to what the usage examples imply). `mobile/src/chain/proofEncoding.ts` (new) — TypeScript port of `scripts/convert_vk.py`'s `_compress_g1`/`_compress_g2` (the ark-serialize compressed-point format), producing `verifier.rs`'s exact 129-byte proof layout from a snarkjs proof object. Verified with a genuine three-way cross-check: a throwaway Rust tool (kept out of the workspace, in scratchpad only) built points with `ark_bn254`/`ark_serialize` and serialized them via `CanonicalSerialize`, `convert_vk.py` was run against the identical coordinates, and the new TypeScript port was tested against both — byte-identical across 8 vectors covering every branch (G1 sign flip, G2 c1-dominant and c1-zero-fallback tie-break, both sign directions). Also stood up `mobile/`'s first Jest test setup (`jest.config.js`, `package.json` `test` script) — 11 passing tests for the encoding module.
    - Still explicitly stubbed, per #55's open items, not attempted here: NFC chip reading, the `.wcd` witness-graph format (no such file exists yet for any `passport-zk-circuits` variant), and the full end-to-end wiring of `zkProving.ts`/`proofEncoding.ts` into `RegisterScreen.tsx` (that screen's TODOs were only lightly updated to point at the new modules, not restructured).
57. [DONE] NFC passport chip reading — research + Android native module scaffolding (closes the last "no library found" gap from #55).
    - **The real foundational libraries, both confirmed current and production-proven**: **JMRTD** (Java/Kotlin, Android) — latest v0.8.1 on Maven Central, LGPL, full BAC + PACE + Active/Chip/Terminal Authentication (`PACEProtocol.java`, `AAProtocol.java`, `EACCAProtocol.java`/`EACTAProtocol.java` all present in source). Confirmed this is exactly what Rarimo's own production Android app (`rarime-android-app`, native Kotlin, not React Native) uses — `implementation("org.jmrtd:jmrtd:0.7.27")` pinned directly in their `build.gradle.kts`, alongside `net.sf.scuba:scuba-sc-android` (the Android `IsoDep`-to-JMRTD-`CardService` bridge — confirmed real class `net.sf.scuba.smartcards.IsoDepCardService`). **AndyQ/NFCPassportReader** (Swift, iOS) — pushed as recently as 2026-07-01, 860 stars, MIT, same protocol coverage (`BACHandler.swift`, `PACEHandler.swift`, `ChipAuthenticationHandler.swift`), confirmed via source that `NFCPassportModel.getDataGroup(_:)` returns raw `Data` bytes per data group (`DataGroup1.swift`/`DataGroup15.swift`/`SOD.swift` are dedicated classes).
    - **Existing ready-made RN wrappers don't expose what we need.** Found and evaluated `react-native-nfc-passport-reader` (npm, real, v0.2.5, explicitly built on `NFCPassportReader` for iOS per its own README) — but its bridge (`NativeNfcPassportReader.ts`) only returns parsed convenience fields (name, DOB, gender, nationality, MRZ string, photo), not raw DG1/DG15/SOD bytes. It's a legitimate package for standard KYC-style verification, just the wrong shape for feeding a ZK circuit's raw data-group inputs. No off-the-shelf RN package was found that surfaces the raw bytes.
    - **Decided path**: a custom, thin RN native module per platform, wrapping JMRTD (Android) / NFCPassportReader (iOS) directly, exposing raw DG1/DG15/SOD (+ AA challenge/signature) bytes through the bridge instead of parsed fields. Real per-platform native engineering, but built on the same libraries already proven in a real shipped app (Rarimo's own), not experimental territory.
    - Real JMRTD API confirmed by reading actual source (not guessed) before scaffolding: `PassportService(CardService, ...)`, `.open()`, `.sendSelectApplet(hasPACESucceeded)`, `.doBAC(AccessKeySpec)` / `.doPACE(AccessKeySpec, oid, params, parameterId)`, `.getInputStream(short fid)` → `CardFileInputStream`; `BACKey(String documentNumber, String dateOfBirth, String dateOfExpiry)`; Android bridge via `IsoDepCardService`.
    - **iOS spec (not built — no `ios/` native project exists in this repo yet, and creating one is a separate, larger undertaking noted elsewhere in this item).** When it does: wrap `AndyQ/NFCPassportReader` (MIT) with a thin Swift `RCTBridgeModule`, mirroring the Android module's shape exactly so the JS bridge (`mobile/src/native/nfcPassportReader.ts` — see log entry directly below for its Android build) can share one interface across platforms. Real API confirmed from source (`PassportReader.swift`) — note this uses modern Swift concurrency, not a completion handler: `public func readPassport(mrzKey: String, tags: [DataGroupId] = [], aaChallenge: [UInt8]? = nil, skipSecureElements: Bool = true, skipCA: Bool = false, skipPACE: Bool = false, useExtendedMode: Bool = false, customDisplayMessage: ((NFCViewDisplayMessage) -> String?)? = nil) async throws -> NFCPassportModel`. `mrzKey` is the BAC/PACE seed derived from the same three MRZ fields (document number, DOB, expiry) as Android's `BACKey` — PACE is attempted automatically unless `skipPACE: true` is passed, so this library gets PACE "for free" where Android's module (BAC-only per #58 below) doesn't yet. Request `tags: [.DG1, .DG15, .SOD]` explicitly rather than the default empty/full read. Once the `await` resolves, pull raw bytes via `passport.getDataGroup(.DG1)?.data` / `.DG15` (confirmed to return `Data` in `NFCPassportModel.swift`) and `passport.sod` (confirmed dedicated `SOD.swift` class) — base64-encode for the bridge, matching the Android module's `{ dg1, dg15, sod }` shape. `Podfile`/SPM wiring is standard once `ios/` exists; needs `NFCReaderUsageDescription` + the ISO7816 `select-identifiers` entitlement in `Info.plist` (see the `react-native-nfc-passport-reader` package's README, referenced above, for the exact entitlement values other consumers of this library use).
58. [PARTIAL] NFC passport chip reading — Android native module implemented, cross-referenced line-by-line against real JMRTD/scuba source (see #57 for the research this builds on; not compiled — see caveat below).
    - Cloned the actual source repos to read every API call before writing it: JMRTD (`github.com/ElMostafaIdrassi/jmrtd`, mirrors the SourceForge canonical repo, at a commit matching the `0.8.6` release per its `CHANGELOG.txt`) and scuba (`github.com/ugochirico/SCUBA`, mirrors `net.sf.scuba`). This surfaced two corrections to #57's assumed API: `PassportService.getInputStream(short fid)` (1-arg) is `@Deprecated` in real source in favor of `getInputStream(short fid, int maxBlockSize)` (used instead); and `BACProtocol.doBAC` throws `org.jmrtd.CardServiceProtocolException` on failure, not the older `BACDeniedException` (itself `@Deprecated` in current JMRTD) that seemed like the obvious guess.
    - **Version choice**: `org.jmrtd:jmrtd:0.8.6` (not the newest `0.8.7`, published the same day this was written per Maven Central's directory timestamps — picked `0.8.6`, ~3 months old at time of writing, for basic soak time instead) + `net.sf.scuba:scuba-sc-android:0.0.26`. Confirmed compatible by reading both artifacts' POMs on Maven Central: both declare the identical transitive `net.sf.scuba:scuba-smartcards:0.0.20` dependency. Diverges from Rarimo's own pin (`jmrtd:0.7.27`) deliberately — the 0.7→0.8 CHANGELOG is bugfixes + BouncyCastle bumps only (DG11/DG12 parsing fixes, no `PassportService`/`BACKey`/`CardFileInputStream` API changes), so the newer line was judged safe.
    - **Implemented** (`mobile/android/app/src/main/java/com/agora/nfc/`): `NfcPassportModule.kt` (classic `ReactContextBaseJavaModule`, matching this app's `newArchEnabled=false`) exposes `readPassport(documentNumber, dateOfBirth, dateOfExpiry, promise)`; a companion-object static (`onTagDiscovered`) receives tags forwarded from `MainActivity`, matches them against a pending promise/BAC-key pair (synchronized against the JS-thread call), and runs the actual BAC + `getInputStream(EF_DG1/EF_DG15/EF_SOD)` read on a background `Thread` (blocking I/O, must not run on the UI thread `onNewIntent` delivers tags on). Resolves with `{dg1, dg15, sod}` as base64 strings; rejects with distinct error codes for BAC failure (`CardServiceProtocolException`, checked before the broader `CardServiceException`), card I/O errors, and connection loss. `NfcPassportPackage.kt` registers it; `MainApplication.kt` wires it in at the existing `// add(MyReactNativePackage())` marker.
    - **`MainActivity.kt`**: added the standard Android NFC foreground-dispatch trio (`onResume`/`onPause`/`onNewIntent`, `IsoDep`-tech-filtered dispatch), plus `setIntent(intent)` in `onNewIntent` (the standard RN deep-link recipe for `singleTask` activities — this activity didn't override `onNewIntent` at all before, so this is also a latent fix for warm-start `democracychain://` auth deep links from log #36, not just NFC). `AndroidManifest.xml` gained `android.permission.NFC` and an optional (`required="false"`) `android.hardware.nfc` feature declaration — neither existed before.
    - **BAC only, not PACE** — confirmed `doPACE`'s real signature from source but didn't implement it: correct use requires first reading/parsing `EF.CardAccess` (root file system, pre-applet-selection) to select the right `PACEInfo`, additional protocol surface not worth the hallucination risk in an uncompiled module. Documented as a known gap in the module's doc comment.
    - **Mobile-side JS/TS**: `mobile/src/native/nfcPassportReader.ts` (new) wraps the bridge, base64-decodes to `Uint8Array` via the same `Buffer` pattern `zkProving.ts` uses, exports `readPassport(mrz: {documentNumber, dateOfBirth, dateOfExpiry})`, documented as Android-only. `RegisterScreen.tsx`'s NFC-step TODO lightly updated to point at it (same light-touch style as the existing proving-step TODO in that file — not wired up, still needs an MRZ-input UI this screen doesn't have).
    - **Verification, stated plainly**: `cd mobile && npx tsc --noEmit` is clean. The Kotlin/Gradle side could **not** be compile-checked at all — this environment has no JDK whatsoever (`which java` finds nothing, no `/usr/lib/jvm`, no passwordless `sudo` to install one), which is a step short of even reaching the anticipated "missing Android SDK" failure; `./gradlew :app:compileDebugKotlin` fails immediately on `JAVA_HOME is not set`. Every JMRTD/scuba API call was instead verified by direct source inspection (file paths and exact signatures recorded above), not compiled. Treat the Kotlin as best-effort-verified-by-reading, not proven-to-build, until it's run through a real Gradle+Android-SDK environment.
59. [DONE] `RegisterScreen.tsx` wired to the real NFC module (log #58) — the first end of this screen that now calls something real instead of a TODO stub. Added an MRZ input form (passport number, DOB, expiry, `YYMMDD`, gated on all three being filled before "Begin Registration" enables) and an Android platform check up front. `start()` now calls the real `readPassport()` from `../native/nfcPassportReader` and, on success, throws a distinguished `NotImplementedError` (caught separately from real failures, so the user sees "scan succeeded, registration not complete yet" with the actual DG1/DG15/SOD byte counts read, rather than a generic failure alert) instead of pretending to complete registration — `setRegistered`/`setPassportName` are no longer called from this screen since it can no longer honestly claim registration finished. Left a concrete, code-shaped TODO for what comes next once circuit-input assembly and the proving key exist (`fetchZkAsset` → `computeWitness` → `generateProof` → `encodeGroth16Proof`, referencing the exact functions already built in `zkProving.ts`/`proofEncoding.ts`). The genuinely new, not-yet-started piece this surfaced clearly: assembling the circuit's `inputs.json` from raw DG1/DG15/SOD bytes (SOD certificate-chain parsing, Merkle proof against `AllowedMerkleRoots`, the Poseidon hashes the circuit expects) — nobody has built this yet; `passport-zk-circuits`' own test/inputs generation pipeline is the reference for the exact schema. `npx tsc --noEmit` clean; not runtime-tested (no device).
60. [PARTIAL] `.wcd` witness-graph generation for the real passport circuit (closes out the file-format-mismatch gap `zkProving.ts`/#55/#56 flagged) — reached a genuine upstream `circom-witnesscalc` bug on the real circuit, after clearing every earlier stage. Not a dead end from environment/resource limits; a specific, reproducible, fileable tool bug.
    - **Stage 0/1 — tool builds and works.** Cloned `github.com/iden3/circom-witnesscalc` (`d48eb7c97857d46b8a75c94ab96f769207263245`) and `github.com/rarimo/passport-zk-circuits` (`30b0be2e83062e19f21237c03317c9a26f2dab59`) into scratchpad. `cargo build --release` (root crate) then `cargo build --release -p build-circuit` (a separate workspace member pulling `circom`'s own compiler crates straight from `github.com/iden3/circom@master`) both succeeded cleanly with this environment's `clang-18`/`libclang-18-dev` (repo's README asks for clang-19; 18 was fine) and system `protoc` — no missing deps. Verified `build-circuit`/`calc-witness` end-to-end against the repo's own small `test_circuits/circuit1.circom` (a trivial multiplier): produced a 184-byte `.wcd` graph, then `calc-witness` consumed it and produced a witness — confirms the whole toolchain genuinely functions here before spending time on the real target.
    - **Stage 2 — found the real circuit-instantiation step.** `passport-zk-circuits` has no committed concrete `.circom` files for any variant (`test/circuits/generated/` is checked in empty, README-only). The actual generator is `test/process_passport.js`'s `processPassport()` (invoked by `npm run test` via `test/automatisationTest.js`), which parses a real passport's ASN.1-encoded SOD plus DG1/DG15 to derive `RegisterIdentityBuilder`'s 10 constructor params (signature/hash algo IDs, doc type, EC/DG1/DG15 byte-shifts, block counts) and writes both the concrete `.circom` file and its matching `inputs.json` — but this needs a **real signed passport** as input, which this environment obviously doesn't have and can't fabricate (a fake ASN.1 SOD wouldn't carry a real PKI signature chain, so it wouldn't tell us anything about whether the params are right). Sidestepped this by going straight to Rarimo's own published release assets instead: GitHub releases of `passport-zk-circuits` ship prebuilt bundles per circuit variant, and the variant names encode the exact constructor params in the same order `processPassport()` uses (confirmed by reading the `old_naming_convention` template string at `test/process_passport.js:783` next to the `writeToCircom` call it labels). Picked `registerIdentity_11_256_3_2_336_216_NA` — an exact match for the `registerIdentity_11_256` family HANDOFF #55/#58 already tied to this repo's `vk_sha256.bin` (confirmed again here: downloaded that release's `.json` VK, `protocol: groth16, curve: bn128, nPublic: 5`, matching). Hand-wrote the concrete `.circom` file from `RegisterIdentityBuilder`'s template using the name-decoded params directly (`11, 256, 3, 2, 336, 216, 0, 0, 0, 0` — sig_algo/dg_hash_type/doc_type/ec_blocks/ec_shift/dg1_shift/dg15_sig_algo=0/dg15_shift=0/dg15_blocks=0/aa_shift=0 for the `_NA` "no DG15/AA" suffix). Caveat worth flagging: `process_passport.js`'s own naming-string expression re-multiplies the already-bit-converted `ec_shift`/`dg1_shift` variables by another `*8` when building the display name, while the actual `writeToCircom` call uses the un-re-multiplied bit values — so the release name's shift digits are not guaranteed to be literally identical to the constructor's raw shift arguments (this looks like a real, separate, latent quirk in that script's naming code, independent of the `circom-witnesscalc` bug below). Used them as-is; sufficient for exercising the tool at real scale even if not guaranteed bit-identical to Rarimo's original build inputs. Checked all of `registerIdentityBuilder.circom`'s and its includes' `include` paths — everything resolves inside the repo's own `circuits/lib/`, no `circomlib`/`node_modules` include paths actually needed at compile time despite `npm install` being run first (harmless, not required).
    - **Stage 3 — compiles clean through R1CS, then hits a real `circom-witnesscalc` bug building the witness graph.** `./build-circuit test/circuits/generated/registerIdentity_11_256_3_2_336_216_NA.circom out.wcd --r1cs out.r1cs -v`: circom frontend resolved all 347 templates (full SHA-256, RSA-PSS-2048 signature verification, 3x Poseidon, Sparse Merkle Tree verification, BabyJubJub EC ops — the real passport-verification circuit, not a stub), produced a clean R1CS (**678,158 non-linear constraints, 650,565 wires**, "R1CS written successfully"), then panicked while building the witness-calculation graph itself:
      ```
      thread 'main' panicked at extensions/build-circuit/src/main.rs:184:5:
      size = 0
      stack backtrace:
         2: build_circuit::operator_argument_instruction_n
         3: build_circuit::process_instruction
         4: build_circuit::run_template
         5: build_circuit::build_graph
         6: build_circuit::main
      ```
      **100% reproducible** — ran twice (once plain, once with `RUST_BACKTRACE=1`), identical panic both times, identical point in the verbose log both times (right after the *second* "Store subcomponent signal (location: Indexed, template: `PassportVerificationBuilder_339`, subcomponent idx: 0, num: 1024): 3223 + 1025 = 4248" line). Not a resource wall: peak RSS 12GB (of 15GB available), wall time 1:50 both runs.
      **Root cause, read from source** (`extensions/build-circuit/src/main.rs`): `operator_argument_instruction_n` unconditionally asserts `size > 0` (line 184) before processing a multi-element argument-fetch. It's reached (via `store_bucket`'s `AddressType::SubcmpSignal` branch around line ~975) whenever a `<==` assignment stores an array into a subcomponent's signal array, sized by `resolve_size_for_template(&store_bucket.context.size, ...)`. `registerIdentityBuilder.circom:172` does `passportVerifier.dg15 <== dg15;`, and `passportVerificationBuilder.circom:73/93` declares `signal input dg15[DG15_BLOCK_NUMBER * HASH_BLOCK_SIZE]` — with our `_NA` variant's `DG15_BLOCK_NUMBER = 0` (no DG15/AA present), that's a **legitimate zero-length array signal assignment**, valid circom (real `circom`+`snarkjs` compiles all of Rarimo's own `*_NA`-suffixed release variants, e.g. `registerIdentity_20_160_3_3_736_200_NA`, `registerIdentity_15_512_3_3_336_248_NA` — several exist in their release history), but `circom-witnesscalc`'s graph-builder has no zero-size short-circuit for this specific store path and hard-panics instead of treating it as a no-op. This is a real, narrow, upstream bug — not an environment limitation — and is concrete enough to file against `iden3/circom-witnesscalc` as-is (repro circuit + exact panic + line number + root-cause read all above).
    - **Where things are left**: nothing committed to the actual repo (per instructions — these are large, non-source artifacts). In scratchpad only (`/tmp/.../scratchpad/`, ephemeral, not part of this repo): `circom-witnesscalc/` (built tool, `target/release/build-circuit` + `calc-witness`), `passport-zk-circuits/` (cloned + `npm install`ed), `wc_output/registerIdentity_11_256_3_2_336_216_NA.r1cs` (194MB, the one real artifact Stage 3 did produce — a clean, complete R1CS for the real circuit) plus both run's logs (`build.log`, `build_backtrace.log`), and `release_assets/` (downloaded VK JSON + partial zip central-directory probes used to confirm the target variant and rule out the official release bundles shipping a `.circom` source — they don't; only compiled `.cpp`/`.dat`/`.wasm`/`circuit_final.zkey`/`verifier.sol` per variant, confirming the file-format gap #55/#56 already flagged is real and this investigation's own instantiation-reconstruction was necessary, not redundant). No `.wcd` graph file exists as a result of this session — Stage 3 did not complete. Whoever picks this up next should either (a) file the bug above against `iden3/circom-witnesscalc` and wait/patch, or (b) special-case zero-size `SubcmpSignal` array stores locally in a fork (the fix looks narrow: short-circuit `operator_argument_instruction_n`/its caller to return `vec![]` when `size == 0` instead of asserting) — not attempted here since fixing the tool itself was explicitly out of scope for this task.
61. [DONE] `circom-witnesscalc` upstream fixes — picked up item #60's option (b) (local fork fix). Fixed 4 real, distinct bugs, each hit sequentially deeper into the same real `registerIdentity_11_256_3_2_336_216_NA` circuit as each prior one was fixed, until `build-circuit` finally ran the whole circuit to completion and wrote a real `out.wcd`. Forked to `github.com/kei-nan/circom-witnesscalc`. Two independent PR branches pushed, both based directly on `main` (not stacked on each other, even though bug 4's fix was originally built on top of bug 3's branch — bug 4 turned out to touch a completely disjoint region of the file from bugs 1–3, so once confirmed via `git apply --check` that its diff applies cleanly to pristine `main`, the branch was rebuilt from `main` and force-pushed over the original stacked version, `a11508d` → `141f161`): `fix/zero-size-array-store` (bugs 1–3, commits `1ffe190`/`b1581db`) and `fix/nested-dynamic-branch` (bug 4, commit `141f161`). Neither PR is actually opened on GitHub yet (no `gh` CLI or API token in this environment) — compare URLs are `https://github.com/kei-nan/circom-witnesscalc/pull/new/fix/zero-size-array-store` and `https://github.com/kei-nan/circom-witnesscalc/pull/new/fix/nested-dynamic-branch` (both base `main`, independently mergeable in either order) — the human needs to click through and paste in the drafted titles/descriptions, which aren't saved anywhere but this file and the conversation that produced them; re-derive from the commit messages if lost. Before writing any of these fixes, checked upstream for existing relevant work: no open PR touches this code (the closest, #41, only touches `parser`/`vm2`/`ast.rs`); issue #21 ("allow signals to be used in ternary condition") confirms the maintainers already know dynamic-ternary support is narrower than it should be, unresolved, no comments. Every fix: reproduced the pre-fix panic against the unpatched binary (`git stash`), reproduced post-fix success, and cross-checked against a *real* `circom` 2.2.3 compiler + wasm witness calculator (built from source at `~/.cargo/git/checkouts/circom-b602d4d383860676/`, already vendored as a `build-circuit` dependency — no separate download needed) on equivalent minimal circuits, confirming the fixed behavior matches the reference implementation's actual output values, not just "doesn't panic." Added `test_circuits/circuit26/27/28/29_*.circom` regression circuits.
    - **Bug 1 (zero-length array store)**: exactly item #60's diagnosis — `operator_argument_instruction_n` asserted `size > 0`; a `_NA` circuit variant's `signal input dg15[0]` (DG15_BLOCK_NUMBER=0) legitimately stores a zero-length array into a subcomponent input. Fixed: short-circuit to `vec![]` for `size == 0` (every call site already treats it as a no-op).
    - **Bug 2 (unset subcomponent output read)**: found immediately after fixing bug 1 — `passport-zk-circuits`' `KaratsubaOverflow(CHUNK_NUMBER)` (recursive Karatsuba multiplication, used for RSA-2048 modular exponentiation in signature verification: `CHUNK_SIZE=64 × CHUNK_NUMBER=32` limbs) has a base case (`CHUNK_NUMBER == 1`) that assigns `out[0]` of its 2-element `out` array but never `out[1]` — a real, upstream *circuit* pattern (not something introduced by this investigation), and the parent level's cross-term formula does read that unassigned `out[1]`. Verified against real circom's own wasm witness calculator on an equivalent minimal circuit (`karatsuba_min.circom` in scratchpad, not committed) that this is legitimate: it silently computes `0` for the never-assigned index (the signal buffer's default), no compile error, no warning, even with `--O0`. `build-circuit` already had this exact default-to-zero pattern for plain `AddressType::Signal` loads (two sites) but was missing it on the two `AddressType::SubcmpSignal` load sites that hit this case; extended to match.
    - **Bug 3 (function call as a ternary branch)**: found after fixing bug 2, in a completely different subsystem — `bigIntFunc.circom`'s `short_div` has `if (norm_b[k] != 0) { ret = short_div_norm(...); } else { ret = short_div_norm(...); }`, a signal-dependent (dynamic) condition whose single-statement branches are function calls, not plain stores. `build-circuit`'s dynamic-branch ("ternary") lowering only handled a branch instruction that's a `Store` or a `Return`; a bare `Instruction::Call` (circom's IR for `ret = someFn(...)`, which embeds its destination in `return_info: ReturnType::Final` rather than wrapping in a separate `Store`) fell through to a hard panic. Fixed by adding `call_function_for_ternary()`, which evaluated the call using the same logic as the file's existing general-purpose `Instruction::Call` handler, then extracted `(Var, lvar_idx)` from `ReturnType::Final`'s destination info the same way `store_function_variable()` did for a plain `Store`. Superseded by bug 4's fix below (both helpers removed as dead code) — kept here for the historical record of what got tried first.
    - **Bug 4 (nested multi-statement dynamic branch) — found, diagnosed, and fixed**: hit immediately after fixing bug 3, one level deeper — `short_div_norm`'s `if (long_gt(...) == 1) { mult = long_sub(...); if (long_gt(...) == 1) { return qhat - 2; } else { return qhat - 1; } } else { return qhat; }`. The if-branch here is *two* statements (an assignment, then a nested dynamic conditional return), not one — `build-circuit`'s ternary lowering hard-asserted every branch was exactly one instruction (`assert_eq!(branch_bucket.if_branch.len(), 1, ...)`). Initially scoped as a materially bigger, higher-risk change than bugs 1–3 and deliberately deferred; picked back up same session at the human's request. **Found prior art before writing code**: checked `iden3/circom-witnesscalc`'s open PRs and issues first (see above) — nothing to build on, confirmed this needed a real implementation, not adaptation of existing work. **Found existing general-purpose infrastructure to reuse**: `process_instruction`'s *template*-level `Instruction::Branch` handling (`collect_branch_stores`/`collect_branch_stores_from_branch`, ~line 330) already does exactly this generalization for template bodies — collect a branch's writes (recursing through nested branches), merge if/else into `TernCond` per variable/signal where they disagree, falling back to the pre-branch value where only one side touched it — the *function*-level code just never got the same treatment. Ported the same reconciliation strategy to functions, but implemented differently at the collection step: rather than a parallel "collect without executing" walker (which is what the template-level code does, and which would silently mis-evaluate any statement that reads a variable another statement in the *same* branch just wrote, since collection there doesn't mutate `cmp.vars` as it goes), each branch is *actually run* through the ordinary recursive interpreter (`process_function_instruction` itself, looped exactly like the existing static-condition case) against its own cloned copy of `fn_vars`, so read-after-write within a branch and arbitrarily nested branches (dynamic or static) both just work via normal recursion. The two resulting states are reconciled three ways: both branches return → ternary of the two return values, propagated up immediately; exactly one returns → deferred via the existing `pending_returns` mechanism (previously only reachable for a single-instruction if-with-no-else; extended to the symmetric else-only-returns case too, via a new `UnoOperation::Lnot` node negating the condition); neither returns → new `merge_ternary_fn_vars` reconciles every local variable var-by-var into a `TernCond`, mirroring the template-level function's exact strategy. This fully subsumed bug 3's `call_function_for_ternary`/`store_function_variable` (a function call as a branch's one statement is just a one-instruction branch, handled by the same general path) — both removed as dead code, net negative diff (`+160/-202` on top of bug 3's commit).
    - **Result**: `build-circuit` now runs the entire real circuit to completion — all the way through `RegisterIdentityBuilder_347` (the top-level template) — and writes a complete `out.wcd`: 49MB, 3.78M nodes after tree-shaking/optimization (down from 4.6M raw). One non-fatal warning along the way, not yet investigated: `[warning] 16864 signals are not set` (plausibly dead signals that get tree-shaken regardless, given 168,794 + 668,100 unused nodes get removed across the two optimization passes right after — not confirmed, flagged for whoever looks at this next). Sanity-checked the resulting graph structurally with `calc-witness` (no real passport data available to compute an actually-meaningful witness): loads cleanly (3.7M nodes, ~90ms) and fails gracefully on missing required inputs (`"missing input signal at offset 5"`) rather than an internal panic, confirming the graph itself isn't corrupt. Full run: ~2:00–2:05 wall-clock, ~11GB peak RSS, each time (four full runs total across this session, one per bug fixed). The regression circuit for this fix, `circuit29_nested_dynamic_branch.circom`, mirrors `short_div_norm`'s exact shape and was checked across all four branch-condition combinations against real circom's wasm calculator - numerically exact match (`48`/`49`/`20`/`20`) between real circom and `build-circuit`+`calc-witness`'s own output, not just "didn't crash."
    - **What's still not done**: the fixes are fork-only (not merged upstream, PRs not even opened yet - see the note above); the actual `out.wcd` produced only exists in this session's ephemeral scratchpad, not published anywhere the mobile app could fetch it from (IPFS, per the log #55/#56 plan); and there may well be a bug 5 in some circuit path this specific input combination never exercised - every one of bugs 1-4 was only found by fixing the previous one and re-running the exact same circuit, so "it built once" is encouraging but not a guarantee against a different code path panicking on different inputs.
62. [PARTIAL] DG1/DG15/SOD → circuit-inputs assembly (mobile) — the item log #59 flagged as "genuinely new work, not yet started." Landed the self-contained half; surfaced a second real blocker (Rarimo's live certificate registry) that turns out to be separate, comparable-sized work of its own, not yet started.
    - **Split the task in two once actually researched**, not before: (A) parsing DG1/DG15/SOD into the circuit's field/shift/limb inputs — fully self-contained, no external services. (B) `slaveMerkleRoot`/`slaveMerkleInclusionBranches` — these prove the passport's DSC is ICAO-trusted, and turn out to come from `CertificatesSMT`, a live Sparse Merkle Tree Rarimo maintains on their own zkRollup (ERC-7812 `EvidenceRegistry`/`Registrar` pattern; `CertificatesSMT` deployed at `0xA8b350d699632569D5351B20ffC1b31202AcEDD8` per docs.rarimo.com/zk-passport/contracts). No documented client API for reading a proof from it was found — same kind of source-diving log #57 did for NFC reading, not done here. Scoped this session to (A); (B) is a new, separate follow-on item.
    - **Built (A)**: `mobile/src/chain/asn1.js` — verbatim copy (not hand-ported, to avoid transcription risk in a 3621-line decoder) of passport-zk-circuits' `test/asn1.js` (itself a fork of Lapo Luchini's asn1js), MIT, with a hand-written `asn1.d.ts` for the slice of its output shape actually used. `mobile/src/chain/sodParser.ts` — TypeScript port of `test/process_passport.js`'s extraction pipeline (CMS SignedData navigation, RSA/ECDSA pubkey+signature extraction, SIGNATURE_TYPE classification, SHA-padding, big-int limb chunking), exporting `buildCircuitInputs(dg1, dg15, sod) -> { variant, inputs }`. Ported function-for-function rather than restructured, since this is dense, previously-tested bit-manipulation logic where "cleaning up" risks silently changing behavior — but with three disclosed, deliberate departures (see the file's own doc comments, each marked "DEPARTURE"): (1) OCTET_STRING byte extraction goes through a `dump`-based DER-length-header-stripping helper instead of asn1.js's `.content()`, which has an undocumented UTF-8-fallback branch for OCTET_STRING that's a real (if low-probability) correctness hazard the original silently inherits; (2) `getDg1Shift`/`getDg15Shift`/`getEcShift` throw when a hash isn't found instead of silently returning a garbage shift computed from the whole haystack's length; (3) `extract_signature`'s PSS-salt lookup fully optional-chains a 4-hop navigation instead of the original's 1-hop chain, which throws a `TypeError` for any plain (non-PSS) RSA signature instead of falling back to "no salt" the way its own `cond ? val : 0` ternary implies it should.
    - **Found and fixed a real bug mid-port**, caught only because the cross-check (below) initially failed: an early draft conflated `dg15Shift` (offset of DG15's hash within the encapsulated content) with `aaShift` (offset of the Active-Authentication public key within DG15 itself) — two genuinely different values the original computes and uses separately. Fixed by computing both.
    - **Found and preserved (rather than "fixed") a separate real quirk**, confirmed empirically: `process_passport.js`'s circuit-variant *display name* string re-multiplies the already-bit-converted `ec_shift`/`dg1_shift` by another `*8`, diverging from the values its own `writeToCircom` call actually compiles the circuit with — independently already flagged as suspicious in log #60 ("not guaranteed to be literally identical to the constructor's raw shift arguments"), now confirmed as real via direct comparison against the unmodified reference script. Since matching Rarimo's actual published release-asset filenames (which use this same buggy naming convention) is the entire point of computing a name, `sodParser.ts` reproduces the doubling in `variant.name` specifically while keeping the real `CircuitVariant` numeric fields correct — documented in-line so nobody mistakes the name's embedded digits for real shift values.
    - **Verification**: hand-built a synthetic but structurally real SOD fixture — genuine ASN.1 DER (hand-encoded CMS SignedData with exactly the two signed attributes — contentType, messageDigest — a real ICAO SOD has; `openssl cms -sign`'s default output was tried first and rejected because it also adds `signingTime`/`smimeCapabilities`, which broke the `getZero` heuristic's "messageDigest is the attribute set's last element" assumption), a real RSA-2048 keypair + self-signed cert, and a real RSASSA-PKCS1v1.5-SHA256 signature (Node's `crypto.sign`). Cloned `rarimo/passport-zk-circuits` and ran its actual, unmodified `processPassport()` on this fixture as ground truth. `mobile/src/chain/sodParser.test.ts` (8 tests) checks the TypeScript port's output against that captured ground truth field-by-field (variant name, pubkey/signature limbs, all three padded bit arrays), *and* — independent of both the reference script and this test file — that the extracted `signedAttributes` bytes cryptographically verify (`crypto.createVerify('RSA-SHA256')`) against the fixture's real signature and public key. All 8 pass. Added `@noble/hashes` (already a transitive dependency via `@polkadot/util-crypto`) as an explicit dependency for the SHA-1/256/384/512 this needs — pure JS, no native module, works under RN/Metro same as the rest of this file.
    - **Wired into `RegisterScreen.tsx`**: the proving step now actually calls `buildCircuitInputs` on the real NFC-read bytes and reports the resolved circuit variant name in its (still-thrown) `NotImplementedError`, moving the screen's failure boundary from "can't even start assembling inputs" to "inputs assembled, still needs skIdentity + Merkle proof + proving key" — real, visible progress, not cosmetic.
    - **Explicitly not attempted**: `skIdentity` generation (the reference script derives it from public SOD bytes, which is fine for deterministic test fixtures and wrong for production — needs to be a real locally-generated secret, not built here); the DG15-present (Active Authentication) branch of the naming/shift logic is ported but *not* independently verified the way the NA path now is (no DG15-bearing fixture was built or cross-checked — flagged in-line in `sodParser.ts` as unverified, including a further inconsistency spotted but not resolved: the reference's own `writeToCircom` call passes `aa_shift` in raw byte units despite its inline comment claiming bits); `.wcd` graph / proving key acquisition (log #61); and item (B) above (Rarimo `CertificatesSMT` proof-fetching) — next person up should expect this to need the same kind of "clone the real source and read it" investigation log #57/#61 did, not another docs pass (docs.rarimo.com's own pages, checked here, don't disclose a client-side proof-fetching method).

63. [PARTIAL] Our own DSC certificate registry, replacing dependency on Rarimo's `CertificatesSMT` — architectural fix for the blocker log #62 left open, not just an integration.
    - **Why this needed a real decision, not just plumbing**: `slaveMerkleRoot` proves a passport's Document Signer Certificate (DSC — the cert that directly signs the SOD, not the CSCA root) is ICAO-trusted, by Merkle-inclusion. Rarimo's own deployment reads that root from `CertificatesSMT`, a Sparse Merkle Tree they host on their own zkRollup. Depending on it would make this chain's citizen registration depend on infrastructure this project doesn't govern — the same category of dependency this codebase already rejected elsewhere (see the Full-vs-Light circuit and IPFS-vs-server-hosted-key decisions earlier in this file). `pallet-identity`'s `AllowedMerkleRoots` was already built governance-gated (`AdminOrigin`) for exactly this situation, so building our own equivalent tree and registering our own root is a better fit than it might first look, not a workaround.
    - **Reverse-engineered the exact tree spec from the real circuit source** (cloned `rarimo/passport-zk-circuits`, not docs — docs don't cover this): `circuits/merkleTree/SMTVerifier.circom` is the standard iden3-style Poseidon Sparse Merkle Tree (`SMTHash1(key,value)=Poseidon([key,value,1])` for leaves, `SMTHash2(L,R)=Poseidon([L,R])` for internal nodes) at a fixed depth of 80. `circuits/passportVerification/passportVerificationBuilder.circom` shows the tree's leaves are keyed by `pubkeyHash` (key=value=pubkeyHash, a set-membership tree), computed per SIGNATURE_TYPE: RSA/RSA-PSS hashes only the modulus's low 960 bits (low 15 64-bit limbs, regrouped into 5×Poseidon inputs) — the full modulus is separately checked elsewhere for signature validity, so this truncation only affects certificate identification, not security; ECDSA hashes `Poseidon([x mod 2^248, y mod 2^248])`. Cross-checked both formulas against that repo's own `test/process_passport.js#getFakeIdenData` (whose "fake" single-leaf root helper turns out to compute the real pubkeyHash formula correctly — only the "no real Merkle siblings" part is fake) using the same RSA modulus sodParser.test.ts's fixture already carries as an independent oracle.
    - **Vendored Poseidon rather than depending on `circomlibjs`**: `mobile/src/chain/poseidon.js` + `poseidon.js`'s constants table are a byte-identical copy of `rarimo/passport-zk-circuits`' own `test/poseidon.js` (itself circomlibjs's reference implementation, MIT), same pattern as the existing `asn1.js` vendoring. Chosen over pulling in `circomlibjs` directly because that package's poseidon build goes through a WASM field-arithmetic backend that isn't guaranteed to run under React Native/Hermes; this file is pure BigInt arithmetic, no WASM, already proven to work in this codebase's RN target.
    - **Built `mobile/src/chain/certificateTree.ts`** (mobile-safe, no heavy deps): `extractPubkeyFromCertificate` (reuses sodParser.ts's shape-based ASN.1 DFS helpers — exported for this purpose — against a standalone X.509 cert instead of a SOD), `computeDscPubkeyHash`, `fieldElementToBytes32BE` (32-byte big-endian, matching `Fr::from_be_bytes_mod_order` in `runtime/src/verifier.rs`), and `verifyInclusion` — a from-scratch reconstruction of `SMTVerifier.circom`'s logic, for mobile to sanity-check a fetched proof before an expensive local proving attempt.
    - **Real bug, caught by tests, worth recording**: a first version of this file also included the actual multi-certificate tree-*building* logic, hand-rolled, and got it wrong twice. First attempt built a naive tree with a real branch at every bit level of a shared key prefix (an explicit `Empty` sibling hashed as literal 0 at each shared level) — multi-certificate tests failed `verifyInclusion`. Second attempt assumed the fix was "path-compress the tree" (skip straight to the actual divergence depth) — cloned `iden3/js-merkletree` (the actual production implementation of this same tree, used across Polygon ID) to check, and found its real behavior matches the *first* (per-bit, real-Empty-siblings) design, not path compression. The actual bug was in `verifyInclusion`'s boundary detection: it scanned for the *first* zero sibling from the top and treated that as "start of padding," which is wrong whenever a real (non-padding) zero sibling occurs first — exactly the per-bit-with-Empty-sides case. Fixed by scanning from the bottom instead: the real/padding boundary is "one past the *last* nonzero sibling," since only trailing zeros (deeper than anywhere the tree ever branched) are actually padding. Added a hand-computed two-leaf regression test (`certificateTree.test.ts`) using keys that deliberately share bit 0, specifically to keep this from regressing silently again.
    - **Built `scripts/certificate-registry/`** (deliberately separate from `mobile/` — this only ever runs off-chain as part of certificate onboarding, never on a phone, and pulls in `@iden3/js-merkletree` + its WASM-based Poseidon, which has no reason to be anywhere near the RN bundle): `buildTree.ts` takes a directory of DER/PEM certificates, builds the tree, self-checks every generated proof against its own output via `verifyInclusion` before writing anything (a bug here would otherwise only surface on-device, mid-registration, for a real citizen), and emits `{root, certificates: [{pubkeyHash, siblings}]}` as 32-byte-BE hex. `buildTree.test.ts` covers zero/one/four-certificate cases.
    - **No pallet code changes needed**: `add_allowed_merkle_root(merkle_root: [u8;32])` (pallet-identity, call index 4) already takes exactly this tool's `root` output and is already `AdminOrigin`-gated — the governance path was already correctly designed, it just needed a root that isn't Rarimo's.
    - **What's still genuinely unsolved, and why it's not a coding task**: the tree's leaves must be real DSC certificates, and DSCs (unlike CSCA roots, which ICAO publishes openly as the "Master List") are normally distributed through ICAO's PKD, a paid/state-membership service this project has no access to. There's no scripted "download everything" path. The realistic model — and the reason `AllowedMerkleRoots` is additive and governance-gated rather than a single fixed value — is incremental: source what's legitimately available (national PKI publication pages, citizens' own passports, open mirrors), verify each DSC chains to a recognized CSCA (which *is* freely available), and add roots one legislature vote at a time as real citizens are actually encountered. Documented in `buildTree.ts`'s own doc comment so this doesn't need re-deriving.

64. [ ] Rarimo's Noir/UltraHonk migration — researched as decision support, no code changed. Confirmed Rarimo is actively moving this whole circuit family from circom/Groth16 (what this project has built against — real VK assets, `ark-groth16` verifier, `rapidsnark` mobile proving) to Noir/UltraHonk (`registerViaNoir`), via `github.com/rarimo/passport-zk-circuits-noir` (actively released; the circom repo's latest release is over a year stale by comparison). The public-signal schema barely changes (`(dg15_pk_hash, passport_hash, dg1_commitment, sk_hash, icao_root)` maps almost 1:1 onto this project's existing 5-signal design), the certificate-tree situation is the same shape (a plain circuit input, not hardwired to Rarimo's contract — our own-tree plan from log #63 should carry over), on-device Noir proving is confirmed shipping in Rarimo's own `rarime-rn-sdk` (a real bare-native `RnNoir` module, not vaporware), and a maintained `no_std`/`ark-bn254` UltraHonk verifier crate exists (`ultrahonk-no-std`, used in production on a live Substrate parachain — directly relevant since our runtime is also Substrate/WASM). Against that: several of the load-bearing pieces are individually stale even in a fast-moving ecosystem, and Rarimo's own stack shows internal version drift (their circuit repo pinned to an older prover version than their own mobile SDK already uses). Also considered `fflonk` (via `snarkjs`) as a third option — same circom circuits as today, no per-circuit trusted setup (like Noir's advantage, without needing Rarimo's migration at all) — but its mobile proving story is completely unresearched, unlike Noir's. **Recommendation: keep building on the current Groth16 path for now; revisit once `passport-zk-circuits-noir`/`registerViaNoir` graduate from "supported alongside legacy" to primary** — a checkpoint, not a decision to ignore this.

65. [DECISION] **Dropped Rarimo entirely as the passport-ZK circuit vendor; replatforming to ZKPassport.** Follow-up to log #64, this time with hands-on verification (real `nargo`/`bb` toolchains installed and a real circuit actually compiled, not just docs review) rather than research-only. Corrects two of log #64's claims and surfaces one new security-relevant fact, on top of which the human made the vendor-drop call — recorded here as a decision, not just a finding.

    - **Verified `register_identity`'s real Noir source** (`github.com/rarimo/passport-zk-circuits-noir`, cloned directly): `fn main(...) -> pub (Field, Field, Field, Field, Field)` returns `(dg15_pk_hash, passport_hash, dg1_commitment, sk_hash, icao_root)` — confirms log #64's claimed 1:1 mapping onto this project's existing 5-signal layout was correct. `icao_root` is a plain `Field` input threaded straight into `smt_verifier(icao_root, leaf, key, inclusion_branches)`, confirming it's not hardwired to any hosted contract either. `noir_dl_lib/src/smt.nr`'s `smt_hash1`/`smt_hash2` (`Poseidon([key,value,1])` / `Poseidon([l,r])`, depth 80, leaf key=value=pubkeyHash) are bit-identical to `certificateTree.ts`'s already-documented spec (log #63) — that file's reverse-engineering from the circom source turned out to be exactly right for the Noir port too.
    - **Actually compiled it** — installed `nargo`/`bb` via the official `noirup`/`bbup` installers in this session's scratchpad (nothing added to this repo). Latest stable `nargo` (beta.25) crashes the circuit with an internal compiler error; pinned to the README's stated `1.0.0-beta.1` it compiles clean (only bignum-library lint noise) and produces a real ACIR artifact — **77,197 ACIR opcodes / 6,001 Brillig opcodes** for the RSA-2048/SHA-1/no-active-auth variant. First genuine build-and-measure of this circuit family in this project, not a docs claim.
    - **Correction to log #64**: *"on-device Noir proving is confirmed shipping in Rarimo's own `rarime-rn-sdk` (a real bare-native `RnNoir` module, not vaporware)" was wrong.* Checked the actual published package (`@rarimo/rarime-rn-sdk` v0.3.1, npm + its GitHub source, not the marketing docs): its ZK dependency is `@solarity/zkit`, which is `snarkjs`-based **circom/Groth16** tooling, not Noir. No `RnNoir` package exists anywhere under the `@rarimo` npm scope. The SDK also hard-requires Expo (`expo-module.config.json`, `expo` peer dependency) — this project's mobile app deliberately isn't on Expo. Rarimo's own mobile SDK has not actually moved off the circom path log #55–62 already integrated against; log #64's "internal version drift" framing was built on this same wrong premise.
    - **Confirmed real, with a version-pinning catch**: `ultrahonk-no-std` (`github.com/zkVerify/ultrahonk_verifier`) is genuinely in production — `zkVerify/zkVerify`'s live runtime has `verifiers/ultrahonk/` → `pallet-ultrahonk-verifier`, compiled into their actual Substrate WASM runtime (proof by construction that it's wasm32/no_std-clean), plus a dedicated `verifier-nostd-check/` CI harness in that repo. But: zkVerify pins **two separate tags** of the crate for two different Barretenberg proof-format eras (`v0.2.1`≈bb 0.84.x, `v0.3.2`≈bb 3.0.x — the tag names are literally derived from `bb --version`), while Rarimo's Noir README states `bb version = 0.66.0`. Running `bbup` against nargo `1.0.0-beta.1` auto-resolves to bb **0.82.2** (not 0.66.0), so the README figure looks stale rather than a real target — but worth independently reproducing before trusting it.
    - **New finding, not in log #64 at all**: `bbup -v 0.66.0` prints a live warning on install — *"There is a critical soundness issue in Ultrahonk in this version of Barretenberg. It is recommended to update to v0.82.2 or greater. Any solidity verifier contracts must be regenerated using a patched version of Barretenberg."* Nothing in this project was ever built against 0.66.0, so nothing here is affected, but it's a concrete reason not to trust any Rarimo-published bb-0.66.0-era Noir artifacts at face value if they ever surface.
    - **ZKPassport (`github.com/zkpassport/circuits`) evaluated as the alternative, and is the one adopted**: Apache-2.0, Noir-native from the start (not a circom port), last pushed 2026-07-24 vs. Rarimo Noir repo's 2025-11-18 (~8 months stale by comparison), 91 stars vs. 16. Unlike Rarimo, ZKPassport ships a complete first-party stack around the circuit: `src/rust` + `src/solidity` verifiers of their own, `zkpassport-sdk` (RN SDK), `mobile-app`, `cloud-prover`, `zkpassport-proof-verifier`. Their `noir_rs` (Rust Noir/Barretenberg bindings, Android+iOS+desktop targets) is actively maintained — last commit 2026-07-13, tracking bb through 5.0.0 — correcting log #64's era-old "this Rust bindings repo looked stale" assumption (that assumption was never re-verified before now, and was wrong). `Swoir` (iOS Noir proving, independent org, pushed 2026-07-13) and `madztheo/noir-react-native-starter` (bare-RN, not Expo, reference wiring — independent developer, no Aztec/Noir-team affiliation, moderate maturity) round out a real, currently-maintained, non-Rarimo mobile-Noir ecosystem.
    - **Decision: drop Rarimo, replatform to ZKPassport.** Made by the human, not inferred — the mobile-proving toolchain was already vendor-neutral (nothing there was ever Rarimo's), so the only real Rarimo-specific asset was the circuit itself, and ZKPassport is currently the more actively developed, more complete, non-Rarimo option covering the same ICAO chain-of-trust problem.
    - **What this actually invalidates, concretely, so nobody assumes it "mostly carries over"**: `runtime/assets/vk_sha256.bin`/`vk_sha1.bin` (Rarimo Groth16 BN254 VKs — need ZKPassport-equivalent VK/verifying-key assets once the verifier scheme is decided), `runtime/src/verifier.rs`'s `RarimoGroth16Verifier` (Groth16-specific, and keyed to Rarimo's exact public-input layout — ZKPassport's layout is **not yet verified** against `pallet-identity`'s expectations, unlike Rarimo's which log #55–62 confirmed matched), `mobile/src/chain/sodParser.ts`/`asn1.js`/`poseidon.js`/`certificateTree.ts` (all vendored line-for-line from Rarimo's exact circuit source per logs #62/#63 — ZKPassport's DG1/DG15/SOD-to-circuit-input mapping and certificate-tree spec have not been reverse-engineered at all yet, and there's no reason to assume they match Rarimo's), `mobile/src/chain/zkProving.ts`/`proofEncoding.ts` (built specifically around `rapidsnark`/Groth16 proof encoding — a Noir/UltraHonk path needs different proving calls and a different proof-byte encoding entirely). **What's unaffected**: the NFC chip-reading work (logs #57–59, JMRTD/NFCPassportReader native modules) — that reads raw DG1/DG15/SOD bytes off the passport chip itself and has nothing to do with which circuit vendor consumes those bytes afterward.
    - **Not yet done, first things for whoever picks this up**: pull ZKPassport's actual circuit entry point and public inputs/outputs (same hands-on treatment this log gave Rarimo's `register_identity` — don't assume the layout without checking); decide the verifier crate (ZKPassport's own `src/rust`, vs. zkVerify's `ultrahonk-no-std` against ZKPassport-produced proofs — check bb-version compatibility the same way this log did for Rarimo); re-derive the certificate/SMT tree spec from ZKPassport's actual source rather than assuming log #63's tree design carries over unchanged; re-plan mobile proving against `zkpassport/noir_rs` or the `Swoir`+`noir_android` pairing (bare-RN, not Expo, per this project's existing constraint).

---

## Key references

- ZKPassport circuits: https://github.com/zkpassport/circuits
- ZKPassport SDK: https://github.com/zkpassport/zkpassport-sdk
- ZKPassport noir_rs (Rust Noir/Barretenberg bindings): https://github.com/zkpassport/noir_rs
- zkVerify ultrahonk-no-std verifier: https://github.com/zkVerify/ultrahonk_verifier
- MACI: https://maci.pse.dev/
- Polkadot OpenGov treasury: https://wiki.polkadot.com/learn/learn-polkadot-opengov-treasury/
- (Historical, no longer used) Rarimo Freedom Tool / passport-zk-circuits — dropped per log #65 above
- Kleros Court V2 (court architecture reference): https://kleros.io/
- Semaphore v4: https://docs.semaphore.pse.dev/
- polkadot-sdk-solochain-template: https://github.com/paritytech/polkadot-sdk-solochain-template
