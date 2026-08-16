/**
 * Hand-written type declaration for the generated `oprfCommitteeCore.js` (see that
 * file's own header and `scripts/build-wasm-core.sh` for what it is and how it's
 * produced). TypeScript prefers this `.d.ts` over inferring types from the generated
 * `.js` file itself when both share a basename — standard pattern for a JS build
 * artifact with hand-authored types.
 *
 * Shape matches `oprf-committee-dev/src/ffi.rs`'s exported C-ABI exactly (see that
 * file's module doc for the authoritative wire format) — this is the same six-export
 * surface `committee-node/src/wasm_host.rs` binds via `wasmtime`, just reached through
 * wasm2js-generated JS instead of a WebAssembly runtime. Regenerated 2026-08-16 via
 * `scripts/build-wasm-core.sh` against the two-round (`oprf_round1`/
 * `oprf_round2_response`) artifact — the previous generated file predated those two
 * exports and only had `oprf_evaluate_query`.
 */

/** The module's linear memory. `.buffer` is a live getter — always re-read it fresh
 * after any `oprf_alloc` call rather than caching a view across calls, since a memory
 * grow (rare for this module's small fixed buffers, but not impossible) replaces the
 * underlying `ArrayBuffer` object. */
export const memory: { buffer: ArrayBuffer };

/** Allocates `len` bytes in the module's own linear memory, returns a byte offset. */
export function oprf_alloc(len: number): number;

/** Frees a buffer previously returned by `oprf_alloc`. */
export function oprf_dealloc(ptr: number, len: number): void;

/**
 * Reads a fixed 160-byte input at `inputPtr`, writes a fixed 192-byte output to
 * `outputPtr`. Returns `0` on success or a negative `ERR_*` code (see
 * `oprf-committee-dev/src/ffi.rs`) on malformed input — `outputPtr` is left untouched
 * on error.
 */
export function oprf_evaluate_query(
  inputPtr: number,
  inputLen: number,
  outputPtr: number,
  outputLen: number,
): number;

/**
 * Threshold round 1: reads a fixed 128-byte input (`sk(32) || b_q.x(32) || b_q.y(32) ||
 * seed(32)`) at `inputPtr`, writes a fixed 320-byte output (five BabyJubJub points
 * `r_i || d_g || d_q || e_g || e_q`, each `x(32) || y(32)`) to `outputPtr`. Returns `0`
 * on success or a negative `ERR_*` code (see `oprf-committee-dev/src/ffi.rs`) on
 * malformed input — `outputPtr` is left untouched on error.
 */
export function oprf_round1(
  inputPtr: number,
  inputLen: number,
  outputPtr: number,
  outputLen: number,
): number;

/**
 * Threshold round 2: reads a fixed 160-byte input (`sk(32) || seed(32) || rho_i(32) ||
 * lambda_i(32) || e(32)` — `seed` must be byte-identical to the one given to the
 * matching `oprf_round1` call) at `inputPtr`, writes a fixed 32-byte output (the
 * response scalar `z_i`) to `outputPtr`. Same return-value contract as `oprf_round1`.
 */
export function oprf_round2_response(
  inputPtr: number,
  inputLen: number,
  outputPtr: number,
  outputLen: number,
): number;
