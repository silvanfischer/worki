//! Where a project's files live: this Mac, or a machine reached over SSH. Git and the few file
//! operations worki does on repositories go through here.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Output;

use tokio::process::Command;
use worki_isolation::ssh::{remote_sh, shell_quote, ssh_command, SshTarget};

use crate::model::Project;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Host {
    Local,
    Ssh(SshTarget),
}

impl Host {
    pub fn of(project: &Project) -> Self {
        match &project.remote {
            Some(r) => Self::Ssh(r.target()),
            None => Self::Local,
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Ssh(_))
    }

    /// Runs `program args…` on the host with `env` set.
    async fn output(&self, program: &str, args: &[String], env: &[(&str, &str)]) -> Result<Output> {
        match self {
            Self::Local => {
                let mut cmd = Command::new(program);
                cmd.args(args).envs(env.iter().copied());
                run(cmd).await
            }
            Self::Ssh(host) => {
                let mut script = String::new();
                for (k, v) in env {
                    script.push_str(&format!("{k}={}; export {k}; ", shell_quote(v)));
                }
                script.push_str(&format!("exec {program} \"$@\""));
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                ssh_output(host, &script, &args).await
            }
        }
    }

    /// `git -C dir args…` with prompts disabled and untranslated output.
    pub async fn git_output<I, S>(&self, dir: &Path, args: I) -> Result<(Vec<String>, Output)>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<String> = args
            .into_iter()
            .map(|a| a.as_ref().to_string_lossy().into_owned())
            .collect();
        let mut full = vec!["-C".to_owned(), dir.to_string_lossy().into_owned()];
        full.extend(args.iter().cloned());
        let out = self
            .output(
                "git",
                &full,
                &[("GIT_TERMINAL_PROMPT", "0"), ("LC_ALL", "C")],
            )
            .await?;
        Ok((args, out))
    }

    pub async fn git<I, S>(&self, dir: &Path, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let (args, out) = self.git_output(dir, args).await?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(Error::Git {
                args: args.join(" "),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            })
        }
    }

    /// Runs `program args…` in `dir` through the user's login shell, so tools installed with
    /// Homebrew and the like (e.g. `gh`) are on PATH even though worki started from the Dock.
    pub async fn tool_output(&self, dir: &Path, program: &str, args: &[&str]) -> Result<Output> {
        let script = "cd \"$1\" && shift && exec \"$@\"";
        let dir = dir.to_string_lossy();
        let mut full: Vec<&str> = vec![&dir, program];
        full.extend(args);
        match self {
            Self::Local => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
                let mut cmd = Command::new(shell);
                cmd.args(["-lc", script, "_"]).args(&full);
                run(cmd).await
            }
            Self::Ssh(host) => {
                let login = format!(
                    "exec \"${{SHELL:-/bin/sh}}\" -lc {} _ \"$@\"",
                    shell_quote(script)
                );
                ssh_output(host, &login, &full).await
            }
        }
    }

    async fn sh(&self, script: &str, args: &[&str]) -> Result<Output> {
        match self {
            Self::Local => {
                let mut full = vec!["-c".to_owned(), script.to_owned(), "_".to_owned()];
                full.extend(args.iter().map(|a| a.to_string()));
                self.output("/bin/sh", &full, &[]).await
            }
            Self::Ssh(host) => ssh_output(host, script, args).await,
        }
    }

    pub async fn exists(&self, path: &Path) -> Result<bool> {
        match self {
            Self::Local => Ok(path.exists()),
            Self::Ssh(_) => Ok(self
                .sh("[ -e \"$1\" ]", &[&path.to_string_lossy()])
                .await?
                .status
                .success()),
        }
    }

    pub async fn create_dir_all(&self, path: &Path) -> Result<()> {
        match self {
            Self::Local => Ok(std::fs::create_dir_all(path)?),
            Self::Ssh(host) => {
                let out = self
                    .sh("mkdir -p \"$1\"", &[&path.to_string_lossy()])
                    .await?;
                if out.status.success() {
                    Ok(())
                } else {
                    Err(remote_error(host, &out.stderr))
                }
            }
        }
    }

    /// A file's text, `None` when it doesn't exist.
    pub async fn read_to_string(&self, path: &Path) -> Result<Option<String>> {
        match self {
            Self::Local => match std::fs::read_to_string(path) {
                Ok(text) => Ok(Some(text)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            },
            Self::Ssh(host) => {
                let out = self
                    .sh(
                        "[ -f \"$1\" ] || exit 3; cat \"$1\"",
                        &[&path.to_string_lossy()],
                    )
                    .await?;
                match out.status.code() {
                    Some(0) => Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned())),
                    Some(3) => Ok(None),
                    _ => Err(remote_error(host, &out.stderr)),
                }
            }
        }
    }

    /// Resolves symlinks like git does, so paths compare equal to what git reports.
    pub async fn canonical(&self, path: &Path) -> PathBuf {
        match self {
            Self::Local => path.canonicalize().unwrap_or_else(|_| path.into()),
            Self::Ssh(_) => {
                let p = path.to_string_lossy();
                match self.sh("cd \"$1\" 2>/dev/null && pwd -P", &[&p]).await {
                    Ok(out) if out.status.success() => {
                        let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                        if text.starts_with('/') {
                            PathBuf::from(text)
                        } else {
                            path.into()
                        }
                    }
                    _ => path.into(),
                }
            }
        }
    }

    /// Expands a leading `~` against the host's home directory.
    pub async fn expand_tilde(&self, path: &Path) -> Result<PathBuf> {
        match self {
            Self::Local => Ok(crate::paths::expand_tilde(path)),
            Self::Ssh(_) => {
                let Ok(rest) = path.strip_prefix("~") else {
                    return Ok(path.into());
                };
                Ok(self.home().await?.join(rest))
            }
        }
    }

    pub async fn home(&self) -> Result<PathBuf> {
        match self {
            Self::Local => Ok(dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))),
            Self::Ssh(host) => {
                let out = self.sh("printf %s \"$HOME\"", &[]).await?;
                let home = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                if out.status.success() && home.starts_with('/') {
                    Ok(PathBuf::from(home))
                } else {
                    Err(Error::Remote {
                        host: host.display(),
                        message: "could not determine the home folder".into(),
                    })
                }
            }
        }
    }
}

/// Runs `script` with `args` through `sh` on `host`. Exit code 255 is ssh's own failure.
async fn ssh_output(host: &SshTarget, script: &str, args: &[&str]) -> Result<Output> {
    let ssh = ssh_command(host, false);
    let mut cmd = Command::new("/usr/bin/ssh");
    cmd.args(&ssh.args)
        .envs(ssh.env)
        .arg(remote_sh(script, args));
    let out = run(cmd).await?;
    if out.status.code() == Some(255) {
        return Err(remote_error(host, &out.stderr));
    }
    Ok(out)
}

async fn run(mut cmd: Command) -> Result<Output> {
    Ok(cmd
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .output()
        .await?)
}

fn remote_error(host: &SshTarget, stderr: &[u8]) -> Error {
    Error::Remote {
        host: host.display(),
        message: String::from_utf8_lossy(stderr).trim().to_owned(),
    }
}
