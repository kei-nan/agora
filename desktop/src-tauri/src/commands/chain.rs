use serde::{Deserialize, Serialize};
use crate::rpc::{RpcClient, storage_prefix};

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
/// Falls back to zeros if the chain is not reachable.
#[tauri::command]
pub async fn chain_status() -> Result<ChainStatusResponse, String> {
    let client = RpcClient::new(NODE_URL);
    match client.chain_block_numbers().await {
        Ok((best, finalized)) => Ok(ChainStatusResponse { best, finalized }),
        Err(_) => Ok(ChainStatusResponse { best: 0, finalized: 0 }),
    }
}

/// Fetches active proposals from pallet-voting storage.
/// Proposal titles come from IPFS (not stored on-chain); the frontend receives
/// the raw on-chain data and resolves content lazily.
#[tauri::command]
pub async fn fetch_proposals() -> Result<Vec<Proposal>, String> {
    let client = RpcClient::new(NODE_URL);
    // "Voting" is the runtime pallet alias (pallet_index 9); "Proposals" is the storage map.
    let prefix = storage_prefix("Voting", "Proposals");
    let keys = match client.get_keys_paged(&prefix).await {
        Ok(k) => k,
        Err(_) => return Ok(vec![]),
    };
    if keys.is_empty() {
        return Ok(vec![]);
    }
    let values = client.query_storage_at(&keys).await.unwrap_or_default();
    let mut proposals = Vec::new();
    for (i, (key_hex, val_opt)) in keys.iter().zip(values.iter()).enumerate() {
        if let Some(val_hex) = val_opt {
            // Value is BlockNumberFor<T> = u32 LE (4 bytes).
            let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
            if bytes.len() >= 4 {
                let end_block = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                // Extract proposal_id from the last 4 bytes of the key after the 32-byte prefix
                // Key = 0x + 16-byte pallet hash + 16-byte storage hash + Blake2_128Concat(key)
                // Blake2_128Concat prepends 16 bytes of hash then the key itself.
                let key_bytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
                let proposal_id = if key_bytes.len() >= 36 {
                    u32::from_le_bytes([
                        key_bytes[32], key_bytes[33], key_bytes[34], key_bytes[35],
                    ])
                } else {
                    i as u32
                };
                proposals.push(Proposal {
                    id: format!("prop-{proposal_id}"),
                    title: format!("Proposal #{proposal_id}"),
                    status: "active".to_string(),
                    proposer: String::new(),
                    votes_for: 0,
                    votes_against: 0,
                    ends_at: end_block as u64,
                    ipfs_hash: String::new(),
                    summary: format!("Voting closes at block {end_block}. Fetch full text from IPFS."),
                });
            }
        }
    }
    Ok(proposals)
}

/// Fetches enacted laws from pallet-constitution storage.
/// SCALE layout: LawTier(1) | LawStatus(1) | version(u32 LE) | content_hash([u8;32])
#[tauri::command]
pub async fn fetch_laws() -> Result<Vec<Law>, String> {
    let client = RpcClient::new(NODE_URL);
    let prefix = storage_prefix("Constitution", "Laws");
    let keys = match client.get_keys_paged(&prefix).await {
        Ok(k) => k,
        Err(_) => return Ok(vec![]),
    };
    if keys.is_empty() {
        return Ok(vec![]);
    }
    let values = client.query_storage_at(&keys).await.unwrap_or_default();
    let mut laws = Vec::new();
    for (i, (key_hex, val_opt)) in keys.iter().zip(values.iter()).enumerate() {
        if let Some(val_hex) = val_opt {
            let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
            // bytes[0] = LawTier enum, bytes[1] = LawStatus enum,
            // bytes[2..6] = version u32 LE, bytes[6..38] = content_hash
            if bytes.len() >= 38 {
                let tier = match bytes[0] { 0 => "ordinary", _ => "constitutional" };
                let status = match bytes[1] { 0 => "active", 1 => "paused", _ => "repealed" };
                let version = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
                let ipfs_hash = format!("0x{}", hex::encode(&bytes[6..38]));

                let key_bytes = hex::decode(key_hex.trim_start_matches("0x")).unwrap_or_default();
                let law_id = if key_bytes.len() >= 36 {
                    u32::from_le_bytes([
                        key_bytes[32], key_bytes[33], key_bytes[34], key_bytes[35],
                    ])
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
                    summary: format!("Status: {status}. v{version}. Fetch full text via IPFS hash above."),
                });
            }
        }
    }
    Ok(laws)
}

/// Fetches treasury expenditure records from pallet-treasury-ledger.
/// SCALE layout: dept_id(u32 LE) | amount(u128 compact) | metadata_hash([u8;32])
#[tauri::command]
pub async fn fetch_treasury() -> Result<Vec<TreasuryEntry>, String> {
    let client = RpcClient::new(NODE_URL);
    let prefix = storage_prefix("TreasuryLedger", "ExpenditureLog");
    let keys = match client.get_keys_paged(&prefix).await {
        Ok(k) => k,
        Err(_) => return Ok(vec![]),
    };
    if keys.is_empty() {
        return Ok(vec![]);
    }
    let values = client.query_storage_at(&keys).await.unwrap_or_default();
    let mut entries = Vec::new();
    for (i, val_opt) in values.iter().enumerate() {
        if let Some(val_hex) = val_opt {
            let bytes = hex::decode(val_hex.trim_start_matches("0x")).unwrap_or_default();
            if bytes.len() >= 4 {
                let dept_id = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                // Balance is compact-encoded u128; skip it and grab the trailing 32-byte hash.
                let ipfs_hash = if bytes.len() >= 37 {
                    format!("0x{}", hex::encode(&bytes[bytes.len() - 32..]))
                } else {
                    String::new()
                };
                entries.push(TreasuryEntry {
                    id: format!("tx-{i}"),
                    department: format!("Department {dept_id}"),
                    amount: "—".to_string(),
                    currency: "AGR".to_string(),
                    description: "On-chain expenditure record.".to_string(),
                    timestamp: 0,
                    ipfs_hash,
                });
            }
        }
    }
    Ok(entries)
}

/// Fetches court rulings from pallet-courts storage.
/// Case data requires IPFS for ruling text; returns skeleton metadata.
#[tauri::command]
pub async fn fetch_rulings() -> Result<Vec<Ruling>, String> {
    let client = RpcClient::new(NODE_URL);
    let prefix = storage_prefix("Courts", "Rulings");
    let keys = match client.get_keys_paged(&prefix).await {
        Ok(k) => k,
        Err(_) => return Ok(vec![]),
    };
    if keys.is_empty() {
        return Ok(vec![]);
    }
    let values = client.query_storage_at(&keys).await.unwrap_or_default();
    let mut rulings = Vec::new();
    for (i, val_opt) in values.iter().enumerate() {
        if let Some(_val_hex) = val_opt {
            // Verdict enum: 0 = Upheld, 1 = Overturned
            let bytes = hex::decode(_val_hex.trim_start_matches("0x")).unwrap_or_default();
            let outcome = match bytes.first() {
                Some(0) => "upheld",
                Some(1) => "overturned",
                _ => "unknown",
            };
            rulings.push(Ruling {
                id: format!("ruling-{i}"),
                case_title: format!("Case #{i}"),
                level: 0,
                outcome: outcome.to_string(),
                summary: "Fetch full ruling from IPFS hash stored in case record.".to_string(),
                ipfs_hash: String::new(),
                timestamp: 0,
            });
        }
    }
    Ok(rulings)
}
