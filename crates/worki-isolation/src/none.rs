use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::{command_path, Backend, Check, Isolation, Launch, LaunchRequest, Workspace};

/// Resolves `command` the way the human's own terminal would (login + interactive shell, so
/// version managers initialised in rc files are honoured).
pub fn host_which(command: &str) -> Option<String> {
    let out = Command::new(user_shell())
        .args(["-l", "-i", "-c", "command -v \"$1\"", "_", command])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    command_path(&out)
}

/// Runs agents as the human user, with no boundary at all.
pub struct NoIsolation;

fn user_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/zsh".into())
}

pub(crate) fn shell_args(req: &LaunchRequest) -> Vec<String> {
    let mut args = vec!["-l".to_string()];
    if let Some(command) = &req.command {
        if req.interactive {
            args.push("-i".into());
        }
        args.push("-c".into());
        args.push(command.clone());
    }
    args
}

impl Backend for NoIsolation {
    fn isolation(&self) -> Isolation {
        Isolation::None
    }

    fn launch(&self, req: &LaunchRequest) -> anyhow::Result<Launch> {
        Ok(Launch {
            program: user_shell(),
            args: shell_args(req),
            env: req.env.clone(),
            clear_env: false,
            cwd: req.cwd.clone(),
        })
    }

    fn grant_access(&self, _ws: &Workspace) -> anyhow::Result<()> {
        Ok(())
    }

    fn doctor(&self, _ws: Option<&Workspace>) -> Vec<Check> {
        Vec::new()
    }

    fn which(&self, command: &str) -> Option<String> {
        host_which(command)
    }

    fn home(&self) -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(command: Option<&str>, interactive: bool) -> LaunchRequest {
        LaunchRequest {
            cwd: "/w".into(),
            command: command.map(Into::into),
            interactive,
            env: vec![("A".into(), "1".into())],
            session: None,
        }
    }

    #[test]
    fn builds_login_shell_invocations() {
        assert_eq!(shell_args(&req(None, true)), ["-l"]);
        assert_eq!(
            shell_args(&req(Some("exec claude"), true)),
            ["-l", "-i", "-c", "exec claude"]
        );
        assert_eq!(
            shell_args(&req(Some("pnpm i"), false)),
            ["-l", "-c", "pnpm i"]
        );
        let l = NoIsolation.launch(&req(None, true)).unwrap();
        assert!(!l.clear_env);
        assert_eq!(l.env, [("A".to_string(), "1".to_string())]);
    }
}
