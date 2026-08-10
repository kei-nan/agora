# Agora — Project Context for Claude Code

## What We're Building
A blockchain-based distributed democracy platform for real government adoption.
Full separation of powers (legislature, executive, judiciary) enforced by smart contracts.

## Current State
- Ubuntu 24.04 (WSL2), Rust 1.96 stable
- Chain builds and runs in dev mode
- **All 11 pallets are wired into the runtime**, index 8–18 (see `docs/project/README.md` for
  detail — `HANDOFF.md` is now just a thin pointer there, split out 2026-08-01).
  `pallet-emergency-council` is in `runtime/Cargo.toml`, configured in
  `runtime/src/configs/mod.rs`, and present at `pallet_index(15)` in the
  `#[frame_support::runtime]` macro in `runtime/src/lib.rs` — confirmed 2026-08-08 by reading
  the file directly. (This section claimed until 2026-08-04 that it was missing; that claim was
  itself stale by 2026-08-08 — check `runtime/src/lib.rs` directly rather than trusting this note
  if it matters for what you're doing.)
- Desktop app (Tauri 2) functional — reads real chain data, has Claude AI agent panel
- Mobile: `android/` is a real, committed native project (Gradle 8.6, a hand-written
  `NfcPassportModule.kt` NFC native module) with the JS/TS test suite passing (140 tests
  across 11 suites, up from the 77 changelog #80 originally verified); no JDK/Android SDK in
  this WSL2 environment yet, so no Gradle build has actually run here — `ios/` still doesn't
  exist (see `docs/project/apps/mobile.md`, changelog #80)

## Critical Build Command
Always build with:
```bash
WASM_BUILD_RUSTFLAGS="-C link-arg=--allow-undefined" cargo build --release
```
This env var is also in . Without it, the WASM runtime build fails due to a
substrate-wasm-builder 26.0.1 incompatibility with Rust 1.84+.

To run the dev chain:
```bash
./target/release/agora-node --dev --tmp
```

## Architecture Decisions (Locked In)

### What We Integrate (don't build from scratch)
- **ZKPassport** — passport NFC ZK proof circuits, Noir/UltraHonk (open source, actively maintained)
  - Repo: https://github.com/zkpassport/circuits
  - SDK: https://github.com/zkpassport/zkpassport-sdk
  - Replaces the earlier Rarimo integration — dropped 2026-07-30 (see HANDOFF.md log #65 for why:
    Rarimo's own mobile SDK never actually shipped Noir proving, and ZKPassport is the more
    actively maintained, more complete non-Rarimo stack covering the same problem)
  - Saves the equivalent circuit-engineering work vs. building ICAO passport verification from scratch;
    the earlier Rarimo-specific integration has since been reworked against ZKPassport's actual
    circuit shape: `verifier.rs` verifies real bb 5.0.0 UltraHonk proofs (changelog #72),
    `certificateTree.ts` was rebuilt against ZKPassport's depth-16 Poseidon2 tree (changelog #66),
    and `sodParser.ts`/`zkProving.ts`/`proofEncoding.ts` were reworked too (confirmed 2026-08-08 by
    reading the files directly — SOD parsing itself was never Rarimo-specific, only proof encoding
    was, and that part is now ZKPassport-shaped). Still open: no real ZKPassport proof has ever gone
    through the verifier end-to-end (gated on the OPRF committee below), and whether a real outer
    proof actually accepts the `disclosure`/`migrate-disclosure` subproofs is unconfirmed
- **MACI** (Minimal Anti-Collusion Infrastructure) — receipt-free anonymous voting
  - Docs: https://maci.pse.dev/
  - Plug Rarimo nullifier as eligibility gate
- **Polkadot OpenGov treasury pattern** — adapt for on-chain budget tracking

### What We Build (genuinely novel)
- Liquid democracy delegation layer (transitive, revocable, per-topic, with delegation caps)
- On-chain law ledger (versioned, IPFS content + on-chain hash, constitutional vs ordinary tiers)
- Petition → signature threshold → referendum pipeline
- AI-first court system (AI ruling → human jury appeal, rulings on-chain, auto-enforcement)
- Smart-contract separation of powers (legislature/executive/judiciary domains)

## Identity System
- Biometric passport NFC scan on mobile (custom JMRTD/NFCPassportReader native modules; ZKPassport Noir circuits for the ZK proof, see HANDOFF.md log #65)
- On-device face match (Apple Vision iOS / MobileFaceNet Android)
- Liveness detection (blink/turn)
- ZK proof generated on device — nothing leaves the phone
- Nullifier = Poseidon2(national_id || country_code) — stable across passport renewals
- Passport must be valid at registration AND at vote time
- Recovery = re-scan valid passport
- Passport-only for v1 (country allowlist — some countries lack stable national ID in NFC chip)

## Voting System
- Semaphore v4 / MACI for anonymous unlinkable votes
- Liquid democracy: direct vote OR delegate (transitive, revocable, per-topic)
- Delegation caps: no single delegate can hold >X% of votes
- Petitions: citizen signatures → threshold → votable referendum
- Batched voting epochs (Switzerland model) — not continuous voting

## Government Structure (Separation of Powers)
All enforced by smart contract boundaries:
- **Legislature**: passes laws, approves budget, votes on referenda
- **Executive**: executes budget, manages treasury (cannot make laws)
- **Judiciary**: AI-first courts with human appeal, can invalidate laws (auto-enforced on-chain).
  Also the rights-protection backstop: a standalone Human Rights Commission with veto power was
  the original design but was removed in favor of this — enacting a Structural or Foundational
  law automatically opens a court challenge (`AutoChallengeHook`) that an AI judge reviews
  immediately, with the normal human jury appeal path available from there. No separate HRC
  origin or veto call exists in the codebase.
- **Emergency Council**: time-locked powers with hard coded sunset clause
- **Elections Commission**: candidate eligibility, result certification
- **Anti-Corruption module**: asset disclosure, conflict-of-interest registry, ZK whistleblower
- **Audit Office**: financial audit hooks on every treasury transaction

## Court System (AI-First)
- Level 0: AI judge (instant, cites specific laws, reasoning stored on IPFS hash on-chain)
- Level 1: Random jury of 7 citizens (appeal from Level 0)
- Level 2: Larger jury of 21 citizens (constitutional questions)
- AI model updates require on-chain governance vote (supermajority)
- Human overrides feed back as training signal
- Rulings auto-enforce: invalidated law → contract paused, illegal treasury tx → frozen

## Treasury
- Real-time public budget ledger (adapt Polkadot OpenGov pattern)
- Per-department spend caps enforced by contract
- All spending tagged with source metadata
- Stablecoin-based to start (fiat bridge Phase 2)
- Audit hooks on every transaction

## Mobile App
- React Native (iOS + Android)
- Custom native modules (JMRTD/Android, NFCPassportReader/iOS) for NFC passport reading; ZKPassport Noir/UltraHonk circuits for ZK proof generation (see HANDOFF.md log #65 — replaces the earlier Rarimo/circom integration)
- On-device face match (Apple Vision / MobileFaceNet via TFLite)
- @polkadot/api for Substrate chain interaction
- Wallet stored in iOS Secure Enclave / Android Keystore

## Desktop App
Standalone native desktop app (laptop/PC) for citizens to browse and engage with the system.
Runs without a server — connects directly to the chain and optionally to a cloud AI.

### Stack
- **Tauri 2** — Rust backend, React/TS frontend, ships as a small native binary (~10MB)
- **smoldot** light client embedded — syncs to chain p2p, no full node required
- **IPFS** — fetches law/proposal content by on-chain hash (via gateway or local node)

### Authentication
- QR code challenge flow: desktop displays a one-time QR code
- User scans with mobile app → phone generates ZK proof → signs a desktop session token
- The signing key and biometric anchor never leave the phone
- Desktop receives a time-limited bearer token for read + submit actions

### AI Agent Features (optional cloud, degrades gracefully offline)
- Citizens can ask natural language questions about any law, proposal, ruling, or budget item
- Agent reads the IPFS content for the item and answers in context
- Works when internet is available; gracefully disabled when offline
- Agent is **read-only** on-chain — it can draft actions (e.g. suggest a delegation) but the user must confirm and sign on their phone
- AI provider: Claude API (configurable); no AI data stored server-side beyond the session

### What the desktop app covers (read-heavy, no voting)
- Browse active proposals, laws, court rulings, treasury spend
- Ask AI questions about any item ("what does Article 7 of this bill change?")
- View delegation graph and personal voting history
- Monitor treasury ledger in real time
- Notifications for proposals entering voting epoch

### What stays on mobile only
- Passport NFC scan and ZK proof generation
- Casting votes (requires hardware-backed key)
- Signing any on-chain transaction

## Monorepo Structure
```
democracy-chain/
├── node/              ← chain binary (agora-node)
├── runtime/           ← WASM runtime (agora-runtime)
├── pallets/
│   ├── pallet-identity/          ← citizen registry, ZK proof verification, OPRF mailbox (index 8)
│   ├── pallet-voting/            ← MACI, liquid democracy, referenda            (index 9)
│   ├── pallet-treasury-ledger/   ← public budget ledger, audit hook             (index 10)
│   ├── pallet-courts/            ← AI judge (oracle-accepted), jury selection, auto-enforcement (index 11)
│   ├── pallet-constitution/      ← law ledger, petitions, auto-challenge to courts (index 12)
│   ├── pallet-legislature/       ← collective origin for law/budget motions     (index 13)
│   ├── pallet-elections/         ← Elections Commission, candidates             (index 14)
│   ├── pallet-emergency-council/ ← time-locked emergency powers, auto-sunset   (index 15)
│   ├── pallet-audit/             ← treasury audit trail, flag/clear/dispute     (index 16)
│   ├── pallet-anticorruption/    ← asset disclosure, conflict registry, ZK whistleblower (index 17)
│   └── pallet-executive/         ← parliamentary executive/Cabinet               (index 18)
├── circuits/          ← Noir ZK circuits (oprf-identity-anchor: built, proven against a dev
│                         simulator, not a real committee — see docs/project/next-steps.md #8)
├── mobile/            ← React Native app; android/ real + committed (JS tests pass, no
│                         JDK/SDK here to build it yet); ios/ not started
├── committee/         ← separate mobile app for OPRF committee-member duty (changelog #082/#083)
├── committee-node/    ← laptop/Pi OPRF committee-member container component (changelog #083)
├── oprf-committee-dev/← OPRF crypto core, real wasm32 build (changelog #083); dev/test only,
│                         not a real committee
├── court-oracle/      ← AI-ruling oracle service: polls filed cases, calls Claude, publishes
│                         reasoning to IPFS, submits submit_ai_ruling (changelog #086)
├── desktop/           ← Tauri 2 app (functional: chain RPC + Claude AI agent)
└── CLAUDE.md          ← this file
```

## Remaining Work (in priority order)

**This list was significantly stale until 2026-08-04 — several items below were already done and
some real gaps weren't listed at all. `docs/project/next-steps.md` is the actively-maintained,
authoritative version of this list; treat this section as a summary, not the source of truth.**

1. **OPRF committee service** — the actual blocker for identity registration going live. All of
   the surrounding machinery is built and tested (governance model, circuits, on-chain mailbox,
   a real `wasm32` crypto core, a mobile app and a laptop/Pi container for running a node — see
   changelog #082/#083) but no real committee exists: no DKG ceremony, no founding-group key
   material, `OprfCommitteeKeys`/`CommitteeMembers` empty. This is also what gates the first
   genuine end-to-end ZK verification test — `runtime/src/verifier.rs` is complete and verifies
   real bb 5.0.0 UltraHonk proofs, but no real ZKPassport proof has gone through it yet.
2. **An AI-ruling oracle service** — `pallet-courts` has real, tested on-chain machinery to
   *accept* a ruling (`submit_ai_ruling`, a real `OracleOrigin`) and a separate `finalize_ruling`
   call that applies the verdict. `court-oracle/` (changelog #086) is now a real standalone
   service that polls filed cases, builds context from chain storage, calls Claude for a
   ruling, publishes reasoning to IPFS, and submits `submit_ai_ruling`. The desktop app's
   existing Claude integration remains a separate, read-only citizen Q&A feature, not this.
   `court-oracle` now also schedules `finalize_ruling`: it polls cases in `AIRulingIssued`
   status, and once the current block passes `AIRulingBlock[case_id] + AppealWindowBlocks` with
   no appeal filed (status still `AIRulingIssued`, not moved to `InJuryAppeal`), it recovers the
   verdict from the ruling document it originally published to IPFS and submits
   `finalize_ruling`, signed by the same oracle key `submit_ai_ruling` already uses (both calls
   share the same `OracleOrigin` gate). Still PARTIAL, not done: never run against a real
   chain/Claude API/IPFS daemon (unit-tested at the pure-logic level only, 47/47 passing); and
   `Courts::set_oracle_account` has never been called on a real chain. See
   `court-oracle/README.md` and `docs/project/next-steps.md` item 10 for the full accounting.
3. **Mobile app native build** — `android/` and its NFC module already exist and are committed;
   blocked on (a) no JDK/Android SDK in this environment to run `./gradlew assembleDebug`, and
   (b) the OPRF committee service above, which gates on-device ZK proof generation.
4. **VRF jury randomness** — deliberately descoped, not simply unstarted: a commit-then-delayed-
   reveal scheme replaced block-hash selection in `pallet-courts`, closing the worst grinding
   attack; genuine BABE/SASSAFRAS VRF would need a full consensus swap away from Aura.
5. **Stablecoin bridge** — Phase 2; treasury currently uses native AGR token.

Already done, despite earlier versions of this list still saying otherwise: QR auth (chain-side
verification), per-referendum foundational/constitutional threshold, IPFS content fetching on
desktop, batched Swiss-model voting epochs, and the Anti-Corruption module.

See `docs/project/README.md` for full pallet-by-pallet status and the completed-work log.

## Key References
- ZKPassport circuits: https://github.com/zkpassport/circuits
- ZKPassport SDK: https://github.com/zkpassport/zkpassport-sdk
- MACI: https://maci.pse.dev/
- Kleros Court V2 (court architecture reference): https://kleros.io/
- Polkadot OpenGov treasury: https://wiki.polkadot.com/learn/learn-polkadot-opengov-treasury/
- polkadot-sdk-solochain-template: https://github.com/paritytech/polkadot-sdk-solochain-template
- Semaphore v4: https://docs.semaphore.pse.dev/
