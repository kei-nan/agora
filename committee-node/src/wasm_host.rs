//! Loads the OPRF crypto-core Wasm module and calls `oprf_evaluate_query` — OR, if no compiled
//! module exists yet, falls back to an obviously-fake stub. This module never fabricates output
//! that could be mistaken for real cryptography — the stub path returns a fixed, clearly-
//! labeled sentinel and logs loudly that it did so.
//!
//! ## Wasm ABI — real, reconciled against `oprf-committee-dev/src/ffi.rs` (changelog #082's
//! Wasm-compilation work). This was previously an invented placeholder; it now matches the
//! actual, tested `oprf_evaluate_query` export byte-for-byte.
//!
//! Exports used (see `oprf-committee-dev/src/ffi.rs` module docs for the authoritative spec):
//! - `memory`: the module's linear memory.
//! - `oprf_alloc(len: i32) -> i32`: allocates `len` bytes in guest memory, returns a pointer.
//! - `oprf_dealloc(ptr: i32, len: i32)`: frees a buffer previously returned by `oprf_alloc`.
//! - `oprf_evaluate_query(input_ptr: i32, input_len: i32, output_ptr: i32, output_len: i32) -> i32`:
//!   reads a fixed [`INPUT_LEN`] = 160-byte input (`sk(32) || b_q.x(32) || b_q.y(32) ||
//!   ds_dlog(32) || seed(32)`, every field a big-endian BN254 `Fr`), writes a fixed
//!   [`OUTPUT_LEN`] = 192-byte output (`pk.x/y(64) || dlog_e/dlog_s(64) ||
//!   response_blinded.x/y(64)`) to `output_ptr`, and returns `0` on success or a negative
//!   `ERR_*` code on malformed input (see `ffi.rs`) — `output_ptr` is left untouched on error.
//!
//! Two fields the earlier placeholder ABI didn't account for at all:
//! - `ds_dlog`: a **public**, fixed domain-separator constant (not secret) — see [`DS_DLOG_BE`].
//! - `seed`: **fresh CSPRNG randomness this host must supply on every call.** The wasm module
//!   has no OS entropy source of its own on `wasm32-unknown-unknown` by design (see `ffi.rs`) —
//!   reusing a seed across two calls leaks the committee member's secret key (classic
//!   Chaum-Pedersen/Schnorr nonce-reuse attack). This host sources it from `rand::rngs::OsRng`,
//!   which — unlike the wasm module — does have real OS entropy available.
//!
//! ## Threshold (Option B) round 1 / round 2 — also real, also reconciled against `ffi.rs`
//!
//! The genuine `t`-of-`n` protocol (`oprf-committee-dev/src/threshold.rs`) needs two round
//! trips instead of one, so the module exports two more functions with the same fixed-width
//! C-ABI shape as `oprf_evaluate_query`:
//!
//! - `oprf_round1(input_ptr, input_len, output_ptr, output_len) -> i32`: reads
//!   [`ROUND1_INPUT_LEN`] = 128 bytes (`sk(32) || b_q.x(32) || b_q.y(32) || seed(32)` — no
//!   `ds_dlog`, because round 1 computes no proof yet), writes [`ROUND1_OUTPUT_LEN`] = 320
//!   bytes: five BabyJubJub points `r_i || d_g || d_q || e_g || e_q`, each `x(32) || y(32)`.
//! - `oprf_round2_response(input_ptr, input_len, output_ptr, output_len) -> i32`: reads
//!   [`ROUND2_INPUT_LEN`] = 160 bytes (`sk(32) || seed(32) || rho_i(32) || lambda_i(32) ||
//!   e(32)`), writes [`ROUND2_OUTPUT_LEN`] = 32 bytes: the response scalar `z_i`.
//!
//! Both use the same five `ERR_*` codes as `oprf_evaluate_query` and add no new ones, so
//! [`describe_error_code`] covers all three exports without extension (verified against
//! `ffi.rs`: `round1` can return `ERR_BAD_INPUT_LEN`/`ERR_BAD_OUTPUT_LEN`/`ERR_ZERO_SECRET_KEY`/
//! `ERR_BLINDED_QUERY_NOT_ON_CURVE`/`ERR_BLINDED_QUERY_NOT_IN_SUBGROUP`; `round2_response`, which
//! never sees a curve point, can return the first three).
//!
//! ### The `seed` rule inverts here — and that is a real, sharp edge
//!
//! [`CryptoCore::evaluate`] draws its own fresh `seed` internally on every call, because a
//! single call is the whole protocol. Round 1 and round 2 are **two calls that must share one
//! seed**: `oprf_round2_response` re-derives the exact same internal nonces by re-seeding the
//! same `StdRng` (see `ffi.rs::round2_response`'s comment on matching `threshold::round1`'s
//! draw order), and the module keeps no state between calls to do this any other way. So this
//! host cannot generate the seed itself for round 2 — it has no way to know which earlier
//! round-1 call a given round-2 call belongs to. Both wrappers therefore take `seed` as a
//! parameter, and the **caller** (`main.rs`, per-query state) owns it:
//!
//! - fresh, OS-sourced, unpredictable **per query** (reuse across queries is nonce reuse and
//!   leaks the member's secret share, exactly as for `evaluate`);
//! - **identical** across the matching round-1/round-2 pair for one query. A mismatched seed is
//!   silently wrong, not an error: `ffi.rs` says so explicitly — you get a `z_i` that simply
//!   fails to combine into a valid proof, with nothing at this boundary able to detect it.
//!
//! Consequence worth stating plainly: a stored round-1 seed is live nonce material, as
//! sensitive as the secret share itself until the query completes. Where and how `main.rs`
//! holds it between rounds is that file's problem, not this one's, but it is not a "just a
//! random number, store it anywhere" value.
//!
//! `rho_i`, `lambda_i` and `e` are the opposite: entirely **public** aggregation values the
//! caller computes natively from already-public chain data (`threshold::binding_factor`,
//! `lagrange_coefficient`, `combined_challenge` — deliberately not behind the FFI boundary,
//! see `ffi.rs`'s module docs). These wrappers pass them through as opaque big-endian bytes and
//! do no validation of their own beyond length.

use anyhow::Context;
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

