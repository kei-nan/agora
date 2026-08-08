//! Builds and signs the `submit_oprf_response` extrinsic by hand — no `subxt`, no dependency on
//! the `agora-runtime` crate (see Cargo.toml's dependency comment for why: that crate pulls in
//! the whole WASM-builder build graph and its `WASM_BUILD_RUSTFLAGS` quirk from /CLAUDE.md, for
//! a `RuntimeCall` variant that doesn't exist in it yet anyway).
//!
//! This hand-encodes the same wire format `sp_runtime::generic::UncheckedExtrinsic` produces,
//! using `sp-core` (pinned to the exact version — 36.1.0 — the runtime itself uses) purely for
//! the sr25519 signing primitive and hashing, which are genuinely security-sensitive and not
//! worth re-deriving. The SCALE layout below is transcribed from `runtime/src/lib.rs`'s
//! `TxExtension` tuple, current as of this writing:
//!
//! ```text
//! pub type TxExtension = (
//!     frame_system::CheckNonZeroSender<Runtime>,
//!     frame_system::CheckSpecVersion<Runtime>,
//!     frame_system::CheckTxVersion<Runtime>,
//!     frame_system::CheckGenesis<Runtime>,
//!     frame_system::CheckEra<Runtime>,
//!     frame_system::CheckNonce<Runtime>,
//!     frame_system::CheckWeight<Runtime>,
//!     pallet_transaction_payment::ChargeTransactionPayment<Runtime>,
//!     frame_metadata_hash_extension::CheckMetadataHash<Runtime>,
//!     frame_system::WeightReclaim<Runtime>,
//! );
//! ```
//!
//! ## What is solid vs. best-effort here
//!
//! **Solid** (fetched live from the chain, standard/stable wire format): spec_version,
//! transaction_version, genesis hash, account nonce, the sr25519 signing scheme itself
//! (identical derivation/signing as `sp_core::sr25519::Pair`, which is what the real chain's
//! signature verification expects).
//!
//! **Best-effort / needs verification against a live chain once `submit_oprf_response` exists**:
//! - `CheckMetadataHash`'s `Mode` — assumed `Disabled` (no `--enable-metadata-hash`-style flag
//!   appears in /CLAUDE.md's build/run instructions, so this is a reasonable default, not a
//!   confirmed one).
//! - `WeightReclaim`'s extra/additional-signed shape — this is a newer transaction extension;
//!   this file assumes it contributes zero bytes to both `extra` and `additional_signed`. If
//!   extrinsics built here are rejected with a "bad signature" or "invalid transaction" error,
//!   this is the first place to check against the exact `frame-system` version in use.
//! - **PROVISIONAL**: `pallet_index`/`call_index`/the exact argument encoding of
//!   `submit_oprf_response` itself — see `config.rs` doc comments. Once the real pallet call
//!   exists, prefer generating this envelope with `subxt`'s dynamic API (which reads live
//!   chain metadata instead of hand-encoding) over maintaining this file further.
//!
//! Uses `Era::Immortal` throughout (no mortality window) for simplicity — a mortal era needs a
//! recent checkpoint block whose hash can go stale between construction and submission, which
//! adds retry-logic complexity out of scope for this skeleton. The tradeoff: an
//! `Immortal` extrinsic remains validly-signed and replayable indefinitely if nonce reuse were
//! ever possible, which it isn't here as long as this is the only signer for the account.

use codec::{Compact, Encode};
use sp_core::crypto::{AccountId32, Pair as _, Ss58Codec};
use sp_core::{blake2_256, sr25519};

use crate::rpc::RpcClient;

/// Transaction format version 4, signed bit set (`0x80 | 0x04`).
const EXTRINSIC_VERSION_SIGNED: u8 = 0x84;
/// `MultiAddress::Id` variant index.
const ADDRESS_VARIANT_ID: u8 = 0x00;
/// `MultiSignature::Sr25519` variant index (`Ed25519 = 0, Sr25519 = 1, Ecdsa = 2`).
const SIGNATURE_VARIANT_SR25519: u8 = 0x01;
/// `Era::Immortal` — single zero byte.
const ERA_IMMORTAL: u8 = 0x00;
/// `frame_metadata_hash_extension::Mode::Disabled` — see module docs, "best-effort" list.
const METADATA_HASH_MODE_DISABLED: u8 = 0x00;

