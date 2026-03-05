//! Amp Code log discovery.
//!
//! Primary source:
//! - `~/.local/share/amp/threads/*.json`
//!
//! Fallback source (if no threads are found):
//! - `~/.amp/file-changes/**/*.{json,jsonl}`

use std::path::{Path, PathBuf};

use super::{AgentExplorer, SessionLog, SessionSource, home_dir, recent_files_with_exts};
use crate::scanner::AgentType;
use async_trait::async_trait;

pub async fn all_log_dirs() -> Vec<PathBuf> {
    let home = match home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };
    log_dirs_in(&home).await
}

pub struct AmpCodeExplorer;

#[async_trait]
impl AgentExplorer for AmpCodeExplorer {
    async fn discover_recent(&self, now: i64, since_secs: i64) -> Vec<SessionLog> {
        let dirs = all_log_dirs().await;
        recent_files_with_exts(&dirs, now, since_secs, &["json", "jsonl"])
            .await
            .into_iter()
            .map(|file| SessionLog {
                agent_type: AgentType::AmpCode,
                source: SessionSource::File(file.path),
                updated_at: Some(file.mtime_epoch),
                match_reasons: Vec::new(),
            })
            .collect()
    }
}

async fn log_dirs_in(home: &Path) -> Vec<PathBuf> {
    let primary_threads = amp_state_dir_in(home).join("threads");
    let mut primary_dirs = Vec::new();
    collect_dirs_with_exts(&primary_threads, &mut primary_dirs, &["json"]).await;

    if !primary_dirs.is_empty() {
        primary_dirs.sort();
        primary_dirs.dedup();
        return primary_dirs;
    }

    let mut fallback_dirs = Vec::new();
    collect_dirs_with_exts(
        &home.join(".amp").join("file-changes"),
        &mut fallback_dirs,
        &["json", "jsonl"],
    )
    .await;
    fallback_dirs.sort();
    fallback_dirs.dedup();
    fallback_dirs
}

fn amp_state_dir_in(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join(".local").join("share").join("amp")
    } else if cfg!(target_os = "windows") {
        if let Ok(appdata) = std::env::var("APPDATA") {
            PathBuf::from(appdata).join("amp")
        } else {
            home.join("AppData").join("Roaming").join("amp")
        }
    } else {
        home.join(".local").join("share").join("amp")
    }
}

async fn collect_dirs_with_exts(root: &Path, results: &mut Vec<PathBuf>, exts: &[&str]) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        let mut has_match = false;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let file_type = match entry.file_type().await {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file()
                && !has_match
                && let Some(ext) = path.extension().and_then(|e| e.to_str())
                && exts.iter().any(|allowed| allowed.eq_ignore_ascii_case(ext))
            {
                has_match = true;
            }
        }

        if has_match {
            results.push(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    #[tokio::test]
    async fn test_amp_log_dirs_prefers_threads_dir() {
        let home = TempDir::new().unwrap();
        let threads_dir = amp_state_dir_in(home.path()).join("threads");
        tokio::fs::create_dir_all(&threads_dir).await.unwrap();
        tokio::fs::write(threads_dir.join("thread-1.json"), "{}")
            .await
            .unwrap();

        let fallback_dir = home.path().join(".amp").join("file-changes").join("x");
        tokio::fs::create_dir_all(&fallback_dir).await.unwrap();
        tokio::fs::write(fallback_dir.join("x.json"), "{}")
            .await
            .unwrap();

        let dirs = log_dirs_in(home.path()).await;
        assert_eq!(dirs, vec![threads_dir]);
    }

    #[tokio::test]
    async fn test_amp_log_dirs_falls_back_when_threads_missing() {
        let home = TempDir::new().unwrap();
        let fallback_dir = home.path().join(".amp").join("file-changes").join("x");
        tokio::fs::create_dir_all(&fallback_dir).await.unwrap();
        tokio::fs::write(fallback_dir.join("x.jsonl"), "{}")
            .await
            .unwrap();

        let dirs = log_dirs_in(home.path()).await;
        assert_eq!(dirs, vec![fallback_dir]);
    }
}
