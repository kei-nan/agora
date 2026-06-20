# Agora — Claude Handoff Context

Read this file at the start of every session. It captures the full project state.
Also read `CLAUDE.md` in this same directory for architecture decisions and references.

---

## Environment

- Ubuntu 24.04 WSL2, Rust 1.96 (via rustup)
- Project root: `~/democracy-chain`
- Chain template: polkadot-sdk-solochain-template (Substrate)

### Critical build command

Always use this — without WASM_BUILD_RUSTFLAGS the WASM build fails on Rust 1.84+:

```bash
WASM_BUILD_RUSTFLAGS="-C link-arg=--allow-undefined" cargo build --release
```

Dev node:
```bash
./target/release/agora-node --dev --tmp
```

---

## Monorepo structure

```
agora/
├── node/                        # chain binary — do not modify
├── runtime/                     # WASM runtime — all 5 pallets now wired in
├── pallets/
│   ├── template/                # original template pallet — reference only
│   ├── pallet-identity/         # crate: pallet-identity-zk
│   ├── pallet-voting/           # crate: pallet-voting
│   ├── pallet-treasury-ledger/  # crate: pallet-treasury-ledger
│   ├── pallet-courts/           # crate: pallet-courts
│   └── pallet-constitution/     # crate: pallet-constitution
├── circuits/                    # Noir ZK circuits (separate toolchain)
├── mobile/                      # React Native scaffold (src/ exists, not yet runnable)
├── CLAUDE.md                    # architecture decisions + key references
└── HANDOFF.md                   # this file
```

All five pallets compile cleanly and are wired into runtime at pallet indices 8–12.
Build is clean with `cargo build --release` (pre-existing template warning only).

---

## Pallet status

### pallet-identity (crate: pallet-identity-zk) — runtime index 8

Storage:
- NullifierRegistry: [u8;32] -> AccountId
- CitizenNullifier: AccountId -> [u8;32]
- CitizenIndex: u32 -> AccountId  (dense, for random jury selection)
- CitizenPosition: AccountId -> u32  (reverse index for O(1) swap-and-pop on revoke)
- TotalCitizens: u32
- SuspendedNullifiers: [u8;32] -> Option<BlockNumber>
  - Key absent = not suspended
  - Value None = suspended indefinitely
  - Value Some(block) = suspended until that block (lazy-expiry: no on_initialize needed)

Calls:
- register_citizen(nullifier, zk_proof, public_inputs) — verifies ZK proof via ZkVerifier trait
- revoke_citizen() — swap-and-pop removes from index; also clears suspension entry
- suspend_citizen(nullifier, until) — root (TODO: court-controlled multisig)
- restore_citizen_rights(nullifier) — root (TODO: court-controlled multisig)

Public methods:
- is_active_citizen(who) — registered AND no active suspension (used by pallet-voting)
- is_citizen(who) — registered regardless of suspension status
- citizen_at(index) / total_citizens() — for jury selection

Config:
- ZkVerifier: ZkProofVerifier trait — runtime uses PassthroughZkVerifier (accepts all)

TODOs:
- Replace PassthroughZkVerifier with real Rarimo Groth16 verifier (BN254, specific vk)
- Country allowlist check on public_inputs[2] (country_code_hash)
- Passport expiry check: public_inputs[1] (expiry_timestamp) > now
- Replace ensure_root with court-controlled multisig origin for suspend/restore

### pallet-voting (crate: pallet-voting) — runtime index 9

Two separate participation systems. Suspension excludes citizens from both.

#### System 1 — MACI 1p1v (laws and elections)

Storage:
- Proposals: proposal_id -> end_block
- VoteCommitments: (proposal_id, nullifier) -> commitment (MACI encrypted)
- Delegations: (AccountId, topic_id) -> delegate AccountId  (per-topic)
- DelegatorCount: (topic_id, AccountId) -> u32  (direct delegator count per delegate)
- NextProposalId counter

Config:
- DelegationCap: u8 = 33  (future: enforce as % of total citizens)
- MaxDelegationsPerDelegate: u32 = 1000  (enforced absolute cap today)
- MaxDelegationDepth: u8 = 10  (cycle detection walk limit)
- BudgetCategoryCount: u32 = 10

