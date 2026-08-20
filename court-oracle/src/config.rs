//! Environment-variable configuration. Deliberately env-only (no config file format invented
//! here), matching the minimal-container-deployment convention this codebase already uses for
//! its other off-chain services.

use anyhow::Context;
use std::path::PathBuf;

pub struct Config {
    /// Chain JSON-RPC endpoint. Default matches the dev chain in /CLAUDE.md
    /// (`./target/release/agora-node --dev --tmp` listens on 127.0.0.1:9944).
    pub node_rpc_url: String,
    /// Path to the age-encrypted key file (see keystore.rs). Mounted into the container as a
    /// volume/secret, never baked into the image.
    pub keys_file: PathBuf,
    /// Passphrase for the age-encrypted key file. Prefer `KEY_PASSPHRASE_FILE` over this in
    /// any real use (env vars leak more easily — process listings, container inspect, crash
    /// dumps) — this raw-value var exists for quick local testing only.
    pub key_passphrase: Option<String>,
    /// Path to a file containing the passphrase (its content, trimmed of trailing newline).
    pub key_passphrase_file: Option<PathBuf>,
    /// How often to poll `Courts::Cases` for new `Filed` cases.
    pub poll_interval_secs: u64,
    /// `pallet-courts`'s runtime pallet index. 11 in this working tree's
    /// `runtime/src/lib.rs` `construct_runtime!` (`#[runtime::pallet_index(11)] pub type
    /// Courts = pallet_courts;`) — confirmed by reading that file, not guessed.
    pub courts_pallet_index: u8,
    /// `submit_ai_ruling`'s call index within pallet-courts. `#[pallet::call_index(1)]` in
    /// `pallets/pallet-courts/src/lib.rs` — confirmed by reading that file, not guessed. The
    /// real call is `submit_ai_ruling(case_id: u32, ruling_hash: [u8; 32], model_version: u32,
    /// verdict: Verdict)`, gated by `T::OracleOrigin` (checks the signer against `OracleMembers`
    /// — an Oracle Council, not a single account, since the M-of-N fix; see README.md's "Oracle
    /// Council" section) and requiring `model_version == CurrentAIModelVersion`
    /// (governance-set via `vote_approve_ai_model`) — `main.rs` fetches that value fresh from
    /// chain before every submission rather than caching or guessing it. `verdict` is frozen
    /// into the pending proposal here (`PendingOracleProposal`/eventually `AIRulingVerdict`
    /// once the council's approval threshold is reached) — `finalize_ruling` below applies
    /// exactly that value later and takes no verdict argument of its own. This call's shape
    /// (case_id, ruling_hash, model_version, verdict) and index were NOT changed by the M-of-N
    /// fix — only its on-chain *effect* changed, from immediate to threshold-gated; see
    /// README.md for what that means for a single running instance of this service.
    pub submit_ai_ruling_call_index: u8,
    /// `finalize_ruling`'s call index within pallet-courts. `#[pallet::call_index(4)]` in
    /// `pallets/pallet-courts/src/lib.rs` — confirmed by reading that file, not guessed. The
    /// real call is `finalize_ruling(case_id: u32)` — no verdict argument; once the Oracle
    /// Council's approval threshold for this proposal is reached, it applies whatever
    /// `submit_ai_ruling` already committed to `AIRulingVerdict` — gated by the *same*
    /// `T::OracleOrigin` as `submit_ai_ruling` (see `EnsureOracle`), and additionally requires
    /// `Cases[case_id].status == AIRulingIssued` and the current block to be strictly past
    /// `AIRulingBlock[case_id] + AppealWindowBlocks` — i.e. the appeal window has closed with no
    /// `appeal_ruling` call in between (an appeal moves status to `InJuryAppeal`, which this
    /// call's own `ensure!` rejects with `InvalidStatus`).
    pub finalize_ruling_call_index: u8,
    /// `pallet-courts`'s `AppealWindowBlocks` config constant — how many blocks after
    /// `submit_ai_ruling` a case may still be appealed. `ConstU32<{ 7 * DAYS }>` in
    /// `runtime/src/configs/mod.rs`, and `DAYS` (`runtime/src/lib.rs`) resolves to `7_200`
    /// blocks at this runtime's 12-second block time, so `7 * DAYS = 50_400` — confirmed by
    /// reading both files, not guessed. Used client-side only to decide *when it's worth
    /// attempting* `finalize_ruling`; the chain enforces the real deadline independently via
    /// `AIRulingBlock`, so a wrong value here only costs a wasted/rejected extrinsic, never an
    /// early finalize.
    pub appeal_window_blocks: u32,
    /// Note on what's deliberately NOT a config field here: `pallet-constitution`,
    /// `pallet-treasury-ledger`, and `pallet-audit`'s runtime pallet *indices* (12, 10, 16)
    /// are not needed anywhere in this service — storage-key hashing is keyed by each
    /// pallet's *name string* (`"Constitution"`, `"TreasuryLedger"`, `"PalletAudit"`, matching
    /// the identifiers used in `construct_runtime!`), not its numeric index. Only the two call
    /// bytes on the one write path (`courts_pallet_index`/`submit_ai_ruling_call_index` above)
    /// need the numeric form, since a SCALE-encoded call is addressed by
    /// `[pallet_index, call_index]`, not by name.
    /// Base URL of a local (or reachable) Kubo-compatible IPFS HTTP API. 127.0.0.1:5001 is the
    /// Kubo daemon's standard default. Never exercised against a real daemon in this sandboxed
    /// environment — see ipfs.rs and README.md.
    pub ipfs_api_url: String,
    /// Claude API model id. Defaults to `claude-opus-5` per this project's understanding of
    /// Anthropic's current model guidance (deliberately not the desktop app's
    /// `commands/agent.rs` choice of `claude-sonnet-4-6` — that call is a read-only citizen
    /// Q&A feature; this one produces a binding on-chain ruling, which is the kind of
    /// correctness-sensitive, non-cost-optimized call this project's own conventions call for
    /// spending more capability on). Overridable via `CLAUDE_MODEL`.
    pub claude_model: String,
    /// Safety valve mirroring this codebase's other off-chain services' stub/dry-run gates:
    /// when true, this service builds and logs the ruling and the IPFS publish, but does NOT
    /// submit `submit_ai_ruling` on-chain. Defaults to false (submit for real) since there is
    /// no "stub" ruling concept here the way there is a stub OPRF evaluation elsewhere in this
    /// codebase — but this flag exists for safely exercising the rest of the pipeline against
    /// a real chain before trusting it to actually rule on cases.
    pub dry_run: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let node_rpc_url =
            std::env::var("NODE_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:9944".to_string());

