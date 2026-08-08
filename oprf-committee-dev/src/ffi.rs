//! C-ABI boundary exposing `oprf::evaluate` — the one function that represents "what a real
//! committee member's node does upon receiving a blinded query" (take the client's blinded
//! query point + this member's own secret key, produce an evaluation point plus a
//! Chaum-Pedersen proof that the same secret key was used as the member's published public
//! key) — for compilation to `wasm32-unknown-unknown` (changelog entry 082: one portable
//! crypto core, loaded by a Wasm runtime on phone/laptop/Pi, instead of three separate ports).
//!
//! This module is deliberately a thin wrapper: it does no cryptography of its own beyond
//! encoding/decoding fixed-width big-endian byte layouts and validating that the untrusted
//! (network-supplied) blinded-query point is well-formed before handing it to
//! `oprf::evaluate`. All of `evaluate`'s own math (`babyjubjub`, `dlog`) is untouched.
//!
//! ## Wire format
//!
//! Every field is a 32-byte big-endian integer (BN254 `Fr`'s canonical width; also large
//! enough to hold any `BABYJUBJUB_Fr` scalar or committee secret key without truncation).
//!
//! `oprf_evaluate_query(input_ptr, input_len, output_ptr, output_len) -> i32`
//!
//! Input (`input_len` must be exactly [`INPUT_LEN`] = 160 bytes), concatenated in this order:
//! 1. `sk`        (32 bytes) — the committee member's own OPRF secret key/share.
//! 2. `b_q.x`      (32 bytes) — the client's blinded-query point, x-coordinate.
//! 3. `b_q.y`      (32 bytes) — the client's blinded-query point, y-coordinate.
//! 4. `ds_dlog`    (32 bytes) — the Chaum-Pedersen domain-separator field element
//!    (`verify_dlog_equality`'s `ds_dlog` / `dlog.nr`'s public domain-separator input).
//! 5. `seed`       (32 bytes) — caller-supplied randomness for the Chaum-Pedersen proof's
//!    nonce. **The wasm module has no OS entropy source of its own on
//!    `wasm32-unknown-unknown`** (see the crate-level note in this file's tests module) — the
//!    host (mobile app / container shell) MUST supply fresh, unpredictable bytes here every
//!    call from its own platform CSPRNG. Reusing a seed across two calls leaks the secret key
//!    (classic Schnorr/Chaum-Pedersen nonce-reuse attack) exactly as it would for any sigma
//!    protocol; nothing in this wrapper can detect or prevent that, by design — entropy is a
//!    host responsibility, deliberately kept out of the portable core.
//!
//! Output (`output_len` must be exactly [`OUTPUT_LEN`] = 192 bytes), written in this order:
//! 1. `pk.x`, `pk.y`               (32 + 32 bytes) — `sk * G`, the member's public key.
//! 2. `dlog_e`, `dlog_s`           (32 + 32 bytes) — the Chaum-Pedersen proof.
//! 3. `response_blinded.x/.y`      (32 + 32 bytes) — `sk * b_q`, the blinded evaluation.
//!
//! Return value: `0` on success; a negative error code (see the `ERR_*` constants) if the
//! input was malformed. On any non-zero return, `output_ptr` is left untouched.
//!
//! ## Memory management
//!
//! [`oprf_alloc`]/[`oprf_dealloc`] let a host (wasmtime/wasmer, with no `wasm-bindgen` glue)
//! allocate/free buffers inside the module's own linear memory to marshal bytes in and out,
//! the standard pattern for a bare `wasm32-unknown-unknown` C-ABI boundary.

use crate::babyjubjub::Point;
use crate::oprf;
use ark_bn254::Fr;
use ark_ff::{BigInteger, PrimeField};
use num_bigint::BigUint;
use rand::SeedableRng;

/// Width of every fixed-size field in the wire format: a big-endian BN254 `Fr` element.
pub const FIELD_LEN: usize = 32;
/// `sk || b_q.x || b_q.y || ds_dlog || seed`.
pub const INPUT_LEN: usize = FIELD_LEN * 5;
/// `pk.x || pk.y || dlog_e || dlog_s || response_blinded.x || response_blinded.y`.
pub const OUTPUT_LEN: usize = FIELD_LEN * 6;

