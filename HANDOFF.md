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

Dev node:
```bash
./target/release/agora-node --dev --tmp
```

---

## Monorepo structure

```
democracy-chain/
├── node/                        # chain binary (agora-node)
├── runtime/                     # WASM runtime (agora-runtime) — all 5 pallets wired in
│   ├── assets/
│   │   ├── vk_sha256.bin        # EMPTY PLACEHOLDER — must populate before production
│   │   └── vk_sha1.bin          # EMPTY PLACEHOLDER — must populate before production
│   └── src/
│       ├── configs/mod.rs       # all pallet Config impls + cross-pallet trait wiring
│       ├── lib.rs               # runtime construction
│       └── verifier.rs          # RarimoGroth16Verifier (gated behind !dev-mode)
├── pallets/
│   ├── pallet-identity/         # crate: pallet-identity-zk
│   ├── pallet-voting/           # crate: pallet-voting
│   ├── pallet-treasury-ledger/  # crate: pallet-treasury-ledger
│   ├── pallet-courts/           # crate: pallet-courts
│   └── pallet-constitution/     # crate: pallet-constitution
├── scripts/
│   └── convert_vk.py            # converts Rarimo snarkjs JSON VK → ark-serialize binary
├── mobile/                      # React Native scaffold (src/ only, not yet runnable)
├── CLAUDE.md
└── HANDOFF.md
```

Build is clean. WASM binary: 455 KB compressed / 1.5 MB uncompressed.

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

Calls:
- `register_citizen(nullifier, zk_proof [≤4096 bytes], public_inputs [≤16 × [u8;32]])`
  - Verifies ZK proof via `ZkVerifier` trait
  - Rarimo Freedom Tool uses 10 public signals; bound is 16
- `revoke_citizen()` — swap-and-pop, clears suspension
- `suspend_citizen(nullifier, until)` — `SuspensionOrigin` (root placeholder)
- `restore_citizen_rights(nullifier)` — `SuspensionOrigin` (root placeholder)

Public helpers:
- `is_active_citizen(who)` — registered AND no active suspension
- `is_citizen(who)` — registered regardless of suspension
- `citizen_at(index)` / `total_citizens()` — for jury selection

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
- Country allowlist check on `public_inputs[5/6]` (country_code_hash)
- Passport expiry check: `public_inputs[2]` (expirationDate) vs current timestamp
- Replace `EnsureRoot` with court-controlled multisig for `SuspensionOrigin`

---

### pallet-voting (crate: pallet-voting) — runtime index 9

#### System 1 — MACI 1p1v (proposals and elections)

Storage:
- `Proposals`: `proposal_id` → `end_block`
- `VoteCommitments`: `(proposal_id, nullifier)` → `commitment` (MACI-encrypted)
- `Delegations`: `(AccountId, topic_id)` → `delegate AccountId`  (per-topic)
- `DelegatorCount`: `(topic_id, AccountId)` → `u32`

Calls: `submit_proposal`, `commit_vote`, `delegate_vote(delegate, topic_id)`, `revoke_delegation(topic_id)`

Delegation guards:
- Cycle detection: walks chain up to `MaxDelegationDepth` (10) hops; treats depth-exhaustion as cycle
- Absolute cap: max 1 000 direct delegators per delegate per topic
- Percentage cap: delegate's count × 100 must be ≤ `DelegationCap` (33) × `total_citizens`

#### System 2 — Quadratic budget voting

Storage:
- `FiscalYearEpoch` / `EpochTokenAllocation` / `CitizenClaimedEpoch` / `BudgetBalance` / `CategoryVotes`

Calls: `start_fiscal_year(tokens)` (root), `claim_fiscal_year_tokens()`, `allocate_budget(category, count)`

Token cost for N votes on a category = N². Refundable by reducing count.

#### System 3 — Referendum pipeline ← NEW

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

Calls:
- `vote_referendum(referendum_id, in_favor: bool)` — one vote per active citizen per referendum
- `finalize_referendum(referendum_id)` — anyone, after `end_block`; enacts law if passed

Internal:
- `create_referendum_internal(petition_id, topic_hash)` — called by PetitionApprover

TODOs:
- Off-chain MACI tally submission with ZK proof (for proposals/elections, not referenda)
- Referendum: make `PassageThreshold` per-referendum-type (simple majority vs supermajority)

---

### pallet-treasury-ledger (crate: pallet-treasury-ledger) — runtime index 10

Storage:
- `DepartmentBudgets`: `department_id` → `Balance`
- `DepartmentSpent`: `department_id` → `Balance`
- `ExpenditureLog`: `index` → `(department_id, amount, ipfs_metadata_hash [u8;32])`
- `FrozenDepartments`: `department_id` → `bool`

Calls: `allocate_budget` (root), `record_expenditure` (any signed — TODO: restrict)

Internal: `freeze_department_internal(department_id)` — called by courts enforcement

TODOs:
- `DepartmentSpenders: StorageMap<u32, AccountId>` — only the designated spender per dept
  can call `record_expenditure`

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

