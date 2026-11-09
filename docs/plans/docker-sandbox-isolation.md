# Plan: Docker Sandboxes (`sbx`) as an isolation backend

Status: **draft for review** · Builds on the backend design in [design.md §4.0](../design.md)

## Goal

A third isolation choice next to *None* and *User Isolation*: agents run inside a Docker
Sandbox (microVM managed by `sbx`), one sandbox per worktree. Same worki UX as today: pick it
in Settings → Isolation or per project, sessions open as terminal tabs, Changes/Diff keep
working on the host checkout.

What it adds over User Isolation: a real VM boundary (kernel, filesystem, processes), Linux
toolchains, and network egress policies; secrets injected by Docker's proxy instead of
environment variables.

## What `sbx` gives us (checked against the installed CLI)

| Need | `sbx` feature |
|------|---------------|
| Create per worktree | `sbx create --name N AGENT PATH [PATH:ro …]` |
| Terminal sessions | `sbx exec -it -w DIR N CMD` (starts a stopped sandbox first) |
| Env for a session | `sbx exec -e KEY` (bare key → value from the `sbx` process env, not argv) |
| Status / inventory | `sbx ls --json` (format still to inspect, daemon unreachable from my sandbox) |
| Teardown | `sbx stop N`, `sbx rm --force N` |
| Files in/out | `sbx cp`, `sbx exec … cat` |
| Credentials | `sbx secret set anthropic|github|openai …` — proxy injects, agent never sees the value |
| Network rules | `sbx policy allow/deny`, `--deny-network` at create |
| Health | `sbx daemon status`, `sbx diagnose` |
| Agent images | built-in agents: claude, codex, gemini, cursor, opencode, shell, … |

## Design

### 1. Config

```rust
enum Isolation {
    None,
    MacUser { user: String },
    DockerSandbox {
        /// Built-in sbx agent image the sandbox is created from ("claude", "codex", "shell").
        image: String,
        cpus: Option<u32>,
        memory: Option<String>,          // "8g"
        /// Hosts denied at creation; finer rules via `sbx policy`.
        deny_network: Vec<String>,
    },
}
```

`scope_key` = `docker-sandbox`. Picker label: **Docker Sandbox**.

### 2. One sandbox per worktree

- Name: `worki-<project-slug>-<branch-slug>-<6-char hash of worktree path>` (sbx allows
  letters, digits, `-`, `.`).
- Created lazily in `grant_access(ws)` when a session or setup script first needs it;
  idempotent (look it up in `sbx ls --json` first).
- Mounts (bind mode):
  - worktree — read/write
  - `<repo>/.git` — read/write (commits land in the shared object store)
  - `<repo>/.git/config`, `<repo>/.git/hooks`, worktree `.git` pointer, admin `gitdir`/`commondir` — `:ro`
    (same rule as User Isolation: nothing the agent writes may run as you on the host)
- Removed when the worktree is removed (new trait hook, see §5), stopped when worki quits
  (configurable later).

### 3. Launching sessions

`Backend::launch` returns:

```
program: sbx
args:    exec -it -w <worktree> -e TERM -e WORKI_SESSION_ID … <sandbox> bash -lc '<command>'
env:     TERM=… WORKI_SESSION_ID=…   (values only in sbx's own env, never argv)
```

The sandbox name is derived from `req.cwd` (the worktree). Agent profiles keep working: the
profile command (`claude --model opus`) runs inside the sandbox.

### 4. Agents and auth

- **Use my installation** doesn't apply (host binaries are macOS, the sandbox is Linux); the
  Agents panel asks the backend for capabilities and hides it. **Install** works: the
  profile's install command runs inside the sandbox via `sbx exec`.
- **Which agents exist**: per image (claude image ships claude). A worktree started with
  `claude` can still open a Shell tab. Multiple different agents in one worktree need the
  `shell` image plus installs, or a custom template — see open questions.
- **Copy my config**: via `sbx cp` into the sandbox home, reusing the existing kit
  definitions (paths rewritten to the sandbox home).
- **Sign in**: store the token with `sbx secret set anthropic` instead of passing
  `CLAUDE_CODE_OAUTH_TOKEN` as env — Docker's proxy injects it, the agent never holds it.
  worki's own secrets for this scope go through the same command.

### 5. Backend trait changes

Additions (default implementations keep existing backends unchanged):

```rust
trait Backend {
    fn capabilities(&self) -> Capabilities;       // share_host, install, copy_config, secrets_mode
    fn release(&self, ws: &Workspace) -> Result<()>; // worktree removed → sbx rm
    fn store_secret(&self, name: &str, value: &str) -> Result<()>; // default: worki vault + env
}
```

