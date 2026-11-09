# worki — design

A macOS desktop app for running coding agents (Claude Code, Codex, plain shells, …) in
parallel git worktrees, each agent running as a separate, less-privileged macOS user.

Status: **draft for review** · Stack: Rust + Tauri 2 + React/TypeScript

---

## 1. Goals and non-goals

**Goals**

- Left sidebar listing projects (git repos) and their active worktrees, with live agent status.
- Main area with one or more sessions per worktree: an agent terminal (and later a structured
  agent UI), plus extra shells.
- Create / remove worktrees from the app, optionally running a per-project setup script.
- Agents run as a dedicated macOS user — one global user by default, overridable per project,
  or "run as me" per project.
- Modern native-feeling UI (not a TUI), macOS first.

**Non-goals (for now)**

- Windows/Linux support (keep the code portable where it's free, but don't design for it).
- Network sandboxing of agents, remote/SSH hosts, cloud sandboxes.
- Pushing, opening PRs, or any write to remotes. Read-only `gh` is fine for status.

---

## 2. High-level architecture

```
┌──────────────────────────── Tauri app (worki.app) ─────────────────────────────┐
│                                                                                │
│  WebView (React + TS)                     Rust core                            │
│  ┌──────────────┬──────────────────┐      ┌─────────────────────────────────┐  │
│  │ Sidebar      │ Session tabs     │ IPC  │ ProjectService  (git CLI)       │  │
│  │  projects    │ ┌──────────────┐ │◄────►│ SessionHost     (portable-pty)  │  │
│  │   worktrees  │ │ xterm.js     │ │cmds +│ Isolation       (users, ACLs)   │  │
│  │   status ●   │ │ (WebGL)      │ │chans │ StatusBus       (hooks socket)  │  │
│  │              │ └──────────────┘ │      │ Store           (SQLite)        │  │
│  └──────────────┴──────────────────┘      └───────────┬─────────────────────┘  │
└───────────────────────────────────────────────────────┼────────────────────────┘
                                                        │ sudo -n -u _worki …
                                     ┌──────────────────▼──────────────────┐
                                     │ agent process (claude / codex / zsh) │
                                     │ uid=_worki, cwd=<worktree>           │
                                     │ hooks → worki-hook → unix socket     │
                                     └──────────────────────────────────────┘
```

### Repository layout

```
worki/
├── Cargo.toml                 # workspace
├── crates/
│   ├── worki-core/            # domain: projects, worktrees, config, store
│   ├── worki-pty/             # SessionHost: PTY spawn, I/O pumping, replay buffer
│   ├── worki-isolation/       # agent users, sudoers, ACLs, launcher, doctor
│   └── worki-hook/            # tiny CLI agents call from hooks → status socket
├── src-tauri/                 # Tauri shell: commands, channels, window setup
└── ui/                        # React + Vite + TS + Tailwind, xterm.js
```

Keeping domain logic out of `src-tauri` means it is unit-testable without a window and makes a
later move of `SessionHost` into a background daemon (§9) mechanical.

### Key crate choices

| Concern            | Choice                                    | Why                                                               |
|--------------------|-------------------------------------------|-------------------------------------------------------------------|
| App shell          | `tauri` 2                                 | Small binary, WKWebView, good IPC, mature plugin set              |
| PTY                | `portable-pty`                            | Battle-tested (wezterm), handles macOS PTY quirks                 |
| Terminal render    | `@xterm/xterm` + webgl, fit, unicode11, web-links addons | Best-in-class web terminal; used by VS Code     |
| Git                | `git` CLI via `tokio::process`            | Full worktree support, respects user config/hooks; libgit2 worktree support is partial |
| Persistence        | `rusqlite` (bundled)                      | Projects, worktree metadata, session history; easy migrations    |
| Async              | `tokio`                                   | Already in Tauri                                                  |
| Window chrome      | `window-vibrancy`, `titleBarStyle: Overlay` | Native sidebar translucency, traffic lights over sidebar        |
| UI kit             | Tailwind + shadcn/ui (Radix) + lucide icons | Modern look, accessible primitives, no heavy framework          |
| Frontend state     | Zustand + TanStack Query                  | Simple stores; query cache for git status                         |

