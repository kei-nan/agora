use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::State;
use uuid::Uuid;

use crate::commands::chain;

/// How long an issued bearer token stays valid. Matches the previous (unenforced) `expiresAt`
/// window this replaces.
const SESSION_TTL_SECS: u64 = 86_400;

/// Signing context `sp-core::sr25519`/`@polkadot/keyring` bakes into every sr25519 signature.
/// `mobile/src/chain/identity.ts`'s `getSigningKeypair()` returns a `Keyring({ type: 'sr25519' })`
/// pair and signs with plain `keypair.sign(...)`, which uses this same fixed context under the
/// hood — verification must use the identical context or every legitimate signature fails.
const SR25519_SIGNING_CONTEXT: &[u8] = b"substrate";

#[derive(Serialize, Deserialize, Clone)]
pub struct Session {
    #[serde(rename = "nullifierHash")]
    pub nullifier_hash: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: u64,
    /// Opaque bearer token for read+submit actions (CLAUDE.md's desktop auth design). The
    /// frontend holds this only to pass it back on privileged commands — `SessionStore` on the
    /// Rust side is the actual authority on whether it's valid; nothing should be inferred from
    /// its mere presence in frontend state.
    pub token: String,
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

/// A validated session, keyed by bearer token. This — not any frontend state — is the source of
/// truth for "is this caller authenticated," and it is only ever populated after a signature
/// verified against the identity's on-chain-registered public key (see `verify_challenge_signature`
/// and `handle_auth_callback`).
#[derive(Clone)]
pub struct SessionRecord {
    pub nullifier_hash: String,
    pub expires_at: u64,
}

#[derive(Clone)]
pub struct SessionStore(pub Arc<Mutex<HashMap<String, SessionRecord>>>);

impl SessionStore {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::new())))
    }
}

/// Validates a bearer token against the server-held `SessionStore`. This is the check every
/// privileged command (e.g. a submit-transaction passthrough) must call before acting — an
/// expired or unknown token is rejected here, not merely flagged in the UI. Returns the
/// session's nullifier hash on success.
pub fn require_valid_session(store: &SessionStore, token: &str) -> Result<String, String> {
    let map = store.0.lock().map_err(|e| e.to_string())?;
    match map.get(token) {
        Some(record) if record.expires_at > unix_now() => Ok(record.nullifier_hash.clone()),
        Some(_) => Err("session expired".into()),
        None => Err("invalid or unknown session token".into()),
    }
}

/// Posted by the mobile app to the local callback server once the QR is scanned.
#[derive(Deserialize)]
struct AuthCallback {
    challenge: String,
    #[serde(rename = "nullifierHash")]
    nullifier_hash: String,
    /// Sr25519 signature over the raw challenge UTF-8 bytes, hex-encoded. (Historically
    /// documented here as "Ed25519" and left unverified — it's actually sr25519: mobile's
    /// `AuthScreen.tsx` signs with the same `sr25519` `KeyringPair` `getSigningKeypair()`
    /// returns for every other chain call. See `verify_challenge_signature`.)
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
    sessions: State<'_, PendingSessions>,
    session_store: State<'_, SessionStore>,
) -> Result<(), String> {
    let sessions = sessions.0.clone();
    let session_store = session_store.0.clone();
    tokio::spawn(async move {
        run_callback_server(port, sessions, session_store).await;
    });
    Ok(())
}

async fn run_callback_server(
    port: u16,
    sessions: Arc<Mutex<HashMap<String, Option<Session>>>>,
    session_store: Arc<Mutex<HashMap<String, SessionRecord>>>,
) {
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
        let session_store = session_store.clone();
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
                Ok(cb) => handle_auth_callback(cb, &sessions, &session_store).await,
                Err(_) => ("400 Bad Request", "{\"error\":\"invalid json\"}".to_string()),
            };

            // No `Access-Control-Allow-Origin` header: this endpoint is only ever POSTed to by
            // the mobile app's native `fetch` (React Native, not a browser page — see
            // AuthScreen.tsx), and the desktop frontend never reads it cross-origin either (it
            // polls via `auth_poll_session`, a Tauri IPC call, not HTTP). A CORS header does
            // nothing to authenticate the caller — it only controls whether a *browser* is
            // allowed to read the response — so a wildcard here bought no real protection while
            // advertising this sensitive local endpoint as fetchable from any web page's JS.
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\n\r\n{body_text}",
                len = body_text.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

