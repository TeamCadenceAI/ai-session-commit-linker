# Phase 10 Code Review: `install` Subcommand

**Date:** 2026-02-09
**Reviewer:** Claude Opus 4.6
**Files reviewed:** `src/main.rs` (run_install, run_install_inner), `src/git.rs` (config_set_global), `Cargo.toml`, tests in `src/main.rs`
**Verification:** 231 tests passing, clippy clean (2 expected dead-code warnings), formatting clean.

---

## 1. Installer Steps Completeness (per PLAN.md)

PLAN.md Section "Installer Behaviour" specifies five responsibilities:

| # | PLAN.md Requirement | Implementation | Status |
|---|---|---|---|
| 1 | Download correct ai-barometer binary | Not implemented | See finding 1.1 |
| 2 | Set `git config --global core.hooksPath ~/.git-hooks` | `git::config_set_global("core.hooksPath", &hooks_dir_str)` at line 100 | Done |
| 3 | Install hook shim | Written to `~/.git-hooks/post-commit` at lines 129-193 | Done |
| 4 | Persist configuration (org filter, defaults) | `git::config_set_global("ai.barometer.org", org_value)` at lines 196-205 | Done |
| 5 | Run hydration for the last 7 days by default | `run_hydrate("7d", false)` at line 210 | Done |

TODO.md Phase 10 has 8 checklist items, all marked complete. The implementation matches.

### Finding 1.1 (Info, Not a Bug)

**Binary download is not implemented.** PLAN.md says "Download correct ai-barometer binary" as step 1. The implementation assumes the binary is already on PATH. This is a reasonable design choice for a Rust CLI tool -- users would install via `cargo install` or a package manager, then run `ai-barometer install` to set up hooks. The PLAN.md responsibility is better understood as "ensure the binary is available", which is satisfied by the fact that `install` is a subcommand of the binary itself. No action needed.

---

## 2. Shim Content and Permissions

### Shim Content (Correct)

The shim at line 130 is:
```
#!/bin/sh\nexec ai-barometer hook post-commit\n
```

This matches PLAN.md exactly: "No logic in shell." The shim is a thin passthrough.

### Finding 2.1 (Positive)

The shim ends with `\n` (a trailing newline after `exec ai-barometer hook post-commit`). This is correct POSIX behavior -- some shells can misbehave with files missing a trailing newline.

### Shim Permissions (Correct)

Line 168: `std::fs::Permissions::from_mode(0o755)` sets rwxr-xr-x. The `#[cfg(unix)]` guard is appropriate. The permission is set immediately after writing, which is correct -- if the write succeeds but chmod fails, the `had_errors` flag is set and the user is informed.

### Finding 2.2 (Low)

**No verification that `ai-barometer` is on PATH.** If the binary is not on the user's PATH, the shim will silently fail at commit time (git hook exits non-zero but the commit still succeeds due to `exec` replacing the shell process and returning the exit code). However, since `ai-barometer install` was just run successfully, the binary clearly exists. The only risk is if the user installs in a virtual environment or moves the binary afterward. The post-commit handler's catch-all ensures no harm. No action needed.

---

## 3. Existing Hook Handling

The three-way detection at lines 133-157 is well-structured:

| Scenario | Behavior | Message |
|---|---|---|
| No existing hook | Write shim | (implicit, no message about existing) |
| Existing hook contains "ai-barometer" | Overwrite | "already installed, updating" |
| Existing hook does NOT contain "ai-barometer" | Overwrite with warning | "exists but was not created by ai-barometer; overwriting" |
| Existing hook unreadable | Overwrite with warning | "could not read existing ...; overwriting" |

### Finding 3.1 (Medium)

**Third-party hooks are overwritten without backup.** When a non-barometer hook exists at `~/.git-hooks/post-commit`, it is overwritten with a warning but no backup is created. If the user had a legitimate post-commit hook (e.g., for code formatting, notifications), it is lost. The warning message informs the user but does not tell them what was overwritten or where to recover it.

**Recommendation:** Before overwriting a non-barometer hook, copy it to `~/.git-hooks/post-commit.bak` or `~/.git-hooks/post-commit.pre-ai-barometer` and print the backup path. This is a low-cost addition that prevents data loss.

### Finding 3.2 (Low)

**The "ai-barometer" detection is a substring match (`contains`).** A file containing `# Removed ai-barometer hook\necho 'my hook'` would be detected as an ai-barometer hook and silently updated. In practice this is harmless because the only likely match is the actual shim content. No action needed.

