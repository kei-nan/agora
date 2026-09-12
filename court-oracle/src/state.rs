//! Local, on-disk persistence for which case_ids this instance has already fully processed.
//!
//! `main.rs`'s `already_processed`/`finalize_processed` sets used to be in-memory-only
//! `HashSet<u32>`s — correct while the process stays up, but a crash or restart in the narrow
//! window between "submitted `submit_ai_ruling`/`finalize_ruling` to the chain" and "recorded
//! locally as done" meant the next run would re-poll the same `Filed`/`AIRulingIssued` case,
//! ask Claude again (a real, billed API call), and attempt a redundant resubmission.
//!
//! Chain-side idempotency already prevents that resubmission from double-applying anything —
//! `pallet-courts`'s own status checks (`Filed`/`AIRulingIssued`/etc.) reject a call that no
//! longer applies to a case's current state — so this was never a correctness or fund-safety
//! bug, only a wasted Claude API call on the (hopefully rare) restart-during-the-gap case. This
//! module closes that gap by persisting the two sets to a small JSON file next to wherever this
//! service runs, loaded on startup and rewritten after every new entry.
//!
//! Write is "atomic enough" for this purpose: content is written to a sibling temp file first,
//! then renamed into place. A rename is atomic on the same filesystem on both Linux and Windows,
//! so a crash mid-save either leaves the old (still-valid) state file untouched or the new one
//! fully written — never a half-written, corrupt JSON file that would fail to load on the next
//! startup.
//!
//! ## `processed`/`finalized` mean *confirmed*, not merely *submitted*
//!
//! `author_submitExtrinsic` returning `Ok(tx_hash)` is pool acceptance, not dispatch success —
//! `submit_ai_ruling`/`finalize_ruling` are both gated by `EnsureOracleCouncilApproved`-style
//! origin/status checks that run at block-*execution* time, not transaction-pool validation time,
//! so a call can still be rejected (or never get included at all) with no error ever surfacing
//! here. `pending_rulings`/`pending_finalizations` below track "submitted, awaiting on-chain
//! confirmation" separately from `processed`/`finalized` ("a later poll actually observed the
//! case's status move" — see `main.rs`'s `ruling_confirmed_by_status`/
//! `finalization_confirmed_by_status`). A case only ever moves from pending to
//! confirmed, never the reverse, and `main.rs` never inserts directly into `processed`/
//! `finalized` from a submission result alone (`DRY_RUN` is the one deliberate exception: no real
//! extrinsic is ever submitted in dry-run mode, so there is nothing to confirm).

use crate::cases::Verdict;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The already-computed ruling for a case whose `submit_ai_ruling` extrinsic was submitted but
/// not yet confirmed on-chain — cached so a resubmission attempt (if the first one never took
/// effect) never needs a second, redundantly-billed Claude call or IPFS publish for the same
/// case_id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRuling {
    pub ruling_hash: [u8; 32],
    pub verdict: Verdict,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedState {
    /// case_ids whose `submit_ai_ruling` a LATER poll has confirmed actually took effect
    /// on-chain (the case's status was observed to have moved off `Filed`) — see this module's
    /// doc comment. Mirrors `main.rs`'s old `already_processed` name.
    pub processed: HashSet<u32>,
    /// case_ids whose `finalize_ruling` a LATER poll has confirmed actually took effect on-chain
    /// (the case's status was observed to have left `AIRulingIssued`) — see this module's doc
    /// comment. Mirrors `main.rs`'s old `finalize_processed` name.
    pub finalized: HashSet<u32>,
    /// case_ids for which `submit_ai_ruling` was submitted (`author_submitExtrinsic` returned
    /// `Ok`) but no later poll has yet confirmed it took effect — i.e. the case's status was
    /// still `Filed` on the most recent poll. `#[serde(default)]` so a state file written before
    /// this field existed still loads (as "nothing pending", the safe direction: at worst a
    /// resubmission is skipped until it naturally resolves as a fresh `Filed` case instead of a
    /// pending one, never treated as falsely confirmed).
    #[serde(default)]
    pub pending_rulings: HashMap<u32, PendingRuling>,
    /// Same idea as `pending_rulings`, for `finalize_ruling` — case_ids submitted but not yet
    /// confirmed to have left `AIRulingIssued`.
    #[serde(default)]
    pub pending_finalizations: HashSet<u32>,
}

