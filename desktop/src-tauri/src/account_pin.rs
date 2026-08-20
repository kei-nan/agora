//! Trust-on-first-use (TOFU) pin store for nullifier → AccountId lookups.
//!
//! ## Why this exists
//!
//! `commands::chain::lookup_registered_account` resolves which on-chain `AccountId` a QR-auth
//! callback's `nullifierHash` belongs to via a plain, unauthenticated `state_getKeysPaged`/
//! `state_getStorage` JSON-RPC call to a hardcoded local node — see that function's own
//! `TRUST BOUNDARY` doc comment for the full accounting of why. There is no state-proof/consensus
//! verification of that response, so a malicious or compromised local node (or a MITM between
//! this process and its RPC endpoint) can return an attacker-chosen `AccountId` for any
//! nullifier, and `commands::auth::handle_auth_callback` would previously verify the callback's
//! signature against *that* forged key and mint a real bearer session for an arbitrary victim
//! identity.
//!
//! ## What this closes, and what it deliberately does not
//!
//! The structurally correct fix is a state-proof verified against a trusted, independently
//! obtained finalized header hash (Merkle/trie proof checking via `state_getReadProof`), or a
//! full protocol restructure so this lookup goes through the already-verified JS-side smoldot
//! light client (`desktop/src/chain/client.ts`). Both were evaluated for this pass and set aside
//! as genuine architecture decisions rather than something to ship unilaterally:
//!   - State-proof verification against a header obtained from the *same* untrusted RPC endpoint
//!     buys nothing — a node malicious enough to lie about one storage value can trivially
//!     fabricate a self-consistent header + proof to match, since it computes both locally. Doing
//!     that verification honestly requires an independently trusted finalized header, which this
//!     Rust process has no way to obtain today without embedding a second, real light client
//!     (consensus/GRANDPA-justification verification) — a new major dependency.
//!   - Routing the lookup through the JS light client instead means the Rust-side HTTP callback
//!     server would block on a round trip to a frontend that may not be warmed up (smoldot sync
//!     takes real time) or focused, which is a genuine threading/UX trade-off in the callback
//!     server's design, not a small tweak.
//!
//! This store is the narrower, real mitigation that *is* tractable this pass: it remembers,
//! per-nullifier, the first `AccountId` this desktop install ever saw the RPC endpoint vouch for,
//! persisted to disk so the pin survives app restarts. Every subsequent lookup for that same
//! nullifier must return the *same* `AccountId` or the callback is rejected outright. This closes
//! the realistic, high-value attack: a node that is compromised (or MITM'd) *after* a citizen has
//! already logged in from this desktop once cannot silently redirect their future logins to an
//! attacker-controlled account.
//!
//! **Residual risk, stated plainly:** this is TOFU — the very first lookup for a nullifier this
//! desktop has never seen before is trusted at face value, exactly as before. If the RPC endpoint
//! is already malicious/compromised at the moment of someone's *first* login here, the forged
//! account gets pinned as if it were genuine, and every subsequent login for that nullifier will
//! keep trusting it (this is TOFU's standard weakness — an attacker who wins the very first race
//! wins for good, same as SSH host-key pinning). It also means a legitimate on-chain identity
//! rotation (recovery re-scan, key rotation) will be rejected by this desktop install as a
//! "mismatch" until the pin is cleared — deleting the pin file
//! (`<app_data_dir>/account_pins.json`) resets trust for every nullifier back to first-use. This
//! is a real usability trade-off of choosing security over silent trust here; it is not a bug.
//! Do not treat a `Matched`/`FirstUsePinned` result as a cryptographic proof of chain state — it
//! is a consistency check against this desktop's own history, nothing more.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Outcome of checking a freshly RPC-looked-up `AccountId` against this store's pin history for
/// a nullifier.
#[derive(Debug, PartialEq, Eq)]
pub enum PinCheckResult {
    /// No pin existed for this nullifier yet; `account` has now been pinned as the trusted
    /// `AccountId` for it (see the module doc's TOFU residual-risk caveat).
    FirstUsePinned,
    /// A pin already existed and the freshly looked-up account matches it.
    Matched,
    /// A pin already existed for a **different** account. This is the case this store exists to
    /// catch: the RPC source now claims a different `AccountId` owns this nullifier than it did
    /// on a previous, successful login from this desktop install — consistent with a forged or
    /// tampered lookup response.
    Mismatch { pinned: [u8; 32] },
}

