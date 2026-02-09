//! Pending retry system (stub).
//!
//! Manages pending records for commits that could not be resolved at
//! hook time. Full implementation in Phase 7; this module provides
//! the minimal stubs needed by the post-commit hook handler.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A record for a commit that could not be resolved at hook time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingRecord {
    /// Full commit hash.
    pub commit: String,
    /// Absolute path to the repository root.
    pub repo: String,
    /// Unix epoch timestamp of the commit.
    pub commit_time: i64,
    /// Number of resolution attempts so far.
    pub attempts: u32,
    /// Unix epoch timestamp of the last attempt.
    pub last_attempt: i64,
}

/// Return the pending directory: `~/.ai-barometer/pending/`.
///
/// Creates the directory if it does not exist.
///
/// Phase 7 will implement this fully.
pub fn pending_dir() -> anyhow::Result<PathBuf> {
    let home = crate::agents::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    let dir = home.join(".ai-barometer").join("pending");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

/// Write a pending record for a commit that could not be resolved.
///
/// Phase 7 will implement this fully. For now, writes a minimal JSON file
/// to the pending directory.
pub fn write_pending(commit: &str, repo: &str, commit_time: i64) -> anyhow::Result<()> {
    let dir = pending_dir()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let record = serde_json::json!({
        "commit": commit,
        "repo": repo,
        "commit_time": commit_time,
        "attempts": 1,
        "last_attempt": now,
    });

    let path = dir.join(format!("{}.json", commit));
    std::fs::write(&path, serde_json::to_string_pretty(&record)?)?;
    Ok(())
}

/// List all pending records for a given repository.
///
/// Phase 7 will implement this fully. For now, reads all `.json` files
/// in the pending directory and filters by repo path.
pub fn list_for_repo(repo: &str) -> anyhow::Result<Vec<PendingRecord>> {
    let dir = match pending_dir() {
        Ok(d) => d,
        Err(_) => return Ok(Vec::new()),
    };

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut records = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(Vec::new()),
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let value: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let record_repo = value.get("repo").and_then(|v| v.as_str()).unwrap_or("");
        if record_repo != repo {
            continue;
        }

        records.push(PendingRecord {
            commit: value
                .get("commit")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            repo: record_repo.to_string(),
            commit_time: value
                .get("commit_time")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            attempts: value.get("attempts").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            last_attempt: value
                .get("last_attempt")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
        });
    }

    Ok(records)
}

/// Remove the pending record for a given commit.
///
/// Phase 7 will implement this fully.
pub fn remove(commit: &str) -> anyhow::Result<()> {
    let dir = pending_dir()?;
    let path = dir.join(format!("{}.json", commit));
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pending_record_struct() {
        let record = PendingRecord {
            commit: "abcdef0123456789abcdef0123456789abcdef01".to_string(),
            repo: "/Users/foo/bar".to_string(),
            commit_time: 1_700_000_000,
            attempts: 1,
            last_attempt: 1_700_000_060,
        };
        assert_eq!(record.commit, "abcdef0123456789abcdef0123456789abcdef01");
        assert_eq!(record.repo, "/Users/foo/bar");
        assert_eq!(record.attempts, 1);
    }
}
