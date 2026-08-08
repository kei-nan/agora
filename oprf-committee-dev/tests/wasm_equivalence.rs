//! **The actual deliverable of changelog entry 082's wasm-packaging task.** Loads the real
//! compiled `wasm32-unknown-unknown` `oprf_committee_dev.wasm` module through `wasmi` (a
//! pure-Rust Wasm interpreter, added as a dev-dependency purely for this test) and proves it
//! produces byte-identical output to calling `ffi::evaluate_query` natively, on the same
//! known-answer fixture `ffi::tests`/`oprf::tests`/`committee.rs` already use.
//!
//! This is what lets the wasm build claim the same validation status as the native crate
//! already has (see the top-level README's "What is real here"): it isn't a second, separately
//! trusted implementation — it's proven to be the exact same computation, just compiled to a
//! different target.
//!
//! Two things this test deliberately does NOT (re-)prove:
//! - That the crate's cryptography matches upstream `oprf-nr`/`TaceoLabs` KAT vectors — that's
//!   already `oprf::tests`/`dlog::tests`/etc.'s job, run natively, unaffected by this file.
//! - That the wasm build's own math is correct in isolation — it isn't re-derived here; this
//!   test only checks *equivalence* to the native computation this crate already validated.

use std::path::{Path, PathBuf};
use std::process::Command;

use ark_bn254::Fr;
use ark_ff::PrimeField;
use num_bigint::BigUint;
use wasmi::{Engine, Linker, Module, Store};

use oprf_committee_dev::babyjubjub::Point;
use oprf_committee_dev::ffi::{self, INPUT_LEN, OUTPUT_LEN};
use oprf_committee_dev::oprf;

fn fe(dec: &str) -> Fr {
    Fr::from_be_bytes_mod_order(&BigUint::parse_bytes(dec.as_bytes(), 10).unwrap().to_bytes_be())
}

/// Same fixture as `ffi::tests::sample_input` / `oprf::tests::generated_committee_response_round_trips`
/// (freshly-generated, non-upstream key material — the actual simulator/committee-member path
/// this crate exists for, not KAT replay) so the wasm build is checked against a real protocol
/// round, not a degenerate all-zero input.
fn sample_input() -> [u8; INPUT_LEN] {
    let ds_dlog = fe("1523098184080632582082867317389990410064981862");
    let client_input = fe("999999999999999999999999999");
    let beta = BigUint::from(4242424242424242u64);
    let b_q = oprf::blinded_query(&beta, client_input);
    let sk = BigUint::from(778899112233u64);

    let mut input = [0u8; INPUT_LEN];
    let sk_bytes = sk.to_bytes_be();
    input[32 - sk_bytes.len()..32].copy_from_slice(&sk_bytes);
    input[32..64].copy_from_slice(&ffi::field_to_be(&b_q.x));
    input[64..96].copy_from_slice(&ffi::field_to_be(&b_q.y));
    input[96..128].copy_from_slice(&ffi::field_to_be(&ds_dlog));
    // Fixed "seed" for a reproducible equivalence check. A real committee-member host must
    // supply fresh CSPRNG bytes here every call (see ffi.rs's module doc) — reuse is only
    // valid inside this deterministic byte-for-byte comparison.
    input[128..160].copy_from_slice(&[0x42; 32]);
    input
}

/// Builds the real `wasm32-unknown-unknown` release artifact via a nested `cargo build`,
/// using a target-dir separate from the outer `cargo test` invocation's own (avoids any
/// lock contention, keeps this test a single self-contained `cargo test` rather than a
/// documented two-step build+test process) and returns the compiled module's bytes.
fn build_wasm_module() -> Vec<u8> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_dir = manifest_dir.join("target").join("wasm-equivalence-check");

    let status = Command::new(env!("CARGO"))
        .current_dir(&manifest_dir)
        .args(["build", "--release", "--lib", "--target", "wasm32-unknown-unknown"])
        .arg("--target-dir")
        .arg(&target_dir)
        .status()
        .expect(
            "failed to invoke `cargo build --target wasm32-unknown-unknown` — is the \
             wasm32-unknown-unknown target installed (`rustup target add wasm32-unknown-unknown`)?",
        );
    assert!(status.success(), "wasm32-unknown-unknown build of oprf-committee-dev failed");

    let wasm_path: &Path =
        &target_dir.join("wasm32-unknown-unknown").join("release").join("oprf_committee_dev.wasm");
    std::fs::read(wasm_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", wasm_path.display()))
}

