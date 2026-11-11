//! Isolation backends decide *where* an agent process runs: directly as the human user, as a
//! dedicated macOS user, and later in a sandbox profile or micro-VM. Every backend turns the same
//! [`LaunchRequest`] into a concrete [`Launch`], so callers never special-case a mechanism.

mod acl;
mod container;
mod mac_user;
mod none;
pub mod ssh;
pub mod vault;

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

pub use container::{
    sandbox_name, Container, Engine, DEFAULT_CONTAINER_IMAGE, DEFAULT_SANDBOX_IMAGE,
};
pub use mac_user::{MacUser, AGENT_GROUP, DEFAULT_AGENT_USER};
pub use none::{host_which, NoIsolation};
pub use ssh::Ssh;

/// A concrete isolation mechanism and its parameters. New mechanisms are new variants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Isolation {
    #[default]
    #[serde(alias = "me")]
    None,
    #[serde(alias = "user")]
    MacUser { user: String },
    /// A Docker Sandbox (`sbx`) per worktree for a built-in agent (`image`), optionally created
    /// from a saved template instead of the agent's default image.
    DockerSandbox {
        image: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<String>,
    },
    /// A Docker container per worktree.
    Docker { image: String },
    /// A Podman container per worktree (in the Podman machine VM).
    Podman { image: String },
    /// Another machine reached over SSH; only for projects that live there.
    #[serde(rename_all = "camelCase")]
    Ssh {
        host: String,
        #[serde(default)]
        user: Option<String>,
        #[serde(default)]
        port: Option<u16>,
        #[serde(default)]
        keep_alive: bool,
    },
}

/// Per-project choice: follow the global default or pick a mechanism.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProjectIsolation {
    #[default]
    Inherit,
    #[serde(alias = "me")]
    None,
    #[serde(alias = "user")]
    MacUser {
        user: String,
    },
    DockerSandbox {
        image: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<String>,
    },
    Docker {
        image: String,
    },
    Podman {
        image: String,
    },
}

impl ProjectIsolation {
    pub fn resolve(&self, global: &Isolation) -> Isolation {
        match self {
            Self::Inherit => global.clone(),
            Self::None => Isolation::None,
            Self::MacUser { user } => Isolation::MacUser { user: user.clone() },
            Self::DockerSandbox { image, template } => Isolation::DockerSandbox {
                image: image.clone(),
                template: template.clone(),
            },
            Self::Docker { image } => Isolation::Docker {
                image: image.clone(),
            },
            Self::Podman { image } => Isolation::Podman {
                image: image.clone(),
            },
        }
    }
}

impl Isolation {
    pub fn backend(&self) -> Box<dyn Backend> {
        match self {
            Self::None => Box::new(NoIsolation),
            Self::MacUser { user } => Box::new(MacUser::new(user.clone())),
            Self::DockerSandbox { image, template } => {
                Box::new(Container::new(Engine::Sbx, image.clone()).with_template(template.clone()))
            }
            Self::Docker { image } => Box::new(Container::new(Engine::Docker, image.clone())),
            Self::Podman { image } => Box::new(Container::new(Engine::Podman, image.clone())),
            Self::Ssh { keep_alive, .. } => {
                Box::new(Ssh::new(self.ssh_target().unwrap_or_default(), *keep_alive))
            }
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            Self::None => Ok(()),
            Self::MacUser { user } => {
                let valid = !user.is_empty()
                    && user.len() <= 31
                    && !user.starts_with('-')
                    && user
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b));
                anyhow::ensure!(valid, "invalid user name: {user}");
                Ok(())
            }
            Self::DockerSandbox { image, template } => {
                let valid = !image.is_empty()
                    && image.len() <= 40
                    && image
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
                anyhow::ensure!(valid, "invalid sandbox image: {image}");
                if let Some(template) = template {
                    validate_image_ref(template)?;
                }
                Ok(())
            }
            Self::Docker { image } | Self::Podman { image } => validate_image_ref(image),
            Self::Ssh { .. } => self.ssh_target().unwrap_or_default().validate(),
        }
    }

    pub fn ssh_target(&self) -> Option<ssh::SshTarget> {
        match self {
            Self::Ssh {
                host, user, port, ..
            } => Some(ssh::SshTarget::new(host.clone(), user.clone(), *port)),
            _ => None,
        }
    }

    /// Stable identifier for things kept per isolation, such as secrets.
    pub fn scope_key(&self) -> String {
        match self {
            Self::None => "none".into(),
            Self::MacUser { user } => format!("macUser:{user}"),
            Self::DockerSandbox { .. } => "dockerSandbox".into(),
            Self::Docker { .. } => "docker".into(),
            Self::Podman { .. } => "podman".into(),
            Self::Ssh { .. } => format!("ssh:{}", self.ssh_target().unwrap_or_default().display()),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::None => "Me".into(),
            Self::MacUser { user } => user.clone(),
            Self::DockerSandbox { .. } => "sandbox".into(),
            Self::Docker { .. } => "docker".into(),
            Self::Podman { .. } => "podman".into(),
            Self::Ssh { .. } => self.ssh_target().unwrap_or_default().display(),
        }
    }
}

