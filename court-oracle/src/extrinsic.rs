//! Builds and signs the `submit_ai_ruling` extrinsic by hand — no `subxt`, no dependency on the
//! `agora-runtime` crate (see Cargo.toml's `[workspace]` comment for why: that crate pulls in
//! the whole WASM-builder build graph and its `WASM_BUILD_RUSTFLAGS` quirk from /CLAUDE.md, for
//! a call this service can encode directly with far less build-graph coupling).
//!
//! This hand-encodes the same wire format `sp_runtime::generic::UncheckedExtrinsic` produces,
//! using `sp-core` (pinned to the exact version — 36.1.0 — the runtime itself uses) purely for
//! the sr25519 signing primitive and hashing, which are genuinely security-sensitive and not
//! worth re-deriving. The SCALE layout below is transcribed from `runtime/src/lib.rs`'s
//! `TxExtension` tuple, current as of this writing (confirmed by reading that file, not
//! assumed):
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
//! **Solid** (fetched live from the chain, standard/stable wire format, or read directly from
//! `pallets/pallet-courts/src/lib.rs`): spec_version, transaction_version, genesis hash,
//! account nonce, the sr25519 signing scheme itself, and
//! `submit_ai_ruling(case_id: u32, ruling_hash: [u8; 32], model_version: u32)`'s exact
//! argument shape and call index (1) — read directly, not guessed.
//!
//! `model_version` must match `pallet_courts::CurrentAIModelVersion` exactly (a plain
//! `StorageValue<u32>`, governance-set via `vote_approve_ai_model`) or `submit_ai_ruling`
//! rejects the call with `UnapprovedAIModel`/`NoApprovedAIModel` — `main.rs` fetches this
//! fresh from chain immediately before building each extrinsic, never caches or guesses it.
//!
//! **Best-effort / needs verification against a live chain**:
//! - `CheckMetadataHash`'s `Mode` — assumed `Disabled` (no `--enable-metadata-hash`-style flag
//!   appears in /CLAUDE.md's build/run instructions, so this is a reasonable default, not a
//!   confirmed one).
//! - `WeightReclaim`'s extra/additional-signed shape — a newer transaction extension; this file
//!   assumes it contributes zero bytes to both `extra` and `additional_signed`. If extrinsics
//!   built here are rejected with a "bad signature" or "invalid transaction" error, this is the
//!   first place to check against the exact `frame-system` version in use.
//!
//! Uses `Era::Immortal` throughout (no mortality window) for simplicity — a mortal era needs a
//! recent checkpoint block whose hash can go stale between construction and submission, which
//! adds retry-logic complexity out of scope for a service this size. The tradeoff: an
//! `Immortal` extrinsic remains validly-signed and replayable indefinitely if nonce reuse were
//! ever possible, which it isn't here as long as this is the only signer for the oracle
//! account.

use codec::{Compact, Encode};
use sp_core::crypto::{AccountId32, Pair as _, Ss58Codec};
use sp_core::{blake2_256, sr25519};

use crate::cases::Verdict;
use crate::rpc::RpcClient;

/// Transaction format version 4, signed bit set (`0x84`).
const EXTRINSIC_VERSION_SIGNED: u8 = 0x84;
/// `MultiAddress::Id` variant index.
const ADDRESS_VARIANT_ID: u8 = 0x00;
/// `MultiSignature::Sr25519` variant index (`Ed25519 = 0, Sr25519 = 1, Ecdsa = 2`).
const SIGNATURE_VARIANT_SR25519: u8 = 0x01;
/// `Era::Immortal` — single zero byte.
const ERA_IMMORTAL: u8 = 0x00;
/// `frame_metadata_hash_extension::Mode::Disabled` — see module docs, "best-effort" list.
const METADATA_HASH_MODE_DISABLED: u8 = 0x00;

/// Arguments to `pallet-courts::submit_ai_ruling(case_id, ruling_hash, model_version)`.
pub struct SubmitAiRuling {
    pub case_id: u32,
    pub ruling_hash: [u8; 32],
    pub model_version: u32,
}

/// Arguments to `pallet-courts::finalize_ruling(case_id, verdict)` — the no-appeal-path call
/// that applies an AI ruling's verdict once the appeal window has closed unappealed (see
/// `Cases`/`AIRulingBlock` in `pallets/pallet-courts/src/lib.rs`, `#[pallet::call_index(4)]`).
/// Gated by the same `T::OracleOrigin` as `submit_ai_ruling` — see this pallet's
/// `EnsureOracle`, which checks the signer against `OracleAccount` storage — so this crate's
/// existing oracle signing key (already used for `submit_ai_ruling`) is also the correct signer
/// here; no separate key or origin is needed.
pub struct FinalizeRuling {
    pub case_id: u32,
    pub verdict: Verdict,
}

