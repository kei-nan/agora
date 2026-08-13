//! Raw JSON-RPC chain client.
//!
//! Deliberately mirrors `desktop/src-tauri/src/rpc.rs` byte-for-byte in spirit: this project's
//! existing convention for Rust chain connectivity is a small hand-rolled JSON-RPC client
//! (`reqwest` + `state_getStorage`/`state_getKeysPaged`/`state_queryStorageAt`/manual SCALE
//! decoding), not `@polkadot/api` (that's JS-only) and not `subxt` (a heavier, code-generated
//! client the project has not adopted anywhere else). See the desktop app's `rpc.rs` module
//! docs and `commands/chain.rs` for the precedent this file follows.
//!
//! Additions over the desktop version: `get_runtime_version`, `get_block_hash`, and
//! `next_account_index` (read side needed to build a signed extrinsic), and
//! `submit_extrinsic` (the one write path the desktop app has never needed, since it is
//! read-only — see /CLAUDE.md's "AI Agent is read-only on-chain").
//!
//! One deliberate DIVERGENCE from the desktop copy: `twox128_hex` below is NOT copied
//! verbatim — testing it against a known-answer vector while building this component
//! surfaced a bug in `desktop/src-tauri/src/rpc.rs`'s version (its `r0.to_le()` call is a
//! no-op for `{:016x}` formatting purposes on a little-endian host, so it does not actually
//! byte-reverse anything). See `twox128_hex`'s own doc comment for the full explanation and
//! the corrected implementation used here. Worth fixing upstream in the desktop app too — not
//! done here since that file is outside this task's scope.

use anyhow::Context;
use codec::{Decode, Encode};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct RpcClient {
    url: String,
    client: Client,
    id: AtomicU64,
}

#[derive(Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'a str,
    method: &'a str,
    params: Value,
    id: u64,
}

#[derive(Deserialize)]
struct RpcResponse {
    result: Option<Value>,
    error: Option<Value>,
}

/// Live values needed to build a signed extrinsic (`extrinsic.rs`), all fetched fresh per
/// submission rather than cached — this node submits at most a handful of extrinsics per poll
/// cycle, so there is no meaningful cost to always reading current chain state instead of
/// risking a stale nonce/spec-version.
pub struct RuntimeVersion {
    pub spec_version: u32,
    pub transaction_version: u32,
}

/// One committee member's round-1 broadcast, as stored on-chain in
/// `OprfRound1Commitments: StorageDoubleMap<_, Blake2_128Concat, u64, Blake2_128Concat, u8,
/// BoundedVec<OprfRound1Commitment<T::AccountId>, T::OprfThreshold>, ValueQuery>` — see
/// `pallets/pallet-identity/src/lib.rs`.
///
/// Mirrors the pallet struct field-for-field, in declaration order, because that is exactly
/// what SCALE encodes: a struct is the concatenation of its fields, positionally, with no
/// field names on the wire. If the pallet ever reorders or inserts a field this still
/// compiles and still "decodes" — into garbage. Keep the two in sync by hand.
///
/// `member` is `T::AccountId` = `AccountId32` on this runtime: 32 raw bytes, no length
/// prefix. The five point fields are BabyJubJub points in the 64-byte x-then-y-big-endian
/// layout `OprfQueryRecord::blinded_query` already uses; this client does no point
/// validation, it only moves bytes.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct OprfRound1Commitment {
    pub member: sp_core::crypto::AccountId32,
    pub r_i: [u8; 64],
    pub d_g: [u8; 64],
    pub d_q: [u8; 64],
    pub e_g: [u8; 64],
    pub e_q: [u8; 64],
}

/// One committee member's round-2 response, as stored on-chain in `OprfRound2Responses`
/// (same double-map keying and `ValueQuery` semantics as [`OprfRound1Commitment`]'s map).
/// `z_i` is a single BN254 scalar field element — the output of
/// `oprf-committee-dev/src/threshold.rs::round2_response`. Same field-order caveat as
/// [`OprfRound1Commitment`] applies.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct OprfRound2Response {
    pub member: sp_core::crypto::AccountId32,
    pub z_i: [u8; 32],
}