/// `sk || b_q.x || b_q.y || ds_dlog || seed`, each field 32 bytes. Must match
/// `oprf-committee-dev/src/ffi.rs::INPUT_LEN` exactly.
///
/// Not currently called from `main.rs`: the orchestration loop moved entirely to the
/// `round1`/`round2_response` two-round protocol (see module docs), which fully supersedes
/// the single-shot `evaluate()` this constant belongs to. Kept — not deleted — because it maps
/// to a real, still-exported, still-correct wasm ABI function (`oprf_evaluate_query`, unrelated
/// to the mailbox redesign) with its own solid test coverage; unlike the extrinsic-encoding
/// dead code removed elsewhere in this rewrite, nothing here is misleading or wrong, only
/// currently unused by this one binary's orchestration choice.
#[allow(dead_code)]
const INPUT_LEN: i32 = 160;
/// `pk.x/y || dlog_e/dlog_s || response_blinded.x/y`, each field 32 bytes. Must match
/// `oprf-committee-dev/src/ffi.rs::OUTPUT_LEN` exactly. See [`INPUT_LEN`]'s doc comment.
#[allow(dead_code)]
const OUTPUT_LEN: i32 = 192;

/// `sk || b_q.x || b_q.y || seed`, each field 32 bytes (`FIELD_LEN * 4`). Must match
/// `oprf-committee-dev/src/ffi.rs::ROUND1_INPUT_LEN` exactly. Copied from that file rather than
/// re-derived by hand; `threshold_input_length_constants_match_the_real_module_exactly` (and its
/// output-length twin) pins all four threshold constants below against the real artifact in both
/// directions, so a constant change over there can't silently desync this file.
// Not yet called from `main.rs` — the query-loop wiring for the threshold rounds is a
// separate, parallel piece of work (this file deliberately does not touch `main.rs`).
// Exercised in full by this module's tests against the real artifact; the attribute
// suppresses the resulting bin-target dead-code warning, not a real "unused" finding.
#[allow(dead_code)]
const ROUND1_INPUT_LEN: i32 = 128;
/// `r_i.x/y || d_g.x/y || d_q.x/y || e_g.x/y || e_q.x/y` — five points, `FIELD_LEN * 10`. Must
/// match `oprf-committee-dev/src/ffi.rs::ROUND1_OUTPUT_LEN` exactly.
#[allow(dead_code)]
const ROUND1_OUTPUT_LEN: i32 = 320;
/// `sk || seed || rho_i || lambda_i || e`, each field 32 bytes (`FIELD_LEN * 5`). Must match
/// `oprf-committee-dev/src/ffi.rs::ROUND2_INPUT_LEN` exactly. Note this is coincidentally the
/// same 160 bytes as [`INPUT_LEN`] but a completely different field layout — do not reuse one
/// constant for the other.
#[allow(dead_code)]
const ROUND2_INPUT_LEN: i32 = 160;
/// `z_i` — a single big-endian BN254 `Fr`. Must match
/// `oprf-committee-dev/src/ffi.rs::ROUND2_OUTPUT_LEN` exactly.
#[allow(dead_code)]
const ROUND2_OUTPUT_LEN: i32 = 32;

/// `DS_DLOG` — ASCII "DLOG Equality Proof" read as a big-endian integer, kept byte-identical
/// to ZKPassport's own `DS_DLOG` (see `circuits/oprf-identity-anchor/lib/identity-anchor/
/// src/lib.nr`: `pub global DS_DLOG: Field = 1523098184080632582082867317389990410064981862;`
/// and `circuits/oprf-identity-anchor/README.md`'s domain-separator table). This is the same
/// constant every proving circuit in this repo already uses — a committee node computing a
/// Chaum-Pedersen proof under a different value would produce a proof the citizen's own
/// `disclosure`/`migrate-disclosure` circuit rejects. **Public, not secret** — safe to hardcode.
pub(crate) const DS_DLOG_BE: [u8; 32] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x44, 0x4c,
    0x4f, 0x47, 0x20, 0x45, 0x71, 0x75, 0x61, 0x6c, 0x69, 0x74, 0x79, 0x20, 0x50, 0x72, 0x6f,
    0x6f, 0x66,
];

/// Mirrors `oprf-committee-dev/src/ffi.rs`'s `ERR_*` constants, for logging a human-readable
/// reason rather than a bare negative number when one of the module's exports rejects an input.
///
/// Shared by all three exports this file calls (`oprf_evaluate_query`, `oprf_round1`,
/// `oprf_round2_response`) — confirmed by reading `ffi.rs`: the threshold round functions
/// introduce **no new error codes**, they return the same five. The two length messages
/// therefore can't name a single expected length any more, since the right answer depends on
/// which export was called; the call site names the export in the surrounding message.
fn describe_error_code(code: i32) -> &'static str {
    match code {
        -1 => "ERR_BAD_INPUT_LEN — input was not the exact length the called export expects \
               (oprf_evaluate_query: 160, oprf_round1: 128, oprf_round2_response: 160)",
        -2 => "ERR_BAD_OUTPUT_LEN — output buffer was not the exact length the called export \
               expects (oprf_evaluate_query: 192, oprf_round1: 320, oprf_round2_response: 32)",
        -3 => "ERR_ZERO_SECRET_KEY — this node's configured OPRF secret key is all-zero",
        -4 => "ERR_BLINDED_QUERY_NOT_ON_CURVE — the citizen's blinded query point is malformed",
        -5 => "ERR_BLINDED_QUERY_NOT_IN_SUBGROUP — blinded query is on-curve but outside the prime-order subgroup",
        _ => "unrecognized error code — oprf-committee-dev/src/ffi.rs's ERR_* constants may have changed",
    }
}

/// See [`INPUT_LEN`]'s doc comment: unused by `main.rs`'s current orchestration (fully moved to
/// the two-round protocol), but a real, correct, well-tested result type for the
/// still-exported single-shot `oprf_evaluate_query` — not deleted.
#[allow(dead_code)]
#[derive(Debug)]
pub struct EvaluationResult {
    /// `sk * G` — this committee member's own public key, as recomputed by the wasm module
    /// from the secret key it was given. **Is submitted on-chain**: `submit_oprf_response`
    /// takes it as the `committee_pubkey` argument and checks it hashes to the
    /// governance-approved `OprfCommitteeKeys` entry for `(OprfSchemeVersion, committee_slot)`
    /// (pallets/pallet-identity/src/lib.rs) — the chain does not look this up itself.
    pub pk: [u8; 64],
    /// `dlog_e || dlog_s` (64 bytes) — the Chaum-Pedersen proof. Maps directly onto
    /// `pallet-identity`'s `OprfResponseRecord::dlog_proof: BoundedVec<u8, ConstU32<64>>`.
    pub dlog_proof: [u8; 64],
    /// `response_blinded.x || response_blinded.y` (64 bytes) — `sk * b_q`, the blinded OPRF
    /// evaluation. Maps directly onto `submit_oprf_response`'s `evaluation: [u8; 64]` param.
    pub evaluation: [u8; 64],
    /// False only for the stub path — set to true whenever a real Wasm module produced this
    /// result, so callers (main.rs) can gate on `allow_stub_submission` in exactly one place.
    pub is_real: bool,
}

