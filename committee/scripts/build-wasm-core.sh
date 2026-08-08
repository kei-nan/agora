#!/usr/bin/env bash
# Regenerates src/crypto/generated/oprfCommitteeCore.js from the real, compiled
# oprf-committee-dev wasm32-unknown-unknown artifact.
#
# WHY THIS EXISTS (read CommitteeCrypto.ts's module doc comment first for the full
# picture): React Native 0.74's default JS engine, Hermes, has never implemented the
# `WebAssembly` global (tracked unresolved for years at
# https://github.com/facebook/hermes/issues/429; the first real Hermes Wasm work landed
# in RN 0.84, six minor versions ahead of what this app pins). Switching this app's JS
# engine to JSC instead — the other RN-supported option — was checked and rejected too:
# JSC's own WebAssembly implementation needs JIT to run, and Apple disallows third-party
# JIT on iOS (confirmed via Callstack's own `polygen` project, which exists specifically
# to route around this by ahead-of-time-compiling wasm with `wasm2c` instead of relying
# on JSC's JIT'd Wasm path), and `jsc-android` explicitly disables its WebAssembly
# support outright — so JSC would not have given a working WebAssembly runtime on
# either platform this app targets.
#
# The approach actually used here is in the same spirit as `polygen` (ahead-of-time
# compile the wasm module into something the engine can already run, instead of relying
# on the engine's own Wasm support) but simpler for this module's needs: Binaryen's
# `wasm2js` tool transpiles the compiled `.wasm` binary into plain ES-module JavaScript
# at BUILD time. The output has no `WebAssembly.*` calls in it at all — it's ordinary
# arithmetic/typed-array JS — so it runs unmodified on Hermes (or JSC, or V8, or Node
# under Jest) with zero engine-level Wasm support required. This is a real, long-standing
# technique (`wasm2js` was built into Binaryen originally to polyfill WebAssembly for
# engines that predate it, e.g. old Safari/IE11) — it is not something invented for this
# task.
#
# Usage:
#   cd oprf-committee-dev && WASM_BUILD_RUSTFLAGS="-C link-arg=--allow-undefined" \
#     cargo build --release --target wasm32-unknown-unknown --lib
#   cd ../committee && ./scripts/build-wasm-core.sh
#
# Requires the `binaryen` npm package (devDependency here) for `wasm-opt`/`wasm2js`.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMMITTEE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$COMMITTEE_DIR/.." && pwd)"

WASM_SRC="$REPO_ROOT/oprf-committee-dev/target/wasm32-unknown-unknown/release/oprf_committee_dev.wasm"
OUT_DIR="$COMMITTEE_DIR/src/crypto/generated"
OUT_FILE="$OUT_DIR/oprfCommitteeCore.js"
TMP_OPT_WASM="$(mktemp -t oprf_committee_dev_opt.XXXXXX.wasm)"
trap 'rm -f "$TMP_OPT_WASM"' EXIT

WASM_OPT="$COMMITTEE_DIR/node_modules/.bin/wasm-opt"
WASM2JS="$COMMITTEE_DIR/node_modules/.bin/wasm2js"

if [ ! -f "$WASM_SRC" ]; then
  echo "error: $WASM_SRC not found." >&2
  echo "Build it first: cd $REPO_ROOT/oprf-committee-dev && WASM_BUILD_RUSTFLAGS=\"-C link-arg=--allow-undefined\" cargo build --release --target wasm32-unknown-unknown --lib" >&2
  exit 1
fi
if [ ! -x "$WASM_OPT" ] || [ ! -x "$WASM2JS" ]; then
  echo "error: binaryen's wasm-opt/wasm2js not found under $COMMITTEE_DIR/node_modules/.bin — run npm install first." >&2
  exit 1
fi

echo "Optimizing $WASM_SRC (-Oz) ..."
"$WASM_OPT" -Oz "$WASM_SRC" -o "$TMP_OPT_WASM"

echo "Transpiling to plain JS (wasm2js -O) ..."
mkdir -p "$OUT_DIR"
{
  echo "/**"
  echo " * GENERATED FILE — do not hand-edit. Regenerate with scripts/build-wasm-core.sh."
  echo " *"
  echo " * A build-time transpilation (via Binaryen's wasm2js, NOT a runtime WebAssembly"
  echo " * loader — see build-wasm-core.sh's own header comment for why) of the real,"
  echo " * compiled oprf-committee-dev wasm32-unknown-unknown artifact"
  echo " * (oprf-committee-dev/src/ffi.rs — the actual committee-evaluation crypto core:"
  echo " * BabyJubJub, Poseidon2, Chaum-Pedersen). Exports oprf_alloc/oprf_dealloc/"
  echo " * oprf_evaluate_query/memory, matching the ABI oprf-committee-dev/src/ffi.rs and"
  echo " * committee-node/src/wasm_host.rs document. Verified byte-identical to the real"
  echo " * Rust ffi::evaluate_query on ffi.rs's own sample_input() fixture — see"
  echo " * ../CommitteeCrypto.test.ts."
  echo " */"
  "$WASM2JS" -O "$TMP_OPT_WASM"
} > "$OUT_FILE"

echo "Wrote $OUT_FILE ($(wc -c < "$OUT_FILE") bytes)"
