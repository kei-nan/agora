# Agora — Handoff Index

Read this first, then follow links below for the topic you need — don't load every file in this
folder for a single question. Also read `/CLAUDE.md` for architecture decisions and references.

## Environment

- Ubuntu 24.04 WSL2, Rust 1.96 (via rustup)
- Project root: `~/democracy-chain` (directory name unchanged; project renamed to **Agora**)
- Runtime crate: `agora-runtime`
- Node binary: `agora-node`

### Critical build command

Always use this — without `WASM_BUILD_RUSTFLAGS` the WASM build fails on Rust 1.84+:

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

## Monorepo structure

```
democracy-chain/
├── node/                          # chain binary (agora-node)
├── runtime/                       # WASM runtime (agora-runtime) — all 12 pallets wired in
│   ├── assets/
│   │   ├── vk_zkpassport_outer_count_4.bin  # ZKPassport outer VK — real, 1888 bytes (log #71); real bb 5.0.0 pairing check since #72
│   │   └── vk_backing_nullifier.bin  # delegate-persona/backing-proof VK (delegate-privacy build, 2026-08-22/23)
│   └── src/
│       ├── configs/mod.rs         # all pallet Config impls + cross-pallet trait wiring
│       ├── lib.rs                 # runtime construction (#[frame_support::runtime] macro, not the legacy construct_runtime!)
│       └── verifier.rs            # ZkPassportUltraHonkVerifier (!dev-mode) — verifying, see changelog #72
├── pallets/                       # see pallets/ below, one file per pallet
├── scripts/
│   └── certificate-registry/      # builds our own DSC Merkle tree (see changelog #63) — off-chain only
├── circuits/
│   └── oprf-identity-anchor/      # forked ZKPassport OPRF circuits (Noir) — see changelog #69
├── mobile/                        # React Native + Android native project (android/ generated)
├── committee/                     # separate mobile app for OPRF committee-member duty (changelog #082/#083)
├── committee-node/                # laptop/Pi OPRF committee-member container component (changelog #083)
├── oprf-committee-dev/            # OPRF crypto core, real wasm32 build (changelog #083); dev/test only, not a real committee
├── court-oracle/                  # AI-ruling oracle service: polls filed cases, calls Claude, publishes reasoning to IPFS, submits submit_ai_ruling (changelog #086)
├── desktop/                       # Tauri 2 app — wired to real chain RPC + Claude AI agent
├── CLAUDE.md
└── HANDOFF.md                     # thin pointer into docs/project/
```

Build is clean. Next available pallet index: **20**.

## Where to look

| Topic | File |
|---|---|
| Runtime features, cross-pallet trait wiring, full citizen→law pipeline | [architecture.md](architecture.md) |
| Per-pallet storage/calls/TODOs (12 pallets, index 8–19) | [pallets/](pallets/) — one file per pallet, see below |
| ZK proof verifier status | [zk-verifier.md](zk-verifier.md) |
| Desktop app (Tauri 2) | [apps/desktop.md](apps/desktop.md) |
| Mobile app (React Native) | [apps/mobile.md](apps/mobile.md) |
| Remaining work, prioritized | [next-steps.md](next-steps.md) |
| External docs/repos referenced throughout | [references.md](references.md) |
| Historical "completed work" log (99 entries as of this writing, chronological, append-only — see `changelog/` for the current highest entry) | [changelog/](changelog/) — chunked by entry range, see below |

### Pallets (`pallets/`)

| Pallet | Index | File |
|---|---|---|
| pallet-identity (crate `pallet-identity-zk`) | 8 | [identity.md](pallets/identity.md) |
| pallet-voting | 9 | [voting.md](pallets/voting.md) |
| pallet-treasury-ledger | 10 | [treasury-ledger.md](pallets/treasury-ledger.md) |
| pallet-courts | 11 | [courts.md](pallets/courts.md) |
| pallet-constitution | 12 | [constitution.md](pallets/constitution.md) |
| pallet-legislature | 13 | [legislature.md](pallets/legislature.md) |
| pallet-elections | 14 | [elections.md](pallets/elections.md) |
| pallet-emergency-council | 15 | [emergency-council.md](pallets/emergency-council.md) |
| pallet-audit | 16 | [audit.md](pallets/audit.md) |
| pallet-anticorruption | 17 | [anticorruption.md](pallets/anticorruption.md) |
| pallet-executive (alias `Cabinet`) | 18 | [executive.md](pallets/executive.md) |
| pallet-accountability-council | 19 | [accountability-council.md](pallets/accountability-council.md) |

### Changelog (`changelog/`)

