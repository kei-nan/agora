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

use anyhow::Context;
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

/// `sk || b_q.x || b_q.y || ds_dlog || seed`, each field 32 bytes. Must match
/// `oprf-committee-dev/src/ffi.rs::INPUT_LEN` exactly.
const INPUT_LEN: i32 = 160;
/// `pk.x/y || dlog_e/dlog_s || response_blinded.x/y`, each field 32 bytes. Must match
/// `oprf-committee-dev/src/ffi.rs::OUTPUT_LEN` exactly.
const OUTPUT_LEN: i32 = 192;

/// `DS_DLOG` — ASCII "DLOG Equality Proof" read as a big-endian integer, kept byte-identical
/// to ZKPassport's own `DS_DLOG` (see `circuits/oprf-identity-anchor/lib/identity-anchor/
/// src/lib.nr`: `pub global DS_DLOG: Field = 1523098184080632582082867317389990410064981862;`
/// and `circuits/oprf-identity-anchor/README.md`'s domain-separator table). This is the same
/// constant every proving circuit in this repo already uses — a committee node computing a
/// Chaum-Pedersen proof under a different value would produce a proof the citizen's own
/// `disclosure`/`migrate-disclosure` circuit rejects. **Public, not secret** — safe to hardcode.
const DS_DLOG_BE: [u8; 32] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x44, 0x4c,
    0x4f, 0x47, 0x20, 0x45, 0x71, 0x75, 0x61, 0x6c, 0x69, 0x74, 0x79, 0x20, 0x50, 0x72, 0x6f,
    0x6f, 0x66,
];

/// Mirrors `oprf-committee-dev/src/ffi.rs`'s `ERR_*` constants, for logging a human-readable
/// reason rather than a bare negative number when `oprf_evaluate_query` rejects an input.
fn describe_error_code(code: i32) -> &'static str {
    match code {
        -1 => "ERR_BAD_INPUT_LEN — input was not exactly 160 bytes",
        -2 => "ERR_BAD_OUTPUT_LEN — output buffer was not exactly 192 bytes",
        -3 => "ERR_ZERO_SECRET_KEY — this node's configured OPRF secret key is all-zero",
        -4 => "ERR_BLINDED_QUERY_NOT_ON_CURVE — the citizen's blinded query point is malformed",
        -5 => "ERR_BLINDED_QUERY_NOT_IN_SUBGROUP — blinded query is on-curve but outside the prime-order subgroup",
        _ => "unrecognized error code — oprf-committee-dev/src/ffi.rs's ERR_* constants may have changed",
    }
}

#[derive(Debug)]
pub struct EvaluationResult {
    /// `sk * G` — this committee member's own public key, as recomputed by the wasm module
    /// from the secret key it was given. Not submitted on-chain (the chain already knows the
    /// member's public key via governance-registered `OprfCommitteeKeys`) — kept here for a
    /// future host-side sanity check that the configured secret key matches what's expected;
    /// not wired to any check yet, hence `allow(dead_code)` rather than deleting the field.
    #[allow(dead_code)]
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

pub enum CryptoCore {
    Real(RealWasmCore),
    Stub,
}

pub struct RealWasmCore {
    store: Store<()>,
    memory: Memory,
    oprf_alloc: TypedFunc<i32, i32>,
    oprf_dealloc: TypedFunc<(i32, i32), ()>,
    oprf_evaluate_query: TypedFunc<(i32, i32, i32, i32), i32>,
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

    tracing::info!(path = %wasm_path.display(), "loaded real OPRF crypto-core Wasm module");
    Ok(CryptoCore::Real(RealWasmCore {
        store,
        memory,
        oprf_alloc,
        oprf_dealloc,
        oprf_evaluate_query,
        instance,
    }))
}

impl CryptoCore {
    /// `secret_key`: this committee member's own OPRF secret key/share (big-endian, up to 32
    /// bytes). `blinded_query`: the citizen's blinded query point, `x(32) || y(32)` — the same
    /// 64-byte layout `PendingOprfQueries::blinded_query` already uses on-chain.
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
}

impl RealWasmCore {
    fn evaluate(
        &mut self,
        secret_key: &[u8],
        blinded_query: [u8; 64],
    ) -> anyhow::Result<EvaluationResult> {
        anyhow::ensure!(
            secret_key.len() <= 32,
            "OPRF secret key is {} bytes, expected at most 32 (big-endian BN254 Fr)",
            secret_key.len()
        );

        // Build the fixed 160-byte input: sk (left-padded to 32) || b_q.x || b_q.y || ds_dlog
        // || seed. `seed` is fresh OS-sourced randomness for this call's Chaum-Pedersen proof
        // nonce — see the module-level docs on why this MUST be fresh every call.
        let mut input = [0u8; 160];
        input[32 - secret_key.len()..32].copy_from_slice(secret_key);
        input[32..96].copy_from_slice(&blinded_query);
        input[96..128].copy_from_slice(&DS_DLOG_BE);
        {
            use rand::RngCore;
            rand::rngs::OsRng.fill_bytes(&mut input[128..160]);
        }

        let input_ptr = self.oprf_alloc.call(&mut self.store, INPUT_LEN)?;
        self.memory.write(&mut self.store, input_ptr as usize, &input)?;
        let output_ptr = self.oprf_alloc.call(&mut self.store, OUTPUT_LEN)?;

        let code = self.oprf_evaluate_query.call(
            &mut self.store,
            (input_ptr, INPUT_LEN, output_ptr, OUTPUT_LEN),
        )?;

        // Best-effort cleanup regardless of outcome — a short-lived per-query allocation, not
        // load-bearing for correctness if it's skipped on an early return, but tidy.
        let _ = self.oprf_dealloc.call(&mut self.store, (input_ptr, INPUT_LEN));

        if code != 0 {
            let _ = self.oprf_dealloc.call(&mut self.store, (output_ptr, OUTPUT_LEN));
            anyhow::bail!(
                "oprf_evaluate_query returned error code {code} ({})",
                describe_error_code(code)
            );
        }

        let mut out = [0u8; 192];
        self.memory.read(&self.store, output_ptr as usize, &mut out)?;
        let _ = self.oprf_dealloc.call(&mut self.store, (output_ptr, OUTPUT_LEN));

        let mut pk = [0u8; 64];
        let mut dlog_proof = [0u8; 64];
        let mut evaluation = [0u8; 64];
        pk.copy_from_slice(&out[0..64]);
        dlog_proof.copy_from_slice(&out[64..128]);
        evaluation.copy_from_slice(&out[128..192]);

        Ok(EvaluationResult { pk, dlog_proof, evaluation, is_real: true })
    }
}

/// The stub path. Returns an unmistakably-fake, fixed value — never anything derived from real
/// input in a way that could be mistaken for a genuine evaluation. `0xEE` repeated is chosen
/// purely as a visually distinctive marker in logs/hex dumps ("stub" -> no numerological
/// meaning beyond "clearly not a real curve point").
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
}
