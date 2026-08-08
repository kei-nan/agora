# `wasm/` — OPRF crypto-core Wasm module

Where the compiled OPRF crypto-core `.wasm` module goes once it exists (the parallel
in-progress effort referenced in this component's README.md and the task that produced this
directory). **As of this writing, no such artifact exists anywhere in this repository** — this
was confirmed by searching for any `.wasm` build output and for any wasm-bindgen/wasm32 target
wiring in `oprf-committee-dev/` (the only existing OPRF-adjacent crate) before this component
was built; neither was found.

## If this directory is empty

The node starts in **STUB mode** (see `../src/wasm_host.rs`): it logs a loud warning and, for
any query it polls, computes an obviously-fake evaluation (`0xEE` repeated bytes, a
`"STUB-NOT-A-REAL-DLOG-PROOF"` proof marker). By default (`ALLOW_STUB_SUBMISSION=false`) that
fake output is logged but **never submitted on-chain** — see the main README.md's safety-valve
section.

## Once the real module lands

1. Confirm its actual exported ABI against what `wasm_host.rs` currently assumes (documented at
   the top of that file — it's an invented convention, not derived from any real module, since
   none existed yet when this was written).
2. Drop the compiled `.wasm` file here (or point `WASM_MODULE_PATH` at wherever it actually
   lives — this directory is just the documented default).
3. Update `wasm_host.rs`'s ABI assumptions (function names/signatures, memory layout) to match
   reality, rather than the other way around.