/// Validates an auth callback and, on success, mints a bearer token held server-side in
/// `session_store`. This is the actual security boundary for the QR-auth flow: a session is
/// only ever created after `verify_challenge_signature` succeeds against the public key
/// on-chain for `nullifier_hash` (fetched independently via `chain::lookup_registered_account`,
/// never trusted from the request body).
async fn handle_auth_callback(
    cb: AuthCallback,
    sessions: &Arc<Mutex<HashMap<String, Option<Session>>>>,
    session_store: &Arc<Mutex<HashMap<String, SessionRecord>>>,
) -> (&'static str, String) {
    let challenge_known = sessions
        .lock()
        .map(|m| m.contains_key(&cb.challenge))
        .unwrap_or(false);
    if !challenge_known {
        return ("404 Not Found", "{\"error\":\"unknown challenge\"}".into());
    }

    let nullifier = match parse_nullifier(&cb.nullifier_hash) {
        Some(n) => n,
        None => {
            return (
                "400 Bad Request",
                "{\"error\":\"invalid nullifierHash\"}".into(),
            )
        }
    };

    // Look up the AccountId actually registered on-chain for this nullifier. `AccountId`s
    // derived from an sr25519 signer are the raw sr25519 public key bytes (Substrate's
    // `MultiSigner::into_account` wraps sr25519/ed25519 pubkeys directly, no hashing) — that's
    // exactly the key `verify_challenge_signature` needs.
    let pubkey = match chain::lookup_registered_account(&nullifier).await {
        Ok(Some(pk)) => pk,
        Ok(None) => {
            return (
                "403 Forbidden",
                "{\"error\":\"identity not registered on-chain\"}".into(),
            )
        }
        Err(e) => {
            eprintln!("[auth] chain lookup failed for callback: {e}");
            return (
                "502 Bad Gateway",
                "{\"error\":\"chain lookup failed\"}".into(),
            );
        }
    };

    if !verify_challenge_signature(&pubkey, &cb.challenge, &cb.signature) {
        eprintln!(
            "[auth] signature verification FAILED for challenge {} — rejecting callback",
            cb.challenge
        );
        return (
            "401 Unauthorized",
            "{\"error\":\"invalid signature\"}".into(),
        );
    }

    let token = new_bearer_token();
    let expires_at = unix_now() + SESSION_TTL_SECS;
    let session = Session {
        nullifier_hash: cb.nullifier_hash.clone(),
        expires_at,
        token: token.clone(),
    };

    let sessions_lock = sessions.lock();
    let store_lock = session_store.lock();
    match (sessions_lock, store_lock) {
        (Ok(mut map), Ok(mut store)) => {
            map.insert(cb.challenge, Some(session));
            store.insert(
                token,
                SessionRecord {
                    nullifier_hash: cb.nullifier_hash,
                    expires_at,
                },
            );
            ("200 OK", "{\"ok\":true}".into())
        }
        _ => (
            "500 Internal Server Error",
            "{\"error\":\"lock poisoned\"}".into(),
        ),
    }
}

fn parse_nullifier(hex_str: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(hex_str.trim_start_matches("0x")).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(arr)
}

/// Verifies an sr25519 signature — the scheme the mobile client actually uses (see
/// `AuthCallback::signature`'s doc comment) — over `challenge`'s raw UTF-8 bytes, matching
/// `AuthScreen.tsx`'s `keypair.sign(Buffer.from(challenge, "utf8"))`.
fn verify_challenge_signature(pubkey: &[u8; 32], challenge: &str, signature_hex: &str) -> bool {
    let Ok(sig_bytes) = hex::decode(signature_hex.trim_start_matches("0x")) else {
        return false;
    };
    let Ok(public) = schnorrkel::PublicKey::from_bytes(pubkey) else {
        return false;
    };
    let Ok(signature) = schnorrkel::Signature::from_bytes(&sig_bytes) else {
        return false;
    };
    public
        .verify_simple(SR25519_SIGNING_CONTEXT, challenge.as_bytes(), &signature)
        .is_ok()
}

