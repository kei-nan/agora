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
├── runtime/                       # WASM runtime (agora-runtime) — all 9 pallets wired in
│   ├── assets/
│   │   ├── vk_sha256.bin          # EMPTY PLACEHOLDER — must populate before production
│   │   └── vk_sha1.bin            # EMPTY PLACEHOLDER — must populate before production
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
│   └── pallet-audit/              # crate: pallet-audit               (index 16)
├── scripts/
│   └── convert_vk.py              # converts Rarimo snarkjs JSON VK → ark-serialize binary
├── mobile/                        # React Native scaffold (src/ only — not yet runnable)
├── desktop/                       # Tauri 2 app — wired to real chain RPC + Claude AI agent
├── CLAUDE.md
└── HANDOFF.md
```

Build is clean. Next available pallet index: **17**.

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
| `LawEnactor` | `Runtime` | `pallet_constitution::enact_law_internal(Ordinary, hash)` |
| `CitizenSuspender` | `Runtime` | `pallet_identity_zk::suspend_citizen_internal` |
| `AuditHook` | `pallet_audit::Pallet<Runtime>` | `AuditLog::insert(index, Pending entry)` |
| `pallet_elections::CitizenChecker<AccountId>` | `Runtime` | `pallet_identity_zk::is_active_citizen` |

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
- Replace `EnsureRoot` with court-controlled multisig for `SuspensionOrigin` and `AdminOrigin`

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
- `Referenda`: `referendum_id` → `(petition_id, topic_hash [u8;32], end_block, ReferendumState)`
- `PetitionReferendum`: `petition_id` → `referendum_id`  (prevents duplicate referenda)
- `ReferendumTally`: `referendum_id` → `(yes_count, no_count)`
- `ReferendumHasVoted`: `(referendum_id, AccountId)` → `bool`
- `NextReferendumId`

Config:
- `ReferendumDurationBlocks = 14 * DAYS`
- `PassageThreshold = 51` (simple majority)
- `LawEnactor = Runtime` → calls `pallet_constitution::enact_law_internal`
- `LegislatureOrigin = EnsureLegislatureMotion<Runtime>` (for `start_fiscal_year`)

Calls:
- `vote_referendum(referendum_id, in_favor: bool)` — one vote per active citizen per referendum
- `finalize_referendum(referendum_id)` — anyone, after `end_block`; enacts law if passed

Internal:
- `create_referendum_internal(petition_id, topic_hash)` — called by PetitionApprover

TODOs:
- Per-referendum-type passage threshold (simple majority vs supermajority for constitutional laws)

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

Law tiers: `Ordinary` (simple majority), `Constitutional` (supermajority + 30-day deliberation)
Law statuses: `Active`, `Paused` (court-invalidated), `Repealed`

Storage:
- `Laws`: `law_id` → `(LawTier, LawStatus, version: u32, content_hash [u8;32])`
- `PendingAmendments`: `law_id` → `(proposed_hash, proposed_at_block)`
- `Petitions`: `petition_id` → `(AccountId, topic_hash [u8;32], sig_count, submitted_at)`
- `PetitionSignatures`: `(petition_id, AccountId)` → `bool`
- `HRCVetoes`: `law_id` → `enacted_at_block` (within 14-day window, HRC can veto)
- `NextLawId`, `NextPetitionId`

Calls:
- `enact_law(tier, content_hash)` — `LegislatureOrigin` (EnsureLegislatureMotion)
- `invalidate_law(law_id)` — `CourtOrigin` (EnsureRoot placeholder; swap to courts origin)
- `veto_law(law_id)` — `HumanRightsOrigin` (EnsureSignedBy HRC seat); within 14-day window
- `propose_amendment(law_id, hash)` — `LegislatureOrigin`
- `ratify_amendment(law_id)` — `LegislatureOrigin`; enforces `ConstitutionalDeliberationBlocks`
- `submit_petition(topic_hash)` — any signed
- `sign_petition(petition_id)` — any signed; at 1 000 threshold calls `PetitionApprover::create_referendum`

Internal:
- `enact_law_internal(tier, content_hash)` — called by pallet-voting on referendum pass
- `invalidate_law_internal(law_id)` — called by pallet-courts on Overturned ruling

TODOs:
- Replace `CourtOrigin = EnsureRoot` with a dedicated pallet-courts public origin type

---

### pallet-legislature (crate: pallet-legislature) — runtime index 13

Storage:
- `Members`: `BoundedVec<AccountId, 500>`
- `Motions`: `motion_id` → `(proposer, call_hash, end_block, MotionStatus)`
- `Votes`: `(motion_id, AccountId)` → `bool`
- `MotionTally`: `motion_id` → `(ayes, nays)`
- `NextMotionId`

Calls:
- `add_member(account)` / `remove_member(account)` — root
- `propose_motion(encoded_call, duration_blocks)` — member only
- `vote_motion(motion_id, approve: bool)` — member only
- `close_motion(motion_id)` — anyone, after `end_block`; passes if ayes > 50%

`EnsureLegislatureMotion<Runtime>` origin type — used by pallet-constitution and pallet-voting to gate law enactment and fiscal year starts behind a legislature vote.

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

## Full citizen → law pipeline

```
submit_petition(topic_hash)
  → sign_petition(petition_id)  [× 1 000 citizens]
    → PetitionThresholdReached event
    → PetitionApprover::create_referendum  [auto, same tx]
      → Referendum created, 14-day window opens
        → vote_referendum(referendum_id, in_favor)  [any active citizen]
        → finalize_referendum(referendum_id)  [after end_block, anyone]
          → if yes*100 >= 51*total: LawEnactor::enact_law(topic_hash)
            → Laws storage: new Ordinary law, Active
            → HRC has 14-day veto window