/// Round 1 of the threshold protocol: this member's partial evaluation plus both FROST-style
/// nonce-commitment pairs. Every field is one BabyJubJub point as `x(32) || y(32)`, the same
/// 64-byte big-endian convention [`EvaluationResult`]'s `pk`/`evaluation` already use (and the
/// same one `pallet-identity` stores on-chain), so these drop straight into a round-1 commitment
/// extrinsic without re-encoding.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Round1Commitment {
    /// `s_i * b_q` — this member's partial (share-weighted) evaluation of the blinded query.
    pub r_i: [u8; 64],
    /// `d * G` — first nonce commitment, base-generator half.
    pub d_g: [u8; 64],
    /// `d * b_q` — first nonce commitment, query half.
    pub d_q: [u8; 64],
    /// `e * G` — second nonce commitment, base-generator half.
    pub e_g: [u8; 64],
    /// `e * b_q` — second nonce commitment, query half.
    pub e_q: [u8; 64],
    /// False only for the stub path — same flag, same purpose, as [`EvaluationResult::is_real`]:
    /// callers gate on `allow_stub_submission` in exactly one place.
    pub is_real: bool,
}

/// Round 2 of the threshold protocol: this member's response scalar, to be combined with the
/// other participants' responses (`threshold::combine_responses`, native, caller-side) into the
/// single group proof.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Round2Response {
    /// `z_i` — one big-endian BN254 `Fr`.
    pub z_i: [u8; 32],
    /// False only for the stub path — see [`Round1Commitment::is_real`].
    pub is_real: bool,
}

pub enum CryptoCore {
    Real(RealWasmCore),
    Stub,
}

pub struct RealWasmCore {
    store: Store<()>,
    memory: Memory,
    oprf_alloc: TypedFunc<i32, i32>,
    oprf_dealloc: TypedFunc<(i32, i32), ()>,
    #[allow(dead_code)] // see INPUT_LEN's doc comment — real export, unused by this binary now
    oprf_evaluate_query: TypedFunc<(i32, i32, i32, i32), i32>,
    oprf_round1: TypedFunc<(i32, i32, i32, i32), i32>,
    oprf_round2_response: TypedFunc<(i32, i32, i32, i32), i32>,
    #[allow(dead_code)]
    instance: Instance,
}

/// Attempts to load `wasm_path`. Returns `CryptoCore::Stub` (not an error) if the file simply
/// doesn't exist yet — that is the expected, common case right now, not a misconfiguration.
/// Returns an actual `Err` if the file exists but fails to load/instantiate (that IS a
/// misconfiguration worth failing loudly on).
pub fn load(wasm_path: &std::path::Path) -> anyhow::Result<CryptoCore> {
    if !wasm_path.exists() {
        tracing::warn!(
            path = %wasm_path.display(),
            "no compiled OPRF crypto-core Wasm module found — running in STUB mode. \
             See wasm_host.rs / README.md. Evaluations produced in this mode are OBVIOUSLY FAKE \
             and must never reach a real chain (see ALLOW_STUB_SUBMISSION, default false). A real \
             module can be built from oprf-committee-dev/ (see its README's wasm32 target)."
        );
        return Ok(CryptoCore::Stub);
    }

    let engine = Engine::default();
    let module = Module::from_file(&engine, wasm_path)
        .with_context(|| format!("failed to compile Wasm module at {}", wasm_path.display()))?;
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[])
        .context("failed to instantiate Wasm module (does it import anything host-side not provided here?)")?;

    let memory = instance
        .get_memory(&mut store, "memory")
        .context("Wasm module does not export a memory named \"memory\"")?;
    let oprf_alloc = instance
        .get_typed_func::<i32, i32>(&mut store, "oprf_alloc")
        .context("Wasm module does not export oprf_alloc(len: i32) -> i32 — see oprf-committee-dev/src/ffi.rs")?;
    let oprf_dealloc = instance
        .get_typed_func::<(i32, i32), ()>(&mut store, "oprf_dealloc")
        .context("Wasm module does not export oprf_dealloc(ptr: i32, len: i32) — see oprf-committee-dev/src/ffi.rs")?;
    let oprf_evaluate_query = instance
        .get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, "oprf_evaluate_query")
        .context(
            "Wasm module does not export oprf_evaluate_query(input_ptr, input_len, output_ptr, \
             output_len) -> i32 — see oprf-committee-dev/src/ffi.rs for the authoritative ABI",
        )?;
    // Both threshold-round exports are treated as REQUIRED, not optional: a module predating
    // them fails to load here with this message rather than loading and then failing later,
    // mid-query, at the first round-1 call. The cost of that choice is that an operator running
    // a stale artifact loses `evaluate()` too — deliberate, since a node that can't complete a
    // threshold query is not usefully "partially working", and a loud startup failure naming
    // the rebuild command is a better failure than a per-query one.
    let oprf_round1 = instance
        .get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, "oprf_round1")
        .context(
            "Wasm module does not export oprf_round1(input_ptr, input_len, output_ptr, \
             output_len) -> i32 — the artifact predates the threshold (Option B) rounds; rebuild \
             it with `cd oprf-committee-dev && cargo build --release --target \
             wasm32-unknown-unknown --lib`",
        )?;
    let oprf_round2_response = instance
        .get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, "oprf_round2_response")
        .context(
            "Wasm module does not export oprf_round2_response(input_ptr, input_len, output_ptr, \
             output_len) -> i32 — the artifact predates the threshold (Option B) rounds; rebuild \
             it with `cd oprf-committee-dev && cargo build --release --target \
             wasm32-unknown-unknown --lib`",
        )?;

    tracing::info!(path = %wasm_path.display(), "loaded real OPRF crypto-core Wasm module");
    Ok(CryptoCore::Real(RealWasmCore {
        store,
        memory,
        oprf_alloc,
        oprf_dealloc,
        oprf_evaluate_query,
        oprf_round1,
        oprf_round2_response,
        instance,
    }))
}

impl CryptoCore {
    /// `secret_key`: this committee member's own OPRF secret key/share (big-endian, up to 32
    /// bytes). `blinded_query`: the citizen's blinded query point, `x(32) || y(32)` — the same
    /// 64-byte layout `PendingOprfQueries::blinded_query` already uses on-chain.
    ///
    /// See [`INPUT_LEN`]'s doc comment: unused by `main.rs` now, kept as a real capability.
    #[allow(dead_code)]
    pub fn evaluate(
        &mut self,
        secret_key: &[u8],
        blinded_query: [u8; 64],
    ) -> anyhow::Result<EvaluationResult> {
        match self {
            CryptoCore::Stub => Ok(stub_evaluate()),
            CryptoCore::Real(core) => core.evaluate(secret_key, blinded_query),
        }
    }

