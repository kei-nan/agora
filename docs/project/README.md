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
├── runtime/                       # WASM runtime (agora-runtime) — all 11 pallets wired in
│   ├── assets/
│   │   ├── vk_zkpassport_outer_count_4.bin  # ZKPassport outer VK — real, 1888 bytes (log #71); real bb 5.0.0 pairing check since #72
│   │   ├── vk_sha256.bin          # STALE — Rarimo Groth16 VK, referenced by nothing since #70
│   │   └── vk_sha1.bin            # STALE — ditto
│   └── src/
│       ├── configs/mod.rs         # all pallet Config impls + cross-pallet trait wiring
│       ├── lib.rs                 # runtime construction (construct_runtime!)
│       └── verifier.rs            # ZkPassportUltraHonkVerifier (!dev-mode) — verifying, see changelog #72
├── pallets/                       # see pallets/ below, one file per pallet
├── scripts/
│   ├── convert_vk.py              # STALE — Rarimo snarkjs JSON VK → ark-serialize binary
│   └── certificate-registry/      # builds our own DSC Merkle tree (see changelog #63) — off-chain only
├── circuits/
│   └── oprf-identity-anchor/      # forked ZKPassport OPRF circuits (Noir) — see changelog #69
├── mobile/                        # React Native + Android native project (android/ generated)
├── desktop/                       # Tauri 2 app — wired to real chain RPC + Claude AI agent
├── CLAUDE.md
└── HANDOFF.md                     # thin pointer into docs/project/
```

Build is clean. Next available pallet index: **19**.

## Where to look

| Topic | File |
|---|---|
| Runtime features, cross-pallet trait wiring, full citizen→law pipeline | [architecture.md](architecture.md) |
| Per-pallet storage/calls/TODOs (11 pallets, index 8–18) | [pallets/](pallets/) — one file per pallet, see below |
| ZK proof verifier status | [zk-verifier.md](zk-verifier.md) |
| Desktop app (Tauri 2) | [apps/desktop.md](apps/desktop.md) |
| Mobile app (React Native) | [apps/mobile.md](apps/mobile.md) |
| Remaining work, prioritized | [next-steps.md](next-steps.md) |
| External docs/repos referenced throughout | [references.md](references.md) |
| Historical "completed work" log (72 entries, chronological, append-only) | [changelog/](changelog/) — chunked by entry range, see below |

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
| 82 | [082.md](changelog/082.md) — OPRF committee node architecture for the founding phase: member-hosted phone/laptop/Pi devices, one Wasm-compiled crypto core, on-chain query/response mailbox instead of a relay server; several rejected shortcuts (Redis, Lua, chain-fanout, single-device committees) recorded with reasoning; institutional-operator hybrid considered and set aside |
| 83 | [083.md](changelog/083.md) — first real implementation of entry 82's design: pallet-identity mailbox primitives, a real `wasm32` build of the OPRF crypto core, a new mobile committee app and a new `committee-node` container component, reconciled against each other (call index, Wasm ABI, `OprfResponses` shape all corrected); also fixed a real `twox128_hex` byte-order bug in the desktop app found as a side effect |

Quick lookup for a specific entry number:
```bash
grep -rn "^N\. " docs/project/changelog/
```
