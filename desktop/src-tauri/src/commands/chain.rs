// ARCHITECTURE NOTE (changelog #087): the frontend's actual chain-read path for
// `chain_status`/`fetch_proposals`/`fetch_laws`/`fetch_treasury`/`fetch_department_budgets`/
// `fetch_rulings`/`fetch_legislature_data`/`fetch_elections_data`/`fetch_anticorruption_data`
// no longer goes through these Tauri commands. `desktop/src/lib/invoke.ts` now routes those
// nine command names to `desktop/src/chain/queries.ts`, which reads the same storage items
// through an embedded smoldot light client (`desktop/src/chain/client.ts`) instead of this
// module's plain `reqwest` JSON-RPC. See CLAUDE.md's "Desktop App > Stack" section for why the
// light client lives in the JS frontend rather than here: `smoldot` and `@polkadot/api` were
// already JS-only dependencies, and smoldot's primary distribution is a JS/WASM package, not a
// Rust crate embedded in a host process.
//
// The `#[tauri::command]` functions below are kept — not deleted — for two reasons: they're
// still correct, tested, and behind their own unit tests below; and they remain the actual
// implementation for `auth_verify_nullifier`'s trust-boundary-flagged callers (see
// `lookup_registered_account` below) and `chain_submit_extrinsic`, neither of which moved to
// the JS light client in this pass. If a future change also migrates those, this whole module
// (and `rpc.rs`) may become fully dead code — check `invoke.ts`'s `LIGHT_CLIENT_COMMANDS` map
// before assuming that, since it's the actual source of truth for what the frontend still calls
// here.
use serde::{Deserialize, Serialize};
use crate::rpc::{RpcClient, storage_prefix};
use std::collections::HashMap;
use std::time::Duration;
use tauri::State;

const NODE_URL: &str = "http://127.0.0.1:9944";

#[derive(Serialize, Deserialize)]
pub struct ChainStatusResponse {
    pub best: u64,
    pub finalized: u64,
}

#[derive(Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub title: String,
    pub status: String,
    pub proposer: String,
    #[serde(rename = "votesFor")]
    pub votes_for: u64,
    #[serde(rename = "votesAgainst")]
    pub votes_against: u64,
    #[serde(rename = "endsAt")]
    pub ends_at: u64,
    #[serde(rename = "ipfsHash")]
    pub ipfs_hash: String,
    pub summary: String,
    pub tier: String,
}

#[derive(Serialize, Deserialize)]
pub struct Law {
    pub id: String,
    pub title: String,
    pub tier: String,
    pub version: u32,
    #[serde(rename = "enactedAt")]
    pub enacted_at: u64,
    #[serde(rename = "ipfsHash")]
    pub ipfs_hash: String,
    pub summary: String,
}

#[derive(Serialize, Deserialize)]
pub struct TreasuryEntry {
    pub id: String,
    pub department: String,
    pub amount: String,
    pub currency: String,
    pub description: String,
    pub timestamp: u64,
    #[serde(rename = "ipfsHash")]
    pub ipfs_hash: String,
}

#[derive(Serialize, Deserialize)]
pub struct DepartmentBudget {
    #[serde(rename = "departmentId")]
    pub department_id: u32,
    pub budget: String,
    pub spent: String,
    pub remaining: String,
}

#[derive(Serialize, Deserialize)]
pub struct Ruling {
    pub id: String,
    #[serde(rename = "caseTitle")]
    pub case_title: String,
    pub level: u8,
    pub outcome: String,
    pub summary: String,
    #[serde(rename = "ipfsHash")]
    pub ipfs_hash: String,
    pub timestamp: u64,
}

/// Returns the current best and finalized block numbers from the running chain.
///
/// Propagates a real error when the node is unreachable rather than coercing failure
/// into a fake `{best: 0, finalized: 0}` success — that value is indistinguishable from
/// a genuinely synced chain at genesis, so the frontend needs a real Err to render a
/// distinct "Disconnected" state (see `ChainContext.tsx` / `ChainStatusBar.tsx`).
#[tauri::command]
pub async fn chain_status() -> Result<ChainStatusResponse, String> {
    fetch_chain_status(NODE_URL).await
}

/// Testable core of `chain_status`, parameterized on the node URL so tests can point it
/// at a definitely-unreachable address instead of the hardcoded dev node port.
async fn fetch_chain_status(url: &str) -> Result<ChainStatusResponse, String> {
    let client = RpcClient::new(url);
    client
        .chain_block_numbers()
        .await
        .map(|(best, finalized)| ChainStatusResponse { best, finalized })
        .map_err(|e| format!("chain unreachable: {e}"))
}

