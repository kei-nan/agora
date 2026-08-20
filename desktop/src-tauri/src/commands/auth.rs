use serde::{Deserialize, Serialize};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::State;
use uuid::Uuid;

use crate::account_pin::{AccountPinStore, PinCheckResult};
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

/// A QR-auth challenge slot: pending until the mobile app POSTs back a completed `Session`.
#[derive(Clone)]
struct ChallengeEntry {
    /// Unix timestamp the challenge was generated at — used to enforce `CHALLENGE_TTL_SECS`.
    created_at: u64,
    session: Option<Session>,
}

/// How long an unconfirmed QR challenge stays valid. Must match the frontend's own
/// 5-minute QR timeout (`AuthContext.tsx`'s `requestQr` — `setTimeout(..., 5 * 60 * 1000)`)
/// so the server-side cutoff and the UI cutoff agree. This is enforced here, not just in
/// the frontend, so a late POST to the callback server can't complete a stale challenge.
const CHALLENGE_TTL_SECS: u64 = 5 * 60;

/// Shared session store: challenge UUID → challenge/session state.
/// Wrapped in Arc so the background callback server can update it without unsafe.
#[derive(Clone)]
pub struct PendingSessions(Arc<Mutex<HashMap<String, ChallengeEntry>>>);

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
    let mut map = store.0.lock().map_err(|e| e.to_string())?;
    let now = unix_now();
    // Opportunistic prune: bearer-token sessions past expiry are dropped on every check,
    // rather than accumulating for the process lifetime the way PendingSessions used to.
    map.retain(|_, record| record.expires_at > now);
    match map.get(token) {
        Some(record) if record.expires_at > now => Ok(record.nullifier_hash.clone()),
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

/// Removes entries that are no longer useful: pending challenges older than
/// `CHALLENGE_TTL_SECS` (never completed in time) and completed sessions whose bearer
/// token (`expires_at`) has already passed. Without this the map grows for the lifetime
/// of the process, since nothing else ever removes a completed/expired entry.
fn prune_expired_at(map: &mut HashMap<String, ChallengeEntry>, now: u64) {
    map.retain(|_, entry| match &entry.session {
        Some(session) => session.expires_at > now,
        None => now.saturating_sub(entry.created_at) < CHALLENGE_TTL_SECS,
    });
}

fn prune_expired(map: &mut HashMap<String, ChallengeEntry>) {
    prune_expired_at(map, unix_now());
}

/// Attempts to mark `challenge` complete with `session`, honoring `CHALLENGE_TTL_SECS`.
/// This is the server-side enforcement the frontend's own 5-minute UI timeout can't
/// provide on its own — a late POST from the mobile app for a challenge that's aged out
/// (or that doesn't exist, or that was already completed) is rejected here rather than
/// silently accepted.
fn complete_challenge_at(
    map: &mut HashMap<String, ChallengeEntry>,
    challenge: String,
    session: Session,
    now: u64,
) -> Result<(), &'static str> {
    prune_expired_at(map, now);
    match map.entry(challenge) {
        Entry::Occupied(mut o) if o.get().session.is_none() => {
            let created_at = o.get().created_at;
            o.insert(ChallengeEntry { created_at, session: Some(session) });
            Ok(())
        }
        Entry::Occupied(_) => Err("already_completed"),
        Entry::Vacant(_) => Err("expired_or_unknown"),
    }
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
    let mut map = state.0.lock().map_err(|e| e.to_string())?;
    prune_expired(&mut map);
    map.insert(challenge, ChallengeEntry { created_at: unix_now(), session: None });
    Ok(deep_link)
}