#[derive(Clone)]
pub struct AccountPinStore(Arc<AccountPinStoreInner>);

struct AccountPinStoreInner {
    /// `None` means "in-memory only, never persisted" — used by tests that don't want to touch
    /// disk. Every real (non-test) construction path always supplies a path.
    path: Option<PathBuf>,
    pins: Mutex<HashMap<String, [u8; 32]>>,
}

/// On-disk representation: nullifier hex (no "0x", lowercase) → account hex (no "0x", lowercase).
type PinFile = HashMap<String, String>;

impl AccountPinStore {
    /// Loads pins from `path` if it exists and parses as valid JSON; starts empty otherwise. A
    /// missing or corrupted pin file is deliberately not a fatal error — TOFU pinning degrading
    /// back to "no pin yet" for every nullifier only reduces this store's protection to what it
    /// was before this fix existed, it doesn't block the app from starting or auth from working.
    pub fn load(path: PathBuf) -> Self {
        let pins = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<PinFile>(&raw).ok())
            .map(|file| decode_pin_file(&file))
            .unwrap_or_default();
        Self(Arc::new(AccountPinStoreInner {
            path: Some(path),
            pins: Mutex::new(pins),
        }))
    }

    /// An in-memory-only store, for tests that exercise the pinning logic without touching disk.
    /// `pub(crate)` (not just `#[cfg(test)] fn`) so other modules' test code — e.g.
    /// `commands::auth`'s callback regression tests — can construct one too.
    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self(Arc::new(AccountPinStoreInner {
            path: None,
            pins: Mutex::new(HashMap::new()),
        }))
    }

    /// Checks `account` against any existing pin for `nullifier_hex`, pinning it if this is the
    /// first time this nullifier has been seen by this store. See `PinCheckResult` for the three
    /// possible outcomes and the module doc for what each one means.
    pub fn check_and_pin(&self, nullifier_hex: &str, account: [u8; 32]) -> PinCheckResult {
        let key = normalize_hex(nullifier_hex);
        let mut pins = self.0.pins.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match pins.get(&key) {
            Some(existing) if *existing == account => PinCheckResult::Matched,
            Some(existing) => PinCheckResult::Mismatch { pinned: *existing },
            None => {
                pins.insert(key, account);
                // Best-effort persistence: if the write fails (disk full, permissions, etc.) the
                // pin still holds for the lifetime of this process via the in-memory map above —
                // the worst case is losing the pin across a restart (falls back to TOFU-again for
                // that one nullifier), not a security regression relative to before this fix.
                if let Some(path) = &self.0.path {
                    let _ = persist(path, &pins);
                }
                PinCheckResult::FirstUsePinned
            }
        }
    }
}

fn normalize_hex(hex_str: &str) -> String {
    hex_str.trim_start_matches("0x").to_lowercase()
}

fn decode_pin_file(file: &PinFile) -> HashMap<String, [u8; 32]> {
    file.iter()
        .filter_map(|(k, v)| {
            let bytes = hex::decode(normalize_hex(v)).ok()?;
            let arr: [u8; 32] = bytes.try_into().ok()?;
            Some((normalize_hex(k), arr))
        })
        .collect()
}

fn persist(path: &std::path::Path, pins: &HashMap<String, [u8; 32]>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file: PinFile = pins
        .iter()
        .map(|(k, v)| (k.clone(), hex::encode(v)))
        .collect();
    let json = serde_json::to_string_pretty(&SerializablePinFile(file))?;
    std::fs::write(path, json)
}

/// Thin wrapper purely so `persist` can hand `serde_json::to_string_pretty` a concrete type
/// without importing `Serialize`/`Deserialize` derives onto the plain `HashMap` alias.
#[derive(Serialize, Deserialize)]
struct SerializablePinFile(PinFile);

#[cfg(test)]
mod tests {
    use super::*;

