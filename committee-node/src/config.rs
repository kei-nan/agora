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
    /// **This member's own 1-based position in the on-chain `CommitteeMembers[committee_slot]`
    /// roster**, i.e. the `index` argument to `oprf-committee-dev`'s
    /// `threshold::round1`/`round2_response` and the `i` in its Lagrange coefficient
    /// `λ_i = Π_{j≠i} idx_j / (idx_j - idx_i)`.
    ///
    /// The 1-based convention is not arbitrary and is not this file's to change: it is fixed
    /// by `oprf-committee-dev/src/threshold.rs`'s module docs and by `dkg.rs`, where a
    /// party's share is `f(party_index)` evaluated at a **1-based** `x` — `x = 0` is the
    /// group secret itself, so index 0 is not a member index and would be catastrophic to
    /// pass here. This value must match the member's position in the real roster order used
    /// by the DKG ceremony that produced this node's share.
    ///
    /// **RISK, stated plainly: getting this wrong produces no visible error anywhere.** The
    /// pallet performs no cryptographic verification of round-1/round-2 submissions (a
    /// deliberate scope boundary — see `OprfRound1Commitment`'s doc comment in
    /// `pallets/pallet-identity/src/lib.rs`), so a wrong index is accepted on-chain and
    /// stored. Downstream it silently produces the wrong Lagrange coefficient, so this
    /// member's `z_i` doesn't fit the combination, and the combined proof simply fails to
    /// verify — client-side, at `register_citizen`, attributed to nobody. The failure surfaces
    /// far from its cause and never names this node. Verify against the real roster before
    /// deploying; do not guess.
    ///
    /// Only the lower bound (`>= 1`) is validated here. There is no upper bound to check
    /// against without reading `CommitteeMembers` from the chain, which `from_env()`
    /// deliberately does not do (it is synchronous and must work with no chain reachable);
    /// `main.rs`'s existing roster sanity-check is the natural place for that, and it does
    /// not do it today.
    pub member_index: u64,
    /// This committee's **group** public key `Y = s·G` (64 bytes, `x(32) || y(32)` big-endian
    /// BN254 `Fr`, same point encoding every other 64-byte field in this crate uses) — the
    /// value `submit_oprf_round2`'s combined proof ultimately verifies against.
    ///
    /// Not secret, but also not on-chain in usable form: `OprfCommitteeKeys` stores only
    /// `Poseidon2(Y.x, Y.y)`, a one-way hash, specifically so a hostile chain observer can't
    /// read the raw key off-chain (see `pallets/pallet-identity/src/lib.rs`). It is derivable
    /// by anyone from the DKG's public Feldman commitments
    /// (`oprf-committee-dev::dkg::group_public_key`) without needing any member's
    /// cooperation, but this node has no DKG-ceremony-transcript reader (see
    /// `oprf-committee-dev/src/dkg.rs`'s own "still open" items) — so for now this is a
    /// deployment-time constant every member of this committee must be independently given the
    /// same value for, out of band, at ceremony time. Getting it wrong doesn't error here
    /// either: it just means every combined proof this node contributes to fails to verify
    /// against the wrong key, symmetrically with [`Config::member_index`]'s failure mode.
    pub group_pubkey: [u8; 64],
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

        let member_index = parse_member_index(std::env::var("MEMBER_INDEX").ok())?;
        let group_pubkey = parse_group_pubkey(std::env::var("GROUP_PUBKEY_HEX").ok())?;

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

        let allow_stub_submission = std::env::var("ALLOW_STUB_SUBMISSION")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        Ok(Self {
            node_rpc_url,
            committee_slot,
            member_index,
            group_pubkey,
            keys_file,
            key_passphrase,
            key_passphrase_file,
            wasm_module_path,
            poll_interval_secs,
            pallet_index,
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

/// Parses and validates `MEMBER_INDEX`. Required, no default — see [`Config::member_index`]
/// for the 1-based convention and for why a wrong value fails silently rather than loudly.
///
/// Split out of `from_env()` as a pure function taking the already-read env value so it can
/// be unit-tested deterministically: `std::env::set_var` mutates process-global state shared
/// by all test threads, which makes env-var tests order-dependent and flaky under cargo's
/// default parallel test runner. The env read itself (`std::env::var("MEMBER_INDEX").ok()`)
/// is the one line this leaves untested.
pub fn parse_member_index(raw: Option<String>) -> anyhow::Result<u64> {
    let raw = raw.context(
        "MEMBER_INDEX is required (this member's 1-based position in CommitteeMembers[COMMITTEE_SLOT] \
         — must match the roster order the DKG ceremony used; a wrong value fails silently, see \
         config.rs)",
    )?;
    let index: u64 = raw
        .trim()
        .parse()
        .with_context(|| format!("MEMBER_INDEX must be a positive integer, got {raw:?}"))?;
    anyhow::ensure!(
        index >= 1,
        "MEMBER_INDEX must be >= 1 — indices are 1-based (index 0 is the group secret itself in \
         Shamir/Feldman terms, not a member; see oprf-committee-dev/src/dkg.rs)"
    );
    Ok(index)
}

/// Parses `GROUP_PUBKEY_HEX`: a "0x"-optional 128-hex-char (64-byte) big-endian `x || y` point.
/// Split out as a pure function for the same testability reason as [`parse_member_index`].
pub fn parse_group_pubkey(raw: Option<String>) -> anyhow::Result<[u8; 64]> {
    let raw = raw.context(
        "GROUP_PUBKEY_HEX is required (this committee's group public key Y = s*G, 64 bytes \
         hex-encoded, x then y big-endian — see config.rs for where this comes from)",
    )?;
    let trimmed = raw.trim().trim_start_matches("0x");
    let bytes = hex::decode(trimmed)
        .with_context(|| format!("GROUP_PUBKEY_HEX must be valid hex, got {raw:?}"))?;
    bytes.try_into().map_err(|b: Vec<u8>| {
        anyhow::anyhow!("GROUP_PUBKEY_HEX must decode to exactly 64 bytes, got {}", b.len())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_index_parses_when_present() {
        assert_eq!(parse_member_index(Some("1".to_string())).unwrap(), 1);
        assert_eq!(parse_member_index(Some("7".to_string())).unwrap(), 7);
        // Container env plumbing (Compose/balena) can leave surrounding whitespace.
        assert_eq!(parse_member_index(Some(" 12 \n".to_string())).unwrap(), 12);
    }

    #[test]
    fn member_index_is_required() {
        let err = parse_member_index(None).unwrap_err();
        assert!(err.to_string().contains("MEMBER_INDEX is required"), "got: {err}");
    }

    #[test]
    fn member_index_rejects_malformed_values() {
        for bad in ["", "abc", "1.5", "-3", "0x2", "1,2"] {
            assert!(
                parse_member_index(Some(bad.to_string())).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn member_index_rejects_zero_because_indices_are_one_based() {
        let err = parse_member_index(Some("0".to_string())).unwrap_err();
        assert!(err.to_string().contains(">= 1"), "got: {err}");
    }

    #[test]
    fn group_pubkey_parses_with_and_without_0x_prefix() {
        let hex64 = "ab".repeat(64);
        assert_eq!(parse_group_pubkey(Some(hex64.clone())).unwrap(), [0xabu8; 64]);
        assert_eq!(parse_group_pubkey(Some(format!("0x{hex64}"))).unwrap(), [0xabu8; 64]);
    }

    #[test]
    fn group_pubkey_is_required() {
        assert!(parse_group_pubkey(None).is_err());
    }

    #[test]
    fn group_pubkey_rejects_wrong_length_and_bad_hex() {
        assert!(parse_group_pubkey(Some("ab".repeat(63))).is_err());
        assert!(parse_group_pubkey(Some("ab".repeat(65))).is_err());
        assert!(parse_group_pubkey(Some("zz".repeat(64))).is_err());
    }
}
