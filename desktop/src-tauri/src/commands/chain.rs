use serde::{Deserialize, Serialize};
use crate::rpc::{RpcClient, storage_prefix};
use std::collections::HashMap;
use std::time::Duration;

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
#[tauri::command]
pub async fn chain_status() -> Result<ChainStatusResponse, String> {
    let client = RpcClient::new(NODE_URL);
    match client.chain_block_numbers().await {
        Ok((best, finalized)) => Ok(ChainStatusResponse { best, finalized }),
        Err(_) => Ok(ChainStatusResponse { best: 0, finalized: 0 }),
    }
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
    let ref_keys = client.get_keys_paged(&ref_prefix).await.unwrap_or_default();

    // ── Fetch tallies keyed by referendum_id ─────────────────────────────────
    let tally_prefix = storage_prefix("Voting", "ReferendumTally");
    let tally_keys = client.get_keys_paged(&tally_prefix).await.unwrap_or_default();
    let tally_values = client
        .query_storage_at(&tally_keys)
        .await
        .unwrap_or_default();

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
        .unwrap_or_default();

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
    let keys = client.get_keys_paged(&prefix).await.unwrap_or_default();
    if keys.is_empty() {
        return Ok(vec![]);
    }
    let values = client.query_storage_at(&keys).await.unwrap_or_default();
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
    let keys = client.get_keys_paged(&prefix).await.unwrap_or_default();
    if keys.is_empty() {
        return Ok(vec![]);
    }
    let values = client.query_storage_at(&keys).await.unwrap_or_default();
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

    let budget_keys = client.get_keys_paged(&budget_prefix).await.unwrap_or_default();
    let spent_keys  = client.get_keys_paged(&spent_prefix).await.unwrap_or_default();

    let budget_vals = client.query_storage_at(&budget_keys).await.unwrap_or_default();
    let spent_vals  = client.query_storage_at(&spent_keys).await.unwrap_or_default();

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
    let keys = client.get_keys_paged(&prefix).await.unwrap_or_default();
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
    let case_keys = client.get_keys_paged(&cases_prefix).await.unwrap_or_default();
    let case_values = client.query_storage_at(&case_keys).await.unwrap_or_default();

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
    let ruling_keys = client.get_keys_paged(&ruling_prefix).await.unwrap_or_default();
    if ruling_keys.is_empty() {
        return Ok(vec![]);
    }
    let ruling_values = client.query_storage_at(&ruling_keys).await.unwrap_or_default();

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
