use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone)]
pub struct Session {
    #[serde(rename = "nullifierHash")]
    pub nullifier_hash: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: u64,
}

/// In-memory store: challenge token → completed session (if phone has responded).
/// In production this would be persisted via tauri-plugin-store and verified
/// against the on-chain nullifier registry.
pub struct PendingSessions(pub Mutex<HashMap<String, Option<Session>>>);

/// Generates a one-time challenge token and encodes it as a deep-link URL
/// that the mobile app can scan from the QR code.
///
/// Deep-link format: democracychain://auth?challenge=<uuid>
/// The mobile app signs the challenge with the hardware-backed session key and
/// POSTs back to the local loopback server (or via direct IPC in production).
#[tauri::command]
pub async fn auth_generate_challenge(state: State<'_, PendingSessions>) -> Result<String, String> {
    let challenge = Uuid::new_v4().to_string();
    let deep_link = format!("democracychain://auth?challenge={challenge}");

    state
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .insert(challenge, None);

    Ok(deep_link)
}

/// Polls whether the mobile app has completed the auth challenge.
/// Returns the session if complete, errors with "pending" if not yet done.
///
/// TODO: replace stub with actual token verification against the chain's
/// nullifier registry once pallet-identity is live.
#[tauri::command]
pub async fn auth_poll_session(
    challenge: String,
    state: State<'_, PendingSessions>,
) -> Result<Session, String> {
    let map = state.0.lock().map_err(|e| e.to_string())?;
    match map.get(&challenge) {
        Some(Some(session)) => Ok(session.clone()),
        Some(None) => Err("pending".into()),
        None => Err("unknown challenge".into()),
    }
}