/// A fresh high-entropy bearer token. Two concatenated v4 UUIDs (~244 bits) rather than one —
/// this is a long-lived (`SESSION_TTL_SECS`) credential handed to privileged commands, not a
/// single-use nonce, so it's worth the extra margin over a single UUID's 122 bits.
fn new_bearer_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod signature_tests {
    use super::*;

    /// A signature made by a real sr25519 keypair, over the exact challenge bytes and with the
    /// exact "substrate" signing context, must verify — this is the path every legitimate
    /// mobile login takes.
    #[test]
    fn accepts_a_genuine_sr25519_signature() {
        let keypair = schnorrkel::Keypair::generate();
        let pubkey = keypair.public.to_bytes();
        let challenge = "11111111-2222-3333-4444-555555555555";
        let signature = keypair.sign_simple(SR25519_SIGNING_CONTEXT, challenge.as_bytes());
        let signature_hex = hex::encode(signature.to_bytes());

        assert!(verify_challenge_signature(&pubkey, challenge, &signature_hex));
    }

    /// This is the actual bug the review found: previously the signature was parsed and then
    /// discarded entirely, so a forged/garbage signature would have been accepted right along
    /// with a real one as long as the challenge UUID was known. It must now be rejected.
    #[test]
    fn rejects_forged_signature_for_known_challenge_and_pubkey() {
        let keypair = schnorrkel::Keypair::generate();
        let pubkey = keypair.public.to_bytes();
        let challenge = "11111111-2222-3333-4444-555555555555";
        // 64 arbitrary bytes — the right length, but not a valid signature over anything.
        let forged_hex = hex::encode([0x42u8; 64]);

        assert!(!verify_challenge_signature(&pubkey, challenge, &forged_hex));
    }

    #[test]
    fn rejects_signature_for_a_different_challenge() {
        let keypair = schnorrkel::Keypair::generate();
        let pubkey = keypair.public.to_bytes();
        let signed_challenge = "aaaa";
        let signature = keypair.sign_simple(SR25519_SIGNING_CONTEXT, signed_challenge.as_bytes());
        let signature_hex = hex::encode(signature.to_bytes());

        assert!(!verify_challenge_signature(&pubkey, "bbbb", &signature_hex));
    }

    #[test]
    fn rejects_signature_from_a_different_keypair() {
        let signer = schnorrkel::Keypair::generate();
        let attacker_claimed_owner = schnorrkel::Keypair::generate();
        let challenge = "same-challenge";
        let signature = signer.sign_simple(SR25519_SIGNING_CONTEXT, challenge.as_bytes());
        let signature_hex = hex::encode(signature.to_bytes());

        // Verifying against the wrong (claimed) identity's registered pubkey must fail even
        // though the signature itself is validly formed.
        assert!(!verify_challenge_signature(
            &attacker_claimed_owner.public.to_bytes(),
            challenge,
            &signature_hex
        ));
    }

    #[test]
    fn rejects_malformed_hex_signature() {
        let keypair = schnorrkel::Keypair::generate();
        let pubkey = keypair.public.to_bytes();
        assert!(!verify_challenge_signature(&pubkey, "chal", "not-valid-hex!!"));
        assert!(!verify_challenge_signature(&pubkey, "chal", "ab")); // too short to be a sig
    }

    #[test]
    fn require_valid_session_rejects_unknown_and_expired_tokens() {
        let store = SessionStore::new();
        assert!(require_valid_session(&store, "nonexistent-token").is_err());

        store.0.lock().unwrap().insert(
            "expired-token".into(),
            SessionRecord {
                nullifier_hash: "0xabc".into(),
                expires_at: 1, // way in the past
            },
        );
        assert!(require_valid_session(&store, "expired-token").is_err());

        store.0.lock().unwrap().insert(
            "live-token".into(),
            SessionRecord {
                nullifier_hash: "0xabc".into(),
                expires_at: unix_now() + 3600,
            },
        );
        assert_eq!(
            require_valid_session(&store, "live-token").unwrap(),
            "0xabc"
        );
    }
}
