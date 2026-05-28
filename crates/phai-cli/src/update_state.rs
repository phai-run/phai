use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateState {
    pub last_check: Option<String>,
    pub last_seen_version: Option<String>,
    pub last_error: Option<String>,
    pub exe_path_hash: Option<String>,
}

impl UpdateState {
    pub fn read(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn write_atomic(&self, path: &Path) -> Result<()> {
        let parent = path.parent().with_context(|| {
            format!(
                "Update state path has no parent directory: {}",
                path.display()
            )
        })?;
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self)?;
        // NamedTempFile gives us a per-process unique name in the same
        // directory as the target, eliminating the TOCTOU race that the
        // previous `path.with_extension("tmp")` had when two finance-cli
        // processes wrote update state concurrently.
        let dir = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        let mut tmp = NamedTempFile::new_in(dir)
            .with_context(|| format!("Failed to create temp file in {}", dir.display()))?;
        tmp.write_all(json.as_bytes())
            .context("Failed to write update state")?;
        tmp.as_file_mut()
            .sync_all()
            .context("Failed to fsync update state")?;
        tmp.persist(path)
            .map_err(|e| e.error)
            .with_context(|| format!("Failed to persist {}", path.display()))?;
        Ok(())
    }

    pub fn should_check(&self, exe_path_hash: &str) -> bool {
        if self.exe_path_hash.as_deref() != Some(exe_path_hash) {
            return true;
        }
        let Some(last_check_str) = &self.last_check else {
            return true;
        };
        let Ok(last_check) = chrono::DateTime::parse_from_rfc3339(last_check_str) else {
            return true;
        };
        let elapsed = Utc::now().signed_duration_since(last_check);
        elapsed.num_seconds() as u64 >= CHECK_INTERVAL_SECS
    }

    pub fn mark_checked(&mut self, version: &str, exe_path_hash: &str) {
        self.last_check = Some(Utc::now().to_rfc3339());
        self.last_seen_version = Some(version.to_string());
        self.exe_path_hash = Some(exe_path_hash.to_string());
        self.last_error = None;
    }

    pub fn mark_error(&mut self, error: &str) {
        self.last_check = Some(Utc::now().to_rfc3339());
        self.last_error = Some(error.to_string());
    }
}

