use serde::{Deserialize, Serialize};

// These structs mirror the TypeScript interfaces in the frontend.
// Once smoldot integration is wired, these will be populated from on-chain storage.

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

/// Returns the current best and finalized block numbers.
/// TODO: replace stub with smoldot light client query.
#[tauri::command]
pub async fn chain_status() -> Result<ChainStatusResponse, String> {
    // Stub — smoldot integration goes here.
    Ok(ChainStatusResponse {
        best: 0,
        finalized: 0,
    })
}

/// Fetches active and recent proposals from on-chain storage.
/// TODO: query pallet-voting storage via smoldot.
#[tauri::command]
pub async fn fetch_proposals() -> Result<Vec<Proposal>, String> {
    Ok(vec![])
}

/// Fetches enacted laws from pallet-constitution.
/// TODO: query pallet-constitution storage via smoldot.
#[tauri::command]
pub async fn fetch_laws() -> Result<Vec<Law>, String> {
    Ok(vec![])
}

/// Fetches treasury ledger entries from pallet-treasury-ledger.
/// TODO: query pallet-treasury-ledger storage via smoldot.
#[tauri::command]
pub async fn fetch_treasury() -> Result<Vec<TreasuryEntry>, String> {
    Ok(vec![])
}

/// Fetches court rulings from pallet-courts.
/// TODO: query pallet-courts storage via smoldot.
#[tauri::command]
pub async fn fetch_rulings() -> Result<Vec<Ruling>, String> {
    Ok(vec![])
}
