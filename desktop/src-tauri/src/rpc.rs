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
            .await?
            .json()
            .await?;
        if let Some(err) = resp.error {
            anyhow::bail!("RPC error: {err}");
        }
        resp.result.ok_or_else(|| anyhow::anyhow!("no result from {method}"))
    }

    pub async fn chain_block_numbers(&self) -> anyhow::Result<(u64, u64)> {
        let best_val = self.call("chain_getHeader", Value::Array(vec![])).await?;
        let best_hex = best_val["number"].as_str().unwrap_or("0x0");
        let best = u64::from_str_radix(best_hex.trim_start_matches("0x"), 16).unwrap_or(0);

        let fin_hash = self.call("chain_getFinalizedHead", Value::Array(vec![])).await?;
        let fin_val = self.call("chain_getHeader", Value::Array(vec![fin_hash])).await?;
        let fin_hex = fin_val["number"].as_str().unwrap_or("0x0");
        let finalized = u64::from_str_radix(fin_hex.trim_start_matches("0x"), 16).unwrap_or(0);

        Ok((best, finalized))
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

    /// Fetch a single storage value by its full hex key. Returns `Ok(None)` only when the
    /// key is genuinely absent on-chain — a transport/RPC failure is propagated as `Err`
    /// rather than coerced into `Ok(None)`, so callers can't mistake "node unreachable"
    /// for "key not set".
    pub async fn get_storage(&self, key_hex: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let params = Value::Array(vec![Value::String(key_hex.to_string()), Value::Null]);
        let v = self.call("state_getStorage", params).await?;
        if v.is_null() {
            Ok(None)
        } else {
            let hex = v.as_str().unwrap_or("").trim_start_matches("0x").to_string();
            Ok(Some(hex::decode(hex)?))
        }
    }

    /// Fetch values for a list of storage keys, all at the current best block.
    pub async fn query_storage_at(
        &self,
        keys: &[String],
    ) -> anyhow::Result<Vec<Option<String>>> {
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
}

/// Compute a 16-byte TwoX-128 hash of a UTF-8 string and return it as lowercase hex (no "0x").
/// TwoX-128 = XxHash64 with seed 0 concatenated with XxHash64 with seed 1, both LE.
pub fn twox128_hex(input: &str) -> String {
    use std::hash::Hasher;
    use twox_hash::XxHash64;

    let mut h0 = XxHash64::with_seed(0);
    h0.write(input.as_bytes());
    let r0 = h0.finish();

    let mut h1 = XxHash64::with_seed(1);
    h1.write(input.as_bytes());
    let r1 = h1.finish();

    // `to_le()` is a no-op on any little-endian host and, even where it isn't, only swaps the
    // *value* — `{:016x}` then formats that value's hex digits in big-endian order regardless.
    // Neither step produces the little-endian *byte sequence* TwoX-128 actually needs; that
    // requires hex-encoding `to_le_bytes()` byte-by-byte instead of formatting the integer.
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(&r0.to_le_bytes());
    bytes.extend_from_slice(&r1.to_le_bytes());
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod twox128_tests {
    use super::twox128_hex;

    /// `twox128("System")` is a widely-documented reference value across the Substrate
    /// ecosystem (the `System` pallet's storage-key prefix) — a real known-answer check, not
    /// an invented one.
    #[test]
    fn matches_known_system_pallet_prefix() {
        assert_eq!(twox128_hex("System"), "26aa394eea5630e07c48ae0c9558cef7");
    }
}

/// Returns the "0x"-prefixed 32-byte storage prefix for a pallet + storage item.
pub fn storage_prefix(pallet: &str, item: &str) -> String {
    format!("0x{}{}", twox128_hex(pallet), twox128_hex(item))
}
