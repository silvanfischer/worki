//! Container isolation: one container (or Docker Sandboxes microVM) per worktree, run by `sbx`,
//! `docker` or `podman`. Folders are bind-mounted at the same absolute path, so git worktree
//! pointers resolve unchanged inside; git metadata that could make the human's own git run
//! agent-written code is mounted read-only.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    check_output, command_path, ensure_relative, ensure_valid_env, file_contents, home_path,
    host_which, pack_home_files, Backend, Check, Fix, Isolation, Launch, LaunchRequest, Workspace,
    READ_FILE_SCRIPT, WRITE_FILE_SCRIPT,
};

pub const DEFAULT_SANDBOX_IMAGE: &str = "claude";
/// Built locally by "Set up" for Docker and Podman.
pub const DEFAULT_CONTAINER_IMAGE: &str = "worki-agent:1";
const AGENT_CONTAINERFILE: &str = include_str!("worki-agent.Containerfile");

/// Environments worki creates; anything else belongs to the user and is never touched.
const NAME_PREFIX: &str = "worki-";

/// Writes a command's stdin, e.g. a tar stream.
type Feed = Box<dyn FnOnce(&mut std::process::ChildStdin) -> std::io::Result<()>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Engine {
    /// Docker Sandboxes: a microVM per sandbox, agent images built in.
    Sbx,
    Docker,
    /// Containers in a Podman machine VM.
    Podman,
}

impl Engine {
    fn program(self) -> &'static str {
        match self {
            Engine::Sbx => "sbx",
            Engine::Docker => "docker",
            Engine::Podman => "podman",
        }
    }

    pub fn display(self) -> &'static str {
        match self {
            Engine::Sbx => "Docker Sandboxes (sbx)",
            Engine::Docker => "Docker",
            Engine::Podman => "Podman",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Engine::Sbx => "sandbox",
            _ => "container",
        }
    }

    fn binary(self) -> Result<PathBuf> {
        let name = self.program();
        [
            format!("/opt/homebrew/bin/{name}"),
            format!("/usr/local/bin/{name}"),
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .or_else(|| host_which(name).map(PathBuf::from))
        .with_context(|| format!("{} is not installed", self.display()))
    }

    /// Fallback home when the environment can't be asked.
    fn default_home(self) -> &'static str {
        match self {
            Engine::Sbx => "/home/agent",
            _ => "/root",
        }
    }
}

#[derive(Clone)]
pub struct Container {
    engine: Engine,
    image: String,
    /// sbx only: saved template new sandboxes start from.
    template: Option<String>,
    /// Set for an instance bound to one environment (see [`Backend::instances`]).
    bound: Option<Bound>,
}

#[derive(Debug, Clone)]
struct Bound {
    name: String,
    /// The worktree it serves, used as working directory for commands.
    workdir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Listed {
    name: String,
    running: bool,
    workdir: Option<PathBuf>,
}

impl Container {
    pub fn new(engine: Engine, image: String) -> Self {
        Self {
            engine,
            image,
            template: None,
            bound: None,
        }
    }

    pub fn with_template(mut self, template: Option<String>) -> Self {
        self.template = template;
        self
    }

    /// Removes the worktree's environments from every installed, running engine, whichever
    /// isolation created them. Returns one message per engine that failed.
    pub fn release_everywhere(ws: &Workspace) -> Vec<String> {
        [Engine::Sbx, Engine::Docker, Engine::Podman]
            .into_iter()
            .map(|engine| Self::new(engine, String::new()))
            .filter(|c| c.engine.binary().is_ok() && c.engine_running())
            .filter_map(|c| {
                c.release(ws)
                    .err()
                    .map(|e| format!("{}: {e:#}", c.engine.display()))
            })
            .collect()
    }

    fn bound_to(&self, listed: &Listed) -> Self {
        let workdir = listed
            .workdir
            .clone()
            .unwrap_or_else(|| self.engine.default_home().into());
        Self {
            engine: self.engine,
            image: self.image.clone(),
            template: self.template.clone(),
            bound: Some(Bound {
                name: listed.name.clone(),
                workdir,
            }),
        }
    }

    fn bound(&self) -> Result<&Bound> {
        self.bound
            .as_ref()
            .with_context(|| format!("this needs a specific {}", self.engine.noun()))
    }

    fn command(&self) -> Result<Command> {
        let mut cmd = Command::new(self.engine.binary()?);
        cmd.stdin(Stdio::null());
        Ok(cmd)
    }