impl PersistedState {
    /// Loads state from `path`. A missing file is not an error — it means either a first-ever
    /// run or a fresh volume, and is treated the same as `PersistedState::default()` (nothing
    /// processed yet). Any other read/parse failure IS surfaced as an error: silently ignoring
    /// unreadable state would risk exactly the redundant-Claude-call problem this module exists
    /// to prevent, and a corrupt file is worth an operator's attention rather than a silent
    /// "start from scratch."
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("STATE_FILE at {} is not valid JSON in the expected shape — refusing to guess and silently drop tracked case history", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading STATE_FILE at {}", path.display())),
        }
    }

    /// Writes state to `path` via a write-to-temp-then-rename, so a crash mid-write can never
    /// leave a corrupt file in place (see module doc comment).
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).context("serializing court-oracle state")?;
        let tmp_path = tmp_path_for(path);
        std::fs::write(&tmp_path, &bytes)
            .with_context(|| format!("writing temp state file at {}", tmp_path.display()))?;
        std::fs::rename(&tmp_path, path).with_context(|| {
            format!("renaming temp state file {} into place at {}", tmp_path.display(), path.display())
        })?;
        Ok(())
    }
}

/// Builds the sibling temp-file path used for the atomic-write dance above. A fixed suffix
/// (rather than e.g. a random/pid-based one) is fine here: this service runs as a single
/// instance against a given `STATE_FILE` (the whole point of the M-of-N Oracle Council design
/// is one process per council member, each with its own keys/state), so there's no concurrent
/// writer to collide with.
fn tmp_path_for(path: &Path) -> std::path::PathBuf {
    let mut os_string = path.as_os_str().to_owned();
    os_string.push(".tmp");
    std::path::PathBuf::from(os_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_missing_file_returns_default_not_an_error() {
        let dir = std::env::temp_dir().join(format!("court-oracle-state-test-{}", uniq()));
        let path = dir.join("does-not-exist.json");
        let loaded = PersistedState::load(&path).expect("missing file should load as default");
        assert_eq!(loaded, PersistedState::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir().join(format!("court-oracle-state-test-{}", uniq()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");

        let mut state = PersistedState::default();
        state.processed.insert(1);
        state.processed.insert(42);
        state.finalized.insert(42);

        state.save(&path).expect("save should succeed");
        let loaded = PersistedState::load(&path).expect("load should succeed");
        assert_eq!(loaded, state);

        // The temp file used mid-save must not be left behind after a successful rename.
        assert!(!tmp_path_for(&path).exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_overwrites_previous_contents_rather_than_merging() {
        let dir = std::env::temp_dir().join(format!("court-oracle-state-test-{}", uniq()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");

        let mut first = PersistedState::default();
        first.processed.insert(1);
        first.save(&path).unwrap();

        let mut second = PersistedState::default();
        second.processed.insert(2);
        second.save(&path).unwrap();

        let loaded = PersistedState::load(&path).unwrap();
        assert_eq!(loaded, second, "second save must fully replace, not merge with, the first");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pending_fields_round_trip_through_save_and_load() {
        let dir = std::env::temp_dir().join(format!("court-oracle-state-test-{}", uniq()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");

        let mut state = PersistedState::default();
        state.pending_rulings.insert(
            7,
            PendingRuling { ruling_hash: [9u8; 32], verdict: Verdict::Overturned },
        );
        state.pending_finalizations.insert(11);

        state.save(&path).expect("save should succeed");
        let loaded = PersistedState::load(&path).expect("load should succeed");
        assert_eq!(loaded, state);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_state_file_written_before_the_pending_fields_existed_still_loads() {
        // Simulates a state.json from before pending_rulings/pending_finalizations were added —
        // #[serde(default)] must mean this still loads (as "nothing pending"), not a load error.
        let dir = std::env::temp_dir().join(format!("court-oracle-state-test-{}", uniq()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old-shape.json");
        std::fs::write(&path, br#"{"processed":[1,2],"finalized":[2]}"#).unwrap();

        let loaded = PersistedState::load(&path).expect("old-shape file should still load");
        assert_eq!(loaded.processed, HashSet::from([1, 2]));
        assert_eq!(loaded.finalized, HashSet::from([2]));
        assert!(loaded.pending_rulings.is_empty());
        assert!(loaded.pending_finalizations.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_file_is_a_load_error_not_a_silent_default() {
        let dir = std::env::temp_dir().join(format!("court-oracle-state-test-{}", uniq()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corrupt.json");
        std::fs::write(&path, b"not json at all").unwrap();

        let result = PersistedState::load(&path);
        assert!(result.is_err(), "corrupt state file must surface as an error, never a silent reset");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Cheap per-test-run uniqueness so parallel `cargo test` threads don't collide on the same
    /// temp path (no external crate like `tempfile` is a dependency of this crate today, so this
    /// stays dependency-free rather than adding one just for tests).
    fn uniq() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ^ (std::process::id() as u128) << 64
    }
}
