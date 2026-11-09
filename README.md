# worki

A macOS app for running coding agents such as Claude Code and Codex in parallel git worktrees,
each one optionally isolated from your user account.

## Features

- **Projects and worktrees**: add local repositories or projects on SSH hosts; create worktrees
  from any existing branch, with an optional setup script, and remove them again.
- **Terminals**: agent and shell sessions in xterm.js terminals that survive switching views,
  with a status dot per session and worktree showing whether the agent is working, waiting for
  you or idle.
- **Changes**: git status per worktree and a diff panel for changed files.
- **Isolation**, chosen globally, per project or per worktree:
  - *None*: agents run as you.
  - *User isolation*: agents run as a hidden macOS user (`_worki`) that can only reach the
    worktrees it was given.
  - *Docker Sandboxes (sbx)*, *Docker* or *Podman*: one sandbox or container per worktree,
    with the worktree mounted at the same path and git hooks and config read-only.

  A doctor checks each backend and fixes what it can, such as building the default Alpine
  agent image. Environments are removed together with their worktree or project.
- **Agent setup**: install agents into an isolated environment or reuse your own installation,
  copy your agent configuration over, and pass a Claude Code OAuth token. Secrets live in your
  login Keychain.
- **Open in**: open a worktree in your editor, terminal or any app you add in settings.

## Develop

Requirements: Rust (stable), Node 20+, pnpm and the Xcode command line tools.

```bash
pnpm install
pnpm tauri dev
```

Tests:

```bash
cargo test --workspace
pnpm typecheck
```

End-to-end tests against real container engines (need sbx, Docker Desktop or a Podman machine):

```bash
cargo test -p worki-isolation --test containers -- --ignored
```

## Layout

| Path                     | What                                                                       |
|--------------------------|----------------------------------------------------------------------------|
| `crates/worki-core`      | Projects, worktrees and git (local and over SSH), SQLite store, agent settings, launch |
| `crates/worki-pty`       | PTY sessions with replay buffer and agent activity tracking                |
| `crates/worki-isolation` | Isolation backends: macOS user, sbx, Docker, Podman, SSH                   |
| `src-tauri`              | Tauri app and IPC commands                                                 |
| `ui`                     | React + Tailwind frontend                                                  |

Design notes are in [docs/design.md](docs/design.md).

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

## License

[MIT](LICENSE)