    /// Threshold round 1. `secret_key`: this member's own **share** `s_i` from the DKG (big-
    /// endian, up to 32 bytes) — not a standalone key, though the byte encoding is identical.
    /// `blinded_query`: the citizen's blinded query point, `x(32) || y(32)`, same layout as
    /// [`Self::evaluate`]. `seed`: caller-owned per-query randomness that **must be retained
    /// and replayed** into the matching [`Self::round2_response`] call — see the module docs
    /// for why this parameter exists here when `evaluate` generates its own.
    #[allow(dead_code)]
    pub fn round1(
        &mut self,
        secret_key: &[u8],
        blinded_query: [u8; 64],
        seed: [u8; 32],
    ) -> anyhow::Result<Round1Commitment> {
        match self {
            CryptoCore::Stub => Ok(stub_round1()),
            CryptoCore::Real(core) => core.round1(secret_key, blinded_query, seed),
        }
    }

    /// Threshold round 2. `secret_key`: the same share `s_i` used in round 1. `seed`: **the
    /// exact 32 bytes passed to the matching [`Self::round1`] call for this query** — a
    /// mismatch is silently wrong, not an error (see module docs). `rho_i`, `lambda_i`, `e`:
    /// public aggregation values (this member's binding factor, its Lagrange coefficient for
    /// the chosen participant set, and the combined challenge), each a big-endian BN254 `Fr`,
    /// computed natively by the caller and passed through opaquely here.
    #[allow(dead_code)]
    pub fn round2_response(
        &mut self,
        secret_key: &[u8],
        seed: [u8; 32],
        rho_i: [u8; 32],
        lambda_i: [u8; 32],
        e: [u8; 32],
    ) -> anyhow::Result<Round2Response> {
        match self {
            CryptoCore::Stub => Ok(stub_round2_response()),
            CryptoCore::Real(core) => core.round2_response(secret_key, seed, rho_i, lambda_i, e),
        }
    }
}

/// Left-pads a big-endian secret key/share into the leading 32-byte field of a wire-format
/// input buffer. Shared by all three wrappers so the "at most 32 bytes, right-aligned" rule is
/// stated once instead of three times.
fn write_secret_key(dst: &mut [u8], secret_key: &[u8]) -> anyhow::Result<()> {
    anyhow::ensure!(
        secret_key.len() <= 32,
        "OPRF secret key is {} bytes, expected at most 32 (big-endian BN254 Fr)",
        secret_key.len()
    );
    dst[32 - secret_key.len()..32].copy_from_slice(secret_key);
    Ok(())
}

impl RealWasmCore {
    /// The alloc/write/call/read/dealloc sequence every export on this ABI shares, in one
    /// place: all three take `(input_ptr, input_len, output_ptr, output_len)` and return `0` or
    /// a negative `ERR_*`. Factored out after the threshold rounds made it the third copy —
    /// the failure mode it prevents is a length constant getting out of step in one of three
    /// near-identical bodies. `export_name` is only used to name the export in the error.
    ///
    /// Note the output length passed to the module is always `OUT` itself, so a caller cannot
    /// provoke `ERR_BAD_OUTPUT_LEN` through this path by construction — see
    /// [`Self::call_export_raw`], which the tests use to exercise that code explicitly.
    fn call_export<const OUT: usize>(
        &mut self,
        func: TypedFunc<(i32, i32, i32, i32), i32>,
        export_name: &str,
        input: &[u8],
    ) -> anyhow::Result<[u8; OUT]> {
        let input_len = i32::try_from(input.len())
            .with_context(|| format!("{export_name} input length does not fit the i32 wasm ABI"))?;
        let output_len = i32::try_from(OUT)
            .with_context(|| format!("{export_name} output length does not fit the i32 wasm ABI"))?;

        let input_ptr = self.oprf_alloc.call(&mut self.store, input_len)?;
        self.memory.write(&mut self.store, input_ptr as usize, input)?;
        let output_ptr = self.oprf_alloc.call(&mut self.store, output_len)?;

        let code =
            func.call(&mut self.store, (input_ptr, input_len, output_ptr, output_len))?;

        // Best-effort cleanup regardless of outcome — a short-lived per-query allocation, not
        // load-bearing for correctness if it's skipped on an early return, but tidy.
        let _ = self.oprf_dealloc.call(&mut self.store, (input_ptr, input_len));

        if code != 0 {
            let _ = self.oprf_dealloc.call(&mut self.store, (output_ptr, output_len));
            anyhow::bail!(
                "{export_name} returned error code {code} ({})",
                describe_error_code(code)
            );
        }

        let mut out = [0u8; OUT];
        self.memory.read(&self.store, output_ptr as usize, &mut out)?;
        let _ = self.oprf_dealloc.call(&mut self.store, (output_ptr, output_len));
        Ok(out)
    }

    /// Test-only escape hatch: same sequence as [`Self::call_export`] but with a caller-chosen
    /// `output_len` and the raw `i32` code returned instead of an interpreted result. Exists
    /// solely so the tests can prove the module rejects a wrong-sized output buffer (and, by
    /// pairing that with the happy path, that this file's `*_OUTPUT_LEN` constants are the
    /// exact values the module demands rather than merely large enough).
    #[cfg(test)]
    fn call_export_raw(
        &mut self,
        func: TypedFunc<(i32, i32, i32, i32), i32>,
        input: &[u8],
        output_len: i32,
    ) -> anyhow::Result<i32> {
        let input_len = i32::try_from(input.len())?;
        let input_ptr = self.oprf_alloc.call(&mut self.store, input_len)?;
        self.memory.write(&mut self.store, input_ptr as usize, input)?;
        // Allocate the buffer the module would actually need, so that a *too small* declared
        // `output_len` is the only thing wrong here — this must exercise the module's length
        // check, not hand it a genuinely undersized allocation to scribble past.
        let output_ptr = self.oprf_alloc.call(&mut self.store, ROUND1_OUTPUT_LEN)?;
        let code = func.call(&mut self.store, (input_ptr, input_len, output_ptr, output_len))?;
        let _ = self.oprf_dealloc.call(&mut self.store, (input_ptr, input_len));
        let _ = self.oprf_dealloc.call(&mut self.store, (output_ptr, ROUND1_OUTPUT_LEN));
        Ok(code)
    }