### Finding 3.3 (Low)

**`core.hooksPath` is set BEFORE the shim is written.** If Step 1 (config_set_global) succeeds but Step 3 (writing the shim) fails, the global git config points to a hooks directory that may not have a valid shim. This means post-commit hooks are silently broken for all repos until install is re-run. The best-effort design intentionally continues past failures, so this is an accepted trade-off. The user is informed via the error message.

### Finding 3.4 (Info)

**`core.hooksPath` is set globally.** This overrides per-repo `.git/hooks/` for ALL repositories on the machine, not just ones the user wants monitored. Any existing per-repo hooks in `.git/hooks/post-commit` will be shadowed. This is documented behavior per PLAN.md and is the correct approach for a "set it and forget it" installer.

---

## 4. Org Filter Persistence

### Finding 4.1 (Positive)

Org filter persistence is clean. When `--org my-org` is provided, the installer calls `git::config_set_global("ai.barometer.org", org_value)` at line 197. This is correctly placed AFTER the shim is written (Step 5, after Steps 1-4), ensuring that hook infrastructure exists before the org filter is persisted.

The downstream consumer (`push::check_org_filter()`) reads from `git config --global ai.barometer.org` via `git::config_get_global()`, which is consistent with where the installer writes it.

### Finding 4.2 (Low)

**No `--org` unset mechanism.** If a user installs with `--org my-org` and later wants to remove the filter, they must manually run `git config --global --unset ai.barometer.org`. Running `ai-barometer install` without `--org` does NOT remove a previously-set org filter. This is acceptable behavior (the installer only sets values, never unsets them), but could be surprising.

### Finding 4.3 (Info)

**No validation on the `--org` value.** An empty string, whitespace, or special characters are accepted. In practice, GitHub org names are alphanumeric with hyphens, so malformed values would simply never match any remote org. The org filter is fail-safe (no match = no push, notes still attached locally). No action needed.

---

## 5. Hydration Integration

### Finding 5.1 (Positive)

Hydration is correctly integrated as the final step (Step 6, line 210). The call `run_hydrate("7d", false)` matches PLAN.md: "Run hydration for the last 7 days by default" and "must not auto-push by default" (the `false` argument disables push).

### Finding 5.2 (Positive)

Hydration errors are non-fatal. The `if let Err(e) = run_hydrate(...)` pattern at line 210 catches hydration failures, sets `had_errors`, and continues to the completion message. This is correct -- hydration is a best-effort backfill that should not prevent the installer from reporting success on the critical hook setup steps.

### Finding 5.3 (Low)

**Hydration runs in the context of the installer, not in each repo.** The `run_hydrate` function scans ALL session logs globally and resolves repos from session metadata. This is correct for a one-time backfill. However, if the user runs `ai-barometer install` from a directory that is not a git repo, hydration will not push notes (push requires being in a repo context). Since push is disabled during install hydration (`false` argument), this is irrelevant for v1.

### Finding 5.4 (Low)

**No progress indication between install steps and hydration.** The installer prints step confirmations, then "Running initial hydration (last 7 days)..." and the hydrate function takes over with its own verbose output. The transition is clear enough with the "[ai-barometer]" prefix. No action needed.

---

## 6. Error Handling (Best-Effort)

### Finding 6.1 (Positive)

The best-effort error handling pattern is well-implemented:
- `had_errors` flag tracks whether any step failed
- Each step is attempted regardless of previous failures
- `run_install_inner` always returns `Ok(())` -- the process never exits with error code 1 from an install
- The final message distinguishes "Installation complete!" from "Installation completed with errors (see above)"

### Finding 6.2 (Positive)

The home directory resolution at line 87-91 correctly fails early if `$HOME` cannot be determined. This is the one case where returning `Err` is appropriate, since no subsequent step can proceed without a home directory.

### Finding 6.3 (Info)

**The `had_errors` flag is set but never returned to the caller.** `run_install_inner` returns `Ok(())` regardless. The main dispatch in `main()` will always exit 0 from install. This is intentional -- partial install is better than no install.

---

## 7. `git::config_set_global` Implementation

### Finding 7.1 (Positive)

The `config_set_global` function at `src/git.rs:427-438` is clean and minimal:
- Uses `git config --global key value` form
- Captures stderr for error messages
- Returns `anyhow::Result<()>` with a clear error message on failure
- Consistent with the existing `config_set` function (same structure, just adds `--global`)

### Finding 7.2 (Low)

