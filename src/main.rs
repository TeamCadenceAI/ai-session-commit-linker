mod agents;
mod git;
mod note;
mod pending;
mod scanner;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::process;

/// AI Barometer: attach AI coding agent session logs to Git commits via git notes.
///
/// Provides provenance and measurement of AI-assisted development
/// without polluting commit history.
#[derive(Parser, Debug)]
#[command(name = "ai-barometer", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Install AI Barometer: set up git hooks and run initial hydration.
    Install {
        /// Optional GitHub org filter for push scoping.
        #[arg(long)]
        org: Option<String>,
    },

    /// Git hook entry points.
    Hook {
        #[command(subcommand)]
        hook_command: HookCommand,
    },

    /// Backfill AI session notes for recent commits.
    Hydrate {
        /// How far back to scan, e.g. "7d" for 7 days.
        #[arg(long, default_value = "7d")]
        since: String,

        /// Push notes to remote after hydration.
        #[arg(long)]
        push: bool,
    },

    /// Retry attaching notes for pending (unresolved) commits.
    Retry,

    /// Show AI Barometer status for the current repository.
    Status,
}

#[derive(Subcommand, Debug)]
enum HookCommand {
    /// Post-commit hook: attempt to attach AI session note to HEAD.
    PostCommit,
}

// ---------------------------------------------------------------------------
// Subcommand dispatch
// ---------------------------------------------------------------------------

fn run_install(org: Option<String>) -> Result<()> {
    eprintln!(
        "[ai-barometer] install: org={:?} (not yet implemented)",
        org
    );
    Ok(())
}

/// The post-commit hook handler. This is the critical hot path.
///
/// CRITICAL: This function must NEVER fail the commit. All errors are caught
/// and logged as warnings. The function always exits 0.
///
/// The outer wrapper uses `std::panic::catch_unwind` to catch panics, and
/// an inner `Result` to catch all other errors. Any failure is logged to
/// stderr with the `[ai-barometer]` prefix and silently ignored.
fn run_hook_post_commit() -> Result<()> {
    // Catch-all: catch panics
    let result = std::panic::catch_unwind(|| -> Result<()> { hook_post_commit_inner() });

    match result {
        Ok(Ok(())) => {} // Success
        Ok(Err(e)) => {
            eprintln!("[ai-barometer] warning: hook failed: {}", e);
        }
        Err(_) => {
            eprintln!("[ai-barometer] warning: hook panicked (this is a bug)");
        }
    }

    // Always succeed — never block the commit
    Ok(())
}

/// Inner implementation of the post-commit hook.
///
/// This function is allowed to return errors — the caller (`run_hook_post_commit`)
/// catches all errors and panics.
fn hook_post_commit_inner() -> Result<()> {
    // Step 1: Get repo root, HEAD hash, HEAD timestamp
    let repo_root = git::repo_root()?;
    let head_hash = git::head_hash()?;
    let head_timestamp = git::head_timestamp()?;
    let repo_root_str = repo_root.to_string_lossy().to_string();

    // Step 2: Deduplication — if note already exists, exit early
    if git::note_exists(&head_hash)? {
        return Ok(());
    }

    // Step 3: Collect candidate log directories from agents
    let mut candidate_dirs = Vec::new();
    candidate_dirs.extend(agents::claude::log_dirs(&repo_root));
    candidate_dirs.extend(agents::codex::log_dirs(&repo_root));

    // Step 4: Filter candidate files by ±10 min (600 sec) window
    let candidate_files = agents::candidate_files(&candidate_dirs, head_timestamp, 600);

    // Step 5: Run scanner to find session match
    let session_match = scanner::find_session_for_commit(&head_hash, &candidate_files);

    if let Some(ref matched) = session_match {
        // Step 6a: Parse metadata and verify match
        let metadata = scanner::parse_session_metadata(&matched.file_path);

        if scanner::verify_match(&metadata, &repo_root, &head_hash) {
            // Read the full session log
            let session_log = std::fs::read_to_string(&matched.file_path).unwrap_or_default();

            let session_id = metadata.session_id.as_deref().unwrap_or("unknown");

            // Format the note
            let note_content = note::format(
                &matched.agent_type,
                session_id,
                &repo_root_str,
                &head_hash,
                &session_log,
            )?;

            // Attach the note
            git::add_note(&head_hash, &note_content)?;

            eprintln!(
                "[ai-barometer] attached session {} to commit {}",
                session_id,
                &head_hash[..7]
            );

            // Push logic (stub — Phase 8 will implement fully)
            // For now, we skip pushing entirely.
        } else {
            // Verification failed — treat as no match, write pending
            if let Err(e) = pending::write_pending(&head_hash, &repo_root_str, head_timestamp) {
                eprintln!(
                    "[ai-barometer] warning: failed to write pending record: {}",
                    e
                );
            }
        }
    } else {
        // Step 6b: No match found — write pending record
        if let Err(e) = pending::write_pending(&head_hash, &repo_root_str, head_timestamp) {
            eprintln!(
                "[ai-barometer] warning: failed to write pending record: {}",
                e
            );
        }
    }

    // Step 7: Retry pending commits for this repo (stub — Phase 7 will implement fully)
    retry_pending_for_repo(&repo_root_str, &repo_root);

    Ok(())
}