#[test]
fn wasm_evaluate_query_matches_native_byte_for_byte() {
    let wasm_bytes = build_wasm_module();

    let engine = Engine::default();
    let module = Module::new(&engine, &wasm_bytes[..])
        .expect("compiled oprf_committee_dev.wasm must be a valid wasm module");
    let mut store = Store::new(&engine, ());
    let linker = Linker::<()>::new(&engine);
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .expect("oprf_committee_dev.wasm must instantiate with no host imports required");

    let memory = instance
        .get_memory(&store, "memory")
        .expect("wasm32-unknown-unknown cdylib must export its linear memory as \"memory\"");
    let alloc = instance
        .get_typed_func::<u32, u32>(&store, "oprf_alloc")
        .expect("module must export oprf_alloc(len: i32) -> i32");
    let evaluate = instance
        .get_typed_func::<(u32, u32, u32, u32), i32>(&store, "oprf_evaluate_query")
        .expect(
            "module must export oprf_evaluate_query(in_ptr, in_len, out_ptr, out_len) -> i32",
        );

    let input = sample_input();

    // Marshal the input into the module's own linear memory via its exported allocator —
    // the bare wasm32-unknown-unknown C-ABI pattern (no wasm-bindgen glue).
    let input_ptr =
        alloc.call(&mut store, input.len() as u32).expect("oprf_alloc(input_len) call failed");
    memory.write(&mut store, input_ptr as usize, &input).expect("write input into wasm memory");

    let output_ptr = alloc
        .call(&mut store, OUTPUT_LEN as u32)
        .expect("oprf_alloc(OUTPUT_LEN) call failed");

    let code = evaluate
        .call(&mut store, (input_ptr, input.len() as u32, output_ptr, OUTPUT_LEN as u32))
        .expect("oprf_evaluate_query call trapped");
    assert_eq!(code, 0, "wasm oprf_evaluate_query returned error code {code} on a valid fixture");

    let mut wasm_output = vec![0u8; OUTPUT_LEN];
    memory
        .read(&store, output_ptr as usize, &mut wasm_output)
        .expect("read output from wasm memory");

    // The native call this whole test exists to compare against — the exact same
    // `evaluate_query` function `ffi::tests` already validated, just invoked directly instead
    // of through a Wasm interpreter.
    let native_output = ffi::evaluate_query(&input).expect("native evaluate_query failed");

    assert_eq!(
        wasm_output, native_output,
        "wasm32-unknown-unknown build of oprf_committee_dev must produce byte-identical \
         output to the native build for the same input — any difference here would mean the \
         wasm build cannot inherit the native crate's KAT validation"
    );

    // Cross-check against oprf::evaluate called completely independently (not through
    // ffi::evaluate_query at all) with the same seed, so this test also transitively confirms
    // the wire-format encode/decode round-trips correctly, not just that wasm and native
    // agree with each other in some way that could both be wrong the same way.
    let ds_dlog = fe("1523098184080632582082867317389990410064981862");
    let client_input = fe("999999999999999999999999999");
    let beta = BigUint::from(4242424242424242u64);
    let b_q = oprf::blinded_query(&beta, client_input);
    let sk = BigUint::from(778899112233u64);
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&input[128..160]);
    let mut rng = <rand::rngs::StdRng as rand::SeedableRng>::from_seed(seed);
    let direct = oprf::evaluate(&mut rng, &sk, &b_q, ds_dlog);

    assert_eq!(&wasm_output[0..32], &ffi::field_to_be(&direct.pk.x));
    assert_eq!(&wasm_output[32..64], &ffi::field_to_be(&direct.pk.y));
    assert_eq!(&wasm_output[64..96], &ffi::field_to_be(&direct.dlog_e));
    assert_eq!(&wasm_output[96..128], &ffi::field_to_be(&direct.dlog_s));
    assert_eq!(&wasm_output[128..160], &ffi::field_to_be(&direct.response_blinded.x));
    assert_eq!(&wasm_output[160..192], &ffi::field_to_be(&direct.response_blinded.y));

    // And finally: the produced proof must actually verify, exactly as
    // `ffi::tests::evaluate_query_output_is_a_valid_dlog_proof` checks natively — confirming
    // the wasm-produced proof is not just byte-identical to a native computation but a real,
    // independently-verifiable Chaum-Pedersen proof.
    let pk = Point::new(ffi::field_from_be(&wasm_output[0..32]), ffi::field_from_be(&wasm_output[32..64]));
    let dlog_e = ffi::field_from_be(&wasm_output[64..96]);
    let dlog_s = ffi::field_from_be(&wasm_output[96..128]);
    let response_blinded = Point::new(
        ffi::field_from_be(&wasm_output[128..160]),
        ffi::field_from_be(&wasm_output[160..192]),
    );
    assert!(oprf_committee_dev::dlog::verify(dlog_e, dlog_s, &pk, &b_q, &response_blinded, ds_dlog));
}