impl RpcClient {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into(), client: Client::new(), id: AtomicU64::new(1) }
    }

    pub async fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.id.fetch_add(1, Ordering::Relaxed);
        let body = RpcRequest { jsonrpc: "2.0", method, params, id };
        let resp: RpcResponse = self
            .client
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("RPC transport error calling {method}"))?
            .json()
            .await
            .with_context(|| format!("RPC response was not valid JSON for {method}"))?;
        if let Some(err) = resp.error {
            anyhow::bail!("RPC error calling {method}: {err}");
        }
        resp.result.ok_or_else(|| anyhow::anyhow!("no result from {method}"))
    }

    /// Fetch all storage keys that start with prefix_hex (must include "0x").
    pub async fn get_keys_paged(&self, prefix_hex: &str) -> anyhow::Result<Vec<String>> {
        let mut all_keys: Vec<String> = Vec::new();
        let mut start_key = String::new();
        loop {
            let params = Value::Array(vec![
                Value::String(prefix_hex.to_string()),
                Value::Number(1000.into()),
                Value::String(start_key.clone()),
                Value::Null,
            ]);
            let result = self.call("state_getKeysPaged", params).await?;
            let batch: Vec<String> = serde_json::from_value(result)?;
            let done = batch.len() < 1000;
            if let Some(last) = batch.last().cloned() {
                start_key = last;
            }
            all_keys.extend(batch);
            if done {
                break;
            }
        }
        Ok(all_keys)
    }

    /// Fetch a single storage value by its full hex key. Returns None if the key is absent.
    pub async fn get_storage(&self, key_hex: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let params = Value::Array(vec![Value::String(key_hex.to_string()), Value::Null]);
        let result = self.call("state_getStorage", params).await;
        match result {
            Ok(v) => {
                if v.is_null() {
                    Ok(None)
                } else {
                    let hex_str = v.as_str().unwrap_or("").trim_start_matches("0x").to_string();
                    Ok(Some(hex::decode(hex_str)?))
                }
            }
            Err(_) => Ok(None),
        }
    }

    /// Fetch values for a list of storage keys, all at the current best block.
    pub async fn query_storage_at(&self, keys: &[String]) -> anyhow::Result<Vec<Option<String>>> {
        if keys.is_empty() {
            return Ok(vec![]);
        }
        let key_arr = Value::Array(keys.iter().map(|k| Value::String(k.clone())).collect());
        let params = Value::Array(vec![key_arr, Value::Null]);
        let result = self.call("state_queryStorageAt", params).await?;
        let changes_arr: Vec<Value> = serde_json::from_value(result).unwrap_or_default();
        let changes = changes_arr
            .first()
            .and_then(|r| r["changes"].as_array())
            .cloned()
            .unwrap_or_default();
        let values = keys
            .iter()
            .map(|key| {
                changes
                    .iter()
                    .find(|c| c.as_array().and_then(|p| p[0].as_str()) == Some(key.as_str()))
                    .and_then(|c| c.as_array())
                    .and_then(|p| p[1].as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        Ok(values)
    }

    /// `chain_getBlockHash` for a given block number. `Some(0)` gets the genesis hash — needed
    /// both for `CheckGenesis` and (since this node signs `Era::Immortal` transactions, see
    /// `extrinsic.rs`) for `CheckEra`'s checkpoint hash too.
    pub async fn get_block_hash(&self, number: u64) -> anyhow::Result<[u8; 32]> {
        let params = Value::Array(vec![Value::String(format!("0x{number:x}"))]);
        let result = self.call("chain_getBlockHash", params).await?;
        let hex_str = result.as_str().context("chain_getBlockHash did not return a string")?;
        let bytes = hex::decode(hex_str.trim_start_matches("0x"))?;
        bytes.try_into().map_err(|_| anyhow::anyhow!("block hash was not 32 bytes"))
    }

    /// `state_getRuntimeVersion` — spec/transaction version, needed by `CheckSpecVersion` /
    /// `CheckTxVersion`. Fetched live (not hardcoded from runtime/src/lib.rs's `VERSION`
    /// constant) so this node keeps working across a runtime upgrade without a rebuild.
    pub async fn get_runtime_version(&self) -> anyhow::Result<RuntimeVersion> {
        let result = self.call("state_getRuntimeVersion", Value::Array(vec![])).await?;
        let spec_version = result["specVersion"]
            .as_u64()
            .context("state_getRuntimeVersion missing specVersion")? as u32;
        let transaction_version = result["transactionVersion"]
            .as_u64()
            .context("state_getRuntimeVersion missing transactionVersion")?
            as u32;
        Ok(RuntimeVersion { spec_version, transaction_version })
    }

    /// `system_accountNextIndex` — the standard convenience RPC for "what nonce should my next
    /// transaction use", accounting for transactions already in the pool. Simpler and less
    /// error-prone than manually decoding `System::Account`'s SCALE-encoded `AccountInfo`.
    pub async fn next_account_index(&self, ss58_or_hex_account: &str) -> anyhow::Result<u32> {
        let params = Value::Array(vec![Value::String(ss58_or_hex_account.to_string())]);
        let result = self.call("system_accountNextIndex", params).await?;
        result.as_u64().map(|n| n as u32).context("system_accountNextIndex did not return a number")
    }

    /// Submits a fully-encoded, signed extrinsic (hex, "0x"-prefixed) via `author_submitExtrinsic`.
    /// Returns the extrinsic hash the node assigned it.
    pub async fn submit_extrinsic(&self, extrinsic_hex: &str) -> anyhow::Result<String> {
        let params = Value::Array(vec![Value::String(extrinsic_hex.to_string())]);
        let result = self.call("author_submitExtrinsic", params).await?;
        result.as_str().map(|s| s.to_string()).context("author_submitExtrinsic did not return a hash")
    }

    /// Reads the full round-1 qualifying set for one `(query_id, committee_slot)` pair from
    /// `Identity::OprfRound1Commitments`.
    ///
    /// Returns an empty `Vec` — never an error, never an `Option` — when nothing has been
    /// submitted yet, because the on-chain item is `ValueQuery`: "no entry" and "empty vec"
    /// are the same state there, and a raw `state_getStorage` on a genuinely-never-written
    /// key comes back `null` rather than as the encoding of an empty `BoundedVec`. Both are
    /// funnelled through [`decode_value_query_vec`]. An error return therefore means a real
    /// problem (malformed hex, or bytes that don't decode as this struct shape), not "not
    /// ready yet" — callers can poll this until `len() == OprfThreshold` and treat any `Err`
    /// as a genuine fault.
    pub async fn get_oprf_round1_commitments(
        &self,
        query_id: u64,
        committee_slot: u8,
    ) -> anyhow::Result<Vec<OprfRound1Commitment>> {
        let key = double_map_key_u64_u8("Identity", "OprfRound1Commitments", query_id, committee_slot);
        let raw = self.get_storage(&key).await?;
        decode_value_query_vec(raw)
            .with_context(|| format!("decoding OprfRound1Commitments[{query_id}][{committee_slot}]"))
    }

    /// Reads the full round-2 response set for one `(query_id, committee_slot)` pair from
    /// `Identity::OprfRound2Responses`. Same `ValueQuery` semantics as
    /// [`RpcClient::get_oprf_round1_commitments`] — empty vec, not an error, when unset.
    ///
    /// Not called by `main.rs`'s orchestration loop: this node's job ends once it submits its
    /// *own* round-2 response — combining the whole set into a final proof is the citizen
    /// client's job (see `main.rs`'s module docs and
    /// `docs/project/research/oprf-alternatives/11-genuine-threshold-evaluation-design.md`),
    /// not this component's. Kept, not deleted, as a real and tested capability useful for an
    /// operator wanting to inspect round-2 progress directly (e.g. a monitoring/debug tool).
    #[allow(dead_code)]
    pub async fn get_oprf_round2_responses(
        &self,
        query_id: u64,
        committee_slot: u8,
    ) -> anyhow::Result<Vec<OprfRound2Response>> {
        let key = double_map_key_u64_u8("Identity", "OprfRound2Responses", query_id, committee_slot);
        let raw = self.get_storage(&key).await?;
        decode_value_query_vec(raw)
            .with_context(|| format!("decoding OprfRound2Responses[{query_id}][{committee_slot}]"))
    }
}

/// Compute a 16-byte TwoX-128 hash of a UTF-8 string and return it as lowercase hex (no "0x").
/// TwoX-128 = XxHash64 with seed 0 concatenated with XxHash64 with seed 1, each as its
/// **little-endian byte representation** (not the hex digits of the numeric value).
///
/// NOTE — deliberately NOT copied verbatim from `desktop/src-tauri/src/rpc.rs` despite this
/// module's header saying this file mirrors that one: `desktop/src-tauri/src/rpc.rs` computes
/// this as `format!("{:016x}", r0.to_le())`, but `u64::to_le()` is a no-op on little-endian
/// hosts *for the numeric value* — it does not change what `{:016x}` prints (hex formatting
/// always prints a u64's digits most-significant-first, regardless of any `to_le()`/`to_be()`
/// call, which only matters when reinterpreting the value's byte layout, e.g. via
/// `to_le_bytes()`). Verified against the well-known `twox128("System") ==
/// "26aa394eea5630e07c48ae0c9558cef7"` vector (see the test below): the desktop version's
/// approach produces `"e03056ea4e39aa26f7ce58950cae487c"` for the same input on an
/// x86_64 (little-endian) host — the two 8-byte halves each byte-reversed relative to the
/// correct answer. This file uses `to_le_bytes()` + `hex::encode` instead, which matches the
/// known vector. Worth fixing upstream in `desktop/src-tauri/src/rpc.rs` too — flagged in this
/// component's README rather than changed here, since that file is outside this task's scope.
pub fn twox128_hex(input: &str) -> String {
    use std::hash::Hasher;
    use twox_hash::XxHash64;

    let mut h0 = XxHash64::with_seed(0);
    h0.write(input.as_bytes());
    let r0 = h0.finish();

    let mut h1 = XxHash64::with_seed(1);
    h1.write(input.as_bytes());
    let r1 = h1.finish();

    format!("{}{}", hex::encode(r0.to_le_bytes()), hex::encode(r1.to_le_bytes()))
}

/// Returns the "0x"-prefixed 32-byte storage prefix for a pallet + storage item.
pub fn storage_prefix(pallet: &str, item: &str) -> String {
    format!("0x{}{}", twox128_hex(pallet), twox128_hex(item))
}

/// Blake2-128 of raw bytes, used for the `Blake2_128Concat` storage map key hasher.
pub fn blake2_128(data: &[u8]) -> [u8; 16] {
    sp_core::blake2_128(data)
}

/// Full "0x"-prefixed storage key for a `StorageDoubleMap<_, Blake2_128Concat, u64,
/// Blake2_128Concat, u8, _>`:
///
/// ```text
/// twox128(pallet) ++ twox128(item) ++ blake2_128(key1) ++ key1 ++ blake2_128(key2) ++ key2
/// ```
///
/// i.e. the same `Blake2_128Concat` shape `main.rs` already builds by hand for the
/// single-key `CommitteeMembers` map, applied twice — `Concat` meaning the *un-hashed*
/// SCALE-encoded key is appended after its 16-byte hash (which is what makes the key
/// enumerable/decodable from `state_getKeysPaged` output). Both keys are SCALE-encoded
/// before hashing: `u64` as 8 little-endian bytes, `u8` as its single byte.
///
/// Total length: 32 + (16 + 8) + (16 + 1) = 73 bytes = 146 hex chars after the "0x".
pub fn double_map_key_u64_u8(pallet: &str, item: &str, key1: u64, key2: u8) -> String {
    let prefix = storage_prefix(pallet, item);
    let k1 = key1.encode();
    let k2 = key2.encode();
    format!(
        "{prefix}{}{}{}{}",
        hex::encode(blake2_128(&k1)),
        hex::encode(&k1),
        hex::encode(blake2_128(&k2)),
        hex::encode(&k2),
    )
}

/// Decodes a `ValueQuery`-shaped `Vec<T>`/`BoundedVec<T, _>` storage value.
///
/// Three inputs all mean "nobody has submitted anything yet" and all yield `Ok(vec![])`:
/// `None` (key genuinely never written, so the node returns null — the normal case before
/// the first submission for a query), `Some(&[])` (defensive: a zero-length value), and
/// `Some(&[0x00])` (a real encoding of an empty vec: compact length 0). Anything else is
/// decoded as an ordinary SCALE `Vec<T>` — a `BoundedVec`'s wire format is identical to
/// `Vec`'s (compact length prefix ++ elements); the bound is a compile-time-only constraint
/// on the pallet side and never appears on the wire, so no capacity check happens or can
/// happen here.
///
/// Deliberately strict about trailing bytes: leftover input after a successful decode means
/// the on-chain struct shape has drifted from the mirrored Rust structs above, which is
/// exactly the failure this component should surface loudly rather than half-consume.
pub fn decode_value_query_vec<T: Decode>(raw: Option<Vec<u8>>) -> anyhow::Result<Vec<T>> {
    let Some(bytes) = raw else { return Ok(Vec::new()) };
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let mut cursor = &bytes[..];
    let decoded = Vec::<T>::decode(&mut cursor)
        .map_err(|e| anyhow::anyhow!("SCALE decode failed ({e}) over {} bytes", bytes.len()))?;
    anyhow::ensure!(
        cursor.is_empty(),
        "SCALE decode left {} trailing byte(s) — on-chain struct shape likely differs from the \
         mirrored Rust struct in rpc.rs",
        cursor.len()
    );
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known-answer test: `twox128("System")` is a widely-published Substrate test vector
    /// (e.g. it's the first 16 bytes of every chain's `System::Account` storage prefix).
    #[test]
    fn twox128_matches_known_vector() {
        assert_eq!(twox128_hex("System"), "26aa394eea5630e07c48ae0c9558cef7");
    }

    #[test]
    fn storage_prefix_is_64_hex_chars_plus_0x() {
        let prefix = storage_prefix("Identity", "PendingOprfQueries");
        assert!(prefix.starts_with("0x"));
        assert_eq!(prefix.len(), 2 + 64);
    }

    fn sample_account(byte: u8) -> sp_core::crypto::AccountId32 {
        sp_core::crypto::AccountId32::new([byte; 32])
    }

    fn sample_round1(byte: u8) -> OprfRound1Commitment {
        OprfRound1Commitment {
            member: sample_account(byte),
            r_i: [byte.wrapping_add(1); 64],
            d_g: [byte.wrapping_add(2); 64],
            d_q: [byte.wrapping_add(3); 64],
            e_g: [byte.wrapping_add(4); 64],
            e_q: [byte.wrapping_add(5); 64],
        }
    }

    fn sample_round2(byte: u8) -> OprfRound2Response {
        OprfRound2Response { member: sample_account(byte), z_i: [byte.wrapping_add(9); 32] }
    }

    #[test]
    fn round1_commitment_scale_round_trips() {
        let original = sample_round1(0x11);
        let encoded = original.encode();
        // 32 (AccountId32) + 5 * 64 (points) = 352 bytes, no length prefixes anywhere: a
        // fixed-size struct of fixed-size fields.
        assert_eq!(encoded.len(), 32 + 5 * 64);
        let decoded = OprfRound1Commitment::decode(&mut &encoded[..]).expect("decode");
        assert_eq!(decoded, original);
    }

    #[test]
    fn round2_response_scale_round_trips() {
        let original = sample_round2(0x22);
        let encoded = original.encode();
        assert_eq!(encoded.len(), 32 + 32);
        let decoded = OprfRound2Response::decode(&mut &encoded[..]).expect("decode");
        assert_eq!(decoded, original);
    }

    /// Field order is positional on the wire, so pin it explicitly: the first 32 bytes must
    /// be `member`, the next 64 `r_i`, and so on. This is the check that would actually fail
    /// if the pallet reordered its struct and this mirror wasn't updated.
    #[test]
    fn round1_commitment_field_order_is_positional() {
        let c = sample_round1(0x40);
        let encoded = c.encode();
        assert_eq!(&encoded[0..32], &[0x40u8; 32]);
        assert_eq!(&encoded[32..96], &[0x41u8; 64]);
        assert_eq!(&encoded[96..160], &[0x42u8; 64]);
        assert_eq!(&encoded[160..224], &[0x43u8; 64]);
        assert_eq!(&encoded[224..288], &[0x44u8; 64]);
        assert_eq!(&encoded[288..352], &[0x45u8; 64]);
    }

    #[test]
    fn value_query_vec_round_trips_multiple_round1_entries() {
        let entries = vec![sample_round1(0x01), sample_round1(0x02), sample_round1(0x03)];
        let encoded = entries.encode();
        // Compact length prefix for 3 == single byte 0x0c (3 << 2).
        assert_eq!(encoded[0], 0x0c);
        let decoded: Vec<OprfRound1Commitment> =
            decode_value_query_vec(Some(encoded)).expect("decode");
        assert_eq!(decoded, entries);
    }

    #[test]
    fn value_query_vec_round_trips_multiple_round2_entries() {
        let entries = vec![sample_round2(0x07), sample_round2(0x08)];
        let decoded: Vec<OprfRound2Response> =
            decode_value_query_vec(Some(entries.encode())).expect("decode");
        assert_eq!(decoded, entries);
    }

    /// The three "nobody has submitted yet" shapes a `ValueQuery` double map can present over
    /// raw JSON-RPC: absent key (null), zero-length value, and an explicitly-encoded empty
    /// vec (compact 0). None of them is an error.
    #[test]
    fn missing_or_empty_value_decodes_to_empty_vec() {
        let from_missing: Vec<OprfRound1Commitment> =
            decode_value_query_vec(None).expect("missing key must not error");
        assert!(from_missing.is_empty());

        let from_zero_len: Vec<OprfRound1Commitment> =
            decode_value_query_vec(Some(Vec::new())).expect("zero-length value must not error");
        assert!(from_zero_len.is_empty());

        let from_encoded_empty: Vec<OprfRound2Response> =
            decode_value_query_vec(Some(vec![0x00])).expect("encoded empty vec must not error");
        assert!(from_encoded_empty.is_empty());

        // And the encoding of a real empty Vec is in fact that single 0x00 byte.
        assert_eq!(Vec::<OprfRound2Response>::new().encode(), vec![0x00]);
    }

    #[test]
    fn value_query_vec_rejects_truncated_and_trailing_bytes() {
        let entries = vec![sample_round2(0x05)];
        let encoded = entries.encode();

        let mut truncated = encoded.clone();
        truncated.pop();
        assert!(decode_value_query_vec::<OprfRound2Response>(Some(truncated)).is_err());

        let mut with_trailing = encoded;
        with_trailing.push(0xff);
        let err = decode_value_query_vec::<OprfRound2Response>(Some(with_trailing)).unwrap_err();
        assert!(err.to_string().contains("trailing"), "got: {err}");
    }

    #[test]
    fn double_map_key_has_expected_shape() {
        let key = double_map_key_u64_u8("Identity", "OprfRound1Commitments", 42, 3);
        assert!(key.starts_with("0x"));
        // 32-byte prefix + (16 + 8) + (16 + 1) = 73 bytes.
        assert_eq!(key.len(), 2 + 2 * 73);
        assert!(key.starts_with(&storage_prefix("Identity", "OprfRound1Commitments")));

        let bytes = hex::decode(key.trim_start_matches("0x")).expect("valid hex");
        // Blake2_128Concat appends the un-hashed SCALE-encoded key after its hash.
        assert_eq!(&bytes[32..48], &blake2_128(&42u64.encode())[..]);
        assert_eq!(&bytes[48..56], &42u64.to_le_bytes()[..]);
        assert_eq!(&bytes[56..72], &blake2_128(&3u8.encode())[..]);
        assert_eq!(bytes[72], 3u8);
    }

    /// Different `(query_id, committee_slot)` pairs and different storage items must never
    /// collide — cheap guard against a copy-paste error in the two fetch methods above.
    #[test]
    fn double_map_keys_are_distinct_per_key_and_item() {
        let a = double_map_key_u64_u8("Identity", "OprfRound1Commitments", 1, 0);
        let b = double_map_key_u64_u8("Identity", "OprfRound1Commitments", 1, 1);
        let c = double_map_key_u64_u8("Identity", "OprfRound1Commitments", 2, 0);
        let d = double_map_key_u64_u8("Identity", "OprfRound2Responses", 1, 0);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }
}
