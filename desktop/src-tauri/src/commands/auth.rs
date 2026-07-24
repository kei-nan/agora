use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::State;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone)]
pub struct Session {
    #[serde(rename = "nullifierHash")]
    pub nullifier_hash: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: u64,
}

/// Shared session store: challenge UUID → completed Session (None while pending).
/// Wrapped in Arc so the background callback server can update it without unsafe.
#[derive(Clone)]
pub struct PendingSessions(pub Arc<Mutex<HashMap<String, Option<Session>>>>);

impl PendingSessions {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::new())))
    }
}

/// Posted by the mobile app to the local callback server once the QR is scanned.
#[derive(Deserialize)]
struct AuthCallback {
    challenge: String,
    #[serde(rename = "nullifierHash")]
    nullifier_hash: String,
    /// Ed25519 signature over the challenge bytes, hex-encoded (reserved for future chain verification).
    #[allow(dead_code)]
    signature: String,
}

/// Generate a one-time challenge and return the QR deep-link URL.
///
/// URL: `democracychain://auth?challenge=<uuid>&callback=http://127.0.0.1:<port>/auth`
/// The `port` parameter is chosen by the frontend (random 12000–12999) from
/// `auth_start_callback_server`, which must be called first.
#[tauri::command]
pub async fn auth_generate_challenge(
    state: State<'_, PendingSessions>,
    port: u16,
) -> Result<String, String> {
    let challenge = Uuid::new_v4().to_string();
    let deep_link = format!(
        "democracychain://auth?challenge={challenge}&callback=http://127.0.0.1:{port}/auth"
    );
    state
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .insert(challenge, None);
    Ok(deep_link)
}

/// Poll for a completed auth session. Returns "pending" error if the phone hasn't responded yet.
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

/// Start the local HTTP callback server on the given port.
/// Must be called once on app startup. The port is embedded in QR codes so the
/// mobile app knows where to POST the signed auth response.
///
/// The mobile POSTs JSON: `{ challenge, nullifierHash, signature }`
/// On success the challenge is marked complete; `auth_poll_session` will return the session.
#[tauri::command]
pub async fn auth_start_callback_server(
    port: u16,
    state: State<'_, PendingSessions>,
) -> Result<(), String> {
    let sessions = state.0.clone();
    tokio::spawn(async move {
        run_callback_server(port, sessions).await;
    });
    Ok(())
}

async fn run_callback_server(port: u16, sessions: Arc<Mutex<HashMap<String, Option<Session>>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let addr = format!("127.0.0.1:{port}");
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[auth] callback server bind failed on {addr}: {e}");
            return;
        }
    };
    eprintln!("[auth] callback server listening on {addr}");

    loop {
        let (mut stream, _peer) = match listener.accept().await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let sessions = sessions.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            let n = match stream.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => return,
            };
            let raw = String::from_utf8_lossy(&buf[..n]);

            // Minimal HTTP parser: extract body after the blank header line.
            let body = raw
                .split("\r\n\r\n")
                .nth(1)
                .unwrap_or("")
                .trim()
                .to_string();

            let (status, body_text) = match serde_json::from_str::<AuthCallback>(&body) {
                Ok(cb) => {
                    let session = Session {
                        nullifier_hash: cb.nullifier_hash,
                        expires_at: unix_now() + 86400,
                    };
                    if let Ok(mut map) = sessions.lock() {
                        if map.contains_key(&cb.challenge) {
                            map.insert(cb.challenge, Some(session));
                            ("200 OK", "{\"ok\":true}")
                        } else {
                            ("404 Not Found", "{\"error\":\"unknown challenge\"}")
                        }
                    } else {
                        ("500 Internal Server Error", "{\"error\":\"lock poisoned\"}")
                    }
                }
                Err(_) => ("400 Bad Request", "{\"error\":\"invalid json\"}"),
            };

            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nAccess-Control-Allow-Origin: *\r\n\r\n{body_text}",
                len = body_text.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