`CaseSubject` enum: `General`, `LawChallenge { law_id }`, `TreasuryDispute { department_id }`

Calls: `file_case(subject)`, `submit_ai_ruling` (root), `appeal_ruling`, `select_jury(case_id, size)`, `finalize_ruling` (root), `cast_jury_vote(case_id, verdict)`

Auto-enforcement on `Overturned`:
- `LawChallenge` → `invalidate_law_internal(law_id)` → law paused
- `TreasuryDispute` → `freeze_department_internal(department_id)`

Jury reaches strict majority in `cast_jury_vote` → auto-finalizes via shared `auto_finalize()`.

TODOs:
- AI oracle origin (replace `ensure_root` in `submit_ai_ruling`)
- Level 2 (21-person) jury flow for constitutional cases
- VRF-based jury randomness (current: block hash — manipulable by authors)

---

### pallet-constitution (crate: pallet-constitution) — runtime index 12

Law tiers: `Ordinary` (simple majority), `Constitutional` (supermajority + 30-day deliberation)
Law statuses: `Active`, `Paused` (court-invalidated), `Repealed`

Storage:
- `Laws`: `law_id` → `(LawTier, LawStatus, version: u32, content_hash [u8;32])`
- `PendingAmendments`: `law_id` → `(proposed_hash, proposed_at_block)`
- `Petitions`: `petition_id` → `(AccountId, topic_hash [u8;32], sig_count, submitted_at)`
- `PetitionSignatures`: `(petition_id, AccountId)` → `bool`
- `NextLawId`, `NextPetitionId`

Calls:
- `enact_law(tier, content_hash)` — `LegislatureOrigin` (root placeholder)
- `invalidate_law(law_id)` — root; also has `Active` status guard
- `propose_amendment(law_id, hash)` — `LegislatureOrigin`; guards: Active status, no existing pending amendment
- `ratify_amendment(law_id)` — `LegislatureOrigin`; enforces `ConstitutionalDeliberationBlocks`
- `submit_petition(topic_hash)` — any signed
- `sign_petition(petition_id)` — any signed; at threshold calls `PetitionApprover::create_referendum`

Internal:
- `enact_law_internal(tier, content_hash)` — called by pallet-voting on referendum pass
- `invalidate_law_internal(law_id)` — called by pallet-courts on Overturned ruling

TODOs:
- Replace `EnsureRoot` `LegislatureOrigin` with a proper collective/referendum origin
- Human Rights Commission veto hook on `enact_law`

---

## Full citizen → law pipeline (now complete)

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

## Next steps (in priority order)

1. [DONE] Create monorepo + stub all 5 pallets
2. [DONE] Fix treasury accounting bug
3. [DONE] Wire all 5 pallets into runtime
4. [DONE] Delegation cycle detection + cap enforcement
5. [DONE] Jury selection + cross-pallet auto-enforcement
6. [DONE] ZkVerifier trait + PassthroughZkVerifier placeholder
7. [DONE] React Native mobile scaffold (TypeScript skeleton)
8. [DONE] `is_active_citizen` suspension guard in pallet-voting
9. [DONE] 10-finding code review — all bugs fixed
10. [DONE] Real Rarimo Groth16 verifier infrastructure (ark-groth16, needs VK assets)
11. [DONE] Referendum pipeline: petition → referendum → vote → law enacted
12. [DONE] Populate `runtime/assets/vk_sha256.bin` + `vk_sha1.bin` (convert_vk.py)
13. [ ] `npx react-native init` + Rarimo SDK + native build setup
14. [DONE] Per-department authorized spenders in pallet-treasury-ledger
15. [DONE] Legislature collective origin — new pallet-legislature (index 13) with member/motion/vote/close flow; EnsureLegislatureMotion replaces EnsureRoot in pallet-constitution
16. [DONE] AI oracle origin (`OracleOrigin` config type) in pallet-courts
17. [DONE] Human Rights Commission veto hook in pallet-constitution (14-day veto window)
18. [DONE] CitizenConduct case subject + CitizenSuspender trait + suspend_citizen_internal; courts auto-suspend guilty citizens
19. [DONE] Passport expiry + country allowlist checks in `register_citizen`
20. [DONE] Off-chain MACI tally submission with ZK proof (submit_maci_tally call + PassthroughMACIVerifier)

---

## Key references

- Rarimo Freedom Tool: https://docs.rarimo.com/freedom-tool/
- Rarimo passport-zk-circuits: https://github.com/rarimo/passport-zk-circuits
- MACI: https://maci.pse.dev/
- Polkadot OpenGov treasury: https://wiki.polkadot.com/learn/learn-polkadot-opengov-treasury/
- Kleros Court V2 (court architecture reference): https://kleros.io/
- Semaphore v4: https://docs.semaphore.pse.dev/
- polkadot-sdk-solochain-template: https://github.com/paritytech/polkadot-sdk-solochain-template
