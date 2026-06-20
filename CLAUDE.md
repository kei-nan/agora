# Agora — Project Context for Claude Code

## What We're Building
A blockchain-based distributed democracy platform for real government adoption.
Full separation of powers (legislature, executive, judiciary) enforced by smart contracts.

## Current State
- Substrate solochain template cloned and building at 
- Ubuntu 24.04 (WSL2), Rust 1.96 stable
- Chain runs successfully in dev mode

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
- **Rarimo Freedom Tool** — passport NFC ZK proof + nullifier system (open source, Halborn-audited)
  - Repo: https://github.com/rarimo/passport-zk-circuits
  - Docs: https://docs.rarimo.com/freedom-tool/
  - Saves ~18 months of ZK circuit work
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
- Biometric passport NFC scan on mobile (Rarimo SDK)
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
- **Judiciary**: AI-first courts with human appeal, can invalidate laws (auto-enforced on-chain)
- **Human Rights Commission**: veto on laws violating protected rights (prevents tyranny of majority)
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
- Rarimo SDK for NFC passport reading + ZK proof generation
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

## Monorepo Structure (to be created)
```
agora/
├── node/              ← chain binary (from template)
├── runtime/           ← WASM runtime (from template)
├── pallets/
│   ├── pallet-identity/        ← citizen registry, ZK proof verification
│   ├── pallet-voting/          ← MACI integration, liquid democracy
│   ├── pallet-treasury-ledger/ ← public budget ledger
│   ├── pallet-courts/          ← AI judge, jury selection, ruling ledger
│   └── pallet-constitution/    ← versioned law ledger, amendment process
├── circuits/          ← Noir ZK circuits (separate toolchain)
├── mobile/            ← React Native app (voting, passport auth, ZK proofs)
├── desktop/           ← Tauri 2 app (browse, review, AI questions)
└── CLAUDE.md          ← this file
```

## Next Steps (in order)
1. Create monorepo directory structure and pallet stubs
2. Integrate Rarimo Freedom Tool into pallet-identity
3. Wire MACI into pallet-voting
4. Build liquid democracy delegation on top of MACI
5. Build pallet-treasury-ledger (OpenGov pattern)
6. Build pallet-constitution (law ledger + amendment process)
7. Build pallet-courts (AI judge + jury selection)
8. Scaffold React Native mobile app with Rarimo SDK
9. Scaffold Tauri desktop app with smoldot + Claude AI agent integration

## Key References
- Rarimo Freedom Tool: https://docs.rarimo.com/freedom-tool/
- MACI: https://maci.pse.dev/
- Kleros Court V2 (court architecture reference): https://kleros.io/
- Polkadot OpenGov treasury: https://wiki.polkadot.com/learn/learn-polkadot-opengov-treasury/
- polkadot-sdk-solochain-template: https://github.com/paritytech/polkadot-sdk-solochain-template
- Rarimo passport-zk-circuits: https://github.com/rarimo/passport-zk-circuits
- Semaphore v4: https://docs.semaphore.pse.dev/
