//! Builds and signs `pallet-identity`'s OPRF committee extrinsics by hand — no `subxt`, no
//! dependency on the `agora-runtime` crate (see Cargo.toml's dependency comment for why: that
//! crate pulls in the whole WASM-builder build graph and its `WASM_BUILD_RUSTFLAGS` quirk from
//! /CLAUDE.md, for a `RuntimeCall` variant that doesn't exist in it yet anyway).
//!
//! ## Which calls this file encodes
//!
//! The pallet was rewritten from a single-response design into a genuine two-round threshold
//! protocol, so there are now **two** calls a committee node submits per query:
//! `submit_oprf_round1` (call index 16) and `submit_oprf_round2` (call index 17). See
//! `build_signed_round1` / `build_signed_round2`, both called from `main.rs`'s
//! `try_round1`/`try_round2`. The old single-shot `submit_oprf_response`/`build_signed` (which
//! encoded a call index 16 no longer means) have been removed — that call does not exist on
//! this chain anymore, so keeping an encoder for it around was actively misleading, not just
//! unused.
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

/// Call index of `submit_oprf_round1` within pallet-identity.
///
/// This is the *same* index the removed `submit_oprf_response` used to occupy. Reusing it is
/// safe (and is what the real pallet does — `#[pallet::call_index(16)]` sits on
/// `submit_oprf_round1` today) because a call index only has to be unique *within the current
/// runtime*: nothing on-chain remembers the retired call, and the old call was never
/// successfully submitted from this node against a real chain, so there is no in-flight or
/// historical traffic that a reused 16 could be confused with. It is not a stable ABI number
/// across runtime upgrades, and this node re-reads it from config precisely because of that.
pub const ROUND1_CALL_INDEX: u8 = 16;

/// Call index of `submit_oprf_round2` within pallet-identity — a genuinely new index, never
/// used by any previous call.
pub const ROUND2_CALL_INDEX: u8 = 17;

/// Round 1 of the threshold protocol: this node's share commitment plus the Chaum-Pedersen
/// commitment pair proving it, all as uncompressed 64-byte (x‖y, big-endian) curve points —
/// the same point encoding `PendingOprfQueries.blinded_query` already uses.
///
/// Field names mirror `pallet_identity::Pallet::submit_oprf_round1`'s parameters verbatim
/// (`r_i`, `d_g`, `d_q`, `e_g`, `e_q`) rather than being renamed to something more descriptive,
/// deliberately: the encoding here is positional, so a reader checking this against the pallet
/// source should be able to line the two up name-for-name with no translation step.
pub struct SubmitOprfRound1 {
    pub query_id: u64,
    pub committee_slot: u8,
    pub r_i: [u8; 64],
    pub d_g: [u8; 64],
    pub d_q: [u8; 64],
    pub e_g: [u8; 64],
    pub e_q: [u8; 64],
}

/// Round 2: the single 32-byte scalar response `z_i` that closes out the Chaum-Pedersen proof
/// committed to in round 1. 32 bytes rather than 64 because this is a scalar, not a curve
/// point.
pub struct SubmitOprfRound2 {
    pub query_id: u64,
    pub committee_slot: u8,
    pub z_i: [u8; 32],
}

/// SCALE-encodes the `submit_oprf_round1` call body (no signature/envelope).
///
/// Split out from `build_signed_round1` so the argument encoding — the part most likely to
/// drift out of sync with the pallet — is unit-testable without a live chain to fetch
/// spec_version/nonce/genesis from. See `mod tests`.
fn encode_round1_call(pallet_index: u8, call_index: u8, call: &SubmitOprfRound1) -> Vec<u8> {
    // Argument order and types match `pallet_identity::Pallet::submit_oprf_round1` exactly
    // (pallets/pallet-identity/src/lib.rs, `#[pallet::call_index(16)]`).
    let mut call_bytes = Vec::new();
    call_bytes.push(pallet_index);
    call_bytes.push(call_index);
    call_bytes.extend(call.query_id.encode()); // u64: fixed 8-byte little-endian, not compact
    call_bytes.push(call.committee_slot); // u8: a single raw byte, nothing to encode
    call_bytes.extend(call.r_i); // fixed-size array: raw bytes, no length prefix
    call_bytes.extend(call.d_g); // fixed-size array: raw bytes, no length prefix
    call_bytes.extend(call.d_q); // fixed-size array: raw bytes, no length prefix
    call_bytes.extend(call.e_g); // fixed-size array: raw bytes, no length prefix
    call_bytes.extend(call.e_q); // fixed-size array: raw bytes, no length prefix
    call_bytes
}