`release` is called from `remove_worktree`. `remove()` (Remove isolation) deletes every
`worki-*` sandbox.

### 6. Doctor checks

1. `sbx` installed (Homebrew path or `$PATH`) — fix: show install command.
2. Daemon running (`sbx daemon status`) — fix: `sbx daemon start`.
3. `sbx diagnose` clean (Docker Desktop / VM runtime).
4. Anthropic/GitHub secret present (`sbx secret ls`) — fix: Sign in.
5. Per project: sandbox for the selected worktree exists and is running; mounts are as planned.

### 7. UI

- IsolationPicker: new option with image select, CPU/memory, deny-network list.
- Session tab badge: `sbx` instead of `_worki`.
- Worktree header: small sandbox status (running/stopped) with Stop/Restart in the menu.

## Milestones

| # | Scope |
|---|-------|
| S1 | Spike: verify mount paths, git worktree inside the VM, `exec -it` resize, `ls --json` shape |
| S2 ✅ | Backend: config variant, create/lookup/launch/release/remove, doctor, stop on quit. End-to-end test: `cargo test -p worki-isolation --test docker_sandbox -- --ignored` |
| S3 ✅ | Agents: capabilities in panel; Install and Copy my config run in every worki sandbox and are remembered for new ones (tar stream through `sbx exec -i`); token stays per-session env |
| S4 | UI: picker options, status in header, policies editor (allow/deny hosts) |

## S1 spike results (2026-11-09, sbx v0.47.0)

Run against a throwaway repo + worktree in `/private/tmp`, sandboxes removed afterwards.

| Question | Result |
|----------|--------|
| **Mount paths** | ✅ Same absolute path inside the VM (`virtiofs`). The worktree's `gitdir: /private/tmp/…/.git/worktrees/wt-feature` resolves as-is. → **bind mode works; `--clone` not needed.** |
| **Git in the sandbox** | ✅ `git status` and `git commit` inside the worktree work; the commit shows up on the host branch immediately. |
| **`:ro` for a folder** | ✅ `.git/hooks:ro` → "Read-only file system"; nothing appears on the host. |
| **`:ro` for a file** | ✅ `.git/config:ro` blocks both writing and deleting. |
| **File ownership** | ✅ Files the sandbox creates belong to the host user (uid 501), so worki and the user can edit them. Inside, the agent runs as `agent` (uid 1000, in `sudo` — fine inside the VM). |
| **Env without argv** | ✅ `sbx exec -e KEY` takes the value from sbx's own environment. |
| **TTY** | ✅ `sbx exec -it` inside a PTY reports the terminal size (`stty size` → `26 54`). |
| **`ls --json`** | `{ "sandboxes": [ { name, id, agent, status: running/stopped, workspaces: [paths], created_at, last_used_at } ] }`. Other sandboxes of the user live there too → worki only touches names starting with `worki-`. |
| **Images** | `shell`: Ubuntu 26.04, bash, git, gh, node/npm, python3, curl — **no** claude/codex. `claude`: same plus Claude Code 2.1.306. Agents missing from an image can be installed inside with the profile's install command (runs through `sbx exec`). |
| **Anthropic secret** | ⚠️ `sbx secret set anthropic` documents API keys only; OAuth (`--oauth`) is OpenAI-only locally (Anthropic OAuth only with `--cloud`). A `setup-token` token therefore goes in as env (`-e CLAUDE_CODE_OAUTH_TOKEN`), like today. API keys can use the proxy. |

### Decisions from the spike

1. **Bind mode** with the mount layout from §2 (worktree rw, `.git` rw, `config`/`hooks`/pointers `:ro`).
2. **Default image `claude`**; other agents via their install command inside the sandbox.
   The Agents panel keeps **Install** for this backend (it runs inside the VM) and hides
   **Use my installation** (host binaries are macOS).
3. **Secrets**: `CLAUDE_CODE_OAUTH_TOKEN` via `sbx exec -e`; API-key services may use
   `sbx secret` later (S3).
4. **Ownership**: only sandboxes named `worki-*` are listed, stopped or removed by worki.

5. **Lifecycle**: worki stops all its `worki-*` sandboxes when it quits (`sbx stop`, state is
   kept); the next session in that worktree starts it again (`sbx exec` starts a stopped
   sandbox first). Removing a worktree removes its sandbox (`sbx rm`).

## Out of scope

- **Cloud** sandboxes (`sbx --cloud`).
