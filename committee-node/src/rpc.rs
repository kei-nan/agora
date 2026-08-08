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
}
