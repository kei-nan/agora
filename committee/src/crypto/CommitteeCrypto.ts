/**
 * Injectable boundary for the OPRF committee-member crypto core.
 *
 * `docs/project/changelog/082.md` entry 82 decides the real crypto (BabyJubJub
 * arithmetic, the TACEO Poseidon2 variant, Elligator2, Chaum-Pedersen proof
 * generation — validated as native Rust in `oprf-committee-dev/`, see entry 78)
 * compiles once to a single WebAssembly module shared by every host shell (phone,
 * laptop, Pi container). **That Wasm module is now real** — `oprf-committee-dev/`
 * has a `wasm32-unknown-unknown` build target exporting `oprf_evaluate_query`
 * (see `oprf-committee-dev/src/ffi.rs` for the authoritative ABI spec, and
 * `committee-node/src/wasm_host.rs` for a working Rust host reconciled against it).
 *
 * This module exists so the rest of the app (`chain/oprfCommittee.ts`) never calls
 * "the wasm module" directly — it calls this interface, so swapping the stub below for
 * a real implementation touches only this file plus the real implementation itself.
 *
 * **A real implementation now exists** — `./wasmCommitteeCrypto.ts`'s
 * `wasmCommitteeCrypto`, wired in at app startup by `index.js`'s
 * `setCommitteeCrypto(wasmCommitteeCrypto)` call (changelog #084). It does NOT use a
 * runtime `WebAssembly.instantiate(...)` call — RN 0.74's default Hermes engine has no
 * `WebAssembly` global, and the JSC alternative doesn't give a reliably working one
 * either (JIT'd Wasm disallowed on iOS, disabled outright on `jsc-android`) — instead it
 * loads a build-time `wasm2js` transpilation of the real compiled module (see
 * `wasmCommitteeCrypto.ts`'s own doc comment for the full reasoning and
 * `scripts/build-wasm-core.sh` for how that artifact is produced). `notImplementedCommitteeCrypto`
 * below is kept as the *default* (nothing installs a real implementation until app
 * startup explicitly does), and as the shape every implementation — real or
 * test-fixture — must satisfy; see `evaluateQuery`'s doc comment for the reconciled ABI
 * shape both implementations follow.
 *
 * **What "real" means here, precisely** (see changelog #084 for the full trail): the
 * `wasm2js`-generated JS is verified byte-identical to the actual Rust
 * `ffi::evaluate_query` on a real fixture, and passes under Node/Jest in this
 * environment. It has **not** been run inside an actual Hermes VM on a real phone — no
 * Android/iOS device, emulator, or simulator exists in this environment to check that.
 */

/**
 * The result of evaluating one blinded OPRF query with this member's secret share.
 * Field names/shapes mirror `oprf-committee-dev/src/ffi.rs`'s real 192-byte output
 * layout (`pk.x/y || dlog_e/dlog_s || response_blinded.x/y`) and
 * `committee-node/src/wasm_host.rs::EvaluationResult`, which was reconciled against
 * the same real module — kept in lockstep by hand across the two hosts, the same way
 * this app already keeps `NUM_COMMITTEES` in lockstep with `mobile/`'s copy.
 */
export interface OprfEvaluationResult {
  /**
   * `sk * G` (64 bytes, X‖Y) — this member's own public key, as recomputed by the
   * crypto core from the secret key it was given. Not submitted on-chain (the chain
   * already knows the member's public key via governance-registered
   * `OprfCommitteeKeys`) — present for a possible future sanity check that the
   * configured secret key matches what's expected, not currently used for anything.
   */
  pk: Uint8Array;
  /**
   * `dlog_e || dlog_s` (64 bytes) — the Chaum-Pedersen proof. Maps directly onto
   * `pallet-identity`'s `OprfResponseRecord::dlog_proof: BoundedVec<u8, ConstU32<64>>`
   * — confirmed to be exactly 64 bytes on the real pallet, not an arbitrary blob.
   */
  dlogProof: Uint8Array;
  /**
   * `response_blinded.x || response_blinded.y` (64 bytes) — `sk * blindedQuery`, the
   * blinded OPRF evaluation. Maps directly onto `submitOprfResponse`'s `evaluation:
   * [u8; 64]` parameter, with no further splitting/joining needed.
   */
  evaluation: Uint8Array;
}