/// `input_len` was not exactly [`INPUT_LEN`].
pub const ERR_BAD_INPUT_LEN: i32 = -1;
/// `output_len` was not exactly [`OUTPUT_LEN`].
pub const ERR_BAD_OUTPUT_LEN: i32 = -2;
/// The secret key field was all-zero (degenerate; `0 * G` is the identity, never a valid
/// committee public key).
pub const ERR_ZERO_SECRET_KEY: i32 = -3;
/// The supplied blinded-query point does not satisfy the BabyJubJub curve equation. Rejected
/// here rather than inside `babyjubjub::Point::add` (which would otherwise be reachable via a
/// division-by-zero `.expect()` panic — a wasm trap — on adversarial input coming straight off
/// the wire from an untrusted client's on-chain query).
pub const ERR_BLINDED_QUERY_NOT_ON_CURVE: i32 = -4;
/// The supplied blinded-query point is on-curve but outside the prime-order (`BABYJUBJUB_Fr`)
/// subgroup. Same class of defensive check `dlog::verify` already applies to its own `b`
/// argument on the verifier side; applying it here too means a malformed query is rejected at
/// the committee's own boundary instead of silently producing a proof no verifier will accept.
pub const ERR_BLINDED_QUERY_NOT_IN_SUBGROUP: i32 = -5;

/// Decodes a big-endian byte slice into a BN254 `Fr` element (reducing mod the field order,
/// matching every other KAT-parsing helper in this crate — see `oprf::tests`' own `fe`/
/// `fe_hex`). `pub` so callers assembling wire-format bytes (including this crate's own
/// wasm-vs-native equivalence test in `tests/`) can build/parse the same encoding without
/// duplicating it.
pub fn field_from_be(bytes: &[u8]) -> Fr {
    Fr::from_be_bytes_mod_order(bytes)
}

/// Canonical big-endian encoding of a BN254 `Fr` element, always exactly [`FIELD_LEN`] bytes
/// (BN254's modulus fits in 4 64-bit limbs = 32 bytes; `ark_ff`'s `to_bytes_be` serializes the
/// full limb width, never trims leading zero bytes). `pub` for the same reason as
/// [`field_from_be`].
pub fn field_to_be(f: &Fr) -> [u8; FIELD_LEN] {
    let bytes = f.into_bigint().to_bytes_be();
    let mut out = [0u8; FIELD_LEN];
    debug_assert_eq!(bytes.len(), FIELD_LEN, "BN254 Fr must serialize to exactly 32 bytes");
    out.copy_from_slice(&bytes);
    out
}

/// Safe, allocation-only core of the C-ABI boundary — everything [`oprf_evaluate_query`]
/// does except the raw-pointer marshaling, kept separate so it has ordinary Rust
/// `Result`/slice signatures and can be unit-tested directly (no `unsafe`, no FFI plumbing)
/// while the `#[no_mangle] extern "C"` entry point stays a minimal, auditable shim.
pub fn evaluate_query(input: &[u8]) -> Result<[u8; OUTPUT_LEN], i32> {
    if input.len() != INPUT_LEN {
        return Err(ERR_BAD_INPUT_LEN);
    }

    let sk_bytes = &input[0..32];
    let bq_x_bytes = &input[32..64];
    let bq_y_bytes = &input[64..96];
    let ds_dlog_bytes = &input[96..128];
    let seed_bytes = &input[128..160];

    let sk = BigUint::from_bytes_be(sk_bytes);
    if sk == BigUint::from(0u32) {
        return Err(ERR_ZERO_SECRET_KEY);
    }

    let b_q = Point::new(field_from_be(bq_x_bytes), field_from_be(bq_y_bytes));
    if !b_q.is_on_curve() {
        return Err(ERR_BLINDED_QUERY_NOT_ON_CURVE);
    }
    if !b_q.check_sub_group() {
        return Err(ERR_BLINDED_QUERY_NOT_IN_SUBGROUP);
    }

    let ds_dlog = field_from_be(ds_dlog_bytes);

    let mut seed = [0u8; 32];
    seed.copy_from_slice(seed_bytes);
    // `rand::rngs::StdRng` (ChaCha12) rather than `OsRng`/`thread_rng()`: seeded entirely from
    // caller-supplied bytes, no OS entropy call, which is what keeps this compiling and
    // running identically on `wasm32-unknown-unknown` (no OS underneath) as on native — see
    // the crate-level Cargo.toml comment on why `rand`'s "std"/"getrandom" feature is not
    // enabled at all.
    let mut rng = rand::rngs::StdRng::from_seed(seed);

    let eval = oprf::evaluate(&mut rng, &sk, &b_q, ds_dlog);

    let mut out = [0u8; OUTPUT_LEN];
    out[0..32].copy_from_slice(&field_to_be(&eval.pk.x));
    out[32..64].copy_from_slice(&field_to_be(&eval.pk.y));
    out[64..96].copy_from_slice(&field_to_be(&eval.dlog_e));
    out[96..128].copy_from_slice(&field_to_be(&eval.dlog_s));
    out[128..160].copy_from_slice(&field_to_be(&eval.response_blinded.x));
    out[160..192].copy_from_slice(&field_to_be(&eval.response_blinded.y));
    Ok(out)
}