    fn checked(
        &self,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
        what: &str,
    ) -> Result<String> {
        let out = check_output(self.command()?.args(args).output()?, what)?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Force-removes the named environments.
    fn remove_all(&self, names: &[String], what: &str) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        let force = if self.engine == Engine::Sbx {
            "--force"
        } else {
            "-f"
        };
        let args = ["rm", force]
            .into_iter()
            .chain(names.iter().map(String::as_str));
        self.checked(args, what).map(drop)
    }

    fn list(&self) -> Result<Vec<Listed>> {
        let all = match self.engine {
            Engine::Sbx => {
                parse_sbx_listing(&self.checked(["ls", "--json"], "listing sandboxes")?)?
            }
            _ => parse_ps_listing(&self.checked(
                [
                    "ps",
                    "-a",
                    "--filter",
                    &format!("name={NAME_PREFIX}"),
                    "--format",
                    "json",
                ],
                "listing containers",
            )?)?,
        };
        Ok(all
            .into_iter()
            .filter(|l| l.name.starts_with(NAME_PREFIX))
            .collect())
    }

    fn create(&self, name: &str, worktree: &Path, mounts: &[Mount]) -> Result<()> {
        let mut args: Vec<String> = Vec::new();
        match self.engine {
            Engine::Sbx => {
                args.extend(["create", "--name", name, "-q"].map(String::from));
                match &self.template {
                    Some(template) => args.extend([
                        "-t".into(),
                        template.clone(),
                        "--pull".into(),
                        "never".into(),
                    ]),
                    None => args.extend(["--pull", "missing"].map(String::from)),
                }
                args.push(self.image.clone());
                args.extend(mounts.iter().map(Mount::sbx_arg));
            }
            _ => {
                args.extend(["run", "-d", "--name", name].map(String::from));
                args.extend([
                    "--label".into(),
                    format!("worki.workdir={}", worktree.display()),
                ]);
                for m in mounts {
                    args.extend(["-v".into(), m.volume_arg()]);
                }
                args.extend([
                    "-w".into(),
                    worktree.display().to_string(),
                    self.image.clone(),
                    "sleep".into(),
                    "infinity".into(),
                ]);
            }
        }
        self.checked(&args, &format!("creating the {}", self.engine.noun()))
            .map(drop)
    }

    /// Docker and Podman can't `exec` into a stopped container (sbx starts it itself).
    fn ensure_running(&self, listed: &Listed) -> Result<()> {
        if self.engine != Engine::Sbx && !listed.running {
            self.checked(["start", &listed.name], "starting the container")?;
        }
        Ok(())
    }

    /// Runs `sh -c script` in the bound environment's home with `args` as `$1…`, optionally
    /// feeding stdin.
    fn sh(&self, script: &str, args: &[&str], stdin: Option<Feed>) -> Result<Output> {
        let bound = self.bound()?;
        let mut cmd = self.command()?;
        cmd.arg("exec");
        if stdin.is_some() {
            cmd.arg("-i");
        }
        cmd.args([
            &bound.name,
            "sh",
            "-c",
            &format!("cd \"$HOME\" && {script}"),
            "_",
        ])
        .args(args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let Some(feed) = stdin else {
            return Ok(cmd.output()?);
        };
        let mut child = cmd.stdin(Stdio::piped()).spawn()?;
        let mut input = child.stdin.take().context("stdin")?;
        feed(&mut input)?;
        drop(input);
        Ok(child.wait_with_output()?)
    }

    fn image_present(&self) -> bool {
        self.engine == Engine::Sbx
            || self.command().is_ok_and(|mut c| {
                c.args(["image", "inspect", &self.image])
                    .output()
                    .is_ok_and(|o| o.status.success())
            })
    }

    fn engine_running(&self) -> bool {
        let Ok(mut cmd) = self.command() else {
            return false;
        };
        match self.engine {
            Engine::Sbx => cmd
                .args(["daemon", "status"])
                .output()
                .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("running")),
            _ => cmd
                .arg("info")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success()),
        }
    }
}

/// Runs `cmd`, passing stdout and stderr lines to `log` as they arrive; fails with the last lines
/// when the command does.
fn stream_lines(cmd: &mut Command, log: &mut dyn FnMut(String)) -> Result<()> {
    use std::io::BufRead;
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|r| {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(r).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        })
    })
    .collect();
    drop(tx);
    let mut tail = std::collections::VecDeque::with_capacity(8);
    for line in rx {
        if tail.len() == 8 {
            tail.pop_front();
        }
        tail.push_back(line.clone());
        log(line);
    }
    for r in readers {
        r.join().ok();
    }
    let status = child.wait()?;
    if !status.success() {
        bail!("{}", tail.into_iter().collect::<Vec<_>>().join("\n"));
    }
    Ok(())
}

