# Cadence CLI

Cadence CLI stores AI coding agent session logs in dedicated Git refs:
- `refs/cadence/sessions/data`
- `refs/cadence/sessions/index/branch`
- `refs/cadence/sessions/index/committer`
It adds provenance for AI-assisted development without altering your commit history.

## Install

Prerequisites:
- Git

macOS and Linux:
```sh
curl -sSf https://raw.githubusercontent.com/TeamCadenceAI/cadence-cli/main/install.sh | sh
```

Windows (PowerShell):
```powershell
iwr -useb https://raw.githubusercontent.com/TeamCadenceAI/cadence-cli/main/install.ps1 | iex
```

If `~/.local/bin` is not on your PATH, add it:
```sh
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
```

Build from source:
```sh
cargo build --release
```

## Quick Start

1. Install hooks:
```sh
cadence install
```

2. Make commits as usual.

3. Inspect session refs:
```sh
git show-ref refs/cadence/sessions/data
```

4. Diagnose install issues (hooks, rewrite safety):
```sh
cadence doctor
```

## Updates and Auto-Update

Cadence has two update paths:

1. Manual update commands:
```sh
cadence update --check   # check only
cadence update           # interactive install if newer stable version exists
cadence update -y        # non-interactive manual install
```

2. Background auto-update (unattended):
- Runs from OS scheduler artifacts created/reconciled by `cadence install`.
- Installs stable releases only (no prereleases).
- Uses a shared activity lock so updater does not interfere with commit/push hook paths.
- Uses retry/backoff when checks or installs fail.
- If disabled (`auto_update=false`), scheduled runs exit immediately without installing.

### Control Commands

```sh
cadence auto-update status
cadence auto-update enable
cadence auto-update disable
cadence auto-update uninstall
```

- `enable`: enables unattended background updates and reconciles scheduler artifacts.
- `disable`: keeps scheduler invocation safe but forces updater no-op behavior.
- `uninstall`: removes scheduler artifacts (idempotent) and disables background auto-update intent.

### Visibility and Repair

```sh
cadence status
cadence doctor
cadence doctor --repair
```

- `status` shows updater state, scheduler state, retry/error context, policy, and remediation hints.
- `doctor` flags broken/missing scheduler states and provides concrete fix commands.
- `doctor --repair` reconciles scheduler artifacts based on current user intent (`auto_update` setting).

## How It Works

Cadence installs global Git hooks that scan for recent AI session logs, then stores canonical session objects and indexes after each commit.
Notes can be synced alongside commits without modifying commit history.

If a repository still has the legacy ref `refs/notes/ai-sessions`, Cadence will migrate it to
`refs/cadence/sessions/data` when new session data is ingested.

## Supported Agents

- Claude Code
- Codex
- Cline
- Roo Code
- OpenCode
- Kiro
- Amp Code
- Cursor
- GitHub Copilot
- Antigravity
- Warp

Note: Warp stores sessions in a local SQLite database. In some local-only cases the
assistant output may be missing, so Cadence stores prompts/context without responses.
OpenCode sessions are normalized from fragmented storage (`session`, `message`, `part`)
into one synthetic session log per session ID before ingestion.

## Optional: Encryption

To encrypt stored session logs (local + API recipients):
```sh
cadence keys setup
```

Cadence uses built-in OpenPGP (Rust) and stores an encrypted private key in `~/.cadence/cli/`.
The passphrase is stored in your OS keychain.

## Uninstall

- Disable and remove auto-update scheduler artifacts:
```sh
cadence auto-update uninstall
```

- Remove hooks:
```sh
git config --global --unset core.hooksPath
rm -rf ~/.git-hooks
```

- Remove the binary from your PATH (for example `~/.local/bin/cadence`).

## License

See `LICENSE.md`.
