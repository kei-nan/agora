/**
 * Injectable boundary for the OPRF committee-member crypto core.
 *
 * `docs/project/changelog/082.md` entry 82 decides the real crypto (BabyJubJub
 * arithmetic, the TACEO Poseidon2 variant, Elligator2, Chaum-Pedersen proof
 * generation — validated as native Rust in `oprf-committee-dev/`, see entry 78)
 * compiles once to a single WebAssembly module shared by every host shell (phone,
 * laptop, Pi container).
 *
 * This module exists so the rest of the app (`chain/oprfCommittee.ts`) never calls
 * "the wasm module" directly — it calls this interface, so swapping the stub below for
 * a real implementation touches only this file plus the real implementation itself.
 *
 * ## Two-round migration (matches `committee-node`/`oprf-committee-dev` — see
 * `pallets/pallet-identity/src/lib.rs`'s `submit_oprf_round1`/`submit_oprf_round2`,
 * call indices 16/17)
 *
 * `pallet-identity` was rewritten from a single-response design (`submit_oprf_response`,
 * which no longer exists on-chain) into a genuine `t`-of-`n` threshold protocol —
 * `docs/project/research/oprf-alternatives/11-genuine-threshold-evaluation-design.md`'s
 * "Option B". `committee-node/src/wasm_host.rs` and `oprf-committee-dev/src/ffi.rs` were
 * updated for this already; this file previously was not, and only exposed the retired
 * single-shot `evaluateQuery` ABI, which `chain/oprfCommittee.ts` no longer calls.
 *
 * The real wasm core exports two new functions for this, reconciled against
 * `oprf-committee-dev/src/ffi.rs` and `committee-node/src/wasm_host.rs` byte-for-byte:
 *  - `oprf_round1` — this member's partial evaluation `r_i = s_i·Q` plus two FROST-style
 *    nonce-commitment pairs `(d_g, d_q)`/`(e_g, e_q)`. Takes a caller-supplied `seed` —
 *    see [`CommitteeCrypto.round1`]'s doc comment for why the seed is a parameter here
 *    (not generated internally, unlike the old single-shot `evaluateQuery`).
 *  - `oprf_round2_response` — this member's response scalar `z_i`, given the *same* seed
 *    used for round 1 plus three public aggregation values (`rho_i`, `lambda_i`, `e`).
 *
 * `oprf_evaluate_query` (the old single-shot export) is still a real, unchanged wasm
 * export — `oprf-committee-dev/src/ffi.rs` never removed it, only stopped it being what
 * `committee-node`'s orchestration calls — so [`CommitteeCrypto.evaluateQuery`] is kept
 * here too, unused by `chain/oprfCommittee.ts`'s real duty-fulfillment flow but not
 * misleading: it maps to a real, still-tested capability, mirroring
 * `committee-node/src/wasm_host.rs::CryptoCore::evaluate`'s own "kept, not deleted"
 * choice for the identical reason.
 *
 * **What this file still cannot do**: round 2 needs `rho_i` (binding factor), `lambda_i`
 * (Lagrange coefficient), and `e` (the shared Fiat-Shamir challenge) — all *public*
 * values computed from the locked round-1 set via
 * `oprf-committee-dev::threshold::binding_factor`/`lagrange_coefficient`/
 * `combined_challenge`. Those functions are deliberately native-Rust-only (see
 * `ffi.rs`'s own module doc: "a native caller (e.g. `committee-node`, never itself wasm-
 * compiled) can call those functions directly as an ordinary Rust dependency" — no FFI
 * wrapper exists for them because they touch no secret material). This app has no JS/TS
 * port of that aggregation math. [`CommitteeCrypto.round2Response`] therefore takes
 * `rhoI`/`lambdaI`/`e` as required parameters rather than computing them — same
 * "computed by the caller, passed through opaquely" contract
 * `committee-node/src/wasm_host.rs`'s own `round2_response` wrapper documents — and
 * `chain/oprfCommittee.ts::submitRound2` inherits that same requirement and gap. This is
 * a real, separate, honestly-documented architecture gap, not something this migration
 * papers over.
 */

/**
 * The result of evaluating one blinded OPRF query with this member's secret share
 * (single-shot ABI — `oprf_evaluate_query`). Field names/shapes mirror
 * `oprf-committee-dev/src/ffi.rs`'s real 192-byte output layout (`pk.x/y || dlog_e/
 * dlog_s || response_blinded.x/y`) and `committee-node/src/wasm_host.rs::EvaluationResult`.
 */
export interface OprfEvaluationResult {
  /** `sk * G` (64 bytes, X‖Y) — this member's own public key, as recomputed by the
   * crypto core from the secret key it was given. */
  pk: Uint8Array;
  /** `dlog_e || dlog_s` (64 bytes) — the Chaum-Pedersen proof. */
  dlogProof: Uint8Array;
  /** `response_blinded.x || response_blinded.y` (64 bytes) — `sk * blindedQuery`. */
  evaluation: Uint8Array;
}

/**
 * This member's round-1 broadcast toward a genuine `t`-of-`n` threshold evaluation.
 * Field names/shapes mirror `oprf-committee-dev/src/ffi.rs::round1`'s real 320-byte
 * output layout and `pallet_identity::pallet::OprfRound1Commitment`'s on-chain fields
 * (minus `member`, which the chain fills in from the signed origin) — each field is one
 * BabyJubJub point, 64 bytes, `x(32) || y(32)` big-endian, dropping straight into
 * `submit_oprf_round1`'s arguments with no re-encoding.
 */