/// The committee-member evaluation entry point: given a blinded query and this member's own
/// secret key (see the wire format documented at module level), compute the blinded OPRF
/// evaluation and its Chaum-Pedersen proof. Returns `0` on success (with `output_len` bytes
/// written to `output_ptr`) or a negative `ERR_*` code on malformed input.
///
/// # Safety
/// `input_ptr` must point to `input_len` readable bytes and `output_ptr` to `output_len`
/// writable bytes, both valid for the duration of this call and not aliasing one another.
/// Callers driving this from outside the wasm module (wasmtime/wasmer) should obtain both
/// buffers via [`oprf_alloc`] into the module's own linear memory.
#[no_mangle]
pub unsafe extern "C" fn oprf_evaluate_query(
    input_ptr: *const u8,
    input_len: usize,
    output_ptr: *mut u8,
    output_len: usize,
) -> i32 {
    if output_len != OUTPUT_LEN {
        return ERR_BAD_OUTPUT_LEN;
    }
    let input = core::slice::from_raw_parts(input_ptr, input_len);
    match evaluate_query(input) {
        Ok(out) => {
            core::ptr::copy_nonoverlapping(out.as_ptr(), output_ptr, OUTPUT_LEN);
            0
        }
        Err(code) => code,
    }
}

/// Allocates `len` bytes inside this module's linear memory and returns a pointer to them,
/// for a host with no other way to place bytes into wasm memory before calling
/// [`oprf_evaluate_query`] (the bare `wasm32-unknown-unknown` C-ABI pattern; no
/// `wasm-bindgen` glue is available/assumed on the wasmtime/wasmer hosts this module targets).
#[no_mangle]
pub extern "C" fn oprf_alloc(len: usize) -> *mut u8 {
    let mut buf = vec![0u8; len].into_boxed_slice();
    let ptr = buf.as_mut_ptr();
    core::mem::forget(buf);
    ptr
}

