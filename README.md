# worki

A macOS app for running coding agents in parallel git worktrees. See [docs/design.md](docs/design.md).

Current state: **M0–M3** — projects, worktree create/remove (with setup scripts), git status per
worktree, agent/shell terminal sessions, and pluggable isolation (agents can run as a dedicated
hidden macOS user; secrets come from the Keychain).

## Develop

Requirements: Rust (stable), Node 20+, pnpm, Xcode command line tools.

```bash
pnpm install
pnpm tauri icon src-tauri/icons/source.png   # once, generates bundle icons
pnpm tauri dev
```

Tests:

```bash
cargo test --workspace
pnpm typecheck
```

## Layout

| Path                 | What                                                        |
|----------------------|-------------------------------------------------------------|
| `crates/worki-core`  | Projects, git worktrees (via `git` CLI), SQLite store, setup scripts, agent profiles |
| `crates/worki-pty`   | PTY sessions with replay buffer                             |
| `crates/worki-isolation` | Isolation backends (none, macOS user) — launch wrapping, ACL grants, doctor, admin setup |
| `src-tauri`          | Tauri shell and IPC commands                                |
| `ui`                 | React + Tailwind frontend                                   |

## Per-repo config

Commit a `.worki.toml` to share worktree setup with the team:

```toml
[worktree]
base = "main"
setup = """
cp "$WORKI_ROOT_PATH/.env.local" .env.local
pnpm install
"""
```

A setup script set in the project settings overrides the file.