pub fn validate_image_ref(image: &str) -> anyhow::Result<()> {
    let valid = !image.is_empty()
        && image.len() <= 200
        && !image.starts_with('-')
        && image
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._/:@-".contains(&b));
    anyhow::ensure!(valid, "invalid image reference: {image}");
    Ok(())
}

/// What to run, independent of the isolation mechanism.
#[derive(Debug, Clone)]
pub struct LaunchRequest {
    pub cwd: PathBuf,
    /// Shell code to run; `None` starts an interactive shell.
    pub command: Option<String>,
    /// Interactive sessions get `-i` so rc files (PATH tweaks, nvm, …) load like in a terminal.
    pub interactive: bool,
    /// Keeps stdin open without a terminal, for agents worki talks to over a pipe.
    pub stdin: bool,
    pub env: Vec<(String, String)>,
    /// Set for terminal sessions; backends that keep sessions alive on their own name them by it.
    pub session: Option<SessionMeta>,
}

#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub profile_id: Option<String>,
}

/// A process ready to spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Start from an empty environment instead of inheriting the app's.
    pub clear_env: bool,
    pub cwd: PathBuf,
}

/// Isolated agents run in linked worktrees only: in the main checkout, `.git` itself holds the
/// index and HEAD, so all of it would have to be writable, config and hooks included.
pub(crate) fn require_linked_worktree(repo: &Path, worktree: &Path) -> anyhow::Result<()> {
    let same = match (repo.canonicalize(), worktree.canonicalize()) {
        (Ok(repo), Ok(worktree)) => repo == worktree,
        _ => repo == worktree,
    };
    if same {
        bail!(
            "Isolated agents only work in worktrees, not in the main checkout. Create a worktree for this task."
        );
    }
    Ok(())
}

/// What an isolated agent may reach for one project: the repository's git data and, when given,
/// one assigned worktree. `worktree_root` is only used to clean up grants from older versions.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub repo: PathBuf,
    pub worktree_root: PathBuf,
    pub worktree: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// A binary installed on the host can be used inside (same OS).
    pub share_host_install: bool,
    /// An agent's install command can run inside.
    pub install: bool,
    /// The human's agent config can be copied in.
    pub copy_config: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Fix {
    /// One-time privileged setup (e.g. create the user, install an image).
    Provision,
    /// Unprivileged per-environment setup (config files inside the environment).
    Prepare,
    /// Give the environment access to a project's files.
    GrantAccess,
    /// Build the environment's image; long-running, with streamed output (`Backend::build_image`).
    BuildImage,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: String,
    pub label: String,
    pub ok: bool,
    pub detail: Option<String>,
    pub fix: Option<Fix>,
}

