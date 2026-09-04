# Agora — Project Context for Claude Code

## What We're Building
A blockchain-based distributed democracy platform for real government adoption.
Full separation of powers (legislature, executive, judiciary) enforced by smart contracts.

## Current State
- Ubuntu 24.04 (WSL2), Rust 1.96 stable
- Chain builds and runs in dev mode
- **All 12 pallets are wired into the runtime**, index 8–19 (see `docs/project/README.md` for
  detail — `HANDOFF.md` is now just a thin pointer there, split out 2026-08-01).
  `pallet-emergency-council` is in `runtime/Cargo.toml`, configured in
  `runtime/src/configs/mod.rs`, and present at `pallet_index(15)` in the
  `#[frame_support::runtime]` macro in `runtime/src/lib.rs` — confirmed 2026-08-08 by reading
  the file directly. (This section claimed until 2026-08-04 that it was missing; that claim was
  itself stale by 2026-08-08 — check `runtime/src/lib.rs` directly rather than trusting this note
  if it matters for what you're doing.)
- Desktop app (Tauri 2) functional — reads real chain data, has Claude AI agent panel
- Mobile: `mobile/android/` is a real, committed native project (Gradle 8.6, hand-written
  `NfcPassportModule.kt`/`com.agora.facematch` native modules) with the JS/TS test suite passing
  (356 tests across 26 suites, confirmed by running `npx jest` in `mobile/` 2026-08-29 — up from
  the 77 changelog #80 originally verified, 228→297 per commit `4a628d1`'s own message, 297→300→337→356
  since); no JDK/Android
  SDK in this WSL2 environment yet, so no Gradle build has actually run here — `ios/` still doesn't
  exist (see `docs/project/apps/mobile.md`, changelog #80)

## Critical Build Command
Always build with:
```bash
WASM_BUILD_RUSTFLAGS="-C link-arg=--allow-undefined" cargo build --release
```
This env var is also in . Without it, the WASM runtime build fails due to a
substrate-wasm-builder 26.0.1 incompatibility with Rust 1.84+.

This command now produces a **real build**: `runtime/Cargo.toml`'s `dev-mode` feature (which
gates always-succeed passthrough verifiers — `PassthroughZkVerifier`, `PassthroughAnchorVerifier`,
the MACI passthrough tally verifier, `PassthroughAntiCorruptionZkVerifier`) was removed from
`default` (fixed 2026-08-09; it used to silently ship fake crypto by default even though the
command above never disabled default features). If you need fast local iteration with those
always-accepting passthrough verifiers instead of real cryptographic ones, opt in explicitly:
```bash
WASM_BUILD_RUSTFLAGS="-C link-arg=--allow-undefined" cargo build --release --features dev-mode
```

To run the dev chain (does **not** require the `dev-mode` feature — `--dev` only selects a
runtime genesis preset that seeds balances/aura/grandpa/sudo, nothing identity/ZK-related):
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
- On-device face match (MobileFaceNet Android; Apple Vision iOS still not started, no `ios/`
  project exists) — implemented but **runtime-unverified** (changelog #087): a custom
  `com.agora.facematch` CameraX-based native module (deliberately not `react-native-vision-camera`
  — see #087 for why) compares a live-captured selfie against the passport's DG2 photo via a
  MobileFaceNet TFLite embedding, wired into `RegisterScreen.tsx` in place of the old
  `// TODO: await FaceMatch.verify(...)`. No Android SDK/JDK exists in this environment, so none
  of the new Kotlin has been compiled or run — same standing limitation the NFC module already
  carries. Match threshold is an unvalidated placeholder (no real calibration corpus). See
  `docs/project/next-steps.md` item 12 and `docs/project/changelog/087.md`.
- Liveness detection (blink/turn) — same status as face match above (changelog #087): a 2-shot
  randomized challenge-response (frontal eyes-open baseline, then blink or turn), read via ML Kit
  Face Detection, gates registration before `proving`. Not video-based/anti-spoofing — a prepared
  attacker with video of the real person could plausibly defeat it, a documented residual risk.
- ZK proof generated on device — nothing leaves the phone
- Vote nullifier = ZKPassport's own `scoped_nullifier` circuit output, extracted verbatim from
  the outer proof's public inputs (see `runtime/src/verifier.rs`) — not a value this codebase
  computes itself. A related but distinct value, the OPRF *identity-anchor* used for Sybil
  resistance, is `Poseidon2(DS_IDENTITY_INPUT, personal_number, issuing_country)` (see
  `circuits/oprf-identity-anchor/lib/identity-anchor/src/lib.nr`) — do not conflate the two
- Passport must be valid at registration AND at vote time
- Recovery = re-scan valid passport. Chain-side mechanism now real (2026-08-22):
  `pallet-identity`'s `recover_account` extrinsic takes the same real-verified proof shape as
  `register_citizen`, and when its nullifier matches an existing citizen, rebinds that
  citizen's on-file identity storage from their old (lost) `AccountId` to a new one, fully
  invalidating the old account and rate-limited by a `MinBlocksBetweenRecoveries` cooldown —
  see `docs/project/pallets/identity.md`. Still gated on the same OPRF-committee blocker as
  every other identity call (no real committee exists yet, so no proof can be produced or
  submitted end-to-end), and no mobile-side UI/call wrapper exists yet to drive it (see
  `mobile/src/chain/keystoreWallet.ts`'s doc comment) — the mechanism is real, the flow isn't
  wired up end-to-end.
- Passport-only for v1 (country allowlist — some countries lack stable national ID in NFC chip)
- **Submission-metadata linkability applies to every identity-bearing extrinsic, not just
  votes.** `register_citizen`/`reverify_citizen`/`recover_account`/`migrate_oprf_scheme` are all
  signed calls whose funding source and submission timing are ordinary public chain data — see
  the Voting System section's fuller writeup of this gap for the general reasoning (it applies
  identically here, and to any future delegate-persona/backing submission too).

## Voting System
- MACI for vote-content privacy — a citizen's chosen option is hidden via MACI commitments, but
  *participation* (who voted, when) is not: `commit_vote` is a signed extrinsic, and the signer's
  account is linkable to their citizen identity via pallet-identity's public nullifier map (the
  same structural limitation applies to pallet-anticorruption's whistleblower reports — see that
  pallet's own doc comments for the detail). No Semaphore code exists in this codebase; "anonymous
  unlinkable votes" overstated what MACI alone delivers here — content-hidden, not sender-hidden.
- **Submission-metadata linkability is a distinct, broader gap than the nullifier-map one above,
  and it survives even perfect proof-level unlinkability.** The nullifier-map gap is about a
  signer account that *is* the citizen's own registered account. But even a signer account that
  was never registered as a citizen at all — e.g. a fresh account created specifically to submit
  an anonymized proof — can still be deanonymized by ordinary chain analysis, not cryptanalysis:
  if it was funded by a direct on-chain transfer from the citizen's known account, or if it
  submits in close temporal proximity to other citizen-linked activity, that funding-source or
  timing correlation breaks pseudonymity regardless of what the extrinsic's payload proves. This
  applies to `commit_vote` today and, as of commits `2e07f68`/`e31257a`/`786b792`/`4a628d1`
  (2026-08-22 through 2026-08-23), now also applies for real to the delegate-persona-creation and
  backing-proof schemes — real ZK circuits, pallet wiring in `pallet-elections`
  (`register_as_delegate`/`back_delegate`/`remove_backing`), and mobile integration, all built and
  confirmed non-stub by three independent review agents; see `docs/project/pallets/elections.md`,
  which documents this as done — a mathematically unlinkable ZK derivation does not anonymize
  the *transaction* that reveals it. Checked for a real mitigation before writing this down: no
  relayer, mixnet, or unsigned/ZK-gated submission path exists anywhere in this repo
  (`pallets/pallet-voting`'s and `pallets/pallet-anticorruption`'s own doc comments already
  independently concluded the same thing for `commit_vote`/`submit_whistleblower_report`), and
  neither `pallets/pallet-treasury-ledger` nor anywhere else has a faucet-like mechanism that
  could fund a submission account without a traceable direct transfer. `court-oracle/` and
  `committee-node/` are standalone-service precedent for *a* deployable component, but both
  authenticate as a known council/committee member — neither is a pattern for relaying an
  arbitrary citizen's transaction pseudonymously. Building real submission-layer anonymity (a
  relayer/mixnet, or unsigned extrinsics validated by ZK group-membership instead of a signature)
  is a genuine standalone-infrastructure project, not a local fix to any one pallet.
- Liquid democracy: direct vote OR delegate (transitive, revocable, per-topic)
- Delegation caps: no single delegate can hold >X% of votes — enforced per `topic_id`
  (`pallet-voting`'s `DelegationCap`), and `topic_id_of` derives that id from a referendum's own
  content hash, not a durable category, so the cap is effectively per-bill/per-referendum, not an
  aggregate system-wide ceiling: a delegate can legally sit at the cap on every open referendum
  simultaneously (see `topic_id_of`'s own doc comment in `pallets/pallet-voting/src/lib.rs`)
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
- **Emergency Council**: time-locked powers with hard coded sunset clause (`pallet-emergency-
  council`). A second, fully independent emergency mechanism also exists at the cabinet level:
  `pallet-executive` has its own `vote_declare_emergency`/`ratify_emergency`/`vote_end_emergency`
  flow (2/3 cabinet supermajority to declare, `LegislatureOrigin` only for after-the-fact
  ratification, a hard-coded `MaxEmergencyBlocks` duration cap, its own `ActiveEmergency`
  storage). The two are not related — see `docs/project/pallets/executive.md`'s "Emergency
  powers" section, which documents it explicitly as "a second, separate mechanism from
  `pallet-emergency-council`". Both mechanisms now enforce a post-emergency cooldown (fixed
  2026-08-20): neither previously blocked the same supermajority from redeclaring a fresh
  emergency the block after the last one ended, which could chain into de-facto indefinite
  emergency powers despite `MaxEmergencyBlocks` capping each individual window. Each pallet has
  its own `EmergencyCooldownBlocks` config (7 days in the runtime) and `CooldownUntil` storage
  item. See `docs/project/pallets/emergency-council.md`, `docs/project/pallets/executive.md`, and
  `docs/project/changelog/092.md`.
- **Legislature seating**: fully automatic — no Elections Commission, no candidate certification
  or human-certified result submission. `pallet-elections` seats the top-N delegates by
  liquid-democracy backing directly into `pallet-legislature`; a standalone commissioner-certified
  office-election subsystem existed earlier but was removed (nothing certified its results beyond
  a commissioner's say-so — see `docs/project/pallets/elections.md`). The Prime Minister is then
  chosen by the seated legislature itself via `pallet-executive`'s ranked-choice investiture
  (see `docs/project/pallets/executive.md`), not elected directly by citizens.
- **Anti-Corruption module**: asset disclosure, conflict-of-interest registry, ZK whistleblower.
  Disclosure currency now has real teeth (fixed 2026-08-20, project-review #091 finding 6): a
  `DisclosureChecker<AccountId>` trait, defined in `pallet-elections` and implemented on
  `pallet_anticorruption::Pallet<T>`, is checked per candidate at legislature-seating time
  alongside the existing active-citizen check — a delegate who would otherwise be seated but
  whose asset disclosure has lapsed or was never filed is skipped (next-highest-backed eligible
  delegate fills the seat instead), with `SeatingSkippedNoDisclosure` emitted so the skip is
  visible on-chain. See `docs/project/pallets/anticorruption.md` and
  `docs/project/pallets/elections.md`.
- **Audit Office**: financial audit hooks on every treasury transaction
- **Two placeholder origins still gated by bare `Root`, not a real collective**: in
  `runtime/src/configs/mod.rs`, `pallet_constitution::Config::RevocationOrigin` ("Wire to a
  minority collective (30–40%) for mainnet") and `pallet_elections::Config::ConstitutionalOrigin`
  ("Production should wire this to a dedicated constitutional collective with a 2/3 supermajority
  threshold") are both honestly commented as dev-only `EnsureRoot<AccountId>` stand-ins pending
  that mainnet wiring. This isn't an abstract future concern today: `runtime/src/
  genesis_config_presets.rs:47` seeds a real `SudoConfig { key: Some(root) }`, and `pallet_sudo`
  is wired at `pallet_index(6)` in `runtime/src/lib.rs` — so "Root" currently resolves to whoever
  holds that one genesis key, a literal single private key, not a collective.

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
- On-device face match (MobileFaceNet via TFLite, Android; Apple Vision iOS not started) —
  implemented but runtime-unverified (changelog #087), see Identity System section below
- @polkadot/api for Substrate chain interaction
- Wallet's Sr25519 signing key is encrypted at rest using a non-exportable, hardware-backed key
  (Android Keystore / iOS Secure Enclave); Android Keystore has no native Sr25519 support, so the
  seed is decrypted into app memory transiently to sign — full hardware-backed signing isolation
  is not yet possible on Android without a curve-conversion redesign. iOS has no wallet code at
  all yet (`ios/` doesn't exist, see Current State above), so this is Android-only in practice
  today.

## Desktop App
Standalone native desktop app (laptop/PC) for citizens to browse and engage with the system.
Runs without a server — connects directly to the chain and optionally to a cloud AI.

### Stack
- **Tauri 2** — Rust backend, React/TS frontend, ships as a small native binary (~10MB)
- **Chain connectivity (changelog #089): a real embedded smoldot light client, in the JS
  frontend, not the Rust backend.** `desktop/src/chain/client.ts` drives `smoldot` via
  `@polkadot/api`'s `ScProvider` (a small hand-written adapter bridges smoldot's own async-
  iterator response API to the callback shape `ScProvider` expects — `@substrate/connect`
  itself isn't a dependency). This was a deliberate JS-side choice, not a default: `smoldot`
  and `@polkadot/api` were already JS-only dependencies in `desktop/package.json` (smoldot's
  primary distribution is a JS/WASM package), and a real Rust-embeddable option
  (`smoldot-light` the crate) was evaluated and passed over in favor of following that
  existing signal. `desktop/src/lib/invoke.ts` transparently routes the nine chain-read
  command names (`chain_status`, `fetch_proposals`, `fetch_laws`, `fetch_treasury`,
  `fetch_department_budgets`, `fetch_rulings`, `fetch_legislature_data`,
  `fetch_elections_data`, `fetch_anticorruption_data`) to this light client instead of Tauri
  IPC, so the React pages that call `invoke(...)` needed zero changes. Proven working
  end-to-end in this environment: a real production Vite build, driven headlessly, synced
  smoldot against a real local `agora-node --dev` chain over its actual libp2p `/ws` transport
  and rendered a correct live block number in the app's own `ChainStatusBar` UI — not just "it
  compiles." Two real, load-bearing caveats: (1) the node must be started with an explicit
  `--listen-addr /ip4/0.0.0.0/tcp/30333/ws` — smoldot in a webview can only dial WebSocket
  addresses, not raw TCP, and a plain `agora-node --dev --tmp` (this doc's own "Critical Build
  Command" section) does not open one by default; (2) `desktop/public/chainspecs/
  dev-chainspec-raw.json` is a checked-in `agora-node build-spec --dev --raw` snapshot with an
  empty `bootNodes` that `client.ts` patches at connect time via one plain
  `system_localListenAddresses` RPC call to the local node (peer-discovery metadata, not a
  trust-sensitive state read — every actual state query afterward is independently verified by
  smoldot against finalized GRANDPA consensus) — regenerate that file if the dev genesis ever
  changes (spec_version bump, new pallet, etc.), or smoldot will fail genesis-hash verification
  rather than silently serving stale data.
  The old Rust `reqwest`-based path (`desktop/src-tauri/src/commands/chain.rs`,
  `desktop/src-tauri/src/rpc.rs`) still exists, still works, and is still covered by its own
  tests — it wasn't deleted, because it's still the real implementation behind
  `chain_submit_extrinsic`, `auth_verify_nullifier`, and the QR-auth callback server's internal
  `lookup_registered_account` lookup, none of which moved to the light client in this pass. See
  that file's top-of-module comment and changelog #089 for the full accounting, including the
  still-open trust-boundary gap on `lookup_registered_account` that this change did not close
  (it documents concretely why not, and what closing it would actually require).
- **IPFS** — fetches law/proposal content by on-chain hash (via gateway or local node)

### Authentication
- QR code challenge flow: desktop displays a one-time QR code
- User scans with mobile app → phone generates ZK proof → signs a desktop session token
- The signing key and biometric anchor never leave the phone
- Desktop receives a time-limited bearer token, real server-side session with enforced expiry,
  verified against a real sr25519 signature over the QR challenge — this part is genuinely wired
  end-to-end. The token's authorization model covers both read and submit actions in principle
  (`chain_submit_extrinsic` is a registered, session-gated Tauri command), but **submit is not
  yet wired to anything**: no phone-side flow exists that produces the already-signed extrinsic
  this command expects as input, and no frontend code calls it — read is the only path actually
  in use today.

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
│   ├── pallet-elections/         ← liquid-democracy delegate registry, automatic legislature
│   │                                seating (no Elections Commission)             (index 14)
│   ├── pallet-emergency-council/ ← time-locked emergency powers, auto-sunset   (index 15)
│   ├── pallet-audit/             ← treasury audit trail, flag/clear/dispute     (index 16)
│   ├── pallet-anticorruption/    ← asset disclosure, conflict registry, ZK whistleblower (index 17)
│   ├── pallet-executive/         ← parliamentary executive/Cabinet, PM ranked-choice
│   │                                investiture, minister confirmation            (index 18)
│   └── pallet-accountability-council/ ← independent auditor/investigator appointment oversight,
│                                    barred from legislature/executive overlap    (index 19)
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
   no appeal filed (status still `AIRulingIssued`, not moved to `InJuryAppeal`), it submits
   `finalize_ruling(case_id)` (both calls share the same `OracleOrigin` gate). Verdict binding
   moved earlier, at commit `ad30aa3`: `submit_ai_ruling` is now a 4-arg call (`case_id,
   ruling_hash, model_version, verdict`) that stores `verdict` on-chain in `AIRulingVerdict` at
   submission time, and `finalize_ruling` takes no verdict argument of its own — it just applies
   whatever was already committed, closing the hole where a compromised oracle credential could
   publish reasoning saying one thing and finalize with a different verdict. Still PARTIAL, not
   done: never run against a real chain/Claude API/IPFS daemon (unit-tested at the pure-logic
   level only, 52/52 passing as of 2026-08-23 (`cargo test --release` in `court-oracle/`), up from
   the 47/47 this line previously cited — added IPFS content-hash verification and Claude
   prompt-injection delimiting after a 2026-08-16 review; see `court-oracle/README.md`). **Update, log #090**:
   `Sudo::sudo(Courts::set_oracle_account(...))` was called for real against a dedicated oracle
   account, confirmed via storage query — `court-oracle` was then built and run for real against
   a real chain and a real local IPFS daemon and got as far as a genuine (rejected) call to the
   real Anthropic API; no Claude API key exists anywhere in this environment, so no ruling was
   ever produced and `submit_ai_ruling`/`finalize_ruling` still haven't been exercised against a
   live chain. **Update, 2026-08-20 (project-review #091 finding 3)**: `OracleOrigin` is no
   longer a single settable `OracleAccount` — `set_oracle_account` is gone, replaced by an
   `OracleMembers` council (bounded to 7, matching the pallet's own Level-1 jury size) requiring
   a strict majority (>1/2) before a proposed ruling or finalization takes effect.
   `submit_ai_ruling`/`finalize_ruling` now *propose* (recording the proposer's own approval)
   rather than acting immediately; `approve_ai_ruling` lets other members co-sign, rejecting
   double-approval and non-members. Membership is root-gated via
   `add_oracle_member`/`remove_oracle_member`. Call indices and argument shapes are unchanged, so
   `court-oracle` needed no code changes beyond updated doc comments describing the new
   one-instance-per-council-member deployment model. The same fix also closed an identical
   single-point-of-failure gap on manual admin overrides: `invalidate_law`/`suspend_citizen`/
   `restore_citizen_rights` now go through a parallel `PendingAdminAction`/
   `EnsureOracleCouncilApproved` propose-then-co-sign mechanism in `pallets/pallet-courts/src/
   lib.rs`, mirroring the ruling-approval flow rather than being gated by a single account. See
   `docs/project/pallets/courts.md` and
   `court-oracle/README.md` and `docs/project/next-steps.md` item 10 for the full accounting.
3. **Mobile app native build** — `mobile/android/` and its NFC module already exist and are committed;
   blocked on (a) no JDK/Android SDK in this environment to run `./gradlew assembleDebug`, and
   (b) the OPRF committee service above, which gates on-device ZK proof generation.
4. **VRF jury randomness** — deliberately descoped, not simply unstarted: a commit-then-delayed-
   reveal scheme replaced block-hash selection in `pallet-courts`, closing the worst grinding
   attack; genuine BABE/SASSAFRAS VRF would need a full consensus swap away from Aura.
5. **Stablecoin bridge** — Phase 2; treasury currently uses native AGR token.
6. **On-device face match + liveness detection** — implemented (changelog #087): a custom
   CameraX-based `com.agora.facematch` native module + MobileFaceNet TFLite embedding comparison
   + ML Kit liveness challenge, wired into `RegisterScreen.tsx` in place of the old
   `// TODO: await FaceMatch.verify(...)`. Runtime-unverified — no Android SDK/JDK in this
   environment to compile/run any of it. iOS (Apple Vision) still not started, no `ios/` project
   exists. See `docs/project/next-steps.md` item 12.

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
