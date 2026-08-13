# `committee-node`

A minimal container image + orchestration loop for running one **OPRF committee member's**
node on a laptop or Raspberry Pi (amd64/arm64), per
[changelog #082](../docs/project/changelog/082.md): committee members run their own node on
their own hardware, polling on-chain "mailbox" storage (`pallet-identity`) for pending work,
rather than relying on a relay server. See that changelog entry for the full architecture
decision and its rejected alternatives.

**Read this whole file before running this anywhere real.** This was built in parallel with
two other efforts — the real `pallet-identity` mailbox storage/extrinsics, and the real
Wasm-compiled OPRF crypto core — that were still in flight at the time. **Both have since
landed, and this component has been reconciled against them** (see "What's now reconciled"
below) — most of what was provisional when this file was first written is now confirmed real.
A couple of items remain genuinely unverifiable without a live chain; those are called out
explicitly rather than left ambiguous.

**A bug in the desktop app was found here and has since been fixed in the desktop app too**:
while porting `desktop/src-tauri/src/rpc.rs`'s `twox128_hex`, a unit test against the well-known
`twox128("System") == "26aa394eea5630e07c48ae0c9558cef7"` vector caught that the desktop
version's `format!("{:016x}", r0.to_le())` doesn't do what it appears to — `u64::to_le()` is a
no-op on little-endian hosts *for the numeric value itself*, so `{:016x}` still prints the
value's standard (effectively big-endian) hex digits, byte-reversed relative to the correct
answer. `src/rpc.rs` in this component uses `hex::encode(r0.to_le_bytes())` instead, which
matches the known vector (see its test). This meant `desktop/src-tauri/src/rpc.rs`'s
`storage_prefix` — and therefore every desktop RPC command that calls it
(`fetch_proposals`/`fetch_laws`/`fetch_treasury`/etc.) — was silently computing wrong storage
keys on any little-endian host, returning empty results instead of erroring. **`desktop/src-tauri/src/rpc.rs` has since been fixed with the same `to_le_bytes()`-based approach and a regression test against the same known vector** — this was not a hypothetical, it was confirmed and corrected.

## Option B: genuine threshold evaluation — implemented end to end

`docs/project/research/oprf-alternatives/11-genuine-threshold-evaluation-design.md` records a
real gap this component's original design shared with the rest of the project: every committee
member held an identical copy of the whole secret (not a real share), and the mailbox accepted
the *first* response and stopped — meaning any single member's server was already fully
sufficient to answer for its whole committee. That's now fixed at every layer, including this
component's own orchestration loop:

- **`oprf-committee-dev`** has a real, tested 2-round threshold protocol (`src/threshold.rs`) —
  genuine Feldman shares, FROST-adapted nonce commitments and binding factors, Lagrange
  combination — with a combined proof confirmed to pass the *actual, unmodified* Noir
  `verify_dlog_equality` from the real `oprf-nr` dependency, not just this crate's own Rust
  port of it. The wasm ABI exports `oprf_round1`/`oprf_round2_response` alongside the original
  `oprf_evaluate_query`, confirmed present in the compiled `wasm32-unknown-unknown` artifact
  and confirmed to reproduce a verifiable combined proof **driven entirely through the FFI
  boundary** (`ffi.rs`'s `round1_then_round2_ffi_calls_produce_a_verifiable_combined_proof`
  test).
- **`pallet-identity`** replaced `submit_oprf_response`/`OprfResponses` (single response,
  first-wins) with `submit_oprf_round1`/`submit_oprf_round2` and
  `OprfRound1Commitments`/`OprfRound2Responses` — a real two-round bulletin board, purely
  structural on-chain (no cryptographic verification happens in the pallet itself; see
  `OprfRound1Commitment`'s doc comment in `pallets/pallet-identity/src/lib.rs` for why that's
  a deliberate scope boundary, not an oversight — the combination is verified where OPRF
  proofs have always been verified, client-side at `register_citizen`).
- **This component's orchestration loop is wired up to all of it.** `src/main.rs` polls
  `PendingOprfQueries` exactly as before, and for each one not yet fully answered: attempts
  `submit_oprf_round1` if it hasn't yet (fresh per-query nonce seed, held in memory — see
  `QueryProgress` in `main.rs`); once its own round-1 entry is visible on-chain, attempts
  `submit_oprf_round2` — fetching the current qualifying set (`rpc.rs`'s
  `get_oprf_round1_commitments`), translating each participant's `AccountId` to a DKG party
  index via their position in the `CommitteeMembers[slot]` roster (`fetch_committee_roster`),
  computing this member's binding factor, Lagrange coefficient, and the shared challenge
  natively (`oprf-committee-dev::threshold`, now a real path dependency of this crate — see
  Cargo.toml's comment on why that math doesn't need the wasm boundary the secret-touching
  computation uses), and submitting. **This node has no reliable way to know when
  `OprfThreshold` has actually been reached** (a pallet constant, not a storage value) — so it
  simply attempts round 2 once its own round-1 entry is visible, and treats the pallet's own
  `OprfRound1NotLocked` rejection as "not ready, retry next poll," the same pattern round 1
  already uses for `OprfRound1SetLocked`. 34/34 tests pass, zero-warning build.