Calls: submit_proposal, commit_vote, delegate_vote(delegate, topic_id), revoke_delegation(topic_id)

Implemented:
- Per-topic delegation (one delegation per account per topic)
- Cycle detection (walks chain up to MaxDelegationDepth hops)
- DelegatorCount cap enforcement (rejects if new_count > MaxDelegationsPerDelegate)

#### System 2 — Quadratic budget voting (fiscal priorities only)

Budget tokens are non-transferable and expire each fiscal year; casting N votes on a category
costs N² tokens. This forces genuine prioritisation without enabling vote buying.
Legislature controls line items within each category; citizens control category weights.

Storage:
- FiscalYearEpoch: u32 (incremented by start_fiscal_year)
- EpochTokenAllocation: epoch -> u64 (tokens per citizen for that epoch)
- CitizenClaimedEpoch: AccountId -> u32 (last epoch claimed; prevents double-claim)
- BudgetBalance: AccountId -> u64 (remaining tokens this epoch)
- CategoryVotes: (AccountId, epoch, category_id) -> u32 (votes cast; old epochs ignored)

Calls:
- start_fiscal_year(tokens_per_citizen) — root (TODO: legislature origin)
- claim_fiscal_year_tokens() — citizen, once per epoch; tokens expire with epoch
- allocate_budget(category_id, vote_count) — replaces prior allocation, marginal cost = Δvotes²

#### TODOs (both systems)
- Percentage-based delegation cap (needs TotalCitizens via CitizenSelector trait)
- Off-chain MACI tally submission with ZK proof

### pallet-treasury-ledger (crate: pallet-treasury-ledger) — runtime index 10

Storage:
- DepartmentBudgets: department_id -> Balance
- DepartmentSpent: department_id -> Balance
- ExpenditureLog: index -> (department_id, amount, ipfs_metadata_hash [u8;32])
- FrozenDepartments: department_id -> bool  (set by courts on illegal treasury ruling)

Config:
- Balance = u128 (same as chain Balance)

Calls: allocate_budget (root), record_expenditure

Fixed:
- Spend accounting: new_spent = spent.checked_add(amount) stored before inserting log
- Freeze check: record_expenditure returns DepartmentFrozen if department is frozen

Internal:
- freeze_department_internal(department_id) — called by courts auto-enforcement

TODOs:
- Authorized spender per department (any signed account can currently record)

### pallet-courts (crate: pallet-courts) — runtime index 11

Enums: CaseStatus, Verdict, CaseSubject {General, LawChallenge{law_id}, TreasuryDispute{department_id}}

Storage:
- Cases: case_id -> (AccountId, CaseStatus, Option<[u8;32]>, CaseSubject)
- Rulings: case_id -> Verdict
- JuryPool: case_id -> BoundedVec<AccountId, 21>
- NextCaseId counter

Config:
- AppealWindowBlocks = 7 * DAYS
- CitizenSelector: impl by Runtime -> reads pallet-identity CitizenIndex
- LawEnforcer: impl by Runtime -> calls pallet_constitution::invalidate_law_internal
- TreasuryEnforcer: impl by Runtime -> calls pallet_treasury_ledger::freeze_department_internal

Calls: file_case(subject), submit_ai_ruling (root), appeal_ruling, select_jury(case_id, jury_size), finalize_ruling (root)

Implemented:
- select_jury: random selection from citizen index via block hash entropy + CitizenSelector trait
- auto-enforcement in finalize_ruling: Overturned+LawChallenge -> pauses law; Overturned+TreasuryDispute -> freezes dept
- CaseSubject types for targeted enforcement

TODOs:
- AI oracle origin (replace ensure_root in submit_ai_ruling)
- Jury voting mechanism (finalize_ruling is root-only placeholder)
- Appeal window block-time enforcement
- Level 2 (21-person) jury flow for constitutional questions

### pallet-constitution (crate: pallet-constitution) — runtime index 12

