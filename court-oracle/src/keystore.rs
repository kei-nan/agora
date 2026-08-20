//! Key storage — **explicit placeholder, not real tamper-resistant storage.**
//!
//! This service needs one credential: the sr25519 seed for the oracle account registered as an
//! Oracle Council member on `pallet-courts::OracleMembers` (via `add_oracle_member`, root-only
//! — that governance step is outside this service's job, see README.md). Under the M-of-N
//! Oracle Council design this seed is one *member's* key, not a sole controller — see
//! README.md's "Oracle Council (M-of-N ruling approval)" section. It's kept off disk in
//! plaintext using a
//! real, standard encryption format (`age`, age-encryption.org/v1) rather than stored raw or
//! behind a bespoke scheme. That is a modest speed bump against casual disk inspection, not a
//! defense against a motivated attacker with access to a running or powered-off host — the
//! same honest caveat this pattern carries wherever else it's used in this codebase for a
//! service-held signing key.
//!
//! ## File format (before encryption)
//! ```json
//! { "oracle_account_seed": "<64 hex chars, raw sr25519 seed>" }
//! ```
//! Create one with:
//! ```bash
//! echo '{"oracle_account_seed":"..."}' | age -p > court-oracle-secrets.age
//! ```

use age::secrecy::SecretString;
use anyhow::Context;
use serde::Deserialize;
use std::io::Read;
use std::path::Path;

#[derive(Deserialize)]
pub struct Secrets {
    pub oracle_account_seed: String,
}

impl Secrets {
    pub fn oracle_account_seed_bytes(&self) -> anyhow::Result<[u8; 32]> {
        let bytes = hex::decode(self.oracle_account_seed.trim_start_matches("0x"))
            .context("oracle_account_seed is not valid hex")?;
        bytes.try_into().map_err(|_| anyhow::anyhow!("oracle_account_seed must decode to 32 bytes"))
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