- **New config**: `MEMBER_INDEX` (this node's 1-based `CommitteeMembers[slot]` roster position
  — the DKG party index; getting it wrong fails silently downstream, see its doc comment) and
  `GROUP_PUBKEY_HEX` (this committee's group public key, needed to compute the shared
  challenge — not derivable from chain state alone, since `OprfCommitteeKeys` stores only a
  hash of it; see `Config::group_pubkey`'s doc comment for the full reasoning and why this is,
  for now, a deployment-time constant every member must be given independently at ceremony
  time).

**What's still honestly unverified, not silently claimed working:** nothing here has run
against a real chain or inside a real 2-round exchange with other real nodes — no chain is
running in this environment, and no real committee or DKG ceremony exists yet (see "Still
open" below). The transaction-extension assumptions this file's module docs already flagged as
best-effort are unchanged. Treat this as a specification that compiles and passes its own unit
tests, not as empirically proven against a live multi-party exchange.

## What's real here

- **The JSON-RPC chain client** (`src/rpc.rs`) — a real, working port of
  `desktop/src-tauri/src/rpc.rs`'s approach (raw JSON-RPC via `reqwest`, `twox128`/`blake2_128`
  storage-key hashing, `state_getKeysPaged`/`state_queryStorageAt`/`state_getStorage`), the same
  pattern this project already uses for chain connectivity in Rust — no `@polkadot/api`
  (JS-only), no `subxt` (a heavier client this project hasn't adopted anywhere else).
- **The orchestration loop** (`src/main.rs`): poll `PendingOprfQueries`, decode each pending
  query, drive it through the two-round threshold protocol (`submit_oprf_round1` then
  `submit_oprf_round2` — see the "Option B" section above), on a configurable interval.
- **Extrinsic construction and signing** (`src/extrinsic.rs`): a real, hand-encoded
  `UncheckedExtrinsic`, sr25519-signed with `sp-core` pinned to the exact version
  (`36.1.0`) the runtime itself uses, so the signing scheme is byte-for-byte what the chain
  expects. The *envelope* (nonce, era, spec/tx version, genesis hash, the `TxExtension` tuple
  shape) is transcribed directly from `runtime/src/lib.rs` and is solid; the *call itself*
  (pallet index, call indices 16/17, argument layout for `submit_oprf_round1`/`round2`) is
  checked against the real, current pallet source (see the "Option B" section) but — like the
  rest of this file's transaction-extension assumptions — never confirmed against a live chain.
- **The Wasm loading path** (`src/wasm_host.rs`): real `wasmtime` module loading and invocation,
  used whenever a `.wasm` file is actually present at `WASM_MODULE_PATH`.
- **Key file decryption** (`src/keystore.rs`): a real `age` (age-encryption.org/v1) passphrase
  decryption of the mounted secrets file.
- **The Dockerfile**: a real multi-stage build, reasoned through for multi-arch
  (linux/amd64 + linux/arm64) via `docker buildx` + QEMU emulation — see the Dockerfile's own
  header comment for why emulation was chosen over cross-compilation, and its "NOTE ON
  VERIFICATION" for the honest build-status caveat (below).

## What's stubbed

- **The OPRF crypto core is now real** — `oprf-committee-dev/` gained a `wasm32-unknown-unknown`
  build target (`cargo build --release --target wasm32-unknown-unknown --lib`), and
  `wasm_host.rs` has been reconciled against its real `oprf_evaluate_query` C-ABI (see "What's
  now reconciled" below). Deploying this component still requires actually building that
  artifact and mounting it at `WASM_MODULE_PATH` — nothing does that automatically as part of
  this component's own build. When the file isn't present there, `wasm_host.rs` falls back to a
  fixed, **obviously fake** placeholder (`0xEE`-repeated bytes) and logs loudly that it did so —
  this fallback is intentional (a missing module shouldn't crash the process), not a sign the
  real core doesn't exist. It never fabricates anything that could pass for real cryptographic
  output.
- **Submission of stub output is disabled by default.** `ALLOW_STUB_SUBMISSION` (default
  `false`) gates whether a stub evaluation is actually sent on-chain via
  `submit_oprf_response`, or just logged. Only flip this against a throwaway dev chain with no
  real citizens registered — see `src/config.rs`'s doc comment.

## What's now reconciled (was provisional, confirmed against the real landed work)

| Item | Was assumed | Now confirmed real | Where |
|---|---|---|---|
| `pallet-identity`'s runtime pallet index | 8 | **Still 8** — was already solid, unchanged | `config.rs` `PALLET_INDEX` |
| `submit_oprf_response`'s call index | Guessed **13** | **Real value is 16** (`submit_oprf_query` is 15) — pallet-identity landed with more existing calls ahead of these two than guessed. `config.rs`'s default and doc comment updated. | `config.rs` `CALL_INDEX` |
| `PendingOprfQueries`/`CommitteeMembers`/`OprfResponses` hasher | Guessed `Blake2_128Concat` | **Confirmed correct** — the real pallet uses `Blake2_128Concat` throughout | `pallets/pallet-identity/src/lib.rs` |
| `blinded_query: [u8; 64]` x/y split | Guessed `x = bytes[0..32], y = bytes[32..64]` | **Confirmed correct** — the real pallet's own doc comment states this exact "x-then-y-big-endian" layout | `pallets/pallet-identity/src/lib.rs`'s `OprfQueryRecord`/`OprfResponseRecord` docs |
| `dlog_proof` bound | Unbounded `Vec<u8>` guess | Real pallet bounds it to **exactly 64 bytes** (`BoundedVec<u8, ConstU32<64>>`) — and the real Wasm core's proof (`dlog_e(32) \|\| dlog_s(32)`) is exactly 64 bytes, so this was never actually a risk, just unconfirmed | Both sides independently checked, not just assumed to match |
| The Wasm module's ABI | **Fully invented** (`alloc`/`evaluate_query`, separate x/y params, variable-length output) | **Replaced entirely** with the real `oprf_alloc`/`oprf_evaluate_query`/`oprf_dealloc` C-ABI: fixed 160-byte input (`sk\|\|b_q.x\|\|b_q.y\|\|ds_dlog\|\|seed`), fixed 192-byte output (`pk\|\|dlog_e/dlog_s\|\|response_blinded`). The invented ABI had no `ds_dlog`/`seed` inputs at all — a real gap, not just a naming mismatch. `wasm_host.rs` rewritten; see its module docs for the full real spec, and `wasm_host::tests::loads_the_real_module_and_the_abi_marshaling_round_trips` for a test against the actual compiled artifact (not just the documented shape). | `src/wasm_host.rs` |

## Still open — genuinely unverifiable without a live chain

These weren't reconciled because nothing short of a real submission can confirm them:

| Assumption | Where | What to check |
|---|---|---|
| `CheckMetadataHash`'s `Mode` = `Disabled` | `extrinsic.rs` | Test against a live `--dev` chain |
| `WeightReclaim`'s extra/additional-signed contribute zero bytes | `extrinsic.rs` | First thing to check if a real submission gets rejected as a bad signature |

If this hand-encoded envelope proves fragile once there's a real call to test against,
consider switching to `subxt`'s dynamic API (reads live chain metadata instead of hand-encoding
the extension tuple) rather than continuing to patch this file by hand.

## Building

```bash
cd committee-node
cargo build --release          # native build; validated to compile cleanly on this dev
                                # machine (Rust 1.96 stable, per /CLAUDE.md) with zero warnings
```

This crate does **not** need /CLAUDE.md's `WASM_BUILD_RUSTFLAGS` workaround — that quirk is
specific to building `agora-runtime` to WASM (`substrate-wasm-builder`), which this crate
deliberately doesn't depend on (see `Cargo.toml`'s dependency comment).

### Docker image (multi-arch)

```bash
docker buildx create --use      # once, if you don't already have a buildx builder
docker buildx build --platform linux/amd64,linux/arm64 -t committee-node:dev .
```

**Build status, honestly**: the Rust crate above was actually compiled (`cargo build`, zero
warnings) in the environment this was developed in. The Docker image itself was **not**
actually built — no Docker daemon was reachable in that sandbox (only a Windows-side Docker
Desktop binary path was visible, with no WSL2 integration active). The multi-arch approach
(buildx + QEMU emulation, not cross-compilation) is reasoned through in the Dockerfile's own
comments, and every dependency in play — `wasmtime`, `sp-core`'s `full_crypto` feature
(`schnorrkel`/`curve25519-dalek`/etc.) — is pure-Rust-or-portable-C and already used in real
ARM-hosted Substrate deployments elsewhere in the ecosystem, so there's no known reason it
wouldn't build for `linux/arm64`. Treat "does the image actually build" as **unverified**
until someone runs the `buildx build` command above with a working Docker daemon.

### Running

```bash
docker run --rm \
  -e NODE_RPC_URL=http://host.docker.internal:9944 \
  -e COMMITTEE_SLOT=0 \
  -e KEY_PASSPHRASE_FILE=/run/secrets/passphrase \
  -v $PWD/keys:/keys:ro \
  -v $PWD/wasm:/wasm:ro \
  -v $PWD/local-dev/passphrase.txt:/run/secrets/passphrase:ro \
  committee-node:dev
```

See `docker-compose.example.yml` for a fuller local-dev example (copy to
`docker-compose.yml` and adjust — not invoked automatically). Full list of environment
variables is in `src/config.rs`'s doc comments; the required ones are `COMMITTEE_SLOT` and one
of `KEY_PASSPHRASE`/`KEY_PASSPHRASE_FILE`.

## Key storage

See `keys/README.md` for the full version. Short version: the secrets file is encrypted at
rest with `age` (a real, standard, audited format — not project-invented crypto), which is a
modest speed bump against casual disk inspection. It is **explicitly not** tamper-resistant
storage, and does **not** answer changelog #082's own open question about real hardware-backed
key custody for member-hosted devices (stock Raspberry Pi has no secure boot / hardware root of
trust — see that changelog entry's own reasoning). Solving that is a hardware/procurement
question out of scope for this component, by the same task instruction that produced it.

## Cloud deployment for institutional operators

The founding-phase model is moving from citizen-hosted phones/laptops/Pis (this section's
original target) to **5 independent committees of ~8-15 named institutions each** — see
`docs/project/research/oprf-alternatives/00-index.md`. For that model, **`deploy/` is the current
path**: `deploy/README.md` is a step-by-step runbook, `deploy/harden-host.sh` and
`deploy/docker-compose.prod.yml` implement the Tier 0/1 hardening checklist from
`docs/project/research/oprf-alternatives/09-cloud-security-hardening.md`, and
`docs/project/research/oprf-alternatives/08-cloud-hosting-providers.md` is the provider menu (a
menu, not a single vendor — see that document for why centralizing hosting across operators would
undermine the whole point of the multi-institution design). That research also recommends
**explicitly retiring the balenaCloud path below for the institutional model**: a
project-controlled fleet-update mechanism is exactly the kind of single point of update-authority
the 5-committee independence structure exists to avoid, once operators have their own IT staff
capable of running `docker compose` on their own schedule. The balenaCloud section is kept below,
unmodified, as the still-relevant path if the citizen-hosted device model is ever used instead —
not deleted, since which model wins is not yet decided.

## Adapting this image for balenaCloud (citizen-hosted device model only)

Not integrated for real here (no real device/account exists to integrate against) — this is
the intended path, for whoever picks this up when the founding-phase device fleet is real:

1. **Base image**: swap `rust:slim-bookworm` (builder) / `debian:bookworm-slim` (runtime) for
   balena's own base images (e.g. `balenalib/raspberrypi4-64-debian:bookworm-run` for the
   runtime stage) so the image gets balena's init system and update-agent hooks for free —
   balena's docs cover this substitution for any existing Dockerfile-based project.
2. **balena.yml / docker-compose.yml**: a single-container app like this one can usually ship
   as-is (balena builds the `Dockerfile` directly per device architecture); balena's
   multicontainer format (`docker-compose.yml` at the fleet root) is only needed if this ever
   grows a companion container (e.g. a local log shipper).
3. **Secrets**: replace the local `KEY_PASSPHRASE_FILE`-mounted-volume pattern with balenaCloud
   **device variables** (per-device, settable from the dashboard/API, not baked into the
   image or the fleet-wide config) — same shape, different mounting mechanism.
4. **Updates**: balenaCloud's fleet-management/OTA update mechanism is exactly what changelog
   #082 cites it for — pushing a new image to member-hosted devices without exposing SSH. What
   it does **not** answer (per that changelog entry's own "Still open" section): *who* is
   authorized to author such an update without becoming a new single point of compromise (a
   firmware/image-signing key that touches every committee member's device is itself a
   supply-chain target) — unresolved, and out of scope for this component.
5. **Networking**: `NODE_RPC_URL` would need to point at a real chain endpoint reachable from
   member devices (not `127.0.0.1`/`host.docker.internal`) — a plain operational config change,
   no code change.

## Out of scope (by explicit task instruction)

- **Hardware-backed key custody / TPM integration** for the Raspberry Pi option. Changelog #082
  already names this an open, unsolved problem; this component does not attempt to solve it.
- **Real balenaCloud integration** (no device/account exists). The section above is
  documentation of the intended path only.
- **Hardening this into a production deployment.** This is a minimal, honestly-documented
  skeleton — see the task's own "Constraints" for why: a working, honestly-documented skeleton
  was the goal, not a hardened production system.