Storage:
- Laws: law_id -> (LawTier, LawStatus, version: u32, content_hash [u8;32])
- PendingAmendments: law_id -> (proposed_hash, proposed_at_block)
- NextLawId counter

Config:
- ConstitutionalDeliberationBlocks = 30 * DAYS

Calls: enact_law (root), invalidate_law (root), propose_amendment (any), ratify_amendment (root)

Internal:
- invalidate_law_internal(law_id) — called by courts auto-enforcement (sets LawStatus::Paused)

TODOs:
- Legislature origin (replace ensure_root with collective/referendum origin)
- Petition -> signature threshold -> referendum pipeline (not started)
- Human Rights Commission veto hook

---

## Pallets wired into runtime

runtime/src/lib.rs pallet indices:
- 8: Identity = pallet_identity_zk
- 9: Voting = pallet_voting
- 10: TreasuryLedger = pallet_treasury_ledger
- 11: Courts = pallet_courts
- 12: Constitution = pallet_constitution

Cross-pallet trait wiring in runtime/src/configs/mod.rs:
- PassthroughZkVerifier: accepts all ZK proofs (replace before production)
- Runtime impls CitizenSelector -> reads pallet_identity_zk storage
- Runtime impls LawEnforcer -> calls pallet_constitution::invalidate_law_internal
- Runtime impls TreasuryEnforcer -> calls pallet_treasury_ledger::freeze_department_internal

---

## Mobile scaffold

`mobile/` has TypeScript skeleton only — not runnable yet:
- src/chain/api.ts          — WsProvider + ApiPromise connection helper
- src/chain/identity.ts     — registerCitizen, isCitizen
- src/chain/voting.ts       — submitProposal, commitVote, delegateVote, revokeDelegation
- src/screens/RegisterScreen.tsx — passport NFC flow stub (TODOs for Rarimo SDK)
- src/screens/VoteScreen.tsx     — proposal list stub
- src/App.tsx                    — NavigationContainer with two screens

TODOs before mobile is runnable:
- `npx react-native init DemocracyChain` to generate iOS/Android native scaffolding
- `npm install` in mobile/
- Install Rarimo React Native passport reader SDK
- Install MobileFaceNet TFLite native module for Android liveness
- Wire real pair/keyring management (iOS Secure Enclave / Android Keystore)

---

## Next steps (in priority order)

1. [DONE] Create monorepo structure and stub all 5 pallets
2. [DONE] Fix DepartmentSpent accounting bug in pallet-treasury-ledger
3. [DONE] Wire all 5 pallets into runtime/src/lib.rs
4. [DONE] Delegation cycle detection + cap enforcement in pallet-voting
5. [DONE] Jury selection + cross-pallet auto-enforcement in pallet-courts
6. [DONE] ZkVerifier trait in pallet-identity (PassthroughZkVerifier placeholder)
7. [DONE] React Native mobile scaffold (src/ skeleton only)
8. [DONE] Wire is_active_citizen check in pallet-voting (suspend guard for all 4 calls)
9. [ ] Replace PassthroughZkVerifier with real Rarimo Groth16 verifier
10. [ ] Jury voting mechanism + appeal window enforcement in pallet-courts
11. [ ] Legislature/courts/HRC origins replacing ensure_root in pallet-constitution
12. [ ] Petition -> threshold -> referendum pipeline
13. [ ] Court-controlled multisig origin for suspend_citizen / restore_citizen_rights
14. [ ] `npx react-native init` + Rarimo SDK + native build setup

---

## Key references

- Rarimo Freedom Tool: https://docs.rarimo.com/freedom-tool/
- Rarimo passport-zk-circuits: https://github.com/rarimo/passport-zk-circuits
- MACI: https://maci.pse.dev/
- Polkadot OpenGov treasury: https://wiki.polkadot.com/learn/learn-polkadot-opengov-treasury/
- Kleros Court V2 (court architecture reference): https://kleros.io/
- Semaphore v4: https://docs.semaphore.pse.dev/
- polkadot-sdk-solochain-template: https://github.com/paritytech/polkadot-sdk-solochain-template