    fn acct(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    #[test]
    fn first_lookup_for_a_nullifier_is_pinned() {
        let store = AccountPinStore::in_memory();
        let result = store.check_and_pin("0xaaaa", acct(1));
        assert_eq!(result, PinCheckResult::FirstUsePinned);
    }

    #[test]
    fn repeated_lookup_with_the_same_account_matches() {
        let store = AccountPinStore::in_memory();
        store.check_and_pin("0xaaaa", acct(1));
        let result = store.check_and_pin("0xaaaa", acct(1));
        assert_eq!(result, PinCheckResult::Matched);
    }

    /// The core regression test: a forged/tampered RPC response that later claims a *different*
    /// account owns a nullifier this store already pinned must be caught, not trusted.
    #[test]
    fn forged_lookup_returning_a_different_account_for_a_pinned_nullifier_is_rejected() {
        let store = AccountPinStore::in_memory();
        store.check_and_pin("0xaaaa", acct(1)); // legitimate first login, pins account #1

        // A compromised/MITM'd RPC endpoint later claims the SAME nullifier belongs to a
        // different (attacker) account.
        let result = store.check_and_pin("0xaaaa", acct(0xEE));

        assert_eq!(result, PinCheckResult::Mismatch { pinned: acct(1) });
    }

    #[test]
    fn different_nullifiers_are_tracked_independently() {
        let store = AccountPinStore::in_memory();
        store.check_and_pin("0xaaaa", acct(1));
        let result = store.check_and_pin("0xbbbb", acct(2));
        // A different nullifier pinning a different account is NOT a mismatch — each nullifier
        // has its own independent pin.
        assert_eq!(result, PinCheckResult::FirstUsePinned);
    }

    #[test]
    fn nullifier_hex_is_normalized_with_and_without_0x_prefix_and_case() {
        let store = AccountPinStore::in_memory();
        store.check_and_pin("0xAAAA", acct(1));
        // Same nullifier, different casing/prefix presentation, same account: must still match.
        assert_eq!(store.check_and_pin("aaaa", acct(1)), PinCheckResult::Matched);
        // Same nullifier presentation variants, DIFFERENT account: must still be caught.
        assert_eq!(
            store.check_and_pin("AAAA", acct(0xEE)),
            PinCheckResult::Mismatch { pinned: acct(1) }
        );
    }

    fn temp_pin_path() -> PathBuf {
        std::env::temp_dir().join(format!("agora-desktop-pin-test-{}.json", uuid::Uuid::new_v4()))
    }

    /// Pins must survive a reload from disk — i.e. survive an app restart — since that's the
    /// entire point of persisting them (an in-process-only pin would reset on every launch,
    /// giving an attacker a fresh TOFU race on every desktop restart).
    #[test]
    fn pins_persist_across_a_reload_from_disk() {
        let path = temp_pin_path();

        let store1 = AccountPinStore::load(path.clone());
        assert_eq!(store1.check_and_pin("0xaaaa", acct(1)), PinCheckResult::FirstUsePinned);

        let store2 = AccountPinStore::load(path.clone());
        let result = store2.check_and_pin("0xaaaa", acct(0xEE));
        assert_eq!(
            result,
            PinCheckResult::Mismatch { pinned: acct(1) },
            "a pin written by one store instance must be enforced by a fresh instance loaded from the same path"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// A missing pin file must not be treated as an error — the store should simply start empty
    /// (equivalent to "no history yet" for every nullifier), matching the module doc's promise
    /// that a load failure degrades gracefully rather than blocking startup.
    #[test]
    fn missing_pin_file_loads_as_empty_store_without_error() {
        let path = temp_pin_path(); // deliberately never created
        let store = AccountPinStore::load(path);
        assert_eq!(store.check_and_pin("0xaaaa", acct(1)), PinCheckResult::FirstUsePinned);
    }

    /// A corrupted pin file (not valid JSON) must degrade to an empty store rather than panic —
    /// same reasoning as the missing-file case above.
    #[test]
    fn corrupted_pin_file_loads_as_empty_store_without_error() {
        let path = temp_pin_path();
        std::fs::write(&path, b"this is not json{{{").unwrap();

        let store = AccountPinStore::load(path.clone());
        let result = store.check_and_pin("0xaaaa", acct(1));

        assert_eq!(result, PinCheckResult::FirstUsePinned);
        let _ = std::fs::remove_file(&path);
    }
}