/// Frees a buffer previously returned by [`oprf_alloc`].
///
/// # Safety
/// `ptr`/`len` must be exactly the pointer and length returned by a prior [`oprf_alloc`] call
/// that has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn oprf_dealloc(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, len, len));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dlog;

    fn fe(dec: &str) -> Fr {
        Fr::from_be_bytes_mod_order(&BigUint::parse_bytes(dec.as_bytes(), 10).unwrap().to_bytes_be())
    }

    /// Builds a valid 160-byte input from the same fixture `oprf::tests` and `committee.rs`
    /// use, so this module's own tests exercise real, already-validated field values rather
    /// than arbitrary bytes.
    fn sample_input() -> ([u8; INPUT_LEN], BigUint, Point, Fr) {
        let ds_dlog = fe("1523098184080632582082867317389990410064981862");
        let client_input = fe("999999999999999999999999999");
        let beta = BigUint::from(4242424242424242u64);
        let b_q = oprf::blinded_query(&beta, client_input);
        let sk = BigUint::from(778899112233u64);

        let mut input = [0u8; INPUT_LEN];
        let sk_bytes = sk.to_bytes_be();
        input[32 - sk_bytes.len()..32].copy_from_slice(&sk_bytes);
        input[32..64].copy_from_slice(&field_to_be(&b_q.x));
        input[64..96].copy_from_slice(&field_to_be(&b_q.y));
        input[96..128].copy_from_slice(&field_to_be(&ds_dlog));
        // Deterministic "seed" for reproducible tests — a real host must use fresh entropy.
        input[128..160].copy_from_slice(&[7u8; 32]);
        (input, sk, b_q, ds_dlog)
    }

    #[test]
    fn evaluate_query_matches_direct_native_call() {
        let (input, sk, b_q, ds_dlog) = sample_input();
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&input[128..160]);

        let out = evaluate_query(&input).expect("valid fixture must evaluate");

        let mut rng = rand::rngs::StdRng::from_seed(seed);
        let direct = oprf::evaluate(&mut rng, &sk, &b_q, ds_dlog);

        assert_eq!(&out[0..32], &field_to_be(&direct.pk.x));
        assert_eq!(&out[32..64], &field_to_be(&direct.pk.y));
        assert_eq!(&out[64..96], &field_to_be(&direct.dlog_e));
        assert_eq!(&out[96..128], &field_to_be(&direct.dlog_s));
        assert_eq!(&out[128..160], &field_to_be(&direct.response_blinded.x));
        assert_eq!(&out[160..192], &field_to_be(&direct.response_blinded.y));
    }

    #[test]
    fn evaluate_query_output_is_a_valid_dlog_proof() {
        let (input, _, b_q, ds_dlog) = sample_input();
        let out = evaluate_query(&input).expect("valid fixture must evaluate");

        let pk = Point::new(field_from_be(&out[0..32]), field_from_be(&out[32..64]));
        let dlog_e = field_from_be(&out[64..96]);
        let dlog_s = field_from_be(&out[96..128]);
        let response_blinded =
            Point::new(field_from_be(&out[128..160]), field_from_be(&out[160..192]));

        assert!(dlog::verify(dlog_e, dlog_s, &pk, &b_q, &response_blinded, ds_dlog));
    }

    #[test]
    fn rejects_wrong_input_length() {
        let bytes = vec![0u8; INPUT_LEN - 1];
        assert_eq!(evaluate_query(&bytes), Err(ERR_BAD_INPUT_LEN));
    }

    #[test]
    fn rejects_zero_secret_key() {
        let (mut input, _, _, _) = sample_input();
        for b in input[0..32].iter_mut() {
            *b = 0;
        }
        assert_eq!(evaluate_query(&input), Err(ERR_ZERO_SECRET_KEY));
    }

    #[test]
    fn rejects_off_curve_blinded_query() {
        let (mut input, _, _, _) = sample_input();
        // (1, 1) is not a BabyJubJub point: 168700*1 + 1 != 1 + 168696*1.
        input[32..64].copy_from_slice(&field_to_be(&Fr::from(1u64)));
        input[64..96].copy_from_slice(&field_to_be(&Fr::from(1u64)));
        assert_eq!(evaluate_query(&input), Err(ERR_BLINDED_QUERY_NOT_ON_CURVE));
    }

    #[test]
    fn rejects_two_torsion_blinded_query() {
        // (0, -1): on-curve (checked in babyjubjub.rs's own test) but outside the
        // prime-order subgroup.
        let (mut input, _, _, _) = sample_input();
        input[32..64].copy_from_slice(&field_to_be(&Fr::from(0u64)));
        input[64..96].copy_from_slice(&field_to_be(&(-Fr::from(1u64))));
        assert_eq!(evaluate_query(&input), Err(ERR_BLINDED_QUERY_NOT_IN_SUBGROUP));
    }

    #[test]
    fn rejects_bad_output_len_at_the_ffi_boundary() {
        let (input, _, _, _) = sample_input();
        let mut out = [0u8; OUTPUT_LEN];
        let code = unsafe {
            oprf_evaluate_query(input.as_ptr(), input.len(), out.as_mut_ptr(), OUTPUT_LEN - 1)
        };
        assert_eq!(code, ERR_BAD_OUTPUT_LEN);
    }

    #[test]
    fn ffi_entry_point_matches_the_safe_core() {
        let (input, _, _, _) = sample_input();
        let safe = evaluate_query(&input).unwrap();
        let mut out = [0u8; OUTPUT_LEN];
        let code = unsafe {
            oprf_evaluate_query(input.as_ptr(), input.len(), out.as_mut_ptr(), OUTPUT_LEN)
        };
        assert_eq!(code, 0);
        assert_eq!(out, safe);
    }

    #[test]
    fn alloc_dealloc_round_trip() {
        let ptr = oprf_alloc(160);
        assert!(!ptr.is_null());
        unsafe {
            core::ptr::write_bytes(ptr, 0xAB, 160);
            let slice = core::slice::from_raw_parts(ptr, 160);
            assert!(slice.iter().all(|&b| b == 0xAB));
            oprf_dealloc(ptr, 160);
        }
    }
}