pub fn compute_exe_path_hash() -> Result<String> {
    let exe_path = std::env::current_exe().context("Failed to resolve current executable path")?;
    let canonical = exe_path
        .canonicalize()
        .context("Failed to canonicalize executable path")?;
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn state_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("update-state.json")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn recent_rfc3339() -> String {
        // 1 hour ago — well within 24-hour window
        let t = Utc::now() - chrono::Duration::hours(1);
        t.to_rfc3339()
    }

    fn old_rfc3339() -> String {
        // 25 hours ago — older than the 24-hour window
        let t = Utc::now() - chrono::Duration::hours(25);
        t.to_rfc3339()
    }

    const HASH_A: &str = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111";
    const HASH_B: &str = "bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222";

    // -----------------------------------------------------------------------
    // should_check
    // -----------------------------------------------------------------------

    #[test]
    fn should_check_default_state_returns_true() {
        let state = UpdateState::default();
        assert!(state.should_check(HASH_A));
    }

    #[test]
    fn should_check_recent_check_returns_false() {
        let state = UpdateState {
            last_check: Some(recent_rfc3339()),
            last_seen_version: Some("1.0.0".into()),
            last_error: None,
            exe_path_hash: Some(HASH_A.into()),
        };
        assert!(!state.should_check(HASH_A));
    }

    #[test]
    fn should_check_old_check_returns_true() {
        let state = UpdateState {
            last_check: Some(old_rfc3339()),
            last_seen_version: Some("1.0.0".into()),
            last_error: None,
            exe_path_hash: Some(HASH_A.into()),
        };
        assert!(state.should_check(HASH_A));
    }

    #[test]
    fn should_check_exe_hash_mismatch_returns_true_even_if_recent() {
        let state = UpdateState {
            last_check: Some(recent_rfc3339()),
            last_seen_version: Some("1.0.0".into()),
            last_error: None,
            exe_path_hash: Some(HASH_A.into()),
        };
        // Different hash from a different install path
        assert!(state.should_check(HASH_B));
    }

    #[test]
    fn should_check_malformed_last_check_returns_true() {
        let state = UpdateState {
            last_check: Some("not-a-date".into()),
            last_seen_version: None,
            last_error: None,
            exe_path_hash: Some(HASH_A.into()),
        };
        assert!(state.should_check(HASH_A));
    }

    // -----------------------------------------------------------------------
    // write_atomic + read round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn write_and_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = state_file_path(dir.path());

        let state = UpdateState {
            last_check: Some(recent_rfc3339()),
            last_seen_version: Some("2.0.0".into()),
            last_error: Some("previous error".into()),
            exe_path_hash: Some(HASH_A.into()),
        };

        state.write_atomic(&path).unwrap();

        let loaded = UpdateState::read(&path);
        assert_eq!(loaded.last_seen_version.as_deref(), Some("2.0.0"));
        assert_eq!(loaded.exe_path_hash.as_deref(), Some(HASH_A));
        assert!(loaded.last_check.is_some());
        assert_eq!(loaded.last_error.as_deref(), Some("previous error"));
    }

    #[test]
    fn read_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = state_file_path(dir.path());
        // File doesn't exist
        let state = UpdateState::read(&path);
        assert!(state.last_check.is_none());
        assert!(state.last_seen_version.is_none());
        assert!(state.exe_path_hash.is_none());
    }

    // -----------------------------------------------------------------------
    // mark_checked
    // -----------------------------------------------------------------------

    #[test]
    fn mark_checked_clears_last_error_and_sets_fields() {
        let mut state = UpdateState {
            last_check: None,
            last_seen_version: None,
            last_error: Some("old error".into()),
            exe_path_hash: None,
        };
        state.mark_checked("1.2.3", HASH_A);

        assert!(state.last_error.is_none(), "last_error should be cleared");
        assert_eq!(state.last_seen_version.as_deref(), Some("1.2.3"));
        assert_eq!(state.exe_path_hash.as_deref(), Some(HASH_A));
        assert!(state.last_check.is_some(), "last_check should be set");
    }

    // -----------------------------------------------------------------------
    // mark_error
    // -----------------------------------------------------------------------

    #[test]
    fn mark_error_sets_last_error_and_last_check() {
        let mut state = UpdateState {
            last_check: None,
            last_seen_version: Some("1.0.0".into()),
            last_error: None,
            exe_path_hash: Some(HASH_A.into()),
        };
        state.mark_error("network timeout");

        assert_eq!(state.last_error.as_deref(), Some("network timeout"));
        assert!(state.last_check.is_some(), "last_check should be advanced");
        // last_seen_version is preserved
        assert_eq!(state.last_seen_version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn write_atomic_handles_concurrent_writers() {
        use std::sync::Arc;
        use std::thread;

        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(state_file_path(dir.path()));

        let handles: Vec<_> = (0..16)
            .map(|i| {
                let path = Arc::clone(&path);
                thread::spawn(move || {
                    let state = UpdateState {
                        last_check: Some(recent_rfc3339()),
                        last_seen_version: Some(format!("1.0.{i}")),
                        last_error: None,
                        exe_path_hash: Some(HASH_A.into()),
                    };
                    state.write_atomic(&path).unwrap();
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        // Final file must be a valid, complete JSON write from one of the
        // threads (no partial/truncated content from a clobbered temp).
        let loaded = UpdateState::read(&path);
        assert!(loaded.last_seen_version.is_some());
        assert_eq!(loaded.exe_path_hash.as_deref(), Some(HASH_A));
    }

    #[test]
    fn mark_error_advances_last_check_so_throttle_works() {
        // After mark_error + write, a subsequent read should show should_check==false
        // (assuming hash matches and the check was just now).
        let dir = tempfile::tempdir().unwrap();
        let path = state_file_path(dir.path());

        let mut state = UpdateState {
            last_check: None,
            last_seen_version: None,
            last_error: None,
            exe_path_hash: Some(HASH_A.into()),
        };
        state.mark_error("transient error");
        state.write_atomic(&path).unwrap();

        let loaded = UpdateState::read(&path);
        // Just wrote last_check = now, so should_check should be false
        assert!(
            !loaded.should_check(HASH_A),
            "should throttle after mark_error"
        );
    }
}