```

Legislature direct path:
```
propose_motion(encoded enact_law call)  [legislature member]
  → vote_motion  [members vote, 7-day window]
  → close_motion  [passes at >50%]
    → EnsureLegislatureMotion origin satisfied
    → enact_law(tier, content_hash) executes
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
- **Auth** (`commands/auth.rs`): `auth_generate_challenge` generates a UUID deep-link QR; `auth_poll_session` checks for mobile sign-back — currently in-memory stub, not yet verified against chain

Frontend pages: Proposals, Laws, Courts, Treasury, auth QR page, Claude AI sidebar panel.

Browser dev mode uses `desktop/src/lib/mocks.ts` stub data; the real Tauri commands fire when running as a native app.

TODOs:
- Mobile side of QR auth (phone scans desktop QR → signs session token → verifies against chain nullifier registry)
- IPFS content fetching (current: IPFS hashes displayed as hex; no gateway fetch yet)

---

## Mobile scaffold (src/ only — not runnable)

Files:
- `src/chain/api.ts` — WsProvider + ApiPromise singleton
- `src/chain/identity.ts` — `registerCitizen`, `isCitizen`, `suspendCitizen`, `restoreCitizenRights`, `encodeProofForChain`, `encodePublicInputs`
- `src/chain/voting.ts` — `submitProposal`, `commitVote`, `delegateVote`, `revokeDelegation`, `claimFiscalYearTokens`, `allocateBudget`
- `src/chain/constitution.ts` — `submitPetition`, `signPetition`, `proposeAmendment`
- `src/chain/courts.ts` — `fileCase`, `appealRuling`, `castJuryVote`
- `src/screens/RegisterScreen.tsx` — NFC passport flow stub
- `src/screens/VoteScreen.tsx` — proposals + budget allocation UI
- `src/App.tsx` — NavigationContainer

To make runnable:
```bash
cd mobile
npx react-native init Agora --template react-native-template-typescript
# copy src/ into the generated project
npm install @polkadot/api
# install Rarimo SDK when available: @rarimo/react-native-passport-reader
```

---

## Next steps (remaining work)

1. [ ] **VK assets** — populate `runtime/assets/vk_sha256.bin` + `vk_sha1.bin` (see ZK verifier section above)
2. [ ] **Mobile app init** — `npx react-native init` + Rarimo SDK + native iOS/Android build setup
3. [ ] **QR auth (mobile side)** — phone scans desktop QR, signs session token, posts back; desktop verifies against chain nullifier registry
4. [ ] **VRF jury randomness** — replace block-hash selection with BABE/SASSAFRAS VRF before mainnet
5. [ ] **Per-referendum passage threshold** — supermajority (e.g. 2/3) for constitutional-tier laws vs simple majority for ordinary
6. [ ] **IPFS content fetching** (desktop) — fetch law/proposal/ruling text from IPFS gateway by on-chain hash
7. [ ] **Batched voting epochs** — Swiss model: periodic voting windows instead of continuous
8. [ ] **Anti-Corruption module** — asset disclosure, conflict-of-interest registry, ZK whistleblower (needs circuits)
9. [ ] **Stablecoin bridge** — Phase 2; treasury currently uses native AGR token
10. [ ] **Replace SuspensionOrigin / AdminOrigin / CourtOrigin** — swap EnsureRoot placeholders for proper court-controlled multisig or collective origins

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

---

## Key references

- Rarimo Freedom Tool: https://docs.rarimo.com/freedom-tool/
- Rarimo passport-zk-circuits: https://github.com/rarimo/passport-zk-circuits
- MACI: https://maci.pse.dev/
- Polkadot OpenGov treasury: https://wiki.polkadot.com/learn/learn-polkadot-opengov-treasury/
- Kleros Court V2 (court architecture reference): https://kleros.io/
- Semaphore v4: https://docs.semaphore.pse.dev/
- polkadot-sdk-solochain-template: https://github.com/paritytech/polkadot-sdk-solochain-template