---

## 3. Domain model

```rust
struct Project {
    id: ProjectId,
    name: String,
    repo_path: PathBuf,              // main checkout
    worktree_root: PathBuf,          // where new worktrees go (default below)
    default_base: String,            // e.g. "main"
    run_as: RunAs,                   // Inherit | Me | User(AgentUserName)
    setup_script: Option<String>,    // also loadable from .worki.toml in the repo
}

struct Worktree {
    project: ProjectId,
    path: PathBuf,
    branch: Option<String>,          // None = detached
    head: String,
    is_main: bool,
    // derived/cached: dirty, ahead/behind, diff stat
}

struct Session {
    id: SessionId,
    worktree: PathBuf,
    kind: SessionKind,               // Agent(AgentProfile) | Shell
    run_as: ResolvedUser,
    state: SessionState,             // Starting | Running | WaitingForInput | Idle | Exited(code)
}

struct AgentProfile {               // user-editable presets
    name: String,                    // "Claude Code", "Codex", "Aider", …
    command: Vec<String>,            // ["claude"], ["codex"], …
    env: BTreeMap<String, EnvValue>, // literal or keychain ref
    hooks: HookIntegration,          // ClaudeCode | Codex | None
}
```

Worktrees are **discovered** from `git worktree list --porcelain` rather than stored, so worktrees
created outside the app show up and deleted ones disappear. The store only holds metadata worki
adds (display name, pinned, last session, notes).

**Default worktree location:** `~/worki/worktrees/<project>/<branch-slug>` — outside the repo
(no `.gitignore` noise) and outside TCC-protected folders (`~/Documents`, `~/Desktop`), which the
agent user can never read. Configurable per project.

### Per-project config file (optional)

`.worki.toml` in the repo, so setup is shared with the team:

```toml
[worktree]
base = "main"
setup = """
cp "$WORKI_ROOT_PATH/.env.local" .env.local
pnpm install
"""

[[agents]]
name = "Claude Code"
command = ["claude"]
```

Setup scripts run as the **agent user** in the new worktree (they produce files the agent will
own), with `WORKI_ROOT_PATH`, `WORKI_WORKTREE_PATH`, `WORKI_BRANCH` set.

---

## 4. Isolation

### 4.0 Backends

Isolation is pluggable (`crates/worki-isolation`). A backend implements:

| Method          | Purpose                                                             |
|-----------------|---------------------------------------------------------------------|
| `launch`        | Turn a mechanism-neutral `LaunchRequest` (cwd, shell code, env) into the concrete process to spawn |
| `grant_access`  | Give the environment access to a project's repo and worktree root  |
| `prepare`       | Unprivileged per-environment setup                                  |
| `provision`     | One-time privileged setup (may prompt for a password)               |
| `doctor`        | Health checks, each with an optional `Fix` the UI can trigger       |

Config is a tagged enum (`Isolation::None`, `Isolation::MacUser { user }`); a project either
inherits the global default or picks its own. Sessions and setup scripts both go through
`launch`, so a new mechanism (sandbox-exec profile, micro-VM, container) is one new variant plus
one backend, with no changes to callers or the UI beyond a picker option.

The rest of this section describes the `MacUser` backend.

### 4.1 Configuration

```
global.default_run_as = User("_worki")          # created by onboarding
project.run_as        = Inherit | Me | User(name)
```

Resolution: `project.run_as` → `global.default_run_as` → `Me`. Per-project users are named
`_worki_<slug>`. All agent users are members of the group `_worki_agents`.

### 4.2 One-time privileged setup