/// SCALE-encodes the `submit_oprf_round2` call body (no signature/envelope). Same
/// testability rationale as `encode_round1_call`.
fn encode_round2_call(pallet_index: u8, call_index: u8, call: &SubmitOprfRound2) -> Vec<u8> {
    // Matches `pallet_identity::Pallet::submit_oprf_round2` (`#[pallet::call_index(17)]`).
    let mut call_bytes = Vec::new();
    call_bytes.push(pallet_index);
    call_bytes.push(call_index);
    call_bytes.extend(call.query_id.encode()); // u64: fixed 8-byte little-endian, not compact
    call_bytes.push(call.committee_slot); // u8: a single raw byte, nothing to encode
    call_bytes.extend(call.z_i); // fixed-size array: raw bytes, no length prefix
    call_bytes
}

/// Builds, signs, and hex-encodes a `submit_oprf_round1` extrinsic. Returns the "0x"-prefixed
/// hex string ready for `author_submitExtrinsic`.
///
/// `call_index` is a parameter rather than hardwired to [`ROUND1_CALL_INDEX`] to keep the
/// existing `CALL_INDEX` config override working (see `config.rs`) — pass
/// [`ROUND1_CALL_INDEX`] if you have no configured value.
///
/// **Never exercised against a real chain.** Like everything else in this file, the argument
/// encoding is verified only by the unit tests below (which assert the shape this file
/// intends, not the shape the chain actually decoded), and the transaction-extension
/// assumptions listed in the module docs still apply unchanged.
pub async fn build_signed_round1(
    rpc: &RpcClient,
    seed: &[u8; 32],
    pallet_index: u8,
    call_index: u8,
    call: SubmitOprfRound1,
) -> anyhow::Result<String> {
    let call_bytes = encode_round1_call(pallet_index, call_index, &call);
    sign_and_frame(rpc, seed, call_bytes).await
}

/// Builds, signs, and hex-encodes a `submit_oprf_round2` extrinsic. Same caveats and same
/// `call_index` convention as [`build_signed_round1`]; pass [`ROUND2_CALL_INDEX`] unless a
/// config override says otherwise.
pub async fn build_signed_round2(
    rpc: &RpcClient,
    seed: &[u8; 32],
    pallet_index: u8,
    call_index: u8,
    call: SubmitOprfRound2,
) -> anyhow::Result<String> {
    let call_bytes = encode_round2_call(pallet_index, call_index, &call);
    sign_and_frame(rpc, seed, call_bytes).await
}