export interface OprfRound1Commitment {
  /** `s_i * b_q` — this member's partial (share-weighted) evaluation of the blinded query. */
  rI: Uint8Array;
  /** `d * G` — first nonce commitment, base-generator half. */
  dG: Uint8Array;
  /** `d * b_q` — first nonce commitment, query half. */
  dQ: Uint8Array;
  /** `e * G` — second nonce commitment, base-generator half. */
  eG: Uint8Array;
  /** `e * b_q` — second nonce commitment, query half. */
  eQ: Uint8Array;
}

/**
 * This member's round-2 response scalar. Mirrors
 * `oprf-committee-dev/src/ffi.rs::round2_response`'s real 32-byte output and
 * `pallet_identity::pallet::OprfRound2Response::z_i`.
 */
export interface OprfRound2Response {
  /** One big-endian BN254 `Fr` element — `submit_oprf_round2`'s `z_i` argument verbatim. */
  zI: Uint8Array;
}

/**
 * The crypto core's entry points this app needs.
 *
 * Implementations MUST be pure with respect to their inputs (no hidden state, no
 * network I/O) — this app treats the crypto core as a local library call, exactly as
 * the design doc describes ("no login of its own... a local library call inside
 * whichever host app already has access to the securely-stored share").
 */
export interface CommitteeCrypto {
  /**
   * Single-shot evaluation (`oprf_evaluate_query`) — retained even though
   * `chain/oprfCommittee.ts` no longer calls it for real duty fulfillment (the on-chain
   * `submit_oprf_response` extrinsic it used to feed is retired); see this file's
   * module doc for why it's kept rather than deleted. `blindedQuery` is the whole
   * 64-byte point (matching `PendingOprfQueries.blindedQuery`'s on-chain layout
   * directly); `seed` is a fresh CSPRNG-sourced Chaum-Pedersen proof nonce the caller
   * must supply — reusing it across calls leaks the secret key (see `evaluateQuery`'s
   * real implementation, `wasmCommitteeCrypto.ts`).
   */
  evaluateQuery(secretKeyBytes: Uint8Array, blindedQuery: Uint8Array): Promise<OprfEvaluationResult>;

  /**
   * Threshold round 1 (`oprf_round1`). `secretShareBytes`: this member's own share
   * `s_i` from the DKG (big-endian, up to 32 bytes) — not a standalone key, though the
   * byte encoding is identical to `evaluateQuery`'s `secretKeyBytes`. `blindedQuery`:
   * the citizen's blinded query point, same 64-byte layout as `evaluateQuery`. `seed`:
   * caller-owned per-query randomness that **must be retained and replayed** into the
   * matching {@link round2Response} call for this same `(queryId, committeeSlot)` pair
   * — a mismatched seed is silently wrong, not an error (see
   * `committee-node/src/wasm_host.rs`'s module docs, "the seed rule inverts here"). This
   * is why, unlike `evaluateQuery`, the seed is a caller-supplied parameter rather than
   * generated internally: only the caller (`chain/oprfCommittee.ts`) knows it needs to
   * survive until round 2.
   */
  round1(
    secretShareBytes: Uint8Array,
    blindedQuery: Uint8Array,
    seed: Uint8Array,
  ): Promise<OprfRound1Commitment>;

  /**
   * Threshold round 2 (`oprf_round2_response`). `secretShareBytes`: the same share
   * `s_i` used in {@link round1}. `seed`: the exact 32 bytes passed to the matching
   * `round1` call for this query. `rhoI`/`lambdaI`/`e`: public aggregation values
   * (this member's binding factor, its Lagrange coefficient for the locked
   * participant set, and the shared challenge), each a big-endian BN254 `Fr`,
   * computed by the caller and passed through opaquely here — see this file's module
   * doc for why this app cannot compute them itself yet.
   */
  round2Response(
    secretShareBytes: Uint8Array,
    seed: Uint8Array,
    rhoI: Uint8Array,
    lambdaI: Uint8Array,
    e: Uint8Array,
  ): Promise<OprfRound2Response>;
}

/**
 * STUB — throws unconditionally. There is no real cryptographic math in this file, by
 * design: the actual BabyJubJub/Poseidon2/Chaum-Pedersen computation belongs only in
 * the Wasm module, never reimplemented here. This stands in for a real RN-side
 * Wasm-loading implementation ({@link wasmCommitteeCrypto}).
 */
export const notImplementedCommitteeCrypto: CommitteeCrypto = {
  async evaluateQuery(): Promise<OprfEvaluationResult> {
    throw new Error(
      'CommitteeCrypto.evaluateQuery: not implemented — install a real CommitteeCrypto via ' +
        'setCommitteeCrypto (see index.js). This stub exists only to define the call boundary; ' +
        'do not implement the cryptography here.',
    );
  },
  async round1(): Promise<OprfRound1Commitment> {
    throw new Error(
      'CommitteeCrypto.round1: not implemented — install a real CommitteeCrypto via ' +
        'setCommitteeCrypto (see index.js). This stub exists only to define the call boundary; ' +
        'do not implement the cryptography here.',
    );
  },
  async round2Response(): Promise<OprfRound2Response> {
    throw new Error(
      'CommitteeCrypto.round2Response: not implemented — install a real CommitteeCrypto via ' +
        'setCommitteeCrypto (see index.js). This stub exists only to define the call boundary; ' +
        'do not implement the cryptography here.',
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
