/**
 * Hand-written type declaration for the generated `oprfCommitteeCore.js` (see that
 * file's own header and `scripts/build-wasm-core.sh` for what it is and how it's
 * produced). TypeScript prefers this `.d.ts` over inferring types from the generated
 * `.js` file itself when both share a basename — standard pattern for a JS build
 * artifact with hand-authored types.
 *
 * Shape matches `oprf-committee-dev/src/ffi.rs`'s exported C-ABI exactly (see that
 * file's module doc for the authoritative wire format) — this is the same four-export
 * surface `committee-node/src/wasm_host.rs` binds via `wasmtime`, just reached through
 * wasm2js-generated JS instead of a WebAssembly runtime.
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