/// Poll for a completed auth session. Returns "pending" error if the phone hasn't responded yet.
/// Returns "unknown challenge" both for a challenge id that never existed and for one that
/// has aged out past `CHALLENGE_TTL_SECS` and been pruned — the frontend already stops
/// polling on its own 5-minute timeout, so this is purely a server-side backstop.
#[tauri::command]
pub async fn auth_poll_session(
    challenge: String,
    state: State<'_, PendingSessions>,
) -> Result<Session, String> {
    let mut map = state.0.lock().map_err(|e| e.to_string())?;
    prune_expired(&mut map);
    match map.get(&challenge) {
        Some(entry) => match &entry.session {
            Some(session) => Ok(session.clone()),
            None => Err("pending".into()),
        },
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
    pin_store: State<'_, AccountPinStore>,
) -> Result<(), String> {
    let sessions = sessions.0.clone();
    let session_store = session_store.0.clone();
    let pin_store = pin_store.inner().clone();
    tokio::spawn(async move {
        run_callback_server(port, sessions, session_store, pin_store).await;
    });
    Ok(())
}

async fn run_callback_server(
    port: u16,
    sessions: Arc<Mutex<HashMap<String, ChallengeEntry>>>,
    session_store: Arc<Mutex<HashMap<String, SessionRecord>>>,
    pin_store: AccountPinStore,
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
        let pin_store = pin_store.clone();
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
                Ok(cb) => handle_auth_callback(cb, &sessions, &session_store, &pin_store).await,
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
/// never trusted from the request body). The final bookkeeping step goes through
/// `complete_challenge_at`, which enforces `CHALLENGE_TTL_SECS` and rejects a double-completion
/// server-side — a late callback that slips past the early `challenge_known` check (e.g. because
/// the chain lookup above took a while) still can't complete an aged-out challenge.
///
/// TRUST BOUNDARY CAVEAT — read before touching this function: "verified against the public key
/// on-chain" above is only as trustworthy as the RPC endpoint that supplies that key. The lookup
/// at the `chain::lookup_registered_account` call below has no state-proof/consensus verification
/// (see the TRUST BOUNDARY comment on that function in `commands/chain.rs`) — it's a plain,
/// unauthenticated JSON-RPC call to a hardcoded local node over plaintext HTTP. A
/// malicious/compromised node, or a local MITM on that channel, can hand this function an
/// attacker-chosen public key for any nullifier. What stops that from silently minting a session
/// for a forged identity is `finalize_callback` below running every resolved pubkey through
/// `account_pin::AccountPinStore` — a local trust-on-first-use pin, persisted to disk, keyed by
/// nullifier — before the sr25519 signature check even runs. A returned pubkey that
/// contradicts a nullifier's previously-pinned account is rejected outright as a probable
/// forged/compromised lookup response; see `account_pin.rs`'s module doc for exactly what this
/// does and does not close (short version: it defends every login after the first for a given
/// nullifier, not the first one — full closure still needs the smoldot light-client integration
/// CLAUDE.md documents as architected-for-but-not-built in the Rust backend).
async fn handle_auth_callback(
    cb: AuthCallback,
    sessions: &Arc<Mutex<HashMap<String, ChallengeEntry>>>,
    session_store: &Arc<Mutex<HashMap<String, SessionRecord>>>,
    pin_store: &AccountPinStore,
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

    finalize_callback(&cb, pubkey, sessions, session_store, pin_store)
}

/// Decision core of `handle_auth_callback`, taking the already-resolved `pubkey` as a plain
/// argument instead of performing the chain lookup itself. Split out specifically so tests can
/// exercise the full pin-enforcement + signature-verification + session-minting decision chain
/// — including the security fix, TOFU pin mismatch rejection — without touching the network;
/// `handle_auth_callback` above is the thin wrapper that does the actual
/// `chain::lookup_registered_account` RPC call and nothing else beyond that.
fn finalize_callback(
    cb: &AuthCallback,
    pubkey: [u8; 32],
    sessions: &Arc<Mutex<HashMap<String, ChallengeEntry>>>,
    session_store: &Arc<Mutex<HashMap<String, SessionRecord>>>,
    pin_store: &AccountPinStore,
) -> (&'static str, String) {
    // SECURITY FIX: reject a resolved account that contradicts what this desktop install has
    // previously pinned for this nullifier — see `account_pin.rs` for the full reasoning. This
    // check runs BEFORE the signature check on purpose: a forged lookup response would otherwise
    // sail through `verify_challenge_signature` (an attacker signing with their own key, which
    // the forged lookup dutifully returned, verifies just fine against itself).
    match pin_store.check_and_pin(&cb.nullifier_hash, pubkey) {
        PinCheckResult::Mismatch { .. } => {
            eprintln!(
                "[auth] SECURITY: nullifier {} resolved to an account that contradicts this \
                 desktop's previously-pinned account for it — rejecting callback (possible \
                 compromised/MITM'd RPC endpoint, see account_pin.rs)",
                cb.nullifier_hash
            );
            return (
                "409 Conflict",
                "{\"error\":\"resolved account does not match previously trusted account for this nullifier\"}".into(),
            );
        }
        PinCheckResult::FirstUsePinned | PinCheckResult::Matched => {}
    }

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
            match complete_challenge_at(&mut map, cb.challenge.clone(), session, unix_now()) {
                Ok(()) => {
                    store.insert(
                        token,
                        SessionRecord {
                            nullifier_hash: cb.nullifier_hash.clone(),
                            expires_at,
                        },
                    );
                    ("200 OK", "{\"ok\":true}".into())
                }
                Err("already_completed") => (
                    "409 Conflict",
                    "{\"error\":\"challenge already completed\"}".into(),
                ),
                Err(_) => (
                    "410 Gone",
                    "{\"error\":\"unknown or expired challenge\"}".into(),
                ),
            }
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

#[cfg(test)]
mod pruning_tests {
    use super::*;

    fn mk_entry(created_at: u64, session: Option<Session>) -> ChallengeEntry {
        ChallengeEntry { created_at, session }
    }

    fn mk_session(expires_at: u64) -> Session {
        Session {
            nullifier_hash: "0xabc123".to_string(),
            expires_at,
            token: "test-token".to_string(),
        }
    }

    /// Finding 2, pruning: a pending challenge older than `CHALLENGE_TTL_SECS` is removed,
    /// while a still-fresh one is kept.
    #[test]
    fn prune_removes_stale_pending_challenge_but_keeps_fresh_one() {
        let mut map = HashMap::new();
        map.insert("stale".to_string(), mk_entry(0, None));
        map.insert("fresh".to_string(), mk_entry(800, None)); // age 200s < TTL at now=1000

        prune_expired_at(&mut map, 1000);

        assert!(!map.contains_key("stale"), "challenge past CHALLENGE_TTL_SECS should be pruned");
        assert!(map.contains_key("fresh"), "challenge still within CHALLENGE_TTL_SECS should survive");
    }

    /// Finding 2, pruning: a completed session past its bearer-token `expires_at` is
    /// removed, while a still-valid completed session is kept.
    #[test]
    fn prune_removes_expired_completed_session_but_keeps_valid_one() {
        let mut map = HashMap::new();
        map.insert("old".to_string(), mk_entry(100, Some(mk_session(500))));
        map.insert("current".to_string(), mk_entry(100, Some(mk_session(2000))));

        prune_expired_at(&mut map, 1000);

        assert!(!map.contains_key("old"), "session past its expires_at should be pruned");
        assert!(map.contains_key("current"), "session still before its expires_at should survive");
    }

    /// Finding 2, expiry enforcement: a challenge that was never completed and has aged
    /// past `CHALLENGE_TTL_SECS` must be rejected if someone tries to complete it late —
    /// this is exactly the gap the frontend's own 5-minute timeout can't close on its own.
    #[test]
    fn late_completion_of_expired_challenge_is_rejected() {
        let mut map = HashMap::new();
        map.insert("chal".to_string(), mk_entry(0, None)); // created at t=0

        // Attempt completion at t=1000, well past the 300s TTL.
        let result = complete_challenge_at(&mut map, "chal".to_string(), mk_session(99_999), 1000);

        assert_eq!(result, Err("expired_or_unknown"));
        assert!(!map.contains_key("chal"), "expired challenge should have been pruned in the process");
    }

    /// Sanity check: completion within the TTL window still works.
    #[test]
    fn timely_completion_of_fresh_challenge_succeeds() {
        let mut map = HashMap::new();
        map.insert("chal".to_string(), mk_entry(100, None));

        let result = complete_challenge_at(&mut map, "chal".to_string(), mk_session(99_999), 150);

        assert_eq!(result, Ok(()));
        assert!(map.get("chal").unwrap().session.is_some());
    }

    /// A second completion attempt on an already-completed challenge must not silently
    /// overwrite the first session.
    #[test]
    fn double_completion_is_rejected() {
        let mut map = HashMap::new();
        map.insert("chal".to_string(), mk_entry(100, Some(mk_session(99_999))));

        let result = complete_challenge_at(&mut map, "chal".to_string(), mk_session(99_999), 150);

        assert_eq!(result, Err("already_completed"));
    }
}

#[cfg(test)]
mod finalize_callback_tests {
    use super::*;
    use crate::account_pin::AccountPinStore;

    fn mk_sessions(challenges: &[&str]) -> Arc<Mutex<HashMap<String, ChallengeEntry>>> {
        let mut map = HashMap::new();
        let now = unix_now();
        for c in challenges {
            map.insert(c.to_string(), ChallengeEntry { created_at: now, session: None });
        }
        Arc::new(Mutex::new(map))
    }

    fn signed_callback(
        challenge: &str,
        nullifier_hash: &str,
        keypair: &schnorrkel::Keypair,
    ) -> AuthCallback {
        let signature = keypair.sign_simple(SR25519_SIGNING_CONTEXT, challenge.as_bytes());
        AuthCallback {
            challenge: challenge.to_string(),
            nullifier_hash: nullifier_hash.to_string(),
            signature: hex::encode(signature.to_bytes()),
        }
    }

    /// Sanity check: a brand-new nullifier, valid signature, known challenge → session minted.
    /// (This also exercises the pin store's `FirstUsePinned` path, which the next test relies on
    /// having happened.)
    #[test]
    fn mints_a_session_on_first_use_and_pins_the_account() {
        let sessions = mk_sessions(&["chal-1"]);
        let session_store = Arc::new(Mutex::new(HashMap::new()));
        let pin_store = AccountPinStore::in_memory();
        let keypair = schnorrkel::Keypair::generate();
        let pubkey = keypair.public.to_bytes();
        let cb = signed_callback("chal-1", "0xaaaa", &keypair);

        let (status, _body) = finalize_callback(&cb, pubkey, &sessions, &session_store, &pin_store);

        assert_eq!(status, "200 OK");
        assert_eq!(session_store.lock().unwrap().len(), 1);
    }

    /// THE regression test for this security fix. Before this fix, `lookup_registered_account`'s
    /// return value (from an unauthenticated, unverified RPC call — see its TRUST BOUNDARY doc
    /// comment in `commands/chain.rs`) was trusted blind: a compromised/MITM'd RPC endpoint could
    /// return an attacker-chosen account for any nullifier, and `verify_challenge_signature` would
    /// happily verify the attacker's own signature against the attacker's own (forged-into-place)
    /// key, minting a real bearer session bound to a victim's `nullifier_hash`.
    ///
    /// This test simulates exactly that: a nullifier that legitimately resolved to one account on
    /// a first login, later "resolving" (as a compromised RPC lookup would report) to a completely
    /// different account on a second login attempt for the same nullifier. The account-pin check
    /// added in this fix must reject the second attempt outright, before signature verification
    /// even has a chance to rubber-stamp it.
    #[test]
    fn rejects_a_forged_lookup_that_contradicts_a_previously_pinned_account() {
        let sessions = mk_sessions(&["chal-legit", "chal-attack"]);
        let session_store = Arc::new(Mutex::new(HashMap::new()));
        let pin_store = AccountPinStore::in_memory();

        // Legitimate first login: nullifier N resolves to the real citizen's own account, and the
        // real citizen signs with their own key.
        let legit_keypair = schnorrkel::Keypair::generate();
        let legit_pubkey = legit_keypair.public.to_bytes();
        let legit_cb = signed_callback("chal-legit", "0xaaaa", &legit_keypair);
        let (status1, _) =
            finalize_callback(&legit_cb, legit_pubkey, &sessions, &session_store, &pin_store);
        assert_eq!(status1, "200 OK", "legitimate first login must succeed");

        // Attack: a later callback for the SAME nullifier, where the (now compromised/MITM'd) RPC
        // lookup returns a DIFFERENT, attacker-controlled account. The attacker signs with their
        // own key — which is exactly what a forged lookup response would let them get away with if
        // this pin check didn't exist, since `verify_challenge_signature` alone can't distinguish
        // "the real owner" from "whoever the lookup claims owns it."
        let attacker_keypair = schnorrkel::Keypair::generate();
        let attacker_pubkey = attacker_keypair.public.to_bytes();
        let attack_cb = signed_callback("chal-attack", "0xaaaa", &attacker_keypair);
        let (status2, body2) =
            finalize_callback(&attack_cb, attacker_pubkey, &sessions, &session_store, &pin_store);

        assert_ne!(
            status2, "200 OK",
            "a forged lookup for an already-pinned nullifier must be rejected, not minted into a session"
        );
        assert!(
            body2.contains("does not match"),
            "error should identify this as an account-pin mismatch, got: {body2}"
        );
        assert_eq!(
            session_store.lock().unwrap().len(),
            1,
            "no new session should have been minted for the rejected forged lookup"
        );
    }

    /// A pin that already matches must NOT bypass the signature check — a matching pin only
    /// asserts "this account is consistent with this desktop's history for this nullifier," it is
    /// not itself proof that today's caller controls the corresponding private key.
    #[test]
    fn matching_pin_still_requires_a_valid_signature() {
        let sessions = mk_sessions(&["chal-1", "chal-2"]);
        let session_store = Arc::new(Mutex::new(HashMap::new()));
        let pin_store = AccountPinStore::in_memory();
        let keypair = schnorrkel::Keypair::generate();
        let pubkey = keypair.public.to_bytes();

        let cb1 = signed_callback("chal-1", "0xaaaa", &keypair);
        let (status1, _) = finalize_callback(&cb1, pubkey, &sessions, &session_store, &pin_store);
        assert_eq!(status1, "200 OK");

        // Second callback claiming the same (now-pinned, matching) account, but with a garbage
        // signature instead of a real one.
        let cb2 = AuthCallback {
            challenge: "chal-2".to_string(),
            nullifier_hash: "0xaaaa".to_string(),
            signature: hex::encode([0x42u8; 64]),
        };
        let (status2, _) = finalize_callback(&cb2, pubkey, &sessions, &session_store, &pin_store);

        assert_eq!(status2, "401 Unauthorized");
        assert_eq!(
            session_store.lock().unwrap().len(),
            1,
            "invalid signature must not mint a session even when the account pin matches"
        );
    }
}
