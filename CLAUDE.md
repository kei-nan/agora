# Agora — Project Context for Claude Code

## What We're Building
A blockchain-based distributed democracy platform for real government adoption.
Full separation of powers (legislature, executive, judiciary) enforced by smart contracts.

## Current State
- Ubuntu 24.04 (WSL2), Rust 1.96 stable
- Chain builds and runs in dev mode
- **9 pallets** implemented at runtime indices 8–16 (see HANDOFF.md for detail)
- Desktop app (Tauri 2) functional — reads real chain data, has Claude AI agent panel
- Mobile: TypeScript scaffold exists but is not a runnable React Native project yet

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
    NOTE — the previously-built Rarimo-specific integration (VK assets, `verifier.rs`, `sodParser.ts`,
    `certificateTree.ts`, mobile proving code) all need rework against ZKPassport's actual circuit
    shape, none of that rework has started yet
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
│   ├── pallet-identity/          ← citizen registry, ZK proof verification     (index 8)
│   ├── pallet-voting/            ← MACI, liquid democracy, referenda            (index 9)
│   ├── pallet-treasury-ledger/   ← public budget ledger, audit hook             (index 10)
│   ├── pallet-courts/            ← AI judge, jury selection, auto-enforcement   (index 11)
│   ├── pallet-constitution/      ← law ledger, petitions, HRC veto              (index 12)
│   ├── pallet-legislature/       ← collective origin for law/budget motions     (index 13)
│   ├── pallet-elections/         ← Elections Commission, candidates             (index 14)
│   ├── pallet-emergency-council/ ← time-locked emergency powers, auto-sunset   (index 15)
│   └── pallet-audit/             ← treasury audit trail, flag/clear/dispute     (index 16)
├── circuits/          ← Noir ZK circuits (not yet started)
├── mobile/            ← React Native app (TypeScript scaffold — not runnable yet)
├── desktop/           ← Tauri 2 app (functional: chain RPC + Claude AI agent)
└── CLAUDE.md          ← this file
```

## Remaining Work (in priority order)
1. **VK assets** — populate `runtime/assets/vk_sha256.bin` + `vk_sha1.bin` to enable real ZK proof verification (see `scripts/convert_vk.py`)
2. **Mobile app** — `npx react-native init` + custom NFC native modules + ZKPassport Noir circuits for ZK proof generation
3. **QR auth (mobile side)** — phone scans desktop QR, signs session token, verifies against chain
4. **VRF jury randomness** — replace block-hash selection with BABE/SASSAFRAS VRF
5. **Per-referendum threshold** — supermajority for constitutional-tier laws
6. **IPFS content fetching** (desktop) — fetch law/proposal text from gateway by on-chain hash
7. **Batched voting epochs** — Swiss model periodic windows instead of continuous voting
8. **Anti-Corruption module** — asset disclosure, ZK whistleblower (needs Noir circuits)
9. **Stablecoin bridge** — Phase 2; treasury currently uses native AGR token

See `HANDOFF.md` for full pallet-by-pallet status and completed work log.

## Key References
- ZKPassport circuits: https://github.com/zkpassport/circuits
- ZKPassport SDK: https://github.com/zkpassport/zkpassport-sdk
- MACI: https://maci.pse.dev/
- Kleros Court V2 (court architecture reference): https://kleros.io/
- Polkadot OpenGov treasury: https://wiki.polkadot.com/learn/learn-polkadot-opengov-treasury/
- polkadot-sdk-solochain-template: https://github.com/paritytech/polkadot-sdk-solochain-template
- Semaphore v4: https://docs.semaphore.pse.dev/
