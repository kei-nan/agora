//! Environment-variable configuration. Deliberately env-only (no config file format invented
//! here) — this is what a minimal OCI container / balenaCloud "device env vars" deployment
//! expects, see README.md's balenaCloud section.

use anyhow::Context;
use std::path::PathBuf;

pub struct Config {
    /// Chain JSON-RPC endpoint. Default matches the dev chain in /CLAUDE.md
    /// (`./target/release/agora-node --dev --tmp` listens on 127.0.0.1:9944).
    pub node_rpc_url: String,
    /// Which of the 5 committees (changelog #082/#073) this node's key material belongs to.
    /// 0..NUM_COMMITTEES (5) — see `pallets/pallet-identity/src/lib.rs`'s `NUM_COMMITTEES`.
    pub committee_slot: u8,
    /// Path to the age-encrypted key file (see keystore.rs). Mounted into the container as a
    /// volume/secret, never baked into the image.
    pub keys_file: PathBuf,
    /// Passphrase for the age-encrypted key file. Prefer `KEY_PASSPHRASE_FILE` over this in
    /// any real use (env vars leak more easily — process listings, container inspect, crash
    /// dumps) — this raw-value var exists for quick local testing only.
    pub key_passphrase: Option<String>,
    /// Path to a file containing the passphrase (its content, trimmed of trailing newline).
    /// The preferred way to hand the container its passphrase — e.g. a Docker/Compose secret
    /// or a balenaCloud-mounted file — over the raw env var above.
    pub key_passphrase_file: Option<PathBuf>,
    /// Path to the compiled OPRF crypto-core `.wasm` module. If this path does not exist at
    /// startup, the node runs in STUB mode (see wasm_host.rs) rather than failing — the module
    /// genuinely doesn't exist yet as of this writing (see README "What's stubbed").
    pub wasm_module_path: PathBuf,
    /// How often to poll `PendingOprfQueries` for new work.
    pub poll_interval_secs: u64,
    /// PROVISIONAL: `pallet-identity`'s runtime pallet index. 8 matches the CURRENT
    /// construct_runtime! in runtime/src/lib.rs (`#[runtime::pallet_index(8)] pub type
    /// Identity = pallet_identity_zk;`) — this part is solid, not a guess, since the pallet
    /// itself already exists at that index today.
    pub pallet_index: u8,
    /// `submit_oprf_response`'s call index within pallet-identity. Reconciled against the real
    /// `#[pallet::call_index(16)]` in `pallets/pallet-identity/src/lib.rs` (changelog #082's
    /// on-chain mailbox landed with `submit_oprf_query` at index 15 and `submit_oprf_response`
    /// at 16, preceded by more existing calls than originally guessed here). Still overridable
    /// via `CALL_INDEX` in case the pallet's call ordering changes again.
    pub call_index: u8,
    /// Safety valve: when the Wasm crypto core is stubbed (module file absent), should the
    /// node actually submit the (obviously fake) placeholder response on-chain? Defaults to
    /// false — a stub evaluation is never something that should reach a real chain; the
    /// default keeps a misconfigured/incomplete deployment from doing that by accident. Set to
    /// "true" only for exercising the RPC/extrinsic-submission code path in a throwaway dev
    /// chain with no real citizens registered against it.
    pub allow_stub_submission: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let node_rpc_url =
            std::env::var("NODE_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:9944".to_string());

        let committee_slot: u8 = std::env::var("COMMITTEE_SLOT")
            .context("COMMITTEE_SLOT is required (0..5 — which OPRF committee this node belongs to)")?
            .parse()
            .context("COMMITTEE_SLOT must be a small integer")?;
        anyhow::ensure!(committee_slot < 5, "COMMITTEE_SLOT must be in 0..5 (NUM_COMMITTEES)");

        let keys_file = std::env::var("KEYS_FILE")
            .unwrap_or_else(|_| "/keys/committee-secrets.age".to_string())
            .into();

        let key_passphrase = std::env::var("KEY_PASSPHRASE").ok();
        let key_passphrase_file = std::env::var("KEY_PASSPHRASE_FILE").ok().map(PathBuf::from);
        anyhow::ensure!(
            key_passphrase.is_some() || key_passphrase_file.is_some(),
            "one of KEY_PASSPHRASE or KEY_PASSPHRASE_FILE is required to decrypt KEYS_FILE"
        );

        let wasm_module_path = std::env::var("WASM_MODULE_PATH")
            .unwrap_or_else(|_| "/wasm/oprf-crypto-core.wasm".to_string())
            .into();

        let poll_interval_secs: u64 = std::env::var("POLL_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let pallet_index: u8 = std::env::var("PALLET_INDEX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8);

        let call_index: u8 = std::env::var("CALL_INDEX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(16);

        let allow_stub_submission = std::env::var("ALLOW_STUB_SUBMISSION")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        Ok(Self {
            node_rpc_url,
            committee_slot,
            keys_file,
            key_passphrase,
            key_passphrase_file,
            wasm_module_path,
            poll_interval_secs,
            pallet_index,
            call_index,
            allow_stub_submission,
        })
    }

    /// Resolves the passphrase from either source, preferring the file.
    pub fn resolve_passphrase(&self) -> anyhow::Result<String> {
        if let Some(path) = &self.key_passphrase_file {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("reading KEY_PASSPHRASE_FILE at {}", path.display()))?;
            return Ok(raw.trim_end_matches(['\n', '\r']).to_string());
        }
        self.key_passphrase.clone().context("no passphrase source configured")
    }
}
