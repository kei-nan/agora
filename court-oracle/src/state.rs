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

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedState {
    /// case_ids `submit_ai_ruling` has already been submitted for (mirrors `main.rs`'s
    /// `already_processed`).
    pub processed: HashSet<u32>,
    /// case_ids `finalize_ruling` has already been submitted for (mirrors `main.rs`'s
    /// `finalize_processed`).
    pub finalized: HashSet<u32>,
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