impl Check {
    /// `passed` and `failed` are full sentences for each outcome, so a red check never reads
    /// like a success.
    fn new(id: &str, ok: bool, passed: impl Into<String>, failed: impl Into<String>) -> Self {
        let label = if ok { passed.into() } else { failed.into() };
        Self {
            id: id.into(),
            label,
            ok,
            detail: None,
            fix: None,
        }
    }

    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    fn fix_if_failed(mut self, fix: Fix) -> Self {
        if !self.ok {
            self.fix = Some(fix);
        }
        self
    }
}

pub trait Backend: Send + Sync {
    fn isolation(&self) -> Isolation;

    /// Turns a request into the process that runs it inside this isolation boundary.
    fn launch(&self, req: &LaunchRequest) -> anyhow::Result<Launch>;

    /// Makes sure the environment can work in `ws`. Cheap when already done; may block.
    fn grant_access(&self, ws: &Workspace) -> anyhow::Result<()>;

    /// Saves the worktree's environment, with everything installed in it, as an image or template
    /// named `tag`, and returns the isolation that creates new environments from it.
    fn save_template(&self, _worktree: &Path, _tag: &str) -> anyhow::Result<Isolation> {
        anyhow::bail!("only container isolations can be saved as a snapshot")
    }

    /// Images or sbx templates on this machine that new environments could start from.
    fn templates(&self) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// Like [`Backend::templates`], with when each was created (Unix seconds) if the engine says.
    fn dated_templates(&self) -> anyhow::Result<Vec<(String, Option<i64>)>> {
        Ok(self.templates()?.into_iter().map(|t| (t, None)).collect())
    }

    /// Removes an image or sbx template named `tag`. The engine refuses while an environment
    /// still runs from it.
    fn delete_template(&self, _tag: &str) -> anyhow::Result<()> {
        anyhow::bail!("only container isolations have snapshots")
    }