        let keys_file = std::env::var("KEYS_FILE")
            .unwrap_or_else(|_| "/keys/court-oracle-secrets.age".to_string())
            .into();

        let key_passphrase = std::env::var("KEY_PASSPHRASE").ok();
        let key_passphrase_file = std::env::var("KEY_PASSPHRASE_FILE").ok().map(PathBuf::from);
        anyhow::ensure!(
            key_passphrase.is_some() || key_passphrase_file.is_some(),
            "one of KEY_PASSPHRASE or KEY_PASSPHRASE_FILE is required to decrypt KEYS_FILE"
        );

        let poll_interval_secs: u64 = std::env::var("POLL_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        let courts_pallet_index: u8 = std::env::var("COURTS_PALLET_INDEX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(11);

        let submit_ai_ruling_call_index: u8 = std::env::var("SUBMIT_AI_RULING_CALL_INDEX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);

        let finalize_ruling_call_index: u8 = std::env::var("FINALIZE_RULING_CALL_INDEX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4);

        let appeal_window_blocks: u32 = std::env::var("APPEAL_WINDOW_BLOCKS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50_400);

        let ipfs_api_url = std::env::var("IPFS_API_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:5001".to_string());

        let claude_model = std::env::var("CLAUDE_MODEL")
            .unwrap_or_else(|_| crate::claude::DEFAULT_MODEL.to_string());

        let dry_run = std::env::var("DRY_RUN").map(|v| v == "true" || v == "1").unwrap_or(false);

        Ok(Self {
            node_rpc_url,
            keys_file,
            key_passphrase,
            key_passphrase_file,
            poll_interval_secs,
            courts_pallet_index,
            submit_ai_ruling_call_index,
            finalize_ruling_call_index,
            appeal_window_blocks,
            ipfs_api_url,
            claude_model,
            dry_run,
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