    #[allow(dead_code)] // see INPUT_LEN's doc comment
    fn evaluate(
        &mut self,
        secret_key: &[u8],
        blinded_query: [u8; 64],
    ) -> anyhow::Result<EvaluationResult> {
        // Build the fixed 160-byte input: sk (left-padded to 32) || b_q.x || b_q.y || ds_dlog
        // || seed. `seed` is fresh OS-sourced randomness for this call's Chaum-Pedersen proof
        // nonce — see the module-level docs on why this MUST be fresh every call.
        let mut input = [0u8; INPUT_LEN as usize];
        write_secret_key(&mut input, secret_key)?;
        input[32..96].copy_from_slice(&blinded_query);
        input[96..128].copy_from_slice(&DS_DLOG_BE);
        {
            use rand::RngCore;
            rand::rngs::OsRng.fill_bytes(&mut input[128..160]);
        }

        let out: [u8; OUTPUT_LEN as usize] =
            self.call_export(self.oprf_evaluate_query.clone(), "oprf_evaluate_query", &input)?;

        let mut pk = [0u8; 64];
        let mut dlog_proof = [0u8; 64];
        let mut evaluation = [0u8; 64];
        pk.copy_from_slice(&out[0..64]);
        dlog_proof.copy_from_slice(&out[64..128]);
        evaluation.copy_from_slice(&out[128..192]);

        Ok(EvaluationResult { pk, dlog_proof, evaluation, is_real: true })
    }

    #[allow(dead_code)]
    fn round1(
        &mut self,
        secret_key: &[u8],
        blinded_query: [u8; 64],
        seed: [u8; 32],
    ) -> anyhow::Result<Round1Commitment> {
        // sk (left-padded to 32) || b_q.x || b_q.y || seed. No `ds_dlog` field here — round 1
        // computes no proof, so there is nothing to domain-separate yet; the domain separator
        // enters the threshold protocol later, in the caller's native `combined_challenge`.
        let mut input = [0u8; ROUND1_INPUT_LEN as usize];
        write_secret_key(&mut input, secret_key)?;
        input[32..96].copy_from_slice(&blinded_query);
        input[96..128].copy_from_slice(&seed);

        let out: [u8; ROUND1_OUTPUT_LEN as usize] =
            self.call_export(self.oprf_round1.clone(), "oprf_round1", &input)?;

        // Five consecutive 64-byte points, in `ffi.rs::round1`'s write order.
        let point = |i: usize| {
            let mut p = [0u8; 64];
            p.copy_from_slice(&out[i * 64..(i + 1) * 64]);
            p
        };
        Ok(Round1Commitment {
            r_i: point(0),
            d_g: point(1),
            d_q: point(2),
            e_g: point(3),
            e_q: point(4),
            is_real: true,
        })
    }

    #[allow(dead_code)]
    fn round2_response(
        &mut self,
        secret_key: &[u8],
        seed: [u8; 32],
        rho_i: [u8; 32],
        lambda_i: [u8; 32],
        e: [u8; 32],
    ) -> anyhow::Result<Round2Response> {
        // sk (left-padded to 32) || seed || rho_i || lambda_i || e. `seed` must be byte-identical
        // to the one given to the matching round-1 call — that is what re-derives the same
        // nonces inside the module. Nothing here can check that; see the module docs.
        let mut input = [0u8; ROUND2_INPUT_LEN as usize];
        write_secret_key(&mut input, secret_key)?;
        input[32..64].copy_from_slice(&seed);
        input[64..96].copy_from_slice(&rho_i);
        input[96..128].copy_from_slice(&lambda_i);
        input[128..160].copy_from_slice(&e);

        let z_i: [u8; ROUND2_OUTPUT_LEN as usize] =
            self.call_export(self.oprf_round2_response.clone(), "oprf_round2_response", &input)?;

        Ok(Round2Response { z_i, is_real: true })
    }
}

/// The stub path. Returns an unmistakably-fake, fixed value — never anything derived from real
/// input in a way that could be mistaken for a genuine evaluation. `0xEE` repeated is chosen
/// purely as a visually distinctive marker in logs/hex dumps ("stub" -> no numerological
/// meaning beyond "clearly not a real curve point").
#[allow(dead_code)] // see INPUT_LEN's doc comment
fn stub_evaluate() -> EvaluationResult {
    tracing::warn!(
        "STUB evaluate() called — returning a fixed, fake placeholder. \
         This is NOT a real OPRF evaluation and must never be submitted to a real chain."
    );
    EvaluationResult {
        pk: [0xEE; 64],
        dlog_proof: [0xEE; 64],
        evaluation: [0xEE; 64],
        is_real: false,
    }
}

/// Stub round 1 — same contract as [`stub_evaluate`]: fixed, unmistakably-fake `0xEE` bytes,
/// derived from nothing, `is_real: false`. Note these are not even valid curve points, which is
/// the point: a stub commitment cannot accidentally combine into something a verifier accepts.
#[allow(dead_code)]
fn stub_round1() -> Round1Commitment {
    tracing::warn!(
        "STUB round1() called — returning fixed, fake placeholder commitments. \
         This is NOT a real threshold round-1 output and must never be submitted to a real chain."
    );
    Round1Commitment {
        r_i: [0xEE; 64],
        d_g: [0xEE; 64],
        d_q: [0xEE; 64],
        e_g: [0xEE; 64],
        e_q: [0xEE; 64],
        is_real: false,
    }
}