/// Wraps an already-SCALE-encoded call body in the signed-extrinsic envelope: fetches the
/// signer's nonce and the chain's spec/tx/genesis values, assembles `extra` +
/// `additional_signed` exactly as the runtime's `TxExtension` tuple expects (see module docs
/// for which parts of that are confirmed and which are best-effort), signs, and length-prefixes
/// the result.
///
/// Every call in this file shares this envelope verbatim — only the call body differs — so it
/// lives here once rather than being copy-pasted per call, which is how the pre-rewrite version
/// of this file would have drifted as calls were added.
async fn sign_and_frame(
    rpc: &RpcClient,
    seed: &[u8; 32],
    call_bytes: Vec<u8>,
) -> anyhow::Result<String> {
    let pair = sr25519::Pair::from_seed(seed);
    let account_id = AccountId32::from(pair.public().to_raw());

    let runtime_version = rpc.get_runtime_version().await?;
    let genesis_hash = rpc.get_block_hash(0).await?;
    let nonce = rpc.next_account_index(&account_id.to_ss58check()).await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// `pallet-identity`'s runtime index (runtime/src/lib.rs, `#[runtime::pallet_index(8)]`).
    const PALLET_IDENTITY: u8 = 8;

    /// Distinctive byte-fill per field so a transposed pair of 64-byte arguments (the most
    /// plausible encoding bug here, since five of them are the same type in a row) fails the
    /// assertion instead of silently matching.
    fn round1_fixture() -> SubmitOprfRound1 {
        SubmitOprfRound1 {
            query_id: 0x0102_0304_0506_0708,
            committee_slot: 3,
            r_i: [0xa1; 64],
            d_g: [0xb2; 64],
            d_q: [0xc3; 64],
            e_g: [0xd4; 64],
            e_q: [0xe5; 64],
        }
    }

    #[test]
    fn round1_call_encoding_matches_expected_bytes() {
        let encoded = encode_round1_call(PALLET_IDENTITY, ROUND1_CALL_INDEX, &round1_fixture());

        let mut expected = vec![PALLET_IDENTITY, 16];
        // u64 encodes as 8 fixed little-endian bytes (plain `Encode`, not `Compact`).
        expected.extend([0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
        expected.push(3); // committee_slot: one raw byte
        expected.extend([0xa1; 64]); // r_i
        expected.extend([0xb2; 64]); // d_g
        expected.extend([0xc3; 64]); // d_q
        expected.extend([0xd4; 64]); // e_g
        expected.extend([0xe5; 64]); // e_q

        assert_eq!(encoded, expected);
    }

    /// Guards the fixed-size-array rule specifically: 2 + 8 + 1 + 5*64 = 331. Any stray
    /// compact length prefix in front of one of the arrays would add a byte and fail here.
    #[test]
    fn round1_call_encoding_has_no_array_length_prefixes() {
        let encoded = encode_round1_call(PALLET_IDENTITY, ROUND1_CALL_INDEX, &round1_fixture());
        assert_eq!(encoded.len(), 2 + 8 + 1 + 5 * 64);
    }

    #[test]
    fn round2_call_encoding_matches_expected_bytes() {
        let call = SubmitOprfRound2 {
            query_id: 0x0102_0304_0506_0708,
            committee_slot: 3,
            z_i: [0x7f; 32],
        };
        let encoded = encode_round2_call(PALLET_IDENTITY, ROUND2_CALL_INDEX, &call);

        let mut expected = vec![PALLET_IDENTITY, 17];
        expected.extend([0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
        expected.push(3);
        expected.extend([0x7f; 32]); // z_i: 32-byte scalar, raw, no length prefix

        assert_eq!(encoded, expected);
        assert_eq!(encoded.len(), 2 + 8 + 1 + 32);
    }

    /// `query_id` must be the full 8-byte little-endian `u64`, NOT a SCALE compact integer —
    /// they coincide for small values, so a compact-vs-fixed mistake would pass every test
    /// using a small `query_id` and then break on the 64th real query. This pins both ends.
    #[test]
    fn query_id_is_fixed_width_not_compact() {
        let small = SubmitOprfRound2 { query_id: 1, committee_slot: 0, z_i: [0; 32] };
        let encoded = encode_round2_call(PALLET_IDENTITY, ROUND2_CALL_INDEX, &small);
        assert_eq!(&encoded[2..10], &[1, 0, 0, 0, 0, 0, 0, 0]);

        let big = SubmitOprfRound2 { query_id: u64::MAX, committee_slot: 0, z_i: [0; 32] };
        let encoded = encode_round2_call(PALLET_IDENTITY, ROUND2_CALL_INDEX, &big);
        assert_eq!(&encoded[2..10], &[0xff; 8]);
        assert_eq!(encoded.len(), 2 + 8 + 1 + 32); // width did not change with the value
    }

    /// The two calls must not collide: round1 reuses the retired `submit_oprf_response`'s 16,
    /// round2 takes the new 17.
    #[test]
    fn call_indices_are_16_and_17() {
        assert_eq!(ROUND1_CALL_INDEX, 16);
        assert_eq!(ROUND2_CALL_INDEX, 17);
        assert_ne!(ROUND1_CALL_INDEX, ROUND2_CALL_INDEX);
    }
}
