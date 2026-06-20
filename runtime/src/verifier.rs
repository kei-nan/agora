//! Rarimo Groth16 BN254 ZK passport verifier.
//!
//! Only compiled when the `dev-mode` feature is absent. In dev-mode,
//! `configs/mod.rs` uses `PassthroughZkVerifier` instead.
//!
//! # Proof byte format (129 bytes)
//! ```text
//! [  0.. 32]  A   G1 compressed  (ark-serialize LE, flags in byte 31)
//! [ 32.. 96]  B   G2 compressed  (ark-serialize LE, flags in byte 63)
//! [ 96..128]  C   G1 compressed
//! [128]       variant  0 = SHA-256 passport circuit, 1 = SHA-1 circuit
//! ```
//!
//! Convert the Rarimo snarkjs JSON VKs to binary with `scripts/convert_vk.py`.
//! Download from <https://github.com/rarimo/passport-zk-circuits>.
//!
//! # Public inputs (10 signals, each [u8; 32] big-endian Fr)
//! Index 0: nullifier = Poseidon3(sk_identity, Poseidon1(sk_identity), eventID)
//! Index 1: birthDate  (UTF-8 hex YYMMDD encoded as decimal)
//! Index 2: expirationDate
//! Indices 3-9: name/nationality/citizenship/documentNumber/eventID per circuit version

#![cfg(not(feature = "dev-mode"))]

extern crate alloc;

use alloc::vec::Vec;
use ark_bn254::{Bn254, Fr};
use ark_ff::PrimeField;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_serialize::CanonicalDeserialize;

/// Serialized `VerifyingKey<Bn254>` (ark-serialize compressed) for the SHA-256 circuit.
///
/// Populate with:
/// ```sh
/// python3 scripts/convert_vk.py sha256_verification_key.json runtime/assets/vk_sha256.bin
/// ```
const VK_SHA256: &[u8] = include_bytes!("../assets/vk_sha256.bin");

/// Same for the SHA-1 circuit (older passports).
const VK_SHA1: &[u8] = include_bytes!("../assets/vk_sha1.bin");

/// Implements `ZkProofVerifier` using the real Rarimo Groth16 circuit.
pub struct RarimoGroth16Verifier;

impl pallet_identity_zk::ZkProofVerifier for RarimoGroth16Verifier {
    fn verify(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> bool {
        verify_inner(proof_bytes, public_inputs).unwrap_or(false)
    }
}

fn verify_inner(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> Option<bool> {
    // Proof must be exactly 129 bytes: 128 serialized + 1 variant tag.
    if proof_bytes.len() != 129 {
        return None;
    }

    let vk_bytes: &[u8] = match proof_bytes[128] {
        0 => VK_SHA256,
        1 => VK_SHA1,
        _ => return None,
    };

    // Empty file = VK not installed yet. Reject all proofs rather than panic.
    if vk_bytes.is_empty() {
        return Some(false);
    }

    // Deserialize the verifying key (with on-curve validation).
    let vk = VerifyingKey::<Bn254>::deserialize_compressed(vk_bytes).ok()?;

    // Process into miller-loop form (computes precomputed pairing terms).
    let pvk = Groth16::<Bn254>::process_vk(&vk).ok()?;

    // Deserialize the proof (A, B, C).
    let proof = Proof::<Bn254>::deserialize_compressed(&proof_bytes[..128]).ok()?;

    // Each public input is a big-endian 32-byte Fr element (reduces mod r automatically).
    let pub_signals: Vec<Fr> = public_inputs
        .iter()
        .map(|b| Fr::from_be_bytes_mod_order(b.as_ref()))
        .collect();

    // Run the Groth16 pairing check.
    Groth16::<Bn254>::verify_with_processed_vk(&pvk, &pub_signals, &proof).ok()
}