/// Stub round 2 — see [`stub_round1`].
#[allow(dead_code)]
fn stub_round2_response() -> Round2Response {
    tracing::warn!(
        "STUB round2_response() called — returning a fixed, fake placeholder scalar. \
         This is NOT a real threshold round-2 response and must never be submitted to a real chain."
    );
    Round2Response { z_i: [0xEE; 32], is_real: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end check against the REAL compiled artifact from `oprf-committee-dev` (built via
    /// `cd oprf-committee-dev && cargo build --release --target wasm32-unknown-unknown --lib` —
    /// not part of this crate's own build, so this test skips, rather than fails, if the
    /// artifact isn't present, same as `load()`'s own stub fallback). This is the one test that
    /// exercises the reconciled ABI end-to-end through wasmtime against the real module, not
    /// just against the documented shape.
    ///
    /// Deliberately uses a **zero secret key** rather than a real curve point as the "input
    /// that reaches the module correctly" check: this crate has no dependency on
    /// `oprf-committee-dev`'s curve types (by design — see Cargo.toml), so fabricating a
    /// genuine on-curve BabyJubJub point here would mean guessing coordinates with no way to
    /// confirm they're actually valid. A zero secret key needs no curve math to construct
    /// correctly, and the module's own `ERR_ZERO_SECRET_KEY` check (`ffi.rs`) still exercises
    /// exactly the plumbing this test cares about: allocate input, write 160 bytes, call
    /// `oprf_evaluate_query`, read back a real (non-zero) `i32` return code, map it through
    /// `describe_error_code`. Getting a *specific, correct* error back — not a wasmtime trap,
    /// not a generic failure — is the actual proof the marshaling is right. Golden-vector
    /// correctness of the happy path is already covered by `oprf-committee-dev`'s own
    /// `ffi.rs` tests; this test's job is only "does this crate's byte marshaling match what
    /// the real module expects," which an error path proves just as well as success would.
    #[test]
    fn loads_the_real_module_and_the_abi_marshaling_round_trips() {
        let wasm_path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../oprf-committee-dev/target/wasm32-unknown-unknown/release/oprf_committee_dev.wasm"
        ));
        if !wasm_path.exists() {
            eprintln!(
                "SKIPPED: {} not found — build it first with `cd oprf-committee-dev && cargo \
                 build --release --target wasm32-unknown-unknown --lib`",
                wasm_path.display()
            );
            return;
        }

        let mut core = load(wasm_path).expect("real module must load and instantiate");
        assert!(matches!(core, CryptoCore::Real(_)), "expected Real, not Stub, given the file exists");

        let zero_secret_key = [0u8; 32];
        let arbitrary_blinded_query = [0u8; 64]; // never reached — rejected on the sk check first

        let err = core
            .evaluate(&zero_secret_key, arbitrary_blinded_query)
            .expect_err("a zero secret key must be rejected by the module, not silently accepted");
        let message = err.to_string();
        assert!(
            message.contains("ERR_ZERO_SECRET_KEY"),
            "expected the real module's ERR_ZERO_SECRET_KEY (-3) to come back through \
             describe_error_code(), got: {message}"
        );
    }

    // ── Threshold round 1 / round 2 ──────────────────────────────────────────────────────
    //
    // Same skip-if-absent contract, same purpose, as the test above: these prove *this file's
    // byte marshaling* against the real artifact, not the cryptography. The cryptography is
    // already proven in `oprf-committee-dev/src/ffi.rs`'s own
    // `round1_then_round2_ffi_calls_produce_a_verifiable_combined_proof`, which drives both
    // functions through the FFI boundary and checks the combined proof against the real,
    // unmodified Noir-derived `dlog::verify`. Repeating that here would mean reimplementing
    // BabyJubJub aggregation in a crate that deliberately has no dependency on it.

    /// The BabyJubJub base point, big-endian `x` then `y` — the one genuinely valid,
    /// on-curve, in-prime-order-subgroup point this crate can name without doing any curve
    /// math of its own. Copied from `oprf-committee-dev/src/babyjubjub.rs::Point::generator`
    /// (itself `oprf-nr`'s `BABYJUBJUB_GENERATOR_X/_Y`), whose validity is asserted by that
    /// crate's own `generator_is_on_curve_and_in_subgroup` test. Using it as a stand-in
    /// "blinded query" lets these tests reach round 1's *success* path, which the existing
    /// `evaluate` test could not (see its comment on why it settles for an error path).
    const GENERATOR_XY: [u8; 64] = [
        // x = 5299619240641551281634865583518297030282874472190772894086521144482721001553
        0x0b, 0xb7, 0x7a, 0x6a, 0xd6, 0x3e, 0x73, 0x9b, 0x4e, 0xac, 0xb2, 0xe0, 0x9d, 0x62, 0x77,
        0xc1, 0x2a, 0xb8, 0xd8, 0x01, 0x05, 0x34, 0xe0, 0xb6, 0x28, 0x93, 0xf3, 0xf6, 0xbb, 0x95,
        0x70, 0x51, // y = 16950150798460657717958625567821834550301663161624707787222815936182638968203
        0x25, 0x79, 0x72, 0x03, 0xf7, 0xa0, 0xb2, 0x49, 0x25, 0x57, 0x2e, 0x1c, 0xd1, 0x6b, 0xf9,
        0xed, 0xfc, 0xe0, 0x05, 0x1f, 0xb9, 0xe1, 0x33, 0x77, 0x4b, 0x3c, 0x25, 0x7a, 0x87, 0x2d,
        0x7d, 0x8b,
    ];

    /// A nonzero 32-byte secret share. Value is arbitrary — any nonzero scalar is a valid
    /// share as far as this ABI is concerned.
    const SHARE: [u8; 32] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2e, 0x3b, 0x9a,
        0xc9, 0xf1,
    ];

    /// Loads the real artifact, or returns `None` after printing the same SKIPPED note the
    /// existing test prints (the artifact is built by a different crate for a different target
    /// and is deliberately not a build dependency of this one).
    fn real_core_or_skip() -> Option<CryptoCore> {
        let wasm_path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../oprf-committee-dev/target/wasm32-unknown-unknown/release/oprf_committee_dev.wasm"
        ));
        if !wasm_path.exists() {
            eprintln!(
                "SKIPPED: {} not found — build it first with `cd oprf-committee-dev && cargo \
                 build --release --target wasm32-unknown-unknown --lib`",
                wasm_path.display()
            );
            return None;
        }
        Some(load(wasm_path).expect("real module must load and instantiate"))
    }

    /// The main round-trip: `round1` then `round2_response` with the same seed, both against
    /// the real module, both on the success path.
    ///
    /// The assertions are chosen to actually pin the marshaling rather than just check "no
    /// error came back". The load-bearing one is the **seed-sensitivity split**: `r_i` is
    /// `s_i * b_q`, which involves no nonce at all, while `d_g`/`d_q`/`e_g`/`e_q` are
    /// commitments to the two seed-derived nonces. So re-running round 1 with a *different*
    /// seed and nothing else changed must leave `r_i` byte-identical while changing all four
    /// nonce commitments. That single observation confirms, in one shot, that the `seed` field
    /// lands at the offset the module reads it from (a seed written to the wrong offset, or
    /// dropped, would leave the commitments unchanged too) *and* that this file's split of the
    /// 320-byte output into five points matches `ffi.rs`'s write order (a shifted split would
    /// put nonce-derived bytes in `r_i` and make it vary).
    #[test]
    fn round1_then_round2_marshal_correctly_against_the_real_module() {
        let Some(mut core) = real_core_or_skip() else { return };
        assert!(matches!(core, CryptoCore::Real(_)), "expected Real, not Stub, given the file exists");

        let seed = [0x5Au8; 32];
        let c1 = core
            .round1(&SHARE, GENERATOR_XY, seed)
            .expect("round1 must succeed for a nonzero share and a genuine subgroup point");
        assert!(c1.is_real, "a real module produced this, so is_real must be true");

        // Every point must be a real 64-byte value, not the 0xEE stub marker and not zeroed
        // memory we failed to read back.
        for (name, p) in [
            ("r_i", &c1.r_i),
            ("d_g", &c1.d_g),
            ("d_q", &c1.d_q),
            ("e_g", &c1.e_g),
            ("e_q", &c1.e_q),
        ] {
            assert_eq!(p.len(), 64, "{name} must be exactly 64 bytes (x || y)");
            assert!(p.iter().any(|&b| b != 0), "{name} came back all-zero — output never read?");
            assert!(p.iter().any(|&b| b != 0xEE), "{name} is the stub sentinel, not real output");
        }

        // Determinism: same inputs, same seed, byte-identical output.
        let repeat = core.round1(&SHARE, GENERATOR_XY, seed).expect("round1 must succeed again");
        assert_eq!(repeat.r_i, c1.r_i);
        assert_eq!(repeat.d_g, c1.d_g);
        assert_eq!(repeat.e_q, c1.e_q);

        // The seed-sensitivity split described above.
        let other = core
            .round1(&SHARE, GENERATOR_XY, [0xA5u8; 32])
            .expect("round1 must succeed with a different seed too");
        assert_eq!(
            other.r_i, c1.r_i,
            "r_i = s_i * b_q does not involve the nonces, so changing only the seed must not \
             change it — if it did, the output split or the seed offset is wrong"
        );
        for (name, a, b) in [
            ("d_g", &other.d_g, &c1.d_g),
            ("d_q", &other.d_q, &c1.d_q),
            ("e_g", &other.e_g, &c1.e_g),
            ("e_q", &other.e_q, &c1.e_q),
        ] {
            assert_ne!(
                a, b,
                "{name} is a commitment to a seed-derived nonce, so a different seed must change \
                 it — if it didn't, the seed isn't reaching the module's RNG"
            );
        }

        // Round 2, with the SAME seed (the whole point of the parameter), and public
        // aggregation values the caller would normally compute natively. Their exact values
        // don't matter to this test — only that they're passed through at the right offsets.
        let rho_i = [0x11u8; 32];
        let lambda_i = [0x22u8; 32];
        let e = [0x33u8; 32];
        let r2 = core
            .round2_response(&SHARE, seed, rho_i, lambda_i, e)
            .expect("round2_response must succeed for a nonzero share");
        assert!(r2.is_real);
        assert_eq!(r2.z_i.len(), 32);
        assert!(r2.z_i.iter().any(|&b| b != 0), "z_i came back all-zero — output never read?");
        assert!(r2.z_i.iter().any(|&b| b != 0xEE), "z_i is the stub sentinel, not real output");

        // Same seed + same public values => same z_i; and each of the three public inputs must
        // actually influence the result, which is what proves they land at distinct, correct
        // offsets rather than all being read from one place (or ignored).
        let again = core.round2_response(&SHARE, seed, rho_i, lambda_i, e).unwrap();
        assert_eq!(again.z_i, r2.z_i, "round2_response must be deterministic in its inputs");

        let changed_seed = core.round2_response(&SHARE, [0xA5u8; 32], rho_i, lambda_i, e).unwrap();
        assert_ne!(changed_seed.z_i, r2.z_i, "z_i must depend on the seed-derived nonces");
        let changed_rho = core.round2_response(&SHARE, seed, [0x44u8; 32], lambda_i, e).unwrap();
        assert_ne!(changed_rho.z_i, r2.z_i, "z_i must depend on rho_i (offset 64..96)");
        let changed_lambda = core.round2_response(&SHARE, seed, rho_i, [0x44u8; 32], e).unwrap();
        assert_ne!(changed_lambda.z_i, r2.z_i, "z_i must depend on lambda_i (offset 96..128)");
        let changed_e = core.round2_response(&SHARE, seed, rho_i, lambda_i, [0x44u8; 32]).unwrap();
        assert_ne!(changed_e.z_i, r2.z_i, "z_i must depend on e (offset 128..160)");

        // KNOWN LIMIT of the four assertions above, stated rather than glossed: they prove each
        // public input reaches a *distinct* offset that influences `z_i`, but not *which* offset
        // is which. Verified by mutation testing, not assumed — swapping this wrapper's `rho_i`
        // and `e` writes still passes this whole test, because both are consumed as raw `Fr` at
        // equal width and nothing here can compute the expected `z_i` to compare against. (A
        // `lambda_i` swap is likelier to show up, since `ffi.rs` puts that one through
        // `scalar::from_field` — but "likelier" is not "caught", so treat it the same.) Closing
        // that gap would mean reimplementing BabyJubJub aggregation in a crate that deliberately
        // has no dependency on it; what actually covers it is `ffi.rs`'s own
        // `round1_then_round2_ffi_calls_produce_a_verifiable_combined_proof`, which computes
        // these three values properly and checks the combined proof verifies. The residual risk
        // carried here is a transcription-order error in this one function, reviewable by
        // reading it against `ffi.rs`'s documented `sk || seed || rho_i || lambda_i || e`.
    }

    /// The blinded-query point sits at bytes 32..96 of round 1's input. Feeding a point the
    /// module is known to reject (`(1, 1)` — not on the BabyJubJub curve, the same fixture
    /// `ffi.rs`'s own `rejects_off_curve_blinded_query` uses) must produce specifically
    /// `ERR_BLINDED_QUERY_NOT_ON_CURVE`. Getting *that* code rather than, say,
    /// `ERR_ZERO_SECRET_KEY` or a success is what confirms the point is being written where the
    /// module looks for it, and that round 1's input layout really does omit `ds_dlog`.
    #[test]
    fn round1_rejects_an_off_curve_blinded_query_through_the_real_module() {
        let Some(mut core) = real_core_or_skip() else { return };
        let mut bad = [0u8; 64];
        bad[31] = 1; // x = 1
        bad[63] = 1; // y = 1
        let err = core
            .round1(&SHARE, bad, [1u8; 32])
            .expect_err("an off-curve blinded query must be rejected");
        let message = err.to_string();
        assert!(
            message.contains("ERR_BLINDED_QUERY_NOT_ON_CURVE") && message.contains("oprf_round1"),
            "expected oprf_round1's ERR_BLINDED_QUERY_NOT_ON_CURVE (-4), got: {message}"
        );
    }

    /// Zero-secret-key error path for round 1, mirroring the existing `evaluate` test.
    #[test]
    fn round1_rejects_a_zero_secret_key_through_the_real_module() {
        let Some(mut core) = real_core_or_skip() else { return };
        let err = core
            .round1(&[0u8; 32], GENERATOR_XY, [1u8; 32])
            .expect_err("a zero secret key must be rejected by the module, not silently accepted");
        let message = err.to_string();
        assert!(
            message.contains("ERR_ZERO_SECRET_KEY") && message.contains("oprf_round1"),
            "expected oprf_round1's ERR_ZERO_SECRET_KEY (-3) via describe_error_code(), got: {message}"
        );
    }

    /// Same, for round 2. Worth having separately: round 2 never sees a curve point, so this is
    /// the only input-validation code it has, and it reads the secret key from the same leading
    /// 32 bytes — a wrapper that wrote the key at the wrong offset would fail here.
    #[test]
    fn round2_response_rejects_a_zero_secret_key_through_the_real_module() {
        let Some(mut core) = real_core_or_skip() else { return };
        let err = core
            .round2_response(&[0u8; 32], [1u8; 32], [2u8; 32], [3u8; 32], [4u8; 32])
            .expect_err("a zero secret key must be rejected by the module, not silently accepted");
        let message = err.to_string();
        assert!(
            message.contains("ERR_ZERO_SECRET_KEY") && message.contains("oprf_round2_response"),
            "expected oprf_round2_response's ERR_ZERO_SECRET_KEY (-3), got: {message}"
        );
    }

    /// Pins [`ROUND1_OUTPUT_LEN`] and [`ROUND2_OUTPUT_LEN`] to the module's own expectations
    /// exactly, in both directions: one byte short must return `ERR_BAD_OUTPUT_LEN` (-2), and
    /// the constant itself must return `0`. The public wrappers can't express the first case —
    /// they always pass the constant — so this drives the exports through `call_export_raw`.
    /// A too-*large* length is checked too, since `ffi.rs` uses `!=`, not `<`, and a wrapper
    /// that padded the buffer "to be safe" would break.
    #[test]
    fn threshold_output_length_constants_match_the_real_module_exactly() {
        let Some(core) = real_core_or_skip() else { return };
        let CryptoCore::Real(mut core) = core else {
            panic!("expected Real, not Stub, given the artifact exists")
        };

        let mut r1_input = [0u8; ROUND1_INPUT_LEN as usize];
        r1_input[0..32].copy_from_slice(&SHARE);
        r1_input[32..96].copy_from_slice(&GENERATOR_XY);
        r1_input[96..128].copy_from_slice(&[9u8; 32]);

        let f1 = core.oprf_round1.clone();
        for wrong in [ROUND1_OUTPUT_LEN - 1, ROUND1_OUTPUT_LEN + 1] {
            assert_eq!(
                core.call_export_raw(f1.clone(), &r1_input, wrong).unwrap(),
                -2,
                "oprf_round1 must reject an output_len of {wrong} with ERR_BAD_OUTPUT_LEN"
            );
        }
        assert_eq!(
            core.call_export_raw(f1, &r1_input, ROUND1_OUTPUT_LEN).unwrap(),
            0,
            "ROUND1_OUTPUT_LEN ({ROUND1_OUTPUT_LEN}) must be the length the module accepts"
        );

        let mut r2_input = [0u8; ROUND2_INPUT_LEN as usize];
        r2_input[0..32].copy_from_slice(&SHARE);

        let f2 = core.oprf_round2_response.clone();
        for wrong in [ROUND2_OUTPUT_LEN - 1, ROUND2_OUTPUT_LEN + 1] {
            assert_eq!(
                core.call_export_raw(f2.clone(), &r2_input, wrong).unwrap(),
                -2,
                "oprf_round2_response must reject an output_len of {wrong} with ERR_BAD_OUTPUT_LEN"
            );
        }
        assert_eq!(
            core.call_export_raw(f2, &r2_input, ROUND2_OUTPUT_LEN).unwrap(),
            0,
            "ROUND2_OUTPUT_LEN ({ROUND2_OUTPUT_LEN}) must be the length the module accepts"
        );
    }

    /// Input-length constants, pinned the same way: one byte short of
    /// [`ROUND1_INPUT_LEN`]/[`ROUND2_INPUT_LEN`] must return `ERR_BAD_INPUT_LEN` (-1) while the
    /// constant itself succeeds. Together with the test above, this makes all four threshold
    /// length constants in this file provably the module's own values rather than transcription
    /// guesses.
    #[test]
    fn threshold_input_length_constants_match_the_real_module_exactly() {
        let Some(core) = real_core_or_skip() else { return };
        let CryptoCore::Real(mut core) = core else {
            panic!("expected Real, not Stub, given the artifact exists")
        };

        let mut short_r1 = vec![0u8; ROUND1_INPUT_LEN as usize - 1];
        short_r1[0..32].copy_from_slice(&SHARE);
        let f1 = core.oprf_round1.clone();
        assert_eq!(
            core.call_export_raw(f1, &short_r1, ROUND1_OUTPUT_LEN).unwrap(),
            -1,
            "oprf_round1 must reject a {}-byte input with ERR_BAD_INPUT_LEN",
            short_r1.len()
        );

        let mut short_r2 = vec![0u8; ROUND2_INPUT_LEN as usize - 1];
        short_r2[0..32].copy_from_slice(&SHARE);
        let f2 = core.oprf_round2_response.clone();
        assert_eq!(
            core.call_export_raw(f2, &short_r2, ROUND2_OUTPUT_LEN).unwrap(),
            -1,
            "oprf_round2_response must reject a {}-byte input with ERR_BAD_INPUT_LEN",
            short_r2.len()
        );
    }

    /// The stub path must stay obviously fake and obviously flagged for the new rounds too —
    /// `main.rs` gates submission on `is_real`, so a stub that forgot to set it false would be
    /// the one bug in this file capable of putting fabricated bytes on a real chain.
    #[test]
    fn stub_threshold_rounds_are_flagged_not_real() {
        let mut core = CryptoCore::Stub;
        let c1 = core.round1(&SHARE, GENERATOR_XY, [1u8; 32]).unwrap();
        assert!(!c1.is_real);
        assert_eq!(c1.r_i, [0xEE; 64]);
        let r2 = core.round2_response(&SHARE, [1u8; 32], [2u8; 32], [3u8; 32], [4u8; 32]).unwrap();
        assert!(!r2.is_real);
        assert_eq!(r2.z_i, [0xEE; 32]);
    }
}
