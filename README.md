<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="worki icon">
</p>

<h1 align="center">worki</h1>

<p align="center">
  <b>Run coding agents in parallel, each in its own git worktree, each locked in its own sandbox.</b>
</p>

<p align="center">
  <a href="https://github.com/silvanfischer/worki/releases/latest"><img src="https://img.shields.io/github/v/release/silvanfischer/worki?label=download" alt="Latest release"></a>
  <a href="https://github.com/silvanfischer/worki/actions/workflows/ci.yml"><img src="https://github.com/silvanfischer/worki/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/platform-macOS-lightgrey" alt="macOS">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT license"></a>
</p>

<p align="center">
  <img src="docs/screenshots/main.png" alt="worki with Claude Code running in a Docker Sandbox, two agents waiting under Needs you" width="900">
</p>

Coding agents are at their best when you let them run on their own: accept edits, run the tests,
try again. That's exactly when you don't want them anywhere near your SSH keys, your browser
profile or your other repositories.

worki is a native macOS app that gives every task its own git worktree and its own isolated
environment, and shows all your agents side by side. Isolation isn't a plugin or a wrapper
script; it's how worki starts every agent.

## Isolation, built in

Pick where agents run: globally, per project or per worktree. worki sets the environment up,
checks it and cleans it up again.

| Isolation | What the agent gets | Needs |
|---|---|---|
| **User Isolation** | A hidden macOS user (`_worki`) that can only reach the worktrees it was given. Your home folder stays unreadable. | Nothing extra; a one-time admin setup |
| **Docker Sandboxes** | Its own Linux microVM per worktree | Docker Sandboxes (`sbx`) |
| **Docker** | Its own container per worktree, from a small Alpine image worki builds for you | Docker Desktop |
| **Podman** | The same, on Podman | A Podman machine |
| **None** | Runs as you, for trusted work | — |

<p align="center">
  <img src="docs/screenshots/isolation.png" alt="A worktree's isolation, with passing checks and Save snapshot" width="900">
</p>

- **Only the worktree, nothing else.** The worktree is mounted (or, for User Isolation, granted
  by ACLs) at the same path as on your Mac, so paths in agent output match yours. Of the
  repository's `.git`, only what committing needs is writable; hooks and config stay read-only,
  so an agent can't plant anything that later runs as you.
- **A doctor for every backend.** Checks phrased the way you'd want to read them ("_worki can't
  read files in your home folder") with one-click fixes: create the agent user, start the
  daemon, build the image.
- **Agents ready inside the sandbox.** Install Claude Code or Codex into the environment, or
  reuse your own installation. Copy your agent configuration (settings, `CLAUDE.md`, agents,
  commands, skills) and sign in with a token that uses your subscription, while your own login
  stays untouched. Setup is remembered and repeated for every new environment.
- **Secrets stay in your Keychain.** Tokens are stored per isolation and passed to sessions as
  environment variables.
- **Snapshots.** Install what a project needs (a JDK, system packages) once, save the worktree's
  sandbox or container as a snapshot, and new worktrees start from it with everything installed.
- **Agents stay put.** Environments keep running when worki quits (or stop, if you prefer), and
  Claude Code sessions pick up their conversation when worki starts again.
- **No leftovers.** Environments are removed together with their worktree or project.

<p align="center">
  <img src="docs/screenshots/settings.png" alt="Isolation settings with checks and agents installed in the sandbox" width="900">
</p>

## From task to pull request

<p align="center">
  <img src="docs/screenshots/branch.png" alt="The Branch tab: status against origin/main, commits and the open pull request" width="900">
</p>

- **Commit** the files you pick from the **Changes** tab, or ask the agent to do it.
- **Branch** tab: how the branch stands against `origin/main`, with **Rebase** (it fetches
  first; on conflicts nothing changes and the agent can resolve them), the branch's commits and
  every file it touched.
- **Pull requests** through the GitHub CLI: create one, follow its checks, push new commits, and
  pull review comments into worki to send them to the agent.
- **Review in place.** Comment on any diff line, expand hidden context, and send the comments to
  the agent as one review.

Commits and pull requests can be turned off under Settings › Git. Pull requests need
[`gh`](https://cli.github.com) installed and signed in (`gh auth login`).

## Everything around it

- **Parallel worktrees.** Branch off any existing branch in one step, with an optional setup
  script (`pnpm install`, copying `.env` files) that runs in the same isolation as the agent.
- **See who needs you.** Agents waiting for an answer are listed under **Needs you**, across all
  projects, the moment they ask, with a notification and a Dock badge.
- **Real terminals.** Agent and shell sessions in GPU-rendered terminals that keep running when
  you switch worktrees. Each tab shows its token usage, and YOLO mode is one checkbox per agent.
- **Remote projects.** Work on repositories on an SSH host, with saved hosts you set up once;
  sessions run in tmux there and survive a dropped connection.
- **Keyboard first.** ⌘K jumps to any worktree or action, ⌘T starts an agent, ⌘J shows the
  changes.
- **Open in anything.** Jump to the worktree in your editor or terminal from the toolbar.

## Getting started

1. Download `worki_<version>_universal.dmg` from the
   [latest release](https://github.com/silvanfischer/worki/releases/latest). It runs on
   Apple Silicon and Intel Macs with macOS 13 or later, and is signed and notarized by Apple.
2. Open the DMG and drag **worki** into **Applications**.
3. For container isolation, install Docker Sandboxes, Docker Desktop or Podman. User Isolation
   needs nothing extra.

Add a repository with **+**, create a worktree, pick its isolation, and start an agent. The
doctor walks you through anything that's missing.

### Build from source

You need Rust (stable), Node 20+, pnpm and the Xcode command line tools.

```bash
git clone https://github.com/silvanfischer/worki.git
cd worki
pnpm install
pnpm tauri dev
```

### Share setup with your team

Commit a `.worki.toml` to give every worktree the same base branch and setup script:

```toml
[worktree]
base = "main"
setup = """
cp "$WORKI_ROOT_PATH/.env.local" .env.local
pnpm install
"""
```

A setup script set in the project settings overrides the file.

## Development

```bash
cargo test --workspace
pnpm typecheck
```

The README screenshots are the real UI rendered against demo data (`ui/demo.html`):

```bash
scripts/screenshots.sh
```

End-to-end tests against real container engines (need `sbx`, Docker Desktop or a Podman machine):

```bash
cargo test -p worki-isolation --test containers -- --ignored
```

| Path | What |
|---|---|
| `crates/worki-isolation` | Isolation backends: macOS user, Docker Sandboxes, Docker, Podman, SSH |
| `crates/worki-core` | Projects, worktrees and git (local and over SSH), SQLite store, agent settings |
| `crates/worki-pty` | PTY sessions with replay buffer and agent activity tracking |
| `src-tauri` | Tauri app and IPC commands |
| `ui` | React + Tailwind frontend |

Design notes are in [docs/design.md](docs/design.md).

## License

[MIT](LICENSE)