/// `sbx ls --json`: `{"sandboxes":[{name, status, workspaces:[…]}]}`.
fn parse_sbx_listing(json: &str) -> Result<Vec<Listed>> {
    let value: Value = serde_json::from_str(json).context("unexpected `sbx ls --json` output")?;
    Ok(value["sandboxes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            Some(Listed {
                name: s["name"].as_str()?.to_owned(),
                running: s["status"].as_str() == Some("running"),
                workdir: s["workspaces"][0]
                    .as_str()
                    .map(|w| PathBuf::from(w.split(':').next().unwrap_or(w))),
            })
        })
        .collect())
}

/// `docker ps --format json` prints one object per line with `Names` as a string and `Labels` as
/// `k=v,k=v`; `podman ps --format json` prints an array with `Names` as a list and `Labels` as an
/// object. Both are accepted.
fn parse_ps_listing(text: &str) -> Result<Vec<Listed>> {
    let trimmed = text.trim();
    let items: Vec<Value> = if trimmed.is_empty() {
        Vec::new()
    } else if trimmed.starts_with('[') {
        serde_json::from_str(trimmed).context("unexpected `ps --format json` output")?
    } else {
        trimmed
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?
    };
    Ok(items
        .iter()
        .filter_map(|c| {
            let name = match &c["Names"] {
                Value::String(s) => s.split(',').next()?.to_owned(),
                Value::Array(a) => a.first()?.as_str()?.to_owned(),
                _ => return None,
            };
            let state = c["State"].as_str().unwrap_or_default().to_ascii_lowercase();
            let workdir = match &c["Labels"] {
                Value::Object(m) => m
                    .get("worki.workdir")
                    .and_then(Value::as_str)
                    .map(PathBuf::from),
                Value::String(s) => s
                    .split(',')
                    .find_map(|kv| kv.strip_prefix("worki.workdir="))
                    .map(PathBuf::from),
                _ => None,
            };
            Some(Listed {
                name,
                running: state == "running",
                workdir,
            })
        })
        .collect())
}

/// Stable environment name for a worktree: readable part from the folder name, uniqueness from
/// a hash of the full path. Valid for sbx, Docker and Podman (letters, digits, `-`).
pub fn sandbox_name(worktree: &Path) -> String {
    let base: String = worktree
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let base = base.trim_matches('-');
    let base: String = base.chars().take(30).collect();
    format!(
        "{NAME_PREFIX}{}-{:06x}",
        if base.is_empty() { "worktree" } else { &base },
        fnv1a(worktree) & 0xff_ffff
    )
}