    /// What the Agents panel may offer for this isolation.
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            share_host_install: false,
            install: true,
            copy_config: true,
        }
    }

    /// The concrete environments file and install operations apply to. One for a user account;
    /// one per existing sandbox for per-worktree backends.
    fn instances(&self) -> anyhow::Result<Vec<Box<dyn Backend>>> {
        Ok(vec![self.isolation().backend()])
    }

    /// The environment serving `ws`'s worktree, when the backend keeps one per worktree.
    fn instance_for(&self, _ws: &Workspace) -> Option<Box<dyn Backend>> {
        None
    }

    /// Whether `ws` already has its environment (so `grant_access` will not create a new one).
    fn instance_exists(&self, _ws: &Workspace) -> bool {
        true
    }

    /// The project in `ws` is being removed from worki; drop everything held for it (all its
    /// worktrees' environments, access grants).
    fn forget_project(&self, _ws: &Workspace) -> anyhow::Result<()> {
        Ok(())
    }

    /// The worktree in `ws` is going away; drop anything held for it (e.g. its sandbox).
    fn release(&self, _ws: &Workspace) -> anyhow::Result<()> {
        Ok(())
    }

    /// worki is quitting; pause what can be resumed later.
    fn shutdown(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Whether the environment can run anything right now; the error says what to fix.
    fn ready(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Whether the environment itself still exists (e.g. the user account).
    fn exists(&self) -> bool {
        true
    }

    /// Builds the environment's image, passing each output line to `log` as it arrives.
    fn build_image(&self, _log: &mut dyn FnMut(String)) -> anyhow::Result<()> {
        bail!("this isolation has no image to build")
    }

    /// Unprivileged per-environment setup.
    fn prepare(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// One-time privileged setup; may show a system authentication prompt.
    fn provision(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Undoes `provision` and every grant made for `workspaces`; may prompt like `provision`.
    fn remove(&self, _workspaces: &[Workspace]) -> anyhow::Result<()> {
        Ok(())
    }

    /// Health checks of the environment itself, optionally against one project's files.
    fn doctor(&self, ws: Option<&Workspace>) -> Vec<Check>;

    /// Full path of `command` on the environment's login-shell PATH.
    fn which(&self, command: &str) -> Option<String>;

    /// Home directory inside the environment.
    fn home(&self) -> Option<PathBuf>;

    /// Makes an executable installed on the host usable inside the environment as `name`.
    fn share_host_executable(&self, _name: &str, _host_path: &Path) -> anyhow::Result<()> {
        bail!("sharing host executables is not supported by this isolation")
    }

    /// Copies files and directories, given relative to `from_home`, into the environment's home.
    fn copy_home_files(&self, _from_home: &Path, _paths: &[PathBuf]) -> anyhow::Result<()> {
        bail!("copying files is not supported by this isolation")
    }

    fn read_home_file(&self, _path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
        bail!("reading files is not supported by this isolation")
    }

    fn write_home_file(&self, _path: &Path, _data: &[u8]) -> anyhow::Result<()> {
        bail!("writing files is not supported by this isolation")
    }
}

/// Paths handed to backends must stay inside the home they are resolved against.
pub(crate) fn ensure_relative(path: &Path) -> anyhow::Result<()> {
    if path.components().all(|c| matches!(c, Component::Normal(_))) && !path.as_os_str().is_empty()
    {
        Ok(())
    } else {
        bail!("not a plain relative path: {}", path.display())
    }
}

pub(crate) fn valid_env_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub(crate) fn ensure_valid_env(env: &[(String, String)]) -> anyhow::Result<()> {
    if let Some((bad, _)) = env.iter().find(|(k, _)| !valid_env_name(k)) {
        bail!("invalid environment variable name: {bad}");
    }
    Ok(())
}

pub(crate) fn stderr_text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).trim().to_owned()
}

/// Passes `out` through when the command succeeded, otherwise fails with "`what` failed: stderr".
pub(crate) fn check_output(out: Output, what: &str) -> anyhow::Result<Output> {
    if !out.status.success() {
        bail!("{what} failed: {}", stderr_text(&out));
    }
    Ok(out)
}

/// Script for `read_home_file`: prints the file `$1`, or exits with [`MISSING_FILE`].
pub(crate) const READ_FILE_SCRIPT: &str = r#"if [ -f "$1" ]; then cat "$1"; else exit 3; fi"#;
pub(crate) const MISSING_FILE: i32 = 3;

/// Script for `write_home_file`: writes stdin to `$1`, private to the owner.
pub(crate) const WRITE_FILE_SCRIPT: &str =
    r#"umask 077 && mkdir -p "$(dirname "$1")" && cat > "$1""#;

pub(crate) fn file_contents(out: Output, what: &str) -> anyhow::Result<Option<Vec<u8>>> {
    if out.status.code() == Some(MISSING_FILE) {
        return Ok(None);
    }
    check_output(out, what).map(|o| Some(o.stdout))
}

/// The absolute path a `command -v` run printed last, if it succeeded.
pub(crate) fn command_path(out: &Output) -> Option<String> {
    let path = String::from_utf8_lossy(&out.stdout)
        .lines()
        .last()?
        .trim()
        .to_owned();
    (out.status.success() && path.starts_with('/')).then_some(path)
}

/// The absolute path a successful `$HOME` query printed.
pub(crate) fn home_path(out: &Output) -> Option<PathBuf> {
    let home = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && home.starts_with('/')).then(|| home.into())
}

/// Runs `cmd` with stdout and stderr captured, writing `stdin` to it when given.
pub(crate) fn run_with_stdin(cmd: &mut Command, stdin: Option<&[u8]>) -> anyhow::Result<Output> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let Some(input) = stdin else {
        return Ok(cmd.stdin(Stdio::null()).output()?);
    };
    let mut child = cmd.stdin(Stdio::piped()).spawn()?;
    child.stdin.take().context("stdin")?.write_all(input)?;
    Ok(child.wait_with_output()?)
}

