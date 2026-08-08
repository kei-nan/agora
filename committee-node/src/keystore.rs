//! Key storage — **explicit placeholder, not real tamper-resistant storage.**
//!
//! Per changelog #082's "Still open" section, real hardware-backed key custody for the
//! laptop/Pi committee-member scenario is an open, unsolved question (a stock Raspberry Pi has
//! no secure boot / hardware root of trust, so even a locked-down OS with SSH disabled doesn't
//! stop someone with physical access from pulling the SD card and reading it directly — that's
//! the entry's own "Tamper-resistance...needs an add-on TPM" point). This module does **not**
//! solve that; it is explicitly out of scope here (see the task's own constraints). What it
//! does instead: keeps the two secrets this node needs off disk in plaintext, using a real,
//! standard encryption format (`age`, age-encryption.org/v1) rather than storing them raw or
//! inventing a bespoke scheme. That is a modest speed bump against casual disk inspection, not
//! a defense against a motivated attacker with physical access to a running or powered-off
//! device — see README.md's "Key storage" section for the honest version of this paragraph.
//!
//! ## Two distinct credentials, one file
//!
//! Changelog #082 calls out that a committee member's device holds two credentials doing two
//! different jobs:
//! - `chain_account_seed`: signs the `submit_oprf_response` extrinsic; checked by the chain
//!   against the (not-yet-implemented) `CommitteeMembers` roster for this node's slot.
//! - `oprf_secret_key`: the actual OPRF committee secret the Wasm crypto core evaluates the
//!   query against. Never touches the chain, never leaves this process's memory.
//!
//! Both live in one `age`-encrypted JSON blob for simplicity (one file to mount, one
//! passphrase to manage) — nothing stops splitting them into two files/passphrases later if
//! that turns out to matter operationally; this is a minimal skeleton, not a policy decision.
//!
//! ## File format (before encryption)
//! ```json
//! { "chain_account_seed": "<64 hex chars, raw sr25519 seed>",
//!   "oprf_secret_key": "<hex, whatever length the real Wasm module's secret_key_bytes expects>" }
//! ```
//! Create one with:
//! ```bash
//! echo '{"chain_account_seed":"...","oprf_secret_key":"..."}' | age -p > committee-secrets.age
//! ```

use age::secrecy::SecretString;
use anyhow::Context;
use serde::Deserialize;
use std::io::Read;
use std::path::Path;

#[derive(Deserialize)]
pub struct Secrets {
    pub chain_account_seed: String,
    pub oprf_secret_key: String,
}

impl Secrets {
    pub fn chain_account_seed_bytes(&self) -> anyhow::Result<[u8; 32]> {
        let bytes = hex::decode(self.chain_account_seed.trim_start_matches("0x"))
            .context("chain_account_seed is not valid hex")?;
        bytes.try_into().map_err(|_| anyhow::anyhow!("chain_account_seed must decode to 32 bytes"))
    }

    pub fn oprf_secret_key_bytes(&self) -> anyhow::Result<Vec<u8>> {
        hex::decode(self.oprf_secret_key.trim_start_matches("0x"))
            .context("oprf_secret_key is not valid hex")
    }
}

/// Decrypts `path` (an age-encrypted file, passphrase recipient) and parses the JSON secrets
/// blob inside it. The passphrase never touches disk here — it's read from
/// `Config::resolve_passphrase()` (an env var or a separately-mounted file) and held only in
/// memory for the duration of this call.
pub fn load(path: &Path, passphrase: &str) -> anyhow::Result<Secrets> {
    let encrypted = std::fs::read(path)
        .with_context(|| format!("reading encrypted keys file at {}", path.display()))?;

    let decryptor = age::Decryptor::new(&encrypted[..])
        .context("KEYS_FILE is not a valid age-encrypted file")?;
    let age::Decryptor::Passphrase(decryptor) = decryptor else {
        anyhow::bail!(
            "KEYS_FILE is age-encrypted to one or more recipient keys, not a passphrase — \
             this component only supports the passphrase (scrypt) recipient (`age -p`), \
             see keystore.rs module docs"
        );
    };

    let passphrase = SecretString::from(passphrase.to_string());
    let mut decrypted = Vec::new();
    let mut reader = decryptor
        .decrypt(&passphrase, None)
        .context("failed to decrypt KEYS_FILE — wrong passphrase, or file is corrupt")?;
    reader
        .read_to_end(&mut decrypted)
        .context("reading decrypted key material")?;

    serde_json::from_slice(&decrypted)
        .context("decrypted KEYS_FILE content is not the expected JSON shape — see keystore.rs module docs")
}