/// Attempt to resolve pending commits for the given repository.
///
/// This is a best-effort operation. Any errors during retry are logged
/// and silently ignored.
///
/// Phase 7 will implement the full retry logic. For now, this iterates
/// over pending records and attempts resolution for each.
fn retry_pending_for_repo(repo_str: &str, repo_root: &std::path::Path) {
    let pending_records = match pending::list_for_repo(repo_str) {
        Ok(records) => records,
        Err(_) => return,
    };

    for record in &pending_records {
        // Skip if note already exists (may have been resolved by another mechanism)
        match git::note_exists(&record.commit) {
            Ok(true) => {
                // Already resolved — remove pending record
                let _ = pending::remove(&record.commit);
                continue;
            }
            Ok(false) => {} // Still pending, try to resolve
            Err(_) => continue,
        }

        // Collect candidate dirs and files for this commit
        let mut candidate_dirs = Vec::new();
        candidate_dirs.extend(agents::claude::log_dirs(repo_root));
        candidate_dirs.extend(agents::codex::log_dirs(repo_root));

        let candidate_files = agents::candidate_files(&candidate_dirs, record.commit_time, 600);

        let session_match = scanner::find_session_for_commit(&record.commit, &candidate_files);

        if let Some(ref matched) = session_match {
            let metadata = scanner::parse_session_metadata(&matched.file_path);

            if scanner::verify_match(&metadata, repo_root, &record.commit) {
                let session_log = std::fs::read_to_string(&matched.file_path).unwrap_or_default();

                let session_id = metadata.session_id.as_deref().unwrap_or("unknown");

                let note_content = match note::format(
                    &matched.agent_type,
                    session_id,
                    repo_str,
                    &record.commit,
                    &session_log,
                ) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                if git::add_note(&record.commit, &note_content).is_ok() {
                    eprintln!(
                        "[ai-barometer] retry: attached session {} to commit {}",
                        session_id,
                        &record.commit[..std::cmp::min(7, record.commit.len())]
                    );
                    let _ = pending::remove(&record.commit);
                }
            }
        }
    }
}

fn run_hydrate(since: &str, push: bool) -> Result<()> {
    eprintln!(
        "[ai-barometer] hydrate: since={}, push={} (not yet implemented)",
        since, push
    );
    Ok(())
}

fn run_retry() -> Result<()> {
    eprintln!("[ai-barometer] retry (not yet implemented)");
    Ok(())
}