/// `tar` writing an archive of `paths` (relative to `from_home`) to stdout, following symlinks
/// and leaving out macOS-only metadata the other side can't restore.
pub(crate) fn pack_home_files(from_home: &Path, paths: &[PathBuf]) -> Command {
    let mut tar = Command::new("/usr/bin/tar");
    tar.args([
        "--no-acls",
        "--no-xattrs",
        "--no-mac-metadata",
        "-L",
        "-cf",
        "-",
        "-C",
    ])
    .arg(from_home)
    .args(paths);
    tar
}

pub fn current_user() -> String {
    std::env::var("USER")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| {
            Command::new("/usr/bin/id")
                .arg("-un")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn isolated_agents_need_a_linked_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let wt = dir.path().join("wt");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        let err = require_linked_worktree(&repo, &repo.join(".")).unwrap_err();
        assert!(err.to_string().contains("only work in worktrees"));
        require_linked_worktree(&repo, &wt).unwrap();
    }

    use super::*;

    #[test]
    fn sandbox_template_is_optional_and_validated() {
        let old: Isolation =
            serde_json::from_str(r#"{"kind":"dockerSandbox","image":"claude"}"#).unwrap();
        assert_eq!(
            old,
            Isolation::DockerSandbox {
                image: "claude".into(),
                template: None
            }
        );
        let saved = Isolation::DockerSandbox {
            image: "claude".into(),
            template: Some("worki-app:20261109-1200".into()),
        };
        assert!(saved.validate().is_ok());
        assert!(Isolation::DockerSandbox {
            image: "claude".into(),
            template: Some("--privileged".into()),
        }
        .validate()
        .is_err());
    }

    #[test]
    fn reads_legacy_run_as_values() {
        let p: ProjectIsolation =
            serde_json::from_str(r#"{"kind":"user","user":"_worki_x"}"#).unwrap();
        assert_eq!(
            p,
            ProjectIsolation::MacUser {
                user: "_worki_x".into()
            }
        );
        let p: ProjectIsolation = serde_json::from_str(r#"{"kind":"me"}"#).unwrap();
        assert_eq!(p, ProjectIsolation::None);
        let p: ProjectIsolation = serde_json::from_str(r#"{"kind":"inherit"}"#).unwrap();
        assert_eq!(
            p.resolve(&Isolation::MacUser {
                user: "_worki".into()
            })
            .label(),
            "_worki"
        );
    }

    #[test]
    fn validates_user_names() {
        let user = |u: &str| Isolation::MacUser { user: u.into() };
        assert!(user("_worki").validate().is_ok());
        assert!(user("_worki_my.app-2").validate().is_ok());
        for bad in ["", "-n", "a b", "x;rm", "a,b"] {
            assert!(user(bad).validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn rejects_escaping_paths() {
        assert!(ensure_relative(Path::new(".claude/settings.json")).is_ok());
        for bad in ["", "/etc/passwd", "../x", ".claude/../../x", "./x"] {
            assert!(ensure_relative(Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn validates_container_images() {
        let docker = |i: &str| Isolation::Docker { image: i.into() };
        for ok in ["worki-agent:1", "ghcr.io/org/img:tag", "node@sha256:abc"] {
            assert!(docker(ok).validate().is_ok(), "{ok}");
        }
        for bad in ["", "-it", "img; rm", "img name"] {
            assert!(docker(bad).validate().is_err(), "{bad}");
        }
        let p: ProjectIsolation =
            serde_json::from_str(r#"{"kind":"podman","image":"worki-agent:1"}"#).unwrap();
        assert_eq!(p.resolve(&Isolation::None).scope_key(), "podman");
    }

    #[test]
    fn serializes_tagged() {
        let json = serde_json::to_string(&Isolation::MacUser {
            user: "_worki".into(),
        })
        .unwrap();
        assert_eq!(json, r#"{"kind":"macUser","user":"_worki"}"#);
    }
}