/**
 * The crypto core's one entry point this app needs: evaluate a blinded query under a
 * committee member's OPRF secret share, producing the evaluation and a proof it was
 * computed honestly.
 *
 * **Reconciled signature** — was `evaluateQuery(secretKeyBytes, blindedQueryX,
 * blindedQueryY)` (two separate 32-byte coordinate params, guessed before the real
 * module existed). The real `oprf_evaluate_query` ABI takes one 64-byte
 * `blindedQuery` (matching `PendingOprfQueries.blindedQuery`'s own on-chain layout
 * directly, no splitting needed by callers) and internally also needs a public
 * domain-separator constant (`DS_DLOG`) and fresh CSPRNG randomness (a Chaum-Pedersen
 * proof nonce) per call — neither of which the original guessed signature accounted
 * for at all. Both are treated as **implementation details of a real
 * `CommitteeCrypto`**, not part of this interface: `DS_DLOG` is a fixed public
 * constant any real implementation hardcodes (see
 * `committee-node/src/wasm_host.rs::DS_DLOG_BE` for the same constant on the Rust
 * side), and sourcing fresh entropy for the seed is the real implementation's job
 * (RN's platform CSPRNG), not something calling code in `oprfCommittee.ts` should
 * have to manage or could safely be trusted to supply correctly.
 *
 * Implementations MUST be pure with respect to `secretKeyBytes`/`blindedQuery` (no
 * hidden state, no network I/O) — this app treats it as a local library call, exactly
 * as the design doc describes ("no login of its own... a local library call inside
 * whichever host app already has access to the securely-stored share").
 */
export interface CommitteeCrypto {
  evaluateQuery(secretKeyBytes: Uint8Array, blindedQuery: Uint8Array): Promise<OprfEvaluationResult>;
}

/**
 * STUB — throws unconditionally. There is no real cryptographic math in this file, by
 * design: the actual BabyJubJub/Poseidon2/Chaum-Pedersen computation belongs only in
 * the Wasm module (which now exists and is real — see this file's top-level doc
 * comment), never reimplemented here. This stands in for a real RN-side Wasm-loading
 * implementation, which is still unbuilt (a JS Wasm runtime + ABI bridge, distinct
 * from the module itself).
 */
export const notImplementedCommitteeCrypto: CommitteeCrypto = {
  async evaluateQuery(): Promise<OprfEvaluationResult> {
    throw new Error(
      'CommitteeCrypto.evaluateQuery: not implemented — the real OPRF crypto core exists ' +
        'as a compiled oprf-committee-dev wasm32 artifact (see oprf-committee-dev/src/ffi.rs), ' +
        'but this app has no Wasm runtime wired up yet to load and call it from React Native. ' +
        'This stub exists only to define the call boundary; do not implement the cryptography here.',
    );
  },
};

let _instance: CommitteeCrypto = notImplementedCommitteeCrypto;

/**
 * Returns the currently-installed `CommitteeCrypto` implementation. Defaults to
 * {@link notImplementedCommitteeCrypto}. Production wiring (once a Wasm runtime is
 * bridged in) should call {@link setCommitteeCrypto} once at app startup with a real
 * implementation; tests call it with a fixture (see `CommitteeCrypto.test.ts` and
 * `chain/oprfCommittee.test.ts`).
 */
export function getCommitteeCrypto(): CommitteeCrypto {
  return _instance;
}

/** Installs `impl` as the module-wide `CommitteeCrypto`. See {@link getCommitteeCrypto}. */
export function setCommitteeCrypto(impl: CommitteeCrypto): void {
  _instance = impl;
}

/** Resets the module-wide instance back to {@link notImplementedCommitteeCrypto}. Test-only. */
export function resetCommitteeCrypto(): void {
  _instance = notImplementedCommitteeCrypto;
}