Numbered log entries in the order they happened; newest entries have the most detail and are most
likely to matter for current work (e.g. the ZKPassport migration decision is entries #65–68).
Entries are cross-referenced elsewhere in this folder as `log #N` — grep `changelog/` for `N\. ` to
find a specific one, or jump straight to its range file:

| Entries | File |
|---|---|
| 1–20 | [001-020.md](changelog/001-020.md) |
| 21–40 | [021-040.md](changelog/021-040.md) |
| 41–54 | [041-054.md](changelog/041-054.md) |
| 55–60 | [055-060.md](changelog/055-060.md) |
| 61–64 | [061-064.md](changelog/061-064.md) |
| 65–68 | [065-068.md](changelog/065-068.md) — ZKPassport migration decision + Sybil-resistance architecture |
| 69–70 | [069-070.md](changelog/069-070.md) — forked ZKPassport OPRF circuits onto the MRZ personal-number field (`circuits/oprf-identity-anchor/`); reworked the passport verifier + mobile proving pipeline onto ZKPassport/UltraHonk (verifier is fail-closed — no Rust verifier handles bb 5.0.0 proofs yet) |
| 71 | [071.md](changelog/071.md) — populated `runtime/assets/vk_zkpassport_outer_count_4.bin` with the real `count_4` VK (still fail-closed pending the pairing backend) |
| 72 | [072.md](changelog/072.md) — implemented the real UltraHonk pairing check against the bb 5.0.0 fork; settled where bb puts the pairing-point object (in the proof, not the public inputs) so the `N+5` layout and `pallet-identity`'s indices are confirmed correct |
| 73 | [073.md](changelog/073.md) — OPRF committee governance decision: who runs the committee that log #67 designed and log #69 built circuits for (vOPRF over the passport personal-number field, threshold-split secret, checked via exclusion-proof at registration) |
| 74 | [074.md](changelog/074.md) — extended `circuits/oprf-identity-anchor` from a single-committee design to entry 73's decided 5-independent-committee topology; assessed (but did not implement) what a real `AnchorProofVerifier` would need |
| 75 | [075.md](changelog/075.md) — cleared entry 74's Poseidon2 blocker and built a real (partial) `AnchorProofVerifier`, including a Rust Poseidon2 implementation compatible with `noir-lang/poseidon` v0.3.0 |
| 76 | [076.md](changelog/076.md) — closed entry 75's flagged gap: `verify_reverification`/`verify_migration` are now real checks, not unconditionally permissive |
| 77 | [077.md](changelog/077.md) — `migrate-disclosure`'s outer-circuit ABI empirically confirmed (same fixed 8-field outer-circuit interface as `disclosure`), not just derived by analogy |
| 78 | [078.md](changelog/078.md) — built a DEV/TEST-ONLY simulator for the real `TaceoLabs/oprf-nr` OPRF protocol and used its output to prove and verify `anchor` and `disclosure` end-to-end for the first time |
| 79 | [079.md](changelog/079.md) — closed the mobile-wiring gap entries 76/77 flagged: `register_citizen`, `reverify_citizen`, and `migrate_oprf_scheme` are now all wired in `mobile/src/chain/identity.ts` against their real extrinsic signatures |
| 80 | [080.md](changelog/080.md) — corrected a stale claim in `CLAUDE.md`/`docs/project/apps/mobile.md`: mobile is not "a TypeScript scaffold missing native projects" — `mobile/android/` is real, committed, and has been since commit `0f15f52` |
| 81 | [081.md](changelog/081.md) — closed the one gap entry 78 left open: `migrate` and `migrate-disclosure` are now proven and verified end-to-end under real bb 5.0.0, using `oprf-committee-dev`'s DEV-ONLY simulator run twice (outgoing + incoming committee generation) |
| 82 | [082.md](changelog/082.md) — OPRF committee node architecture for the founding phase: member-hosted phone/laptop/Pi devices, one Wasm-compiled crypto core, on-chain query/response mailbox instead of a relay server; several rejected shortcuts (Redis, Lua, chain-fanout, single-device committees) recorded with reasoning; institutional-operator hybrid considered and set aside |
| 83 | [083.md](changelog/083.md) — first real implementation of entry 82's design: pallet-identity mailbox primitives, a real `wasm32` build of the OPRF crypto core, a new mobile committee app and a new `committee-node` container component, reconciled against each other (call index, Wasm ABI, `OprfResponses` shape all corrected); also fixed a real `twox128_hex` byte-order bug in the desktop app found as a side effect |
| 84 | [084.md](changelog/084.md) — `committee/`'s `CommitteeCrypto` is no longer a throwing stub: a real implementation now loads and calls the actual `oprf-committee-dev` crypto core from React Native, closing the gap entry 83 left open |
| 85 | [085.md](changelog/085.md) — DKG ceremony orchestration tooling for the OPRF founding phase, the specific open question entry 82 flagged and left unresolved ("DKG ceremony mechanics across heterogeneous member-owned devices") |
| 86 | [086.md](changelog/086.md) — built `court-oracle/`, a new standalone Rust service that generates and submits Level-0 AI court rulings, the off-chain half of `pallet-courts`' AI-first court system |
| 87 | [087.md](changelog/087.md) — closed `next-steps.md` item 12: real on-device face match + liveness detection (Android), a custom CameraX `com.agora.facematch` module + MobileFaceNet TFLite embedding comparison + ML Kit blink/turn challenge, wired into `RegisterScreen.tsx` in place of the old TODO; runtime-unverified (no Android SDK/JDK in this environment) |
| 88 | [088.md](changelog/088.md) — design-only decision record: device-update authorship for the OPRF founding-phase citizen-hosted node software, the open question entry 82 left unresolved; no `committee`/`committee-node` code changed |
| 89 | [089.md](changelog/089.md) — wired a real embedded smoldot light client into the desktop app's JS frontend (not the Rust backend) for the nine chain-read commands, replacing the hardcoded-RPC-only path; proven end-to-end syncing against a real local `agora-node --dev` chain over libp2p `/ws` |
| 90 | [090.md](changelog/090.md) — ran `court-oracle` against a real dev chain and a real local IPFS daemon for the first time, getting as far as a genuine (rejected) Anthropic API call; no Claude API key exists in this environment, so no ruling was produced and `submit_ai_ruling`/`finalize_ruling` remain unexercised against a live chain |
| 91 | [091.md](changelog/091.md) — project review (2026-08-18), 7 parallel agents: no critical findings; one confirmed high-severity security gap, one same-commit cross-doc contradiction, one undocumented mobile-registration gap |
| 92 | [092.md](changelog/092.md) — persona-based multi-agent review (security researcher/citizen/product-manager angles) plus two fixes: a post-emergency cooldown added to both `pallet-emergency-council` and `pallet-executive`'s independent emergency mechanism, and `remove_oracle_member` now purges the removed member's already-cast approvals from in-flight proposals |
| 93 | [093.md](changelog/093.md) — new delegate-persona-creation ZK proof: circuit additions in `circuits/oprf-identity-anchor` plus a standalone Rust verifier; circuit + verifier only, no pallet extrinsic calls it yet |
| 94 | [094.md](changelog/094.md) — new backing-nullifier ZK proof for anonymously backing a delegate (Semaphore-style Merkle-membership + range-checked slot index): circuit additions plus a standalone Rust verifier; circuit + verifier only, no pallet extrinsic calls it yet |
| 95 | [095.md](changelog/095.md) — `pallet-elections` now actually consumes entries 93/94's delegate-persona and backing-nullifier proofs in `register_as_delegate`/`back_delegate`/`remove_backing` instead of trusting the caller's signature or tracking backing in plaintext |
| 96 | [096.md](changelog/096.md) — wired the mobile app to the real delegate-persona and backing-nullifier cryptography (entries 93–95): a second, independently-generated Keystore-backed delegate-persona keypair plus updated call sites matching `pallet-elections`' new on-chain signatures |
| 97 | [097.md](changelog/097.md) — project review (2026-08-29): fixed a real `pallet-elections` benchmark regression (stale scope-check fixture) and refreshed stale test-count claims across `CLAUDE.md`/`docs/project/apps/mobile.md`/`docs/project/next-steps.md` |
| 98 | [098.md](changelog/098.md) — three same-day/next-day commits given their own entry: chain-wide DoS/capture/privacy hardening across nine pallets from an independent five-lens review, an accessible QR-code liveness fallback plus Play Integrity attestation on mobile, and closing the reverse-direction legislature/executive↔accountability-council overlap gate |
| 99 | [099.md](changelog/099.md) — seven same-day fix commits given their own entry: bootstrap-lock membership freeze on legislature/emergency-council, an auditor self-resolve fix, a recovered-citizen backing/seating-floor fix in `pallet-elections`, a stale-oracle-proposal recovery path in `pallet-courts`, saturating QV refund math, and mobile signing-key/NFC/liveness-RNG hardening |

Quick lookup for a specific entry number:
```bash
grep -rn "^N\. " docs/project/changelog/
```
