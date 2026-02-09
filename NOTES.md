# Implementation Notes

Notes for future subagents working on subsequent phases.

---

## Phase 1 Decisions

### Rust Edition
- The project uses Rust edition 2024 (set by `cargo init` on Rust 1.91.1). This is the latest edition and requires a recent toolchain.

### CLI Structure
- The CLI uses `clap` derive API with a top-level `Cli` struct containing a `Command` enum.
- The `hook` subcommand uses nested subcommands via a separate `HookCommand` enum. This means invocation is `ai-barometer hook post-commit` which matches the plan's hook shim: `exec ai-barometer hook post-commit`.
- Each subcommand dispatches to a standalone `run_*` function that returns `anyhow::Result<()>`. This keeps the dispatch in `main()` clean and allows each handler to use `?` freely.
- `main()` catches all errors and prints them to stderr with the `[ai-barometer]` prefix, then exits with code 1. This ensures the binary never panics on user-facing errors.

### Error Handling
- Using `anyhow::Result` for all fallible functions. Switched from `Box<dyn std::error::Error>` during the Phase 1 review triage because it was early enough to make the change cheaply. `anyhow` provides better error context (via `.context()`) and is the idiomatic choice for application-level error handling in Rust. If we later need structured error types for specific modules, we can introduce `thiserror` enums that work with `anyhow`.

### Dependencies Included But Not Yet Used
- `serde`, `serde_json`, `sha2`, and `chrono` are declared in Cargo.toml but not imported in main.rs yet. This is intentional -- they are needed starting in Phase 2+. Clippy does not warn about unused crate dependencies.

### Test Strategy
- Tests use `Cli::parse_from()` to verify CLI parsing without spawning a subprocess. This is fast and avoids PATH issues.
- Each `run_*` function has a basic "returns Ok" smoke test. These will evolve into real integration tests as logic is added.

### File Layout
- All code is currently in `src/main.rs`. As Phase 2+ adds modules (git utilities, agent discovery, scanner, etc.), these should be extracted into separate files under `src/` (e.g., `src/git.rs`, `src/agents.rs`, `src/scanner.rs`, `src/note.rs`, `src/pending.rs`).

---

## Phase 1 Review Triage

A code review was conducted after Phase 1. The review found no bugs and confirmed all TODO items were complete. The following issues were triaged and addressed:

### Fixed
1. **Removed redundant `default_value_t = false`** on the `push` bool flag in the `Hydrate` variant. Clap already defaults bool flags to `false`.
2. **Switched to `anyhow` for error handling.** Replaced `Box<dyn std::error::Error>` with `anyhow::Result` throughout. This was cheap to do now with minimal code, and `anyhow` is the standard choice for Rust application error handling.
3. **Added negative CLI parsing tests.** Four new tests using `Cli::try_parse_from()` verify that clap correctly rejects: unknown subcommands, `hook` with no sub-subcommand, `--since` with a missing value, and no subcommand at all.

### Deferred
- **`assert_cmd` integration test:** Deferred to a later phase. Not needed while all logic is in stubs.
- **Edition 2024 downgrade:** Not needed. The edition is fine for this project.
- **`chrono` serde feature review:** Will revisit when timestamps are actually used (Phase 4+).