pub struct SubmitOprfResponse {
    pub query_id: u64,
    pub committee_slot: u8,
    pub evaluation: [u8; 64],
    pub dlog_proof: Vec<u8>,
}

/// Builds, signs, and hex-encodes the extrinsic. Returns the "0x"-prefixed hex string ready for
/// `author_submitExtrinsic`.
pub async fn build_signed(
    rpc: &RpcClient,
    seed: &[u8; 32],
    pallet_index: u8,
    call_index: u8,
    call: SubmitOprfResponse,
) -> anyhow::Result<String> {
    let pair = sr25519::Pair::from_seed(seed);
    let account_id = AccountId32::from(pair.public().to_raw());

    let runtime_version = rpc.get_runtime_version().await?;
    let genesis_hash = rpc.get_block_hash(0).await?;
    let nonce = rpc.next_account_index(&account_id.to_ss58check()).await?;

    // --- call: [pallet_index, call_index] ++ query_id ++ committee_slot ++ evaluation ++ dlog_proof
    let mut call_bytes = Vec::new();
    call_bytes.push(pallet_index);
    call_bytes.push(call_index);
    call_bytes.extend(call.query_id.encode());
    call_bytes.push(call.committee_slot);
    call_bytes.extend(call.evaluation); // fixed-size array: raw bytes, no length prefix
    call_bytes.extend(call.dlog_proof.encode()); // Vec<u8>: compact length prefix + raw bytes

    // --- extra (the bytes physically present in the extrinsic body)
    let mut extra_bytes = Vec::new();
    extra_bytes.push(ERA_IMMORTAL); // CheckEra
    extra_bytes.extend(Compact(nonce).encode()); // CheckNonce
    extra_bytes.extend(Compact(0u128).encode()); // ChargeTransactionPayment (tip = 0)
    extra_bytes.push(METADATA_HASH_MODE_DISABLED); // CheckMetadataHash
    // CheckNonZeroSender / CheckSpecVersion / CheckTxVersion / CheckGenesis / CheckWeight /
    // WeightReclaim all contribute zero bytes to `extra`.

    // --- additional_signed (hashed/signed over, but NOT physically included in the extrinsic)
    let mut additional_signed = Vec::new();
    additional_signed.extend(runtime_version.spec_version.encode()); // CheckSpecVersion
    additional_signed.extend(runtime_version.transaction_version.encode()); // CheckTxVersion
    additional_signed.extend(genesis_hash); // CheckGenesis
    additional_signed.extend(genesis_hash); // CheckEra (Immortal checkpoint == genesis)
    additional_signed.extend(Option::<[u8; 32]>::None.encode()); // CheckMetadataHash
    // CheckNonZeroSender / CheckNonce / CheckWeight / ChargeTransactionPayment / WeightReclaim
    // all contribute zero bytes to `additional_signed`.

    let mut signable = Vec::new();
    signable.extend(&call_bytes);
    signable.extend(&extra_bytes);
    signable.extend(&additional_signed);

    // Substrate's `SignedPayload::using_encoded`: sign the raw payload unless it's over 256
    // bytes, in which case sign its blake2_256 hash instead.
    let signature = if signable.len() > 256 {
        pair.sign(&blake2_256(&signable))
    } else {
        pair.sign(&signable)
    };

    let mut body = Vec::new();
    body.push(EXTRINSIC_VERSION_SIGNED);
    body.push(ADDRESS_VARIANT_ID);
    body.extend(AsRef::<[u8; 32]>::as_ref(&account_id));
    body.push(SIGNATURE_VARIANT_SR25519);
    body.extend(signature.to_raw());
    body.extend(&extra_bytes);
    body.extend(&call_bytes);

    // The UncheckedExtrinsic wire format itself is length-prefixed.
    let mut framed = Compact(body.len() as u32).encode();
    framed.extend(body);

    Ok(format!("0x{}", hex::encode(framed)))
}