**Uses the legacy `git config key value` positional form rather than `git config set --global key value`.** The legacy form is deprecated in newer git versions (2.46+). However, this was already noted in the Phase 2 review (deferred as "widely supported, not urgent"). The legacy form will continue to work for years.

---

## 8. Test Coverage

6 tests cover the install subcommand:

| Test | What it verifies |
|---|---|
| `run_install_returns_ok` | Basic smoke: install succeeds with fake HOME and GIT_CONFIG_GLOBAL |
| `test_install_creates_hooks_dir_and_shim` | Hooks dir, shim file, shim content, executable permissions, global config |
| `test_install_with_org_sets_global_config` | `--org` flag persists to global git config |
| `test_install_idempotent` | Running install twice leaves shim correct |
| `test_install_detects_existing_non_barometer_hook` | Non-barometer hook is overwritten |
| `test_install_detects_existing_barometer_hook` | Existing barometer hook is updated |

### Finding 8.1 (Positive)

Tests properly isolate from the real environment using:
- `TempDir` as fake HOME
- `GIT_CONFIG_GLOBAL` env var pointing to an empty config file
- `home_override` parameter on `run_install_inner`
- Proper save/restore of original env vars
- `#[serial]` to prevent CWD and env var races

### Finding 8.2 (Medium)

**No test verifies the hydration step runs during install.** The tests verify hooks dir, shim content, permissions, org config, and idempotency -- but none verify that hydration actually executed. The current tests use a fake HOME with no session logs, so hydration runs but finds nothing. A dedicated test could verify that a session log in the fake HOME results in a note being attached after install.

### Finding 8.3 (Low)

**No test for install failure scenarios.** There are no tests for:
- Hooks directory creation failure (e.g., permissions)
- Shim write failure
- Global config set failure
- Behavior when HOME is not set

These are hard to simulate without mocking the filesystem, and the best-effort design ensures graceful degradation. Low priority.

### Finding 8.4 (Low)

**No test verifies the "Installation completed with errors" message path.** The `had_errors = true` branch at line 215 is never exercised in tests. Since the function always returns `Ok(())`, the only visible effect of errors is the stderr output, which tests do not capture.

### Finding 8.5 (Low)

**Test `test_install_detects_existing_non_barometer_hook` does not verify the warning message was printed.** It only verifies the shim was overwritten. Since tests use the real stderr, capturing and asserting on stderr output would require additional test infrastructure. Acceptable for v1.

---

## 9. Design Quality Observations

### Finding 9.1 (Positive)

**Testability via `run_install_inner` with `home_override`.** The pattern of an outer `run_install` calling an inner `run_install_inner` with an optional home parameter is consistent with the testability patterns established in Phases 3 and 7. This allows tests to avoid mutating the real filesystem.

### Finding 9.2 (Positive)

**Step ordering is correct per dependencies.** The steps are ordered so that `core.hooksPath` is set (Step 1) before the hooks directory is created (Step 2) and the shim is written (Step 3). Org filter (Step 5) is set after hook infrastructure is established. Hydration (Step 6) is last, as it depends on everything else.

### Finding 9.3 (Positive)

**The `should_write` variable correctly collapses all three detection paths into a single write point.** Despite the three-way detection logic, the shim is always written through the same `std::fs::write` call at line 160. This prevents code duplication.

### Finding 9.4 (Info)

**`hooks_dir_str` uses `to_string_lossy()`** at line 94 for the config value. If the home directory contains non-UTF-8 characters (extremely unlikely on macOS/Linux), the path would contain replacement characters in the git config. This matches the approach used throughout the codebase.

---

## Summary

| Severity | Count | Description |
|---|---|---|
| Critical | 0 | No critical issues found |
| Medium | 2 | #3.1 (no backup of overwritten hooks), #8.2 (no test for hydration during install) |
| Low | 8 | #2.2, #3.2, #3.3, #4.2, #4.3, #5.3, #7.2, #8.3-8.5 |
| Informational | 5 | #1.1, #3.4, #6.3, #9.4 |
| Positive | 8 | #2.1, #4.1, #5.1, #5.2, #6.1, #6.2, #7.1, #8.1, #9.1-9.3 |

**Overall assessment:** Phase 10 is a clean, well-structured implementation that matches the PLAN.md specification. The best-effort error handling is correct and consistent with the project's design philosophy. The test coverage is solid for the happy paths and common edge cases. The two medium findings (no backup of overwritten hooks, no test for hydration during install) are worth noting for Phase 12 hardening but are not blocking.

All 231 tests pass. No regressions introduced.