fn run_status() -> Result<()> {
    eprintln!("[ai-barometer] status (not yet implemented)");
    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Install { org } => run_install(org),
        Command::Hook { hook_command } => match hook_command {
            HookCommand::PostCommit => run_hook_post_commit(),
        },
        Command::Hydrate { since, push } => run_hydrate(&since, push),
        Command::Retry => run_retry(),
        Command::Status => run_status(),
    };

    if let Err(e) = result {
        eprintln!("[ai-barometer] error: {}", e);
        process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_install() {
        let cli = Cli::parse_from(["ai-barometer", "install"]);
        assert!(matches!(cli.command, Command::Install { org: None }));
    }

    #[test]
    fn cli_parses_install_with_org() {
        let cli = Cli::parse_from(["ai-barometer", "install", "--org", "my-org"]);
        match cli.command {
            Command::Install { org } => assert_eq!(org.as_deref(), Some("my-org")),
            _ => panic!("expected Install command"),
        }
    }

    #[test]
    fn cli_parses_hook_post_commit() {
        let cli = Cli::parse_from(["ai-barometer", "hook", "post-commit"]);
        assert!(matches!(
            cli.command,
            Command::Hook {
                hook_command: HookCommand::PostCommit
            }
        ));
    }

    #[test]
    fn cli_parses_hydrate_defaults() {
        let cli = Cli::parse_from(["ai-barometer", "hydrate"]);
        match cli.command {
            Command::Hydrate { since, push } => {
                assert_eq!(since, "7d");
                assert!(!push);
            }
            _ => panic!("expected Hydrate command"),
        }
    }

    #[test]
    fn cli_parses_hydrate_with_flags() {
        let cli = Cli::parse_from(["ai-barometer", "hydrate", "--since", "30d", "--push"]);
        match cli.command {
            Command::Hydrate { since, push } => {
                assert_eq!(since, "30d");
                assert!(push);
            }
            _ => panic!("expected Hydrate command"),
        }
    }

    #[test]
    fn cli_parses_retry() {
        let cli = Cli::parse_from(["ai-barometer", "retry"]);
        assert!(matches!(cli.command, Command::Retry));
    }

    #[test]
    fn cli_parses_status() {
        let cli = Cli::parse_from(["ai-barometer", "status"]);
        assert!(matches!(cli.command, Command::Status));
    }

    #[test]
    fn run_install_returns_ok() {
        assert!(run_install(None).is_ok());
    }

    #[test]
    fn run_hook_post_commit_returns_ok() {
        // The catch-all wrapper ensures this always returns Ok even
        // when called outside a git repo (the inner logic will fail
        // but the error is caught and logged to stderr).
        assert!(run_hook_post_commit().is_ok());
    }

    #[test]
    fn run_hydrate_returns_ok() {
        assert!(run_hydrate("7d", false).is_ok());
    }

    #[test]
    fn run_retry_returns_ok() {
        assert!(run_retry().is_ok());
    }

    #[test]
    fn run_status_returns_ok() {
        assert!(run_status().is_ok());
    }

    // -----------------------------------------------------------------------
    // Negative CLI parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn cli_rejects_unknown_subcommand() {
        let result = Cli::try_parse_from(["ai-barometer", "frobnicate"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_rejects_hook_without_sub_subcommand() {
        let result = Cli::try_parse_from(["ai-barometer", "hook"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_rejects_hydrate_since_missing_value() {
        let result = Cli::try_parse_from(["ai-barometer", "hydrate", "--since"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_rejects_no_subcommand() {
        let result = Cli::try_parse_from(["ai-barometer"]);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Integration test: post-commit hook with a real temp repo
    // -----------------------------------------------------------------------

    use serial_test::serial;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Helper: create a temporary git repo with one commit.
    fn init_temp_repo() -> TempDir {
        let dir = TempDir::new().expect("failed to create temp dir");
        let path = dir.path();

        run_git(path, &["init"]);
        run_git(path, &["config", "user.email", "test@test.com"]);
        run_git(path, &["config", "user.name", "Test User"]);
        std::fs::write(path.join("README.md"), "hello").unwrap();
        run_git(path, &["add", "README.md"]);
        run_git(path, &["commit", "-m", "initial commit"]);

        dir
    }

    /// Run a git command inside the given directory, panicking on failure.
    fn run_git(dir: &std::path::Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(["-C", dir.to_str().unwrap()])
            .args(args)
            .output()
            .expect("failed to run git");
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            panic!("git {:?} failed: {}", args, stderr);
        }
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    /// Helper: get a stable directory to use as a fallback CWD.
    /// This is needed because serial tests may leave CWD in a deleted temp dir
    /// if a previous test panicked before restoring CWD.
    fn safe_cwd() -> PathBuf {
        match std::env::current_dir() {
            Ok(cwd) if cwd.exists() => cwd,
            _ => {
                // CWD is invalid (deleted temp dir from panicked test).
                // Restore to a known-good directory.
                let fallback = std::env::temp_dir();
                std::env::set_current_dir(&fallback).ok();
                fallback
            }
        }
    }

    #[test]
    #[serial]
    fn test_hook_post_commit_attaches_note_to_commit() {
        // Set up temp repo with a commit
        let dir = init_temp_repo();
        let repo_path = dir.path();
        let original_cwd = safe_cwd();

        // Get the actual repo root as git sees it (may differ from dir.path()
        // due to symlinks, e.g. /var -> /private/var on macOS)
        let git_repo_root = run_git(repo_path, &["rev-parse", "--show-toplevel"]);
        let git_repo_root_path = std::path::Path::new(&git_repo_root);

        // Get the HEAD commit hash and timestamp
        let head_hash = run_git(repo_path, &["rev-parse", "HEAD"]);
        let head_ts_str = run_git(repo_path, &["show", "-s", "--format=%ct", "HEAD"]);
        let head_ts: i64 = head_ts_str.parse().unwrap();

        // Create a fake Claude session log directory matching this repo.
        // Use the git-reported repo root for encoding to match what the hook
        // will compute internally.
        let encoded = agents::encode_repo_path(git_repo_root_path);
        let home = agents::home_dir().expect("no HOME");
        let claude_project_dir = home.join(".claude").join("projects").join(&encoded);
        std::fs::create_dir_all(&claude_project_dir).unwrap();

        // Create a fake JSONL session log with the commit hash and metadata.
        // Use the git-reported repo root for cwd to match what verify_match checks.
        let session_content = format!(
            r#"{{"session_id":"test-session-id","cwd":"{cwd}"}}
{{"type":"tool_result","content":"[main {short}] initial commit\n 1 file changed"}}
{{"type":"assistant","message":"Done"}}
"#,
            cwd = git_repo_root,
            short = &head_hash[..7],
        );
        let session_file = claude_project_dir.join("session.jsonl");
        std::fs::write(&session_file, &session_content).unwrap();

        // Set the session file mtime to match the commit time
        let ft = filetime::FileTime::from_unix_time(head_ts, 0);
        filetime::set_file_mtime(&session_file, ft).unwrap();

        // chdir into the repo and run the hook
        std::env::set_current_dir(repo_path).expect("failed to chdir");
        let result = run_hook_post_commit();

        // The hook should always return Ok
        assert!(result.is_ok());

        // Verify a note was attached
        let note_output = run_git(
            repo_path,
            &[
                "notes",
                "--ref",
                "refs/notes/ai-sessions",
                "show",
                &head_hash,
            ],
        );
        assert!(note_output.contains("agent: claude-code"));
        assert!(note_output.contains("session_id: test-session-id"));
        assert!(note_output.contains(&head_hash));
        assert!(note_output.contains("confidence: exact_hash_match"));

        // Clean up: remove the fake Claude dir, restore cwd
        let _ = std::fs::remove_dir_all(&claude_project_dir);
        std::env::set_current_dir(original_cwd).unwrap();
    }

    #[test]
    #[serial]
    fn test_hook_post_commit_deduplication_skips_if_note_exists() {
        let dir = init_temp_repo();
        let repo_path = dir.path();
        let original_cwd = safe_cwd();

        let head_hash = run_git(repo_path, &["rev-parse", "HEAD"]);

        // Manually attach a note first
        run_git(
            repo_path,
            &[
                "notes",
                "--ref",
                "refs/notes/ai-sessions",
                "add",
                "-m",
                "existing note",
                &head_hash,
            ],
        );

        // chdir into repo and run hook
        std::env::set_current_dir(repo_path).expect("failed to chdir");
        let result = run_hook_post_commit();
        assert!(result.is_ok());

        // The note should still be the original one (not overwritten)
        let note_output = run_git(
            repo_path,
            &[
                "notes",
                "--ref",
                "refs/notes/ai-sessions",
                "show",
                &head_hash,
            ],
        );
        assert_eq!(note_output, "existing note");

        std::env::set_current_dir(original_cwd).unwrap();
    }

    #[test]
    #[serial]
    fn test_hook_post_commit_no_match_writes_pending() {
        let dir = init_temp_repo();
        let repo_path = dir.path();
        let original_cwd = safe_cwd();

        let head_hash = run_git(repo_path, &["rev-parse", "HEAD"]);

        // Don't create any session logs — the hook should not find a match

        std::env::set_current_dir(repo_path).expect("failed to chdir");
        let result = run_hook_post_commit();
        assert!(result.is_ok());

        // No note should be attached
        let status = std::process::Command::new("git")
            .args(["-C", repo_path.to_str().unwrap()])
            .args([
                "notes",
                "--ref",
                "refs/notes/ai-sessions",
                "show",
                &head_hash,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success());

        // A pending record should have been written
        let pending_path = agents::home_dir()
            .unwrap()
            .join(".ai-barometer")
            .join("pending")
            .join(format!("{}.json", head_hash));
        assert!(pending_path.exists(), "pending record should exist");

        // Clean up: remove the pending record
        let _ = std::fs::remove_file(&pending_path);

        std::env::set_current_dir(original_cwd).unwrap();
    }

    #[test]
    fn test_hook_post_commit_never_fails_outside_git_repo() {
        // When called outside a git repo, the hook should still return Ok
        // because the catch-all wrapper catches errors.
        // Note: we don't chdir — just call it in whatever CWD we have.
        // If the current dir IS a git repo, inner logic may succeed; that's fine.
        // The important thing is that it NEVER returns Err.
        let result = run_hook_post_commit();
        assert!(result.is_ok());
    }
}