/// Encodes just the `[pallet_index, call_index] ++ arguments` portion of a call — the part
/// specific to which extrinsic is being built. Implemented per call type so `build_signed` can
/// stay call-agnostic (envelope construction, signing, and submission are identical regardless
/// of which pallet-courts call is inside).
pub trait CourtsCall {
    fn encode_call_bytes(&self, pallet_index: u8, call_index: u8) -> Vec<u8>;
}

impl CourtsCall for SubmitAiRuling {
    fn encode_call_bytes(&self, pallet_index: u8, call_index: u8) -> Vec<u8> {
        // [pallet_index, call_index] ++ case_id ++ ruling_hash ++ model_version
        let mut call_bytes = Vec::new();
        call_bytes.push(pallet_index);
        call_bytes.push(call_index);
        call_bytes.extend(self.case_id.encode());
        call_bytes.extend(self.ruling_hash); // fixed-size array: raw bytes, no length prefix
        call_bytes.extend(self.model_version.encode());
        call_bytes
    }
}

impl CourtsCall for FinalizeRuling {
    fn encode_call_bytes(&self, pallet_index: u8, call_index: u8) -> Vec<u8> {
        // [pallet_index, call_index] ++ case_id ++ verdict (single-byte enum discriminant)
        let mut call_bytes = Vec::new();
        call_bytes.push(pallet_index);
        call_bytes.push(call_index);
        call_bytes.extend(self.case_id.encode());
        call_bytes.extend(self.verdict.encode());
        call_bytes
    }
}

/// Builds, signs, and hex-encodes the extrinsic. Returns the "0x"-prefixed hex string ready for
/// `author_submitExtrinsic`. Generic over `CourtsCall` so both `submit_ai_ruling` and
/// `finalize_ruling` share this one envelope/signing implementation — the two calls differ only
/// in their call-bytes encoding.
pub async fn build_signed<C: CourtsCall>(
    rpc: &RpcClient,
    seed: &[u8; 32],
    pallet_index: u8,
    call_index: u8,
    call: C,
) -> anyhow::Result<String> {
    let pair = sr25519::Pair::from_seed(seed);
    let account_id = AccountId32::from(pair.public().to_raw());

    let runtime_version = rpc.get_runtime_version().await?;
    let genesis_hash = rpc.get_block_hash(0).await?;
    let nonce = rpc.next_account_index(&account_id.to_ss58check()).await?;

    let call_bytes = call.encode_call_bytes(pallet_index, call_index);

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

    /// Pure-logic check on the call-byte encoding alone (no RPC): confirms the argument
    /// layout matches what `pallet-courts::submit_ai_ruling(case_id: u32, ruling_hash: [u8;
    /// 32], model_version: u32)` expects to decode — pallet_index, call_index, then case_id as
    /// a plain (non-compact) little-endian u32, then the 32 raw hash bytes with no length
    /// prefix (fixed-size array), then model_version as a plain little-endian u32. Exercises the
    /// real `CourtsCall::encode_call_bytes` implementation, not a hand-duplicated copy.
    #[test]
    fn call_bytes_layout_matches_submit_ai_ruling_signature() {
        let call = SubmitAiRuling { case_id: 42, ruling_hash: [7u8; 32], model_version: 3 };
        let call_bytes = call.encode_call_bytes(11, 1);

        assert_eq!(call_bytes.len(), 2 + 4 + 32 + 4);
        assert_eq!(call_bytes[0], 11);
        assert_eq!(call_bytes[1], 1);
        assert_eq!(&call_bytes[2..6], &42u32.to_le_bytes());
        assert_eq!(&call_bytes[6..38], &[7u8; 32]);
        assert_eq!(&call_bytes[38..42], &3u32.to_le_bytes());
    }

    /// Same idea for `pallet-courts::finalize_ruling(case_id: u32, verdict: Verdict)` —
    /// pallet_index, call_index, case_id as a plain little-endian u32, then the verdict as a
    /// single-byte enum discriminant (Upheld = 0, Overturned = 1 — see `cases::Verdict`).
    #[test]
    fn call_bytes_layout_matches_finalize_ruling_signature() {
        let call = FinalizeRuling { case_id: 42, verdict: Verdict::Overturned };
        let call_bytes = call.encode_call_bytes(11, 4);

        assert_eq!(call_bytes.len(), 2 + 4 + 1);
        assert_eq!(call_bytes[0], 11);
        assert_eq!(call_bytes[1], 4);
        assert_eq!(&call_bytes[2..6], &42u32.to_le_bytes());
        assert_eq!(call_bytes[6], 1); // Overturned's discriminant

        let call = FinalizeRuling { case_id: 7, verdict: Verdict::Upheld };
        let call_bytes = call.encode_call_bytes(11, 4);
        assert_eq!(call_bytes[6], 0); // Upheld's discriminant
    }
}