/// Fetches active referenda from pallet-voting (Voting.Referenda + Voting.ReferendumTally).
///
/// SCALE layout for Referenda value (42 bytes):
///   petition_id: u32 (4 bytes LE)
///   topic_hash:  [u8;32]
///   end_block:   u32 (4 bytes LE)
///   state:       u8  (0=Voting, 1=Passed, 2=Failed)
///   tier:        u8  (0=Ordinary, 1=Constitutional)
///
/// SCALE layout for ReferendumTally value (8 bytes):
///   yes_count: u32 LE
///   no_count:  u32 LE
#[tauri::command]
pub async fn fetch_proposals() -> Result<Vec<Proposal>, String> {
    let client = RpcClient::new(NODE_URL);

    // ── Fetch referendum entries ─────────────────────────────────────────────
    let ref_prefix = storage_prefix("Voting", "Referenda");
    let ref_keys = client
        .get_keys_paged(&ref_prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;

    // ── Fetch tallies keyed by referendum_id ─────────────────────────────────
    let tally_prefix = storage_prefix("Voting", "ReferendumTally");
    let tally_keys = client
        .get_keys_paged(&tally_prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let tally_values = client
        .query_storage_at(&tally_keys)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;

    // Build referendum_id → (yes, no) map
    let mut tallies: HashMap<u32, (u32, u32)> = HashMap::new();
    for (key_hex, val_opt) in tally_keys.iter().zip(tally_values.iter()) {
        if let Some(val_hex) = val_opt {
            let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
            if bytes.len() >= 8 {
                let yes = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                let no  = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                // Extract referendum_id from the last 4 bytes of the storage key
                let kbytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
                let rid = extract_u32_key_suffix(&kbytes);
                tallies.insert(rid, (yes, no));
            }
        }
    }

    if ref_keys.is_empty() {
        return Ok(vec![]);
    }

    let ref_values = client
        .query_storage_at(&ref_keys)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;

    let mut proposals = Vec::new();
    for (key_hex, val_opt) in ref_keys.iter().zip(ref_values.iter()) {
        if let Some(val_hex) = val_opt {
            let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
            if bytes.len() < 42 {
                continue;
            }
            // petition_id: bytes[0..4]
            let topic_hash = hex::encode(&bytes[4..36]);
            let end_block = u32::from_le_bytes([bytes[36], bytes[37], bytes[38], bytes[39]]);
            let state = match bytes[40] {
                0 => "active",
                1 => "passed",
                _ => "rejected",
            };
            let tier = match bytes[41] {
                1 => "constitutional",
                _ => "ordinary",
            };

            let key_bytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
            let referendum_id = extract_u32_key_suffix(&key_bytes);
            let (yes, no) = tallies.get(&referendum_id).copied().unwrap_or((0, 0));

            proposals.push(Proposal {
                id: format!("ref-{referendum_id}"),
                title: format!("Referendum #{referendum_id}"),
                status: state.to_string(),
                proposer: String::new(),
                votes_for: yes as u64,
                votes_against: no as u64,
                ends_at: end_block as u64,
                ipfs_hash: format!("0x{topic_hash}"),
                summary: format!("{tier} · closes block {end_block} · {yes} for / {no} against"),
                tier: tier.to_string(),
            });
        }
    }
    Ok(proposals)
}

/// Fetches enacted laws from pallet-constitution (Constitution.Laws).
///
/// SCALE layout (38+ bytes):
///   tier:         u8  (0=Ordinary, 1=Constitutional)
///   status:       u8  (0=Active, 1=Paused, 2=Repealed)
///   version:      u32 LE
///   content_hash: [u8;32]
#[tauri::command]
pub async fn fetch_laws() -> Result<Vec<Law>, String> {
    let client = RpcClient::new(NODE_URL);
    let prefix = storage_prefix("Constitution", "Laws");
    let keys = client
        .get_keys_paged(&prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    if keys.is_empty() {
        return Ok(vec![]);
    }
    let values = client
        .query_storage_at(&keys)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let mut laws = Vec::new();
    for (i, (key_hex, val_opt)) in keys.iter().zip(values.iter()).enumerate() {
        if let Some(val_hex) = val_opt {
            let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
            if bytes.len() >= 38 {
                let tier = match bytes[0] { 0 => "ordinary", _ => "constitutional" };
                let status = match bytes[1] { 0 => "active", 1 => "paused", _ => "repealed" };
                let version = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
                let ipfs_hash = format!("0x{}", hex::encode(&bytes[6..38]));
                let key_bytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
                let law_id = if key_bytes.len() >= 36 {
                    u32::from_le_bytes([key_bytes[32], key_bytes[33], key_bytes[34], key_bytes[35]])
                } else {
                    i as u32
                };
                laws.push(Law {
                    id: format!("law-{law_id}"),
                    title: format!("Law #{law_id}"),
                    tier: tier.to_string(),
                    version,
                    enacted_at: 0,
                    ipfs_hash,
                    summary: format!("Status: {status} · v{version}. Fetch full text from IPFS."),
                });
            }
        }
    }
    Ok(laws)
}

/// Fetches treasury expenditures from pallet-treasury-ledger (TreasuryLedger.ExpenditureLog).
///
/// SCALE layout for ExpenditureLog value (52 bytes):
///   department_id: u32   (4 bytes LE)
///   amount:        u128  (16 bytes LE — NOT compact, plain u128)
///   metadata_hash: [u8;32]
#[tauri::command]
pub async fn fetch_treasury() -> Result<Vec<TreasuryEntry>, String> {
    let client = RpcClient::new(NODE_URL);
    let prefix = storage_prefix("TreasuryLedger", "ExpenditureLog");
    let keys = client
        .get_keys_paged(&prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    if keys.is_empty() {
        return Ok(vec![]);
    }
    let values = client
        .query_storage_at(&keys)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let mut entries = Vec::new();
    for (i, val_opt) in values.iter().enumerate() {
        if let Some(val_hex) = val_opt {
            let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
            if bytes.len() >= 52 {
                let dept_id = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                let amount = u128::from_le_bytes(
                    bytes[4..20].try_into().unwrap_or([0u8; 16]),
                );
                let ipfs_hash = format!("0x{}", hex::encode(&bytes[20..52]));
                let agr_amount = format_agr(amount);
                entries.push(TreasuryEntry {
                    id: format!("tx-{i}"),
                    department: format!("Department {dept_id}"),
                    amount: agr_amount,
                    currency: "AGR".to_string(),
                    description: format!("Dept {dept_id} expenditure #{i}"),
                    timestamp: 0,
                    ipfs_hash,
                });
            }
        }
    }
    Ok(entries)
}

/// Fetches department budget allocations and spent amounts.
///
/// SCALE: DepartmentBudgets/DepartmentSpent value = u128 (16 bytes LE).
#[tauri::command]
pub async fn fetch_department_budgets() -> Result<Vec<DepartmentBudget>, String> {
    let client = RpcClient::new(NODE_URL);

    let budget_prefix = storage_prefix("TreasuryLedger", "DepartmentBudgets");
    let spent_prefix  = storage_prefix("TreasuryLedger", "DepartmentSpent");

    let budget_keys = client
        .get_keys_paged(&budget_prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let spent_keys = client
        .get_keys_paged(&spent_prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;

    let budget_vals = client
        .query_storage_at(&budget_keys)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let spent_vals = client
        .query_storage_at(&spent_keys)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;

    // Build dept_id → spent map
    let mut spent_map: HashMap<u32, u128> = HashMap::new();
    for (key_hex, val_opt) in spent_keys.iter().zip(spent_vals.iter()) {
        if let Some(val_hex) = val_opt {
            let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
            if bytes.len() >= 16 {
                let amt = u128::from_le_bytes(bytes[..16].try_into().unwrap_or([0u8; 16]));
                let kbytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
                let dept_id = extract_u32_key_suffix(&kbytes);
                spent_map.insert(dept_id, amt);
            }
        }
    }

    let mut budgets = Vec::new();
    for (key_hex, val_opt) in budget_keys.iter().zip(budget_vals.iter()) {
        if let Some(val_hex) = val_opt {
            let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
            if bytes.len() >= 16 {
                let budget_amt = u128::from_le_bytes(bytes[..16].try_into().unwrap_or([0u8; 16]));
                let kbytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
                let dept_id = extract_u32_key_suffix(&kbytes);
                let spent_amt = spent_map.get(&dept_id).copied().unwrap_or(0);
                let remaining = budget_amt.saturating_sub(spent_amt);
                budgets.push(DepartmentBudget {
                    department_id: dept_id,
                    budget: format_agr(budget_amt),
                    spent: format_agr(spent_amt),
                    remaining: format_agr(remaining),
                });
            }
        }
    }
    Ok(budgets)
}

/// Checks whether a nullifier hash (hex-encoded 32 bytes) is registered in pallet-identity.
/// Scans Identity.NullifierRegistry keys and returns true if the nullifier is found.
#[tauri::command]
pub async fn auth_verify_nullifier(nullifier_hex: String) -> Result<bool, String> {
    let client = RpcClient::new(NODE_URL);
    let prefix = storage_prefix("Identity", "NullifierRegistry");
    let keys = client
        .get_keys_paged(&prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let target = hex::decode(nullifier_hex.trim_start_matches("0x"))
        .map_err(|e| format!("invalid nullifier hex: {e}"))?;
    // Each key is: 32-byte prefix + 16-byte blake2_128 hash + 32-byte nullifier
    // So the nullifier occupies the last 32 bytes of the key.
    for key_hex in &keys {
        let key_bytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
        if key_bytes.len() >= 32 && key_bytes[key_bytes.len() - 32..] == target[..] {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Looks up the raw AccountId (32 bytes) registered against a nullifier hash in
/// pallet-identity's `NullifierRegistry`. Returns `None` if the nullifier isn't registered.
///
/// For an sr25519-derived account (what the mobile client actually uses — see
/// `mobile/src/chain/identity.ts`) this AccountId *is* the raw sr25519 public key: Substrate's
/// `MultiSigner::into_account()` wraps an sr25519/ed25519 public key directly with no hashing,
/// only an ecdsa key gets blake2-hashed down to 32 bytes. So the 32 bytes returned here are
/// exactly what `commands::auth::verify_challenge_signature` needs to check a QR-auth callback's
/// signature against the identity that's actually on file — not whatever pubkey the callback
/// body claims.
///
/// TRUST BOUNDARY — NOT YET SOUND (tracked, not fixed here): the `AccountId` this function
/// returns is trusted at face value. It comes from `RpcClient::get_storage`/`get_keys_paged`
/// (`rpc.rs`), which is a plain, unauthenticated `state_getStorage`/`state_getKeysPaged` JSON-RPC
/// call over plaintext HTTP to a hardcoded `NODE_URL` (`http://127.0.0.1:9944`). There is no
/// Merkle/state-proof verification against finalized chain consensus and no TLS — this code just
/// believes whatever bytes the endpoint returns. Anything able to control those RPC responses (a
/// malicious or compromised full node, or a local MITM on that loopback/LAN channel) can return
/// an attacker-chosen `AccountId` for any `nullifierHash`, and `handle_auth_callback` in
/// `commands::auth.rs` will verify the callback's signature against *that* attacker-controlled
/// key and mint a real, valid bearer session bound to an arbitrary victim identity. The sr25519
/// signature check downstream does not close this gap — it only proves the caller controls
/// *some* key; this function is what decides *whose* identity that key gets credited as.
///
/// The real fix is verifying a state proof against finalized consensus via an embedded light
/// client. As of changelog #087 that light client exists and is real — `desktop/src/chain/
/// client.ts` embeds smoldot via `@polkadot/api`'s `ScProvider` and genuinely verifies state
/// against finalized GRANDPA consensus — but it lives in the JS frontend (see that entry for
/// why) and this function was NOT migrated to use it: `lookup_registered_account` is called
/// from `commands::auth::handle_auth_callback`, deep in a Rust-side session-minting flow that
/// the QR-auth HTTP callback server (also Rust-side, necessarily — a webview can't run an HTTP
/// server) depends on. Moving this specific lookup onto the light client would mean either
/// embedding a second, Rust-side light client just for this one call, or restructuring the
/// auth flow so the already-connected JS light client performs the lookup and hands Rust a
/// verified result — both are real protocol/architecture work, out of scope for #087. Do not
/// treat this function's return value as a cryptographically authenticated identity binding
/// until one of those lands. Anyone wiring `chain_submit_extrinsic` (or any other session-gated
/// write path) to an actual phone-side flow MUST NOT assume a session's `nullifier_hash` is
/// trustworthy against a malicious/compromised RPC endpoint — it currently isn't.
pub(crate) async fn lookup_registered_account(nullifier: &[u8; 32]) -> Result<Option<[u8; 32]>, String> {
    let client = RpcClient::new(NODE_URL);
    let prefix = storage_prefix("Identity", "NullifierRegistry");
    let keys = client.get_keys_paged(&prefix).await.map_err(|e| e.to_string())?;
    // Each key is: 32-byte prefix + 16-byte blake2_128 hash + 32-byte nullifier (Blake2_128Concat),
    // so the nullifier occupies the last 32 bytes of the key — same layout auth_verify_nullifier
    // above already relies on.
    for key_hex in &keys {
        let key_bytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
        if key_bytes.len() >= 32 && key_bytes[key_bytes.len() - 32..] == nullifier[..] {
            let value = client.get_storage(key_hex).await.map_err(|e| e.to_string())?;
            if let Some(val_bytes) = value {
                if val_bytes.len() == 32 {
                    let mut account = [0u8; 32];
                    account.copy_from_slice(&val_bytes);
                    return Ok(Some(account));
                }
            }
            return Ok(None);
        }
    }
    Ok(None)
}

/// Forwards an already phone-signed extrinsic to the chain's `author_submitExtrinsic` RPC.
///
/// This is deliberately a thin passthrough, not a general transaction-building API: the phone
/// (never desktop) builds and signs the full SCALE-encoded extrinsic with its hardware-backed
/// key, matching CLAUDE.md's "Signing any on-chain transaction" staying mobile-only. Desktop's
/// job is only to (a) confirm the caller holds a valid, non-expired bearer token from the
/// QR-auth flow (`auth::require_valid_session` — expiry is actually enforced here, not just
/// shown in the UI) and (b) relay the bytes onward; it never sees or touches a private key.
///
/// TODO(protocol — not yet wired to anything): nothing on the phone side produces the input
/// this command expects. The QR-auth flow only ever carries a signed *challenge string*; there
/// is no established phone<->desktop channel for "desktop wants to submit call X, please sign
/// and send it back" — that needs its own round trip (a second QR/deep-link, or some other
/// transport), plus nonce/mortality negotiation so the phone knows what to sign. That's real
/// protocol design work beyond this auth fix, and is intentionally left undone here; this
/// command is only the chain-facing half, ready for whichever phone-side flow is designed next.
#[tauri::command]
pub async fn chain_submit_extrinsic(
    token: String,
    extrinsic_hex: String,
    sessions: State<'_, crate::commands::auth::SessionStore>,
) -> Result<String, String> {
    crate::commands::auth::require_valid_session(&sessions, &token)?;

    let client = RpcClient::new(NODE_URL);
    let result = client
        .call(
            "author_submitExtrinsic",
            serde_json::Value::Array(vec![serde_json::Value::String(extrinsic_hex)]),
        )
        .await
        .map_err(|e| e.to_string())?;
    result
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "unexpected response from author_submitExtrinsic".to_string())
}

/// Fetches court rulings, cross-referencing Courts.Cases for IPFS ruling hashes.
///
/// Cases SCALE value layout:
///   filer:    [u8;32]           (32 bytes — AccountId)
///   status:   u8                (0=Filed,1=AIRulingIssued,2=InJuryAppeal,3=JurySeated,4=FinalRuling)
///   ipfs_opt: u8 + [u8;32]?    (0=None, 1=Some + 32 bytes)
///   subject:  u8 + ...          (0=General, others have extra data)
///
/// Rulings SCALE value: u8 (0=Upheld, 1=Overturned)
#[tauri::command]
pub async fn fetch_rulings() -> Result<Vec<Ruling>, String> {
    let client = RpcClient::new(NODE_URL);

    // ── Fetch Cases for IPFS hash cross-reference ────────────────────────────
    let cases_prefix = storage_prefix("Courts", "Cases");
    let case_keys = client
        .get_keys_paged(&cases_prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let case_values = client
        .query_storage_at(&case_keys)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;

    // case_id → ipfs_hash (32 bytes, may be all zeros if no ruling hash yet)
    let mut case_ipfs: HashMap<u32, String> = HashMap::new();
    for (key_hex, val_opt) in case_keys.iter().zip(case_values.iter()) {
        if let Some(val_hex) = val_opt {
            let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
            // filer(32) + status(1) + Option discriminant(1) + maybe hash(32)
            if bytes.len() >= 34 {
                let ipfs = if bytes[33] == 1 && bytes.len() >= 66 {
                    format!("0x{}", hex::encode(&bytes[34..66]))
                } else {
                    String::new()
                };
                let kbytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
                let case_id = extract_u32_key_suffix(&kbytes);
                case_ipfs.insert(case_id, ipfs);
            }
        }
    }

    // ── Fetch Rulings (final verdicts) ───────────────────────────────────────
    let ruling_prefix = storage_prefix("Courts", "Rulings");
    let ruling_keys = client
        .get_keys_paged(&ruling_prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    if ruling_keys.is_empty() {
        return Ok(vec![]);
    }
    let ruling_values = client
        .query_storage_at(&ruling_keys)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;

    let mut rulings = Vec::new();
    for (key_hex, val_opt) in ruling_keys.iter().zip(ruling_values.iter()) {
        if let Some(val_hex) = val_opt {
            let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
            let outcome = match bytes.first() {
                Some(0) => "upheld",
                Some(1) => "overturned",
                _ => "unknown",
            };
            let kbytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
            let case_id = extract_u32_key_suffix(&kbytes);
            let ipfs_hash = case_ipfs.get(&case_id).cloned().unwrap_or_default();
            rulings.push(Ruling {
                id: format!("ruling-{case_id}"),
                case_title: format!("Case #{case_id}"),
                level: 0,
                outcome: outcome.to_string(),
                summary: format!("Verdict: {outcome}. Fetch full ruling text from IPFS."),
                ipfs_hash,
                timestamp: 0,
            });
        }
    }
    Ok(rulings)
}

/// Fetches IPFS content for a law, proposal, or ruling by its on-chain SHA-256 hash.
/// Converts the 32-byte digest to a CIDv0 and fetches from the public IPFS gateway.
#[tauri::command]
pub async fn fetch_ipfs_content(hash_hex: String) -> Result<String, String> {
    let hash_bytes = hex::decode(hash_hex.trim_start_matches("0x"))
        .map_err(|e| format!("invalid hash hex: {e}"))?;
    let cid = hash_to_cid(&hash_bytes).ok_or("hash must be exactly 32 bytes")?;
    let url = format!("https://ipfs.io/ipfs/{cid}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(&url).send().await
        .map_err(|e| format!("gateway unreachable: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("gateway returned {}", resp.status()));
    }
    resp.text().await.map_err(|e| e.to_string())
}

// ── Legislature + Elections commands ─────────────────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct LegislatureMotion {
    pub id: u32,
    #[serde(rename = "callHash")]
    pub call_hash: String,
    pub proposer: String,
    pub ayes: u32,
    pub nays: u32,
    #[serde(rename = "endBlock")]
    pub end_block: u64,
    pub executed: bool,
}

#[derive(Serialize, Deserialize)]
pub struct LegislatureData {
    pub members: Vec<String>,
    pub motions: Vec<LegislatureMotion>,
}

/// Fetches Legislature.Members (StorageValue<BoundedVec<AccountId,500>>) and
/// Legislature.Motions (StorageMap<u32, Motion>).
///
/// Motion SCALE layout (77 bytes):
///   call_hash: [u8;32]
///   proposer:  [u8;32]
///   ayes:      u32 LE
///   nays:      u32 LE
///   end_block: u32 LE
///   executed:  bool (1 byte)
#[tauri::command]
pub async fn fetch_legislature_data() -> Result<LegislatureData, String> {
    let client = RpcClient::new(NODE_URL);

    // ── Members (single StorageValue — key IS the prefix) ────────────────────
    let members_key = storage_prefix("Legislature", "Members");
    let members_bytes = client
        .get_storage(&members_key)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let mut members: Vec<String> = Vec::new();
    if let Some(bytes) = members_bytes {
        let (count, offset) = decode_compact(&bytes);
        let mut pos = offset;
        for _ in 0..(count as usize) {
            if pos + 32 <= bytes.len() {
                members.push(format!("0x{}", hex::encode(&bytes[pos..pos + 32])));
                pos += 32;
            }
        }
    }

    // ── Motions (StorageMap<Blake2_128Concat, u32, Motion>) ──────────────────
    let motions_prefix = storage_prefix("Legislature", "Motions");
    let motion_keys = client
        .get_keys_paged(&motions_prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let mut motions: Vec<LegislatureMotion> = Vec::new();
    if !motion_keys.is_empty() {
        let values = client
            .query_storage_at(&motion_keys)
            .await
            .map_err(|e| format!("chain unreachable: {e}"))?;
        for (key_hex, val_opt) in motion_keys.iter().zip(values.iter()) {
            if let Some(val_hex) = val_opt {
                let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
                if bytes.len() < 77 {
                    continue;
                }
                let call_hash = format!("0x{}", hex::encode(&bytes[0..32]));
                let proposer   = format!("0x{}", hex::encode(&bytes[32..64]));
                let ayes       = u32::from_le_bytes([bytes[64], bytes[65], bytes[66], bytes[67]]);
                let nays       = u32::from_le_bytes([bytes[68], bytes[69], bytes[70], bytes[71]]);
                let end_block  = u32::from_le_bytes([bytes[72], bytes[73], bytes[74], bytes[75]]);
                let executed   = bytes[76] != 0;
                let kbytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
                let id = extract_u32_key_suffix(&kbytes);
                motions.push(LegislatureMotion { id, call_hash, proposer, ayes, nays, end_block: end_block as u64, executed });
            }
        }
    }

    Ok(LegislatureData { members, motions })
}

// ─────────────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct Delegate {
    pub account: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "backingCount")]
    pub backing_count: u32,
    pub status: String,
    #[serde(rename = "consecutiveTerms")]
    pub consecutive_terms: u32,
    #[serde(rename = "profileIpfsHash")]
    pub profile_ipfs_hash: String,
}

#[derive(Serialize, Deserialize)]
pub struct ElectionsData {
    pub delegates: Vec<Delegate>,
}

/// Fetches PalletElections.Delegates and PalletElections.BackingCount.
///
/// DelegateInfo SCALE (variable — display_name is a compact-prefixed BoundedVec<u8,64>):
///   display_name:       compact_len + utf8_bytes
///   profile_ipfs_hash:  [u8;32]
///   status:             u8  (0=Pending, 1=Active, 2=OnBreak)
///   consecutive_terms:  u32 LE
///   term_start_block:   Option<u32>  (0x00 | 0x01 + 4 bytes)
///   break_until_block:  Option<u32>  (0x00 | 0x01 + 4 bytes)
///   warning_emitted:    bool
#[tauri::command]
pub async fn fetch_elections_data() -> Result<ElectionsData, String> {
    let client = RpcClient::new(NODE_URL);

    // ── Backing counts keyed by AccountId ─────────────────────────────────────
    let backing_prefix = storage_prefix("PalletElections", "BackingCount");
    let backing_keys = client
        .get_keys_paged(&backing_prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let backing_values = client
        .query_storage_at(&backing_keys)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let mut backing_map: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for (key_hex, val_opt) in backing_keys.iter().zip(backing_values.iter()) {
        if let Some(val_hex) = val_opt {
            let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
            if bytes.len() >= 4 {
                let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                // AccountId occupies the last 32 bytes of the Blake2_128Concat key
                let kbytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
                if kbytes.len() >= 32 {
                    let account = format!("0x{}", hex::encode(&kbytes[kbytes.len() - 32..]));
                    backing_map.insert(account, count);
                }
            }
        }
    }

    // ── Delegates ─────────────────────────────────────────────────────────────
    let delegates_prefix = storage_prefix("PalletElections", "Delegates");
    let delegate_keys = client
        .get_keys_paged(&delegates_prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let mut delegates: Vec<Delegate> = Vec::new();
    if !delegate_keys.is_empty() {
        let values = client
            .query_storage_at(&delegate_keys)
            .await
            .map_err(|e| format!("chain unreachable: {e}"))?;
        for (key_hex, val_opt) in delegate_keys.iter().zip(values.iter()) {
            if let Some(val_hex) = val_opt {
                let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
                let mut pos = 0usize;
                // display_name: compact len + utf8 bytes
                let (name_len, consumed) = decode_compact(&bytes[pos..]);
                pos += consumed;
                let display_name = if pos + name_len as usize <= bytes.len() {
                    let s = String::from_utf8_lossy(&bytes[pos..pos + name_len as usize]).to_string();
                    pos += name_len as usize;
                    s
                } else {
                    String::new()
                };
                // profile_ipfs_hash: [u8;32]
                let profile_ipfs_hash = if pos + 32 <= bytes.len() {
                    let h = format!("0x{}", hex::encode(&bytes[pos..pos + 32]));
                    pos += 32;
                    h
                } else {
                    String::new()
                };
                // status: u8
                let status_str = if pos < bytes.len() {
                    let s = match bytes[pos] { 1 => "active", 2 => "on_break", _ => "pending" };
                    pos += 1;
                    s
                } else {
                    "pending"
                };
                // consecutive_terms: u32 (pos not advanced — nothing further is parsed)
                let consecutive_terms = if pos + 4 <= bytes.len() {
                    u32::from_le_bytes([bytes[pos], bytes[pos+1], bytes[pos+2], bytes[pos+3]])
                } else {
                    0
                };
                // Extract account from key (last 32 bytes of Blake2_128Concat key)
                let kbytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
                let account = if kbytes.len() >= 32 {
                    format!("0x{}", hex::encode(&kbytes[kbytes.len() - 32..]))
                } else {
                    String::new()
                };
                let backing_count = backing_map.get(&account).copied().unwrap_or(0);
                delegates.push(Delegate {
                    account,
                    display_name,
                    backing_count,
                    status: status_str.to_string(),
                    consecutive_terms,
                    profile_ipfs_hash,
                });
            }
        }
    }
    // Sort delegates by backing_count descending
    delegates.sort_by(|a, b| b.backing_count.cmp(&a.backing_count));

    Ok(ElectionsData { delegates })
}

// ── Anti-Corruption commands ──────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct AssetDisclosure {
    pub account: String,
    #[serde(rename = "ipfsHash")]
    pub ipfs_hash: String,
    #[serde(rename = "disclosedAt")]
    pub disclosed_at: u64,
    #[serde(rename = "updateDueAt")]
    pub update_due_at: u64,
}

#[derive(Serialize, Deserialize)]
pub struct ConflictEntry {
    pub account: String,
    #[serde(rename = "entityId")]
    pub entity_id: u32,
    #[serde(rename = "conflictType")]
    pub conflict_type: String,
    #[serde(rename = "registeredAt")]
    pub registered_at: u64,
}

#[derive(Serialize, Deserialize)]
pub struct WhistleblowerReport {
    pub id: u32,
    #[serde(rename = "contentHash")]
    pub content_hash: String,
    #[serde(rename = "submittedAt")]
    pub submitted_at: u64,
    pub status: String,
}

#[derive(Serialize, Deserialize)]
pub struct AntiCorruptionData {
    #[serde(rename = "assetDisclosures")]
    pub asset_disclosures: Vec<AssetDisclosure>,
    pub conflicts: Vec<ConflictEntry>,
    pub reports: Vec<WhistleblowerReport>,
    #[serde(rename = "investigatorCount")]
    pub investigator_count: u32,
}

/// Fetches PalletAntiCorruption.AssetDisclosures, ConflictRegistry, WhistleblowerReports,
/// and the Investigators count.
///
/// AssetDeclaration SCALE (40 bytes): ipfs_hash[32] + disclosed_at:u32 LE + update_due_at:u32 LE
/// Key suffix (Blake2_128Concat<AccountId>): last 32 bytes = account.
///
/// ConflictEntry SCALE (5 bytes): conflict_type:u8 (0=Financial,1=Family,2=FormerEmployer,3=BusinessPartner)
///   + registered_at:u32 LE
/// Key suffix (Blake2_128Concat<(AccountId,u32)>): last 36 bytes = account(32) + entity_id:u32 LE(4).
///
/// WhistleblowerReport SCALE (69 bytes): content_hash[32] + submitted_at:u32 LE + status:u8
///   (0=Pending,1=Flagged,2=UnderInvestigation,3=Cleared,4=ReferredToCourts) + nullifier[32]
/// Key suffix (Blake2_128Concat<u32>): last 4 bytes = report id.
#[tauri::command]
pub async fn fetch_anticorruption_data() -> Result<AntiCorruptionData, String> {
    let client = RpcClient::new(NODE_URL);

    // ── Asset disclosures ─────────────────────────────────────────────────────
    let disclosures_prefix = storage_prefix("PalletAntiCorruption", "AssetDisclosures");
    let disclosure_keys = client
        .get_keys_paged(&disclosures_prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let mut asset_disclosures: Vec<AssetDisclosure> = Vec::new();
    if !disclosure_keys.is_empty() {
        let values = client
            .query_storage_at(&disclosure_keys)
            .await
            .map_err(|e| format!("chain unreachable: {e}"))?;
        for (key_hex, val_opt) in disclosure_keys.iter().zip(values.iter()) {
            if let Some(val_hex) = val_opt {
                let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
                if bytes.len() < 40 {
                    continue;
                }
                let ipfs_hash = format!("0x{}", hex::encode(&bytes[0..32]));
                let disclosed_at =
                    u32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]) as u64;
                let update_due_at =
                    u32::from_le_bytes([bytes[36], bytes[37], bytes[38], bytes[39]]) as u64;
                let kbytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
                let account = if kbytes.len() >= 32 {
                    format!("0x{}", hex::encode(&kbytes[kbytes.len() - 32..]))
                } else {
                    String::new()
                };
                asset_disclosures.push(AssetDisclosure { account, ipfs_hash, disclosed_at, update_due_at });
            }
        }
    }

    // ── Conflict-of-interest registry ────────────────────────────────────────
    let conflicts_prefix = storage_prefix("PalletAntiCorruption", "ConflictRegistry");
    let conflict_keys = client
        .get_keys_paged(&conflicts_prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let mut conflicts: Vec<ConflictEntry> = Vec::new();
    if !conflict_keys.is_empty() {
        let values = client
            .query_storage_at(&conflict_keys)
            .await
            .map_err(|e| format!("chain unreachable: {e}"))?;
        for (key_hex, val_opt) in conflict_keys.iter().zip(values.iter()) {
            if let Some(val_hex) = val_opt {
                let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
                if bytes.len() < 5 {
                    continue;
                }
                let conflict_type = match bytes[0] {
                    0 => "financial_interest",
                    1 => "family_relation",
                    2 => "former_employer",
                    3 => "business_partner",
                    _ => "unknown",
                };
                let registered_at = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as u64;
                let kbytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
                let (account, entity_id) = if kbytes.len() >= 36 {
                    let s = kbytes.len();
                    let account = format!("0x{}", hex::encode(&kbytes[s - 36..s - 4]));
                    let entity_id =
                        u32::from_le_bytes([kbytes[s - 4], kbytes[s - 3], kbytes[s - 2], kbytes[s - 1]]);
                    (account, entity_id)
                } else {
                    (String::new(), 0)
                };
                conflicts.push(ConflictEntry {
                    account,
                    entity_id,
                    conflict_type: conflict_type.to_string(),
                    registered_at,
                });
            }
        }
    }

    // ── Whistleblower reports ────────────────────────────────────────────────
    let reports_prefix = storage_prefix("PalletAntiCorruption", "WhistleblowerReports");
    let report_keys = client
        .get_keys_paged(&reports_prefix)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let mut reports: Vec<WhistleblowerReport> = Vec::new();
    if !report_keys.is_empty() {
        let values = client
            .query_storage_at(&report_keys)
            .await
            .map_err(|e| format!("chain unreachable: {e}"))?;
        for (key_hex, val_opt) in report_keys.iter().zip(values.iter()) {
            if let Some(val_hex) = val_opt {
                let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
                if bytes.len() < 37 {
                    continue;
                }
                let content_hash = format!("0x{}", hex::encode(&bytes[0..32]));
                let submitted_at = u32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]) as u64;
                let status = match bytes[36] {
                    0 => "pending",
                    1 => "flagged",
                    2 => "under_investigation",
                    3 => "cleared",
                    4 => "referred_to_courts",
                    _ => "unknown",
                };
                let kbytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
                let id = extract_u32_key_suffix(&kbytes);
                reports.push(WhistleblowerReport {
                    id,
                    content_hash,
                    submitted_at,
                    status: status.to_string(),
                });
            }
        }
    }
    reports.sort_by(|a, b| b.id.cmp(&a.id));

    // ── Investigator count (StorageValue<BoundedVec<AccountId, MaxInvestigators>>) ──
    let investigators_key = storage_prefix("PalletAntiCorruption", "Investigators");
    let investigators_bytes = client
        .get_storage(&investigators_key)
        .await
        .map_err(|e| format!("chain unreachable: {e}"))?;
    let investigator_count = investigators_bytes
        .map(|bytes| decode_compact(&bytes).0)
        .unwrap_or(0);

    Ok(AntiCorruptionData { asset_disclosures, conflicts, reports, investigator_count })
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Converts a raw 32-byte SHA-256 digest to an IPFS CIDv0 string.
fn hash_to_cid(hash_bytes: &[u8]) -> Option<String> {
    if hash_bytes.len() != 32 {
        return None;
    }
    let mut multihash = Vec::with_capacity(34);
    multihash.push(0x12u8); // sha2-256 function code
    multihash.push(0x20u8); // digest length = 32
    multihash.extend_from_slice(hash_bytes);
    Some(bs58::encode(multihash).into_string())
}

/// Decode a SCALE compact-encoded integer from the start of `bytes`.
/// Returns (value, bytes_consumed).
fn decode_compact(bytes: &[u8]) -> (u32, usize) {
    if bytes.is_empty() {
        return (0, 0);
    }
    match bytes[0] & 0b11 {
        0 => ((bytes[0] >> 2) as u32, 1),
        1 => {
            if bytes.len() < 2 { return (0, 2); }
            ((u16::from_le_bytes([bytes[0], bytes[1]]) >> 2) as u32, 2)
        }
        2 => {
            if bytes.len() < 4 { return (0, 4); }
            (u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) >> 2, 4)
        }
        _ => (0, 1), // big-integer mode — not used by these pallets
    }
}

/// Extracts a u32 key from the last 4 bytes of a Blake2_128Concat storage key.
/// Blake2_128Concat layout: prefix(32) + hash(16) + key_bytes(4) → suffix is the raw key.
fn extract_u32_key_suffix(key_bytes: &[u8]) -> u32 {
    if key_bytes.len() >= 4 {
        let s = key_bytes.len();
        u32::from_le_bytes([key_bytes[s-4], key_bytes[s-3], key_bytes[s-2], key_bytes[s-1]])
    } else {
        0
    }
}

/// Format a u128 Balance (in Planck = 1e-12 AGR) as a human-readable AGR string.
/// 1 AGR = 1_000_000_000_000 Planck (12 decimal places, same as DOT/KSM).
fn format_agr(planck: u128) -> String {
    if planck == 0 {
        return "0 AGR".to_string();
    }
    const UNIT: u128 = 1_000_000_000_000;
    let whole = planck / UNIT;
    let frac = planck % UNIT;
    if frac == 0 {
        format!("{whole} AGR")
    } else {
        // Show up to 4 decimal places
        let frac4 = frac / (UNIT / 10_000);
        format!("{whole}.{frac4:04} AGR")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Finding 1: `chain_status` must surface a real error — not a fake
    /// `{best: 0, finalized: 0}` success — when the node is unreachable. Port 1 is
    /// reserved/unused and connections to it are refused immediately and deterministically,
    /// so this doesn't depend on anything actually running (or not running) locally.
    #[tokio::test]
    async fn chain_status_surfaces_disconnected_state_not_fake_success() {
        let result = fetch_chain_status("http://127.0.0.1:1").await;
        assert!(
            result.is_err(),
            "expected an Err distinct from a fake {{best: 0, finalized: 0}} success when unreachable"
        );
    }

    #[test]
    fn format_agr_formats_whole_and_fractional_amounts() {
        assert_eq!(format_agr(0), "0 AGR");
        assert_eq!(format_agr(1_000_000_000_000), "1 AGR");
        assert_eq!(format_agr(1_500_000_000_000), "1.5000 AGR");
    }
}
