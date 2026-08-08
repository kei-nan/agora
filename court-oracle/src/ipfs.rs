//! Minimal IPFS publishing client — `POST /api/v0/add` against a local IPFS daemon's HTTP API
//! (`127.0.0.1:5001` is the standard Kubo default). Confirmed by grep across this codebase: no
//! IPFS *publishing* client existed anywhere before this file — every existing consumer
//! (`desktop/src-tauri/src/commands/chain.rs`'s `fetch_ipfs_content`/`hash_to_cid`) only reads
//! content given an already-known on-chain hash, assuming it's a raw SHA-256 digest that can be
//! wrapped in a CIDv0 multihash header and fetched from a public gateway.
//!
//! ## A real mismatch this file cannot paper over
//!
//! `submit_ai_ruling`'s doc comment calls `ruling_hash` "the IPFS CID of the full reasoning
//! document," but the on-chain field is a fixed `[u8; 32]`, not a CID string — every existing
//! consumer (above) treats it as a raw SHA-256 digest and re-derives a CIDv0 from it via
//! `0x12 0x20 ++ digest` + base58, which only round-trips correctly if the content was added to
//! IPFS as a single **raw** block. A standard `ipfs add` (even for one small file) wraps
//! content in a UnixFS structure serialized as `dag-pb` — so the multihash digest inside the
//! CID `ipfs add` actually returns is `sha256(dag-pb-wrapped bytes)`, NOT
//! `sha256(plaintext document bytes)`. There is no way to satisfy both "submit a hash a caller
//! can recompute by hashing the plaintext document" and "submit a hash that round-trips through
//! a standard `ipfs add` + gateway fetch" at once — something has to give.
//!
//! This file picks the second: it submits the actual digest bytes extracted from the CID
//! `ipfs add` returns, so `fetch_ipfs_content`'s existing CIDv0 reconstruction (`hash_to_cid`)
//! actually works and returns the real document. The cost: nobody can verify `ruling_hash` by
//! hashing the plaintext document themselves — verifying requires re-fetching from IPFS (or
//! re-running `ipfs add` on the same bytes with the same settings) and comparing CIDs, not a
//! bare hash comparison. Flagged plainly here and in the changelog rather than silently picking
//! one and hoping it doesn't matter.
//!
//! **Never exercised against a real IPFS daemon in this sandboxed environment** — no daemon is
//! reachable here. `IpfsClient::add` is untested beyond compiling; the pure CID math functions
//! below (`cidv0_to_digest`/`digest_to_cidv0`) are unit-tested against the same known-answer
//! convention `desktop/src-tauri/src/commands/chain.rs`'s `hash_to_cid` already uses.

use anyhow::Context;
use reqwest::Client;
use serde::Deserialize;

pub struct IpfsClient {
    api_url: String,
    client: Client,
}

#[derive(Deserialize)]
struct AddResponse {
    #[serde(rename = "Hash")]
    hash: String,
}

impl IpfsClient {
    pub fn new(api_url: impl Into<String>) -> Self {
        Self { api_url: api_url.into(), client: Client::new() }
    }

    /// POSTs `content` to `/api/v0/add?cid-version=0`, returning the resulting CIDv0 string.
    /// Requires a reachable Kubo-compatible IPFS daemon with its HTTP API exposed — see this
    /// module's header comment: never exercised against a real daemon here.
    pub async fn add(&self, filename: &str, content: Vec<u8>) -> anyhow::Result<String> {
        let part = reqwest::multipart::Part::bytes(content).file_name(filename.to_string());
        let form = reqwest::multipart::Form::new().part("file", part);
        let url = format!("{}/api/v0/add?cid-version=0", self.api_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .context("IPFS daemon unreachable — is one running at IPFS_API_URL?")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("IPFS add failed: {status}: {text}");
        }
        let parsed: AddResponse = resp
            .json()
            .await
            .context("IPFS add response was not the expected JSON shape")?;
        Ok(parsed.hash)
    }
}

/// Decodes a CIDv0 string (base58btc, sha2-256 multihash) into its raw 32-byte digest — the
/// inverse of `desktop/src-tauri/src/commands/chain.rs`'s `hash_to_cid`. Used to turn what
/// `IpfsClient::add` returns into the `[u8; 32]` `submit_ai_ruling` actually stores on-chain.
/// Pure and testable — no network call.
pub fn cidv0_to_digest(cid: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = bs58::decode(cid).into_vec().context("CID is not valid base58")?;
    anyhow::ensure!(
        bytes.len() == 34,
        "expected a 34-byte CIDv0 multihash (2-byte header + 32-byte digest), got {}",
        bytes.len()
    );
    anyhow::ensure!(
        bytes[0] == 0x12 && bytes[1] == 0x20,
        "CID is not a plain sha2-256 CIDv0 (unexpected multihash header {:02x}{:02x})",
        bytes[0],
        bytes[1]
    );
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&bytes[2..]);
    Ok(digest)
}

/// Encodes a raw 32-byte digest back into a CIDv0 string — mirrors
/// `desktop/src-tauri/src/commands/chain.rs`'s `hash_to_cid` exactly (same bytes, same
/// crate/version), used here only for human-readable logging (so an operator can
/// `ipfs cat <cid>` the ruling without doing the hex/base58 math by hand) and for the round-trip
/// test below. Pure and testable — no network call.
pub fn digest_to_cidv0(digest: &[u8; 32]) -> String {
    let mut multihash = Vec::with_capacity(34);
    multihash.push(0x12u8);
    multihash.push(0x20u8);
    multihash.extend_from_slice(digest);
    bs58::encode(multihash).into_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_cidv0_round_trips() {
        let digest = [7u8; 32];
        let cid = digest_to_cidv0(&digest);
        assert_eq!(cidv0_to_digest(&cid).unwrap(), digest);
    }

    #[test]
    fn digest_cidv0_round_trips_all_zero() {
        let digest = [0u8; 32];
        let cid = digest_to_cidv0(&digest);
        assert_eq!(cidv0_to_digest(&cid).unwrap(), digest);
    }

    #[test]
    fn cidv0_to_digest_rejects_bad_header() {
        // 34 bytes of 0x00, base58-encoded — decodes fine but has the wrong multihash header.
        let bogus = bs58::encode([0x00u8; 34]).into_string();
        assert!(cidv0_to_digest(&bogus).is_err());
    }

    #[test]
    fn cidv0_to_digest_rejects_wrong_length() {
        let too_short = bs58::encode([0x12u8, 0x20, 0xAB]).into_string();
        assert!(cidv0_to_digest(&too_short).is_err());
    }

    #[test]
    fn cidv0_to_digest_rejects_invalid_base58() {
        assert!(cidv0_to_digest("not-valid-base58-!!!").is_err());
    }
}