Only two operations need admin rights: **creating/deleting users** and **installing the sudoers
rule**. Everything else (ACLs on the human's files, launching) runs unprivileged.

MVP: a bundled script `worki-admin.sh` run via
`osascript -e 'do shell script "…" with administrator privileges'`, which shows the standard
macOS admin password dialog. Later: replace with an `SMAppService` privileged helper (needs a
signed app).

`worki-admin.sh create-user <name>`:

```sh
uid=$(next free UID in 400..499)        # <500 → hidden from login window
dscl . -create /Users/$name
dscl . -create /Users/$name UserShell /bin/zsh
dscl . -create /Users/$name RealName "worki agent"
dscl . -create /Users/$name UniqueID $uid
dscl . -create /Users/$name PrimaryGroupID 20      # staff
dscl . -create /Users/$name NFSHomeDirectory /Users/$name
dscl . -create /Users/$name IsHidden 1
dscl . -create /Users/$name Password '*'           # no password, no GUI login
dseditgroup -o edit -a $name -t user _worki_agents
createhomedir -c -u $name
```

`worki-admin.sh install-sudoers <human>` writes `/etc/sudoers.d/worki` (validated with
`visudo -cf` before moving into place):

```
<human> ALL=(%_worki_agents) NOPASSWD: ALL
```

Implementation note: users get UIDs in 450–499 (hidden from the login window); the script lives
in `crates/worki-isolation/src/worki-admin.sh` and is idempotent.

This lets the human run anything **as an agent user** without a password, and nothing more —
privilege only flows downward. Because it's group-based, adding per-project users never needs
another sudoers edit.

### 4.3 Launching

Every agent/shell/setup process for an agent user is spawned with an **empty environment** as:

```
sudo -n -u _worki -H --preserve-env=TERM,…,CLAUDE_CODE_OAUTH_TOKEN -- /bin/zsh -l -i -c 'exec claude'
```

Values are handed over through sudo's own environment and `--preserve-env=<names>` (allowed
because the rule's command is `ALL`, which implies `SETENV`), so secrets never appear in argv
where `ps` would show them. The earlier `env -i …` sketch below is superseded by this.

Original sketch:

```
sudo -n -u _worki -H -- /usr/bin/env -i \
    HOME=/Users/_worki USER=_worki LOGNAME=_worki SHELL=/bin/zsh \
    TERM=xterm-256color COLORTERM=truecolor LANG=en_US.UTF-8 \
    PATH=<agent PATH> \
    WORKI_SESSION_ID=<id> WORKI_SESSION_TOKEN=<random> WORKI_SOCKET=<path> \
    <profile env, e.g. CLAUDE_CODE_OAUTH_TOKEN=…> \
    /bin/zsh -l -c 'cd "$1" && exec "${@:2}"' _ <worktree> <command…>
```

- `-n` makes a missing/broken sudoers rule fail immediately instead of hanging on a prompt; the
  doctor (§4.6) explains the fix.
- `env -i` gives a clean, explicit environment — the human's env (tokens, `SSH_AUTH_SOCK`, …)
  never leaks to the agent by accident.
- A login shell picks up the agent user's own `~/.zprofile` (e.g. Homebrew, nvm) so tools
  resolve the same way they would interactively.
- The command is passed as argv, never interpolated into a shell string.

### 4.4 File access

The agent user must be able to read/write the worktree **and** parts of the main repo's `.git`
(commits write to `.git/objects`, `.git/refs`, `.git/logs`, `.git/worktrees/<name>`).

worki grants this with inheritable macOS ACLs — no root needed because the human owns these
files:

```sh
ACE="allow list,add_file,search,delete,add_subdirectory,delete_child,readattr,writeattr,\
readextattr,writeextattr,readsecurity,read,write,append,execute,file_inherit,directory_inherit"
chmod -R +a "group:_worki_agents $ACE" <worktree>  <repo>/.git
chmod -R +a "user:<human> $ACE"        <worktree>  <repo>/.git   # files the agent creates stay ours
chmod    +a "group:_worki_agents allow search"   <each ancestor dir up to /Users/<human>>
```

- The `search`-only ACE on ancestors allows traversal without listing; it's added only when the
  doctor finds the ancestor isn't already traversable.
- The agent can still touch any branch via the shared `.git`. That's inherent to worktrees and
  acceptable: the isolation goal is "agent can't read my home, keychain, SSH keys, other repos",
  not "agent can't touch other branches of this repo".
- Agent git config (`/Users/_worki/.gitconfig`, written at setup): `safe.directory = *`
  (otherwise git refuses repos owned by another user), `user.name`/`user.email` copied from the
  human's config so commits are attributed sensibly.
- Removing a worktree: `git worktree remove` as the human works because the inherited ACL gives
  the human `delete` on agent-created files.

### 4.5 Credentials and tools for the agent user

- **Agent binaries** must be reachable by the agent user. `/opt/homebrew/bin` works as-is;
  binaries under the human's home (`~/.local/bin/claude`) don't. Onboarding offers to install
  the agent CLI into the agent user's home (`sudo -u _worki -i <installer>`).
- **Claude Code** stores credentials in the macOS Keychain, which is unavailable to a
  `sudo`-launched user with no GUI session. Use a long-lived token instead
  (`claude setup-token` → `CLAUDE_CODE_OAUTH_TOKEN`) or `ANTHROPIC_API_KEY`.
- worki stores such secrets in **the human's** Keychain (`security-framework` crate) and injects
  them per launch through `AgentProfile.env` with `EnvValue::Keychain(item)`. They never touch
  disk in plaintext.
- **GitHub**: a fine-grained, repo-scoped `GH_TOKEN`, injected the same way. No SSH agent
  forwarding by default.

### 4.6 Doctor

A "Check setup" screen (also run on startup, cheap checks only) that reports each item with a
one-click fix where possible:

1. Agent user exists, is hidden, has a home dir.
2. `sudo -n -u _worki true` succeeds.
3. Agent can traverse to and write in the worktree root and `<repo>/.git` (spawn
   `sudo -u _worki test -w …`).
4. Agent `safe.directory` configured.
5. Each agent profile's command resolves for the agent user (`command -v claude`).
6. Required env secrets are present in the Keychain.

---

## 5. Sessions and terminals

### 5.1 SessionHost (Rust)

- Spawns the command in a `portable-pty` PTY sized to the frontend's cols/rows.
- A reader thread per PTY reads into a buffer and flushes to the UI every ~8 ms or 64 KB,
  whichever comes first (avoids per-byte IPC under heavy output).
- Each session keeps a **replay ring buffer** (default 4 MB) of raw output. When a terminal view
  is (re)attached — switching worktrees, WebView reload — the UI gets the buffer first, then the
  live stream. xterm.js re-parses it, so colors and cursor state are restored.
- Resize, write, kill (SIGHUP to process group, then SIGKILL after a timeout).
- On exit, the session stays listed as `Exited(code)` with its buffer until dismissed.

```rust
#[async_trait]
trait SessionHost {
    async fn spawn(&self, spec: SpawnSpec) -> Result<SessionId>;
    async fn attach(&self, id: SessionId, sink: OutputSink) -> Result<ReplaySnapshot>;
    async fn write(&self, id: SessionId, bytes: &[u8]) -> Result<()>;
    async fn resize(&self, id: SessionId, cols: u16, rows: u16) -> Result<()>;
    async fn kill(&self, id: SessionId) -> Result<()>;
}
```

The trait is the seam for the future out-of-process daemon (§9).

### 5.2 IPC

- Commands (`#[tauri::command]`): `list_projects`, `add_project`, `create_worktree`,
  `remove_worktree`, `spawn_session`, `write_session`, `resize_session`, `kill_session`, …
- Output streaming: `tauri::ipc::Channel<InvokeResponseBody>` with **raw bytes**
  (no JSON/base64 per chunk), one channel per attached terminal.
- Events (`app.emit`) for low-rate state: session state changes, worktree list changes,
  doctor results.

### 5.3 Frontend terminal

- One `Terminal` instance per *attached* session; views for sessions that aren't visible are
  disposed and re-attached later via the replay buffer, keeping memory flat with many agents.
- Addons: webgl (fallback to canvas), fit, unicode11, web-links (opens via `tauri-plugin-opener`),
  search.
- macOS keybindings: ⌘C/⌘V, ⌘K clear, ⌥-arrows word jump, ⌘-click links.
- Font: user setting, default `SF Mono` → `Menlo`.

---

## 6. Agent status and notifications

Knowing which agent is **waiting for you** is the main reason to use a manager over plain tabs.
Two sources, best one wins:

1. **Hooks (precise).** For Claude Code, worki writes hook config into the agent user's
   `~/.claude/settings.json` (merged, not overwritten) so `Notification`, `Stop`,
   `UserPromptSubmit`, `SessionStart` call:

   ```
   worki-hook --event stop
   ```

   `worki-hook` reads `WORKI_SESSION_ID`/`WORKI_SESSION_TOKEN` from the env and sends a JSON
   line to `WORKI_SOCKET`, a Unix socket in `/Users/Shared/worki/run/` (dir mode 0770, group
   `_worki_agents` + human ACL). The per-session token prevents one agent from spoofing another
   session's state when they share a user. Codex gets the equivalent via its `notify` config.

2. **Terminal heuristics (generic).** BEL and OSC 9 / OSC 777 desktop-notification sequences
   parsed from the output stream; "no output for N seconds" → `Idle`.

UI surfaces: colored dot per worktree in the sidebar (running / waiting / idle / exited),
dock badge with the count of waiting agents, macOS notification
(`tauri-plugin-notification`) when an agent starts waiting and its window isn't focused;
clicking it focuses that session.

---

## 7. UI

```
┌───────────────────────┬────────────────────────────────────────────────────────┐
│ ● ● ●                 │  feature/login  ·  ~/worki/worktrees/app/feature-login │
│                       │  [Claude ●] [Shell] [+]          ↗ Editor  ↗ Finder  ⋯ │
│ PROJECTS          [+] ├────────────────────────────────────────────────────────┤
│ ▾ app                 │                                                        │
│    main               │                                                        │
│  ● feature/login  +42 │            terminal (xterm.js)                         │
│  ◐ fix/crash      −3  │                                                        │
│    [+ New worktree]   │                                                        │
│ ▸ api                 │                                                        │
│ ▸ docs                │                                                        │
│                       │                                                        │
│ ⚙ Settings  🩺 Doctor │                                                        │
└───────────────────────┴────────────────────────────────────────────────────────┘
 ● waiting for input   ◐ running   ○ idle
```

- **Sidebar:** translucent (vibrancy), projects collapsible, worktrees show branch, status dot,
  diff stat, and the user the agent runs as (small badge when not the default). Context menu:
  open in editor/Finder/terminal, copy path, remove worktree.
- **New worktree dialog:** branch name (new or existing), base branch, run setup script, start
  agent profile immediately (default on).
- **Header:** branch, path, session tabs, quick actions.
- **Command palette (⌘K):** jump to worktree, new worktree, new session, run doctor.
- **Shortcuts:** ⌘1…9 switch worktree, ⌘T new shell tab, ⌘⇧N new worktree, ⌘W close tab.
- Light/dark follow the system.

---

## 8. Structured agent UI (later)

"UI instead of terminal" is feasible for agents that expose a machine protocol:

- **Claude Code:** `claude -p --input-format stream-json --output-format stream-json --verbose`
  over pipes, permission prompts via `--permission-prompt-tool` pointing at a small MCP server
  inside worki; or the Claude Agent SDK.
- **Codex:** `codex app-server` (JSON-RPC over stdio).

Design: `SessionKind::Structured(adapter)` alongside `Terminal`, sharing the same launcher (so
the same user isolation applies) but using pipes instead of a PTY. The UI renders messages,
tool calls with collapsible output, inline diffs, and approve/deny for permissions. The terminal
view stays the default and the fallback; structured mode is a per-profile opt-in.

---

## 9. Process lifetime

**MVP:** sessions live inside the app process; quitting worki kills its agents (with a
confirmation if any are running).

**Later:** move `SessionHost` into `workid`, a per-user LaunchAgent that owns the PTYs and talks
to the app over a Unix socket (same trait, new transport). Agents then survive app restarts and
updates, like tmux. The replay buffer already makes reattach work.

---

## 10. Persistence and paths

| What                    | Where                                                    |
|-------------------------|----------------------------------------------------------|
| App DB (SQLite)         | `~/Library/Application Support/worki/worki.db`           |
| Global settings         | same DB, `settings` table (simple key/value JSON)         |
| Secrets                 | human's login Keychain, service `ch.worki.agent-env`      |
| Logs                    | `~/Library/Logs/worki/` (`tracing` + rolling file)        |
| Hook socket             | `/Users/Shared/worki/run/worki.sock`                      |
| Worktrees (default)     | `~/worki/worktrees/<project>/<branch-slug>`              |

---

## 11. Milestones

| #  | Scope                                                                                 |
|----|---------------------------------------------------------------------------------------|
| M0 | Scaffold: workspace, Tauri 2 + React/Vite/Tailwind, window chrome, static layout      |
| M1 | Projects & worktrees: add repo, list/create/remove worktrees via git CLI, SQLite store, `.worki.toml` setup scripts (run as me) |
| M2 | Terminals as me: SessionHost, xterm.js, tabs, replay buffer, agent profiles           |
| M3 | Isolation: onboarding + `worki-admin.sh`, launcher via sudo, ACL grants, Keychain env injection, doctor, global/per-project `run_as` |
| M4 | Status: `worki-hook`, socket, Claude/Codex hook install, OSC/BEL heuristics, dots, dock badge, notifications |
| M5 | Polish: diff stat + changes panel, command palette, settings UI, open-in-editor, read-only `gh` PR status |
| M6 | Structured agent UI for Claude Code (then Codex)                                       |
| M7 | `workid` daemon, sessions survive restarts; signed build + `SMAppService` helper       |

M0–M3 is the first usable version.

---

## 12. Risks

- **Keychain-backed CLIs** under a non-GUI user (§4.5) — mitigated with token env vars; needs
  checking per agent CLI.
- **ACL drift:** tools that `chmod`/replace files (some package managers, `git gc`) may drop
  inherited ACEs on individual files. Doctor re-checks; a "repair permissions" action re-applies.
- **TCC:** repos inside `~/Documents`/`~/Desktop`/iCloud are unreadable by the agent user
  regardless of ACLs. The add-project flow warns and offers "run as me" for those.
- **`osascript … with administrator privileges`** is functional but legacy; fine for MVP, to be
  replaced by `SMAppService` once the app is signed.
- **WebView terminal performance** with many chatty agents — mitigated by batching, raw-byte
  channels and disposing hidden terminals; measure in M2.

---

## 13. Open questions

1. Name — is `worki` final? It affects bundle id, user names (`_worki`), paths.
2. Default worktree root: `~/worki/worktrees` OK, or inside the repo (`.worktrees/`)?
3. Which agents to support first-class besides Claude Code (Codex? Aider? Gemini CLI?)
4. Do agent sessions need to survive app restarts already in the first version (pull M7's
   daemon forward)?
5. Should the agent user get network-level restrictions later (e.g. `pf` per-UID rules or a
   proxy), or is filesystem isolation enough?
6. Distribution: personal use only (ad-hoc signing) or signed/notarized builds for others?