fn fnv1a(path: &Path) -> u64 {
    path.to_string_lossy()
        .bytes()
        .fold(0xcbf29ce484222325, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Mount {
    path: PathBuf,
    read_only: bool,
}

impl Mount {
    fn sbx_arg(&self) -> String {
        format!(
            "{}{}",
            self.path.display(),
            if self.read_only { ":ro" } else { "" }
        )
    }

    fn volume_arg(&self) -> String {
        let p = self.path.display();
        format!("{p}:{p}{}", if self.read_only { ":ro" } else { "" })
    }
}

/// The worktree writable, and of the repository's `.git` only what committing and fetching
/// write: objects, refs, reflogs and the worktree's own admin folder (HEAD, index, FETCH_HEAD).
/// Everything else in `.git` is read-only, above all config files (including
/// submodules' and other worktrees') that your own git would run commands from.
fn mounts(repo: &Path, worktree: &Path, admin_dir: Option<&Path>) -> Vec<Mount> {
    let git = repo.join(".git");
    let mount = |path: PathBuf, read_only| Mount { path, read_only };
    if worktree == repo {
        return [
            mount(worktree.to_path_buf(), false),
            mount(git.join("hooks"), true),
            mount(git.join("config"), true),
        ]
        .into_iter()
        .filter(|m| m.path.exists())
        .collect();
    }
    let mut writable = vec![git.join("objects"), git.join("refs"), git.join("logs")];
    writable.extend(admin_dir.map(Path::to_path_buf));
    for dir in &writable {
        std::fs::create_dir_all(dir).ok();
    }
    let mut pinned = vec![worktree.join(".git")];
    if let Some(admin) = admin_dir {
        pinned.extend([admin.join("gitdir"), admin.join("commondir")]);
    }
    [mount(worktree.to_path_buf(), false), mount(git, true)]
        .into_iter()
        .chain(writable.into_iter().map(|p| mount(p, false)))
        .chain(pinned.into_iter().map(|p| mount(p, true)))
        .filter(|m| m.path.exists())
        .collect()
}

/// `docker.io/library/app:1` and `localhost/app:1` are the same image as `app:1`.
fn short_image_name(tag: &str) -> String {
    let tag = tag.strip_prefix("docker.io/").unwrap_or(tag);
    let tag = tag.strip_prefix("library/").unwrap_or(tag);
    let tag = tag.strip_prefix("localhost/").unwrap_or(tag);
    if tag.contains(':') || tag.contains('@') {
        tag.to_owned()
    } else {
        format!("{tag}:latest")
    }
}

/// Tags from `sbx template ls --json`: a list (or `{"templates"|"images": [...]}`) of entries with
/// `repository` and `tag`, a `tags` list, or a single `tag`/`name`/`reference`.
fn parse_template_listing(json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let entries = match &value {
        serde_json::Value::Array(a) => a.clone(),
        serde_json::Value::Object(o) => ["templates", "images", "items"]
            .iter()
            .find_map(|k| o.get(*k).and_then(|v| v.as_array()).cloned())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let field = |e: &serde_json::Value, keys: &[&str]| {
        e.as_object().and_then(|o| {
            o.iter()
                .find(|(k, _)| keys.iter().any(|w| k.eq_ignore_ascii_case(w)))
                .and_then(|(_, v)| v.as_str())
                .map(str::to_owned)
        })
    };
    let mut tags = Vec::new();
    for e in &entries {
        if let Some(list) = e
            .as_object()
            .and_then(|o| o.iter().find(|(k, _)| k.eq_ignore_ascii_case("tags")))
            .and_then(|(_, v)| v.as_array())
        {
            tags.extend(list.iter().filter_map(|t| t.as_str()).map(str::to_owned));
        } else if let (Some(repo), Some(tag)) =
            (field(e, &["repository", "repo"]), field(e, &["tag"]))
        {
            tags.push(format!("{repo}:{tag}"));
        } else if let Some(tag) = field(e, &["tag", "name", "reference", "ref"]) {
            tags.push(tag);
        } else if let Some(tag) = e.as_str() {
            tags.push(tag.to_owned());
        }
    }
    tags.retain(|t| !t.is_empty() && !t.contains("<none>"));
    tags
}

fn admin_dir(worktree: &Path) -> Option<PathBuf> {
    let gitfile = std::fs::read_to_string(worktree.join(".git")).ok()?;
    let dir = gitfile.strip_prefix("gitdir:")?.trim();
    Some(worktree.join(dir))
}

/// `exec` arguments (the same for sbx, docker and podman). Values of `env` stay in the CLI's own
/// environment (`-e KEY`), so they never show up in `ps`.
fn exec_args(name: &str, req: &LaunchRequest) -> Vec<String> {
    let mut args = vec!["exec".to_string()];
    if req.interactive {
        args.push("-it".into());
    }
    args.extend(["-w".into(), req.cwd.display().to_string()]);
    for (key, _) in &req.env {
        args.extend(["-e".into(), key.clone()]);
    }
    args.push(name.into());
    args.push("bash".into());
    match &req.command {
        Some(command) => args.extend(["-lc".into(), command.clone()]),
        None => args.push("-l".into()),
    }
    args
}

impl Backend for Container {
    fn isolation(&self) -> Isolation {
        match self.engine {
            Engine::Sbx => Isolation::DockerSandbox {
                image: self.image.clone(),
                template: self.template.clone(),
            },
            Engine::Docker => Isolation::Docker {
                image: self.image.clone(),
            },
            Engine::Podman => Isolation::Podman {
                image: self.image.clone(),
            },
        }
    }

    fn launch(&self, req: &LaunchRequest) -> Result<Launch> {
        ensure_valid_env(&req.env)?;
        let (name, req) = match &self.bound {
            Some(b) => (
                b.name.clone(),
                LaunchRequest {
                    cwd: b.workdir.clone(),
                    ..req.clone()
                },
            ),
            None => (sandbox_name(&req.cwd), req.clone()),
        };
        Ok(Launch {
            program: self.engine.binary()?.display().to_string(),
            args: exec_args(&name, &req),
            env: req.env.clone(),
            clear_env: false,
            cwd: std::env::temp_dir(),
        })
    }

    /// Creates the worktree's environment if needed and makes sure it runs. Without a worktree
    /// there is nothing to do: environments are per worktree.
    fn grant_access(&self, ws: &Workspace) -> Result<()> {
        let Some(worktree) = &ws.worktree else {
            return Ok(());
        };
        let name = sandbox_name(worktree);
        if let Some(listed) = self.list()?.into_iter().find(|s| s.name == name) {
            return self.ensure_running(&listed);
        }
        if !self.image_present() {
            bail!(
                "the image {} isn't available; open Isolation and run Set up",
                self.image
            );
        }
        let admin = admin_dir(worktree);
        self.create(
            &name,
            worktree,
            &mounts(&ws.repo, worktree, admin.as_deref()),
        )
    }

    fn save_template(&self, worktree: &Path, tag: &str) -> Result<Isolation> {
        crate::validate_image_ref(tag)?;
        let name = sandbox_name(worktree);
        let Some(listed) = self.list()?.into_iter().find(|l| l.name == name) else {
            bail!(
                "this worktree has no {} yet; start a session in it first",
                self.engine.noun()
            );
        };
        // sbx only saves stopped sandboxes; the next session starts it again.
        if self.engine == Engine::Sbx && listed.running {
            self.checked(["stop", name.as_str()], "stopping the sandbox")?;
        }
        let args: Vec<&str> = match self.engine {
            Engine::Sbx => vec!["template", "save", &name, tag],
            _ => vec!["commit", &name, tag],
        };
        self.checked(
            &args,
            &format!("saving a snapshot of the {} as {tag}", self.engine.noun()),
        )?;
        Ok(match self.engine {
            Engine::Sbx => Isolation::DockerSandbox {
                image: self.image.clone(),
                template: Some(tag.to_owned()),
            },
            Engine::Docker => Isolation::Docker {
                image: tag.to_owned(),
            },
            Engine::Podman => Isolation::Podman {
                image: tag.to_owned(),
            },
        })
    }

    fn templates(&self) -> Result<Vec<String>> {
        if self.engine.binary().is_err() || !self.engine_running() {
            return Ok(Vec::new());
        }
        let tags = match self.engine {
            Engine::Sbx => parse_template_listing(
                &self.checked(["template", "ls", "--json"], "listing templates")?,
            ),
            _ => self
                .checked(
                    ["image", "ls", "--format", "{{.Repository}}:{{.Tag}}"],
                    "listing images",
                )?
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.contains("<none>"))
                .map(String::from)
                .collect(),
        };
        let mut tags: Vec<String> = tags.iter().map(|t| short_image_name(t)).collect();
        tags.sort();
        tags.dedup();
        Ok(tags)
    }

    fn release(&self, ws: &Workspace) -> Result<()> {
        let Some(worktree) = &ws.worktree else {
            return Ok(());
        };
        let name = sandbox_name(worktree);
        let names: Vec<String> = self
            .list()?
            .into_iter()
            .filter(|l| l.name == name || l.workdir.as_deref() == Some(worktree.as_path()))
            .map(|l| l.name)
            .collect();
        self.remove_all(&names, &format!("removing the {}", self.engine.noun()))
    }

    fn shutdown(&self) -> Result<()> {
        if self.engine.binary().is_err() || !self.engine_running() {
            return Ok(());
        }
        let running: Vec<String> = self
            .list()?
            .into_iter()
            .filter(|s| s.running)
            .map(|s| s.name)
            .collect();
        if running.is_empty() {
            return Ok(());
        }
        let mut args = vec!["stop"];
        if self.engine != Engine::Sbx {
            args.extend(["-t", "3"]);
        }
        args.extend(running.iter().map(String::as_str));
        self.checked(&args, &format!("stopping {}s", self.engine.noun()))
            .map(drop)
    }

    /// Removes the environments of every worktree inside the project's repository or worktree
    /// folder, including worktrees deleted outside worki.
    fn forget_project(&self, ws: &Workspace) -> Result<()> {
        let names: Vec<String> = self
            .list()?
            .into_iter()
            .filter(|l| {
                l.workdir
                    .as_ref()
                    .is_some_and(|w| w.starts_with(&ws.repo) || w.starts_with(&ws.worktree_root))
            })
            .map(|l| l.name)
            .collect();
        self.remove_all(&names, &format!("removing {}s", self.engine.noun()))
    }

    /// Removes every environment worki created with this engine.
    fn remove(&self, _workspaces: &[Workspace]) -> Result<()> {
        let names: Vec<String> = self.list()?.into_iter().map(|s| s.name).collect();
        self.remove_all(&names, &format!("removing {}s", self.engine.noun()))
    }

    fn ready(&self) -> Result<()> {
        self.engine.binary()?;
        if self.engine != Engine::Sbx && !self.engine_running() {
            bail!(
                "{} isn't running; open Isolation to start it",
                self.engine.display()
            );
        }
        Ok(())
    }

    /// Starts the engine (sbx daemon, Podman machine).
    fn provision(&self) -> Result<()> {
        match self.engine {
            Engine::Sbx => self
                .checked(["daemon", "start"], "starting the sandbox daemon")
                .map(drop),
            Engine::Podman => self
                .checked(["machine", "start"], "starting the Podman machine")
                .map(drop),
            Engine::Docker => bail!("Start Docker Desktop, then re-run the checks."),
        }
    }

    /// Builds worki's default agent image from the bundled Containerfile, streaming the build
    /// output (it takes minutes on a fresh machine).
    fn build_image(&self, log: &mut dyn FnMut(String)) -> Result<()> {
        if self.engine == Engine::Sbx || self.image != DEFAULT_CONTAINER_IMAGE {
            bail!("only worki's own image ({DEFAULT_CONTAINER_IMAGE}) is built here");
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("worki-image-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let file = dir.join("Containerfile");
        std::fs::write(&file, AGENT_CONTAINERFILE)?;
        let mut args = vec!["build".to_string()];
        if self.engine == Engine::Docker {
            args.push("--progress=plain".into());
        }
        args.extend([
            "-t".into(),
            DEFAULT_CONTAINER_IMAGE.into(),
            "-f".into(),
            file.display().to_string(),
            dir.display().to_string(),
        ]);
        let result = stream_lines(self.command()?.args(&args), log);
        std::fs::remove_dir_all(&dir).ok();
        result.context("building the agent image")
    }

    fn doctor(&self, _ws: Option<&Workspace>) -> Vec<Check> {
        let mut checks = Vec::new();
        let engine = self.engine.display();
        let binary = self.engine.binary();
        checks.push(
            Check::new(
                "engine",
                binary.is_ok(),
                format!("{engine} is installed"),
                format!("{engine} is not installed"),
            )
            .detail(match &binary {
                Ok(path) => path.display().to_string(),
                Err(_) => match self.engine {
                    Engine::Sbx => "Install it from https://docs.docker.com/ai/sandboxes/".into(),
                    Engine::Docker => {
                        "Install Docker Desktop: https://www.docker.com/products/docker-desktop/"
                            .into()
                    }
                    Engine::Podman => {
                        "Install Podman: https://podman.io (e.g. `brew install podman`)".into()
                    }
                },
            }),
        );
        if binary.is_err() {
            return checks;
        }
        let running = self.engine_running();
        let what = match self.engine {
            Engine::Sbx => "The sandbox daemon",
            Engine::Docker => "Docker",
            Engine::Podman => "The Podman machine",
        };
        let mut check = Check::new(
            "running",
            running,
            format!("{what} is running"),
            format!("{what} is not running"),
        );
        if self.engine == Engine::Docker && !running {
            check = check.detail("Start Docker Desktop.");
        } else {
            check = check.fix_if_failed(Fix::Provision);
        }
        checks.push(check);
        if !running {
            return checks;
        }
        if self.engine != Engine::Sbx {
            let present = self.image_present();
            let mut check = Check::new(
                "image",
                present,
                format!("Image {} is available", self.image),
                format!("Image {} is not available", self.image),
            );
            if self.image == DEFAULT_CONTAINER_IMAGE {
                check = check
                    .detail("worki's agent image: Alpine with git, gh, ripgrep and Claude Code. Building takes a minute or two.")
                    .fix_if_failed(Fix::BuildImage);
            } else if !present {
                check = check.detail(format!(
                    "Pull it with `{} pull {}`.",
                    self.engine.program(),
                    self.image
                ));
            }
            checks.push(check);
        }
        let count = self.list().map(|l| l.len()).unwrap_or(0);
        checks.push(
            Check::new(
                "environments",
                true,
                format!("{count} worki {}(s)", self.engine.noun()),
                "",
            )
            .detail(format!(
                "One per worktree, created from `{}` on first use.",
                self.image
            )),
        );
        checks
    }

    /// Bound to an environment: looked up inside it. Otherwise only what the image ships is known.
    fn which(&self, command: &str) -> Option<String> {
        if self.bound.is_none() {
            let ships = match self.engine {
                Engine::Sbx => command == self.image,
                _ => self.image == DEFAULT_CONTAINER_IMAGE && command == "claude",
            };
            return ships.then(|| format!("in the `{}` image", self.image));
        }
        let out = self
            .sh(r#"bash -lc 'command -v "$1"' _ "$1""#, &[command], None)
            .ok()?;
        command_path(&out)
    }

    fn home(&self) -> Option<PathBuf> {
        self.bound
            .as_ref()
            .and_then(|_| self.sh(r#"printf %s "$HOME""#, &[], None).ok())
            .and_then(|out| home_path(&out))
            .or_else(|| Some(self.engine.default_home().into()))
    }

    fn instances(&self) -> Result<Vec<Box<dyn Backend>>> {
        if self.bound.is_some() {
            return Ok(vec![Box::new(self.clone())]);
        }
        let mut out: Vec<Box<dyn Backend>> = Vec::new();
        for listed in self.list()? {
            self.ensure_running(&listed)?;
            out.push(Box::new(self.bound_to(&listed)));
        }
        Ok(out)
    }

    fn instance_for(&self, ws: &Workspace) -> Option<Box<dyn Backend>> {
        let name = sandbox_name(ws.worktree.as_ref()?);
        let listed = self.list().ok()?.into_iter().find(|l| l.name == name)?;
        self.ensure_running(&listed).ok()?;
        Some(Box::new(self.bound_to(&listed)))
    }

    fn instance_exists(&self, ws: &Workspace) -> bool {
        let Some(worktree) = &ws.worktree else {
            return true;
        };
        let name = sandbox_name(worktree);
        self.list()
            .map(|l| l.iter().any(|s| s.name == name))
            .unwrap_or(false)
    }

    /// Streams a tar of the host files into `tar -x` inside the environment's home.
    fn copy_home_files(&self, from_home: &Path, paths: &[PathBuf]) -> Result<()> {
        for p in paths {
            ensure_relative(p)?;
        }
        if paths.is_empty() {
            return Ok(());
        }
        let mut pack = pack_home_files(from_home, paths);
        let out = self.sh(
            "tar -xf - --no-same-owner",
            &[],
            Some(Box::new(move |stdin| {
                let mut pack = pack.stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
                std::io::copy(&mut pack.stdout.take().expect("piped"), stdin)?;
                pack.wait()?;
                Ok(())
            })),
        )?;
        check_output(out, &format!("copying into the {}", self.engine.noun())).map(drop)
    }

    fn read_home_file(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        ensure_relative(path)?;
        let out = self.sh(READ_FILE_SCRIPT, &[&path.to_string_lossy()], None)?;
        file_contents(out, &format!("reading {}", path.display()))
    }

    fn write_home_file(&self, path: &Path, data: &[u8]) -> Result<()> {
        ensure_relative(path)?;
        let data = data.to_vec();
        let out = self.sh(
            WRITE_FILE_SCRIPT,
            &[&path.to_string_lossy()],
            Some(Box::new(move |stdin| {
                std::io::Write::write_all(stdin, &data)
            })),
        )?;
        check_output(out, &format!("writing {}", path.display())).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_stable_valid_and_distinct() {
        let a = sandbox_name(Path::new("/Users/me/worki/worktrees/app/feature-login"));
        let b = sandbox_name(Path::new("/Users/me/worki/worktrees/api/feature-login"));
        assert_eq!(
            a,
            sandbox_name(Path::new("/Users/me/worki/worktrees/app/feature-login"))
        );
        assert_ne!(a, b);
        assert!(a.starts_with("worki-feature-login-"));
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert!(sandbox_name(Path::new("/x/Agent Login_v2!")).starts_with("worki-agent-login-v2-"));
    }

    #[test]
    fn mounts_keep_git_metadata_read_only() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let wt = root.path().join("wt");
        let admin = repo.join(".git/worktrees/wt");
        std::fs::create_dir_all(repo.join(".git/hooks")).unwrap();
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        for f in [
            repo.join(".git/config"),
            wt.join(".git"),
            admin.join("gitdir"),
            admin.join("commondir"),
        ] {
            std::fs::write(f, "x").unwrap();
        }
        let m = mounts(&repo, &wt, Some(&admin));
        assert_eq!(
            m[0],
            Mount {
                path: wt.clone(),
                read_only: false
            }
        );
        assert_eq!(
            m[1],
            Mount {
                path: repo.join(".git"),
                read_only: true
            }
        );
        for rw in [
            repo.join(".git/objects"),
            repo.join(".git/refs"),
            repo.join(".git/logs"),
            admin.clone(),
        ] {
            let at = m.iter().position(|x| x.path == rw);
            assert!(
                at.is_some_and(|i| !m[i].read_only && i > 1),
                "{rw:?} not writable after .git in {m:?}"
            );
        }
        let admin_at = m.iter().position(|x| x.path == admin).unwrap();
        for ro in [
            wt.join(".git"),
            admin.join("gitdir"),
            admin.join("commondir"),
        ] {
            let at = m.iter().position(|x| x.path == ro);
            assert!(
                at.is_some_and(|i| m[i].read_only && i > admin_at),
                "{ro:?} not read-only over the admin folder in {m:?}"
            );
        }
        let hooks = Mount {
            path: repo.join(".git/hooks"),
            read_only: true,
        };
        assert_eq!(
            hooks.sbx_arg(),
            format!("{}:ro", repo.join(".git/hooks").display())
        );
        assert_eq!(
            hooks.volume_arg(),
            format!("{0}:{0}:ro", repo.join(".git/hooks").display())
        );

        let main = mounts(&repo, &repo, None);
        assert_eq!(
            main[0],
            Mount {
                path: repo.clone(),
                read_only: false
            }
        );
        assert!(
            !main.contains(&Mount {
                path: repo.join(".git"),
                read_only: false
            }),
            "main checkout already contains .git"
        );
    }

    #[test]
    fn shortens_image_names() {
        assert_eq!(
            short_image_name("docker.io/library/worki-sam:1"),
            "worki-sam:1"
        );
        assert_eq!(short_image_name("localhost/worki-sam:1"), "worki-sam:1");
        assert_eq!(short_image_name("ghcr.io/me/app:2"), "ghcr.io/me/app:2");
        assert_eq!(short_image_name("app"), "app:latest");
    }

    #[test]
    fn reads_template_listings() {
        assert_eq!(
            parse_template_listing(
                r#"[{"Repository":"worki-app","Tag":"1"},{"tags":["a:1","b:2"]}]"#
            ),
            ["worki-app:1", "a:1", "b:2"]
        );
        assert_eq!(
            parse_template_listing(
                r#"{"images":[{"tag":"myimage:v1.0","id":"abc"},{"repository":"<none>","tag":"<none>"}]}"#
            ),
            ["myimage:v1.0"]
        );
        assert!(parse_template_listing("not json").is_empty());
    }

    #[test]
    fn exec_passes_env_by_name_only() {
        let req = LaunchRequest {
            cwd: "/Users/me/wt".into(),
            command: Some("exec claude".into()),
            interactive: true,
            env: vec![("CLAUDE_CODE_OAUTH_TOKEN".into(), "sk-secret".into())],
            session: None,
        };
        let args = exec_args("worki-wt-abc123", &req);
        assert_eq!(
            args,
            [
                "exec",
                "-it",
                "-w",
                "/Users/me/wt",
                "-e",
                "CLAUDE_CODE_OAUTH_TOKEN",
                "worki-wt-abc123",
                "bash",
                "-lc",
                "exec claude"
            ]
        );
        assert!(!args.iter().any(|a| a.contains("sk-secret")));
        let script = LaunchRequest {
            interactive: false,
            command: Some("pnpm i".into()),
            ..req
        };
        assert_eq!(
            &exec_args("n", &script)[..3],
            ["exec", "-w", "/Users/me/wt"]
        );
    }

    #[test]
    fn streams_output_lines_and_reports_failures() {
        let mut lines = Vec::new();
        stream_lines(
            Command::new("/bin/sh").args(["-c", "echo one; echo two >&2"]),
            &mut |l| lines.push(l),
        )
        .unwrap();
        lines.sort();
        assert_eq!(lines, ["one", "two"]);
        let err = stream_lines(
            Command::new("/bin/sh").args(["-c", "echo boom >&2; exit 3"]),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn parses_sbx_listing() {
        let json = r#"{"sandboxes":[{"name":"worki-a-1","status":"running","workspaces":["/a","/a/.git/hooks:ro"]},{"name":"claude-sam","status":"stopped"}]}"#;
        let l = parse_sbx_listing(json).unwrap();
        assert_eq!(
            l[0],
            Listed {
                name: "worki-a-1".into(),
                running: true,
                workdir: Some("/a".into())
            }
        );
        assert!(!l[1].running);
        assert!(parse_sbx_listing("{}").unwrap().is_empty());
    }

    #[test]
    fn parses_docker_and_podman_listings() {
        let docker = r#"{"Names":"worki-a-1","State":"running","Labels":"worki.workdir=/w/a,other=x"}
{"Names":"worki-b-2","State":"exited","Labels":""}"#;
        let d = parse_ps_listing(docker).unwrap();
        assert_eq!(
            d[0],
            Listed {
                name: "worki-a-1".into(),
                running: true,
                workdir: Some("/w/a".into())
            }
        );
        assert_eq!(
            d[1],
            Listed {
                name: "worki-b-2".into(),
                running: false,
                workdir: None
            }
        );

        let podman =
            r#"[{"Names":["worki-a-1"],"State":"running","Labels":{"worki.workdir":"/w/a"}}]"#;
        assert_eq!(
            parse_ps_listing(podman).unwrap()[0].workdir,
            Some("/w/a".into())
        );
        assert!(parse_ps_listing("").unwrap().is_empty());
        assert!(parse_ps_listing("[]").unwrap().is_empty());
    }
}
