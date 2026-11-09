use std::path::Path;

use worki_isolation::ssh::shell_quote;
use worki_isolation::{LaunchRequest, SessionMeta};

use crate::model::AgentProfile;

/// Environment every session gets, regardless of isolation.
pub fn session_env(session_id: &str, worktree: &Path) -> Vec<(String, String)> {
    vec![
        ("TERM".into(), "xterm-256color".into()),
        ("COLORTERM".into(), "truecolor".into()),
        ("TERM_PROGRAM".into(), "worki".into()),
        (
            "LANG".into(),
            std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".into()),
        ),
        ("WORKI_SESSION_ID".into(), session_id.into()),
        (
            "WORKI_WORKTREE_PATH".into(),
            worktree.to_string_lossy().into_owned(),
        ),
    ]
}

/// An agent profile, or a plain shell for `None`. `exec` hands the PTY to the agent itself.
pub fn session_request(
    profile: Option<&AgentProfile>,
    cwd: &Path,
    env: Vec<(String, String)>,
    session: Option<SessionMeta>,
) -> LaunchRequest {
    LaunchRequest {
        cwd: cwd.into(),
        command: profile.map(agent_command),
        interactive: true,
        env,
        session,
    }
}

fn agent_command(profile: &AgentProfile) -> String {
    match profile.executable() {
        // Claude runs hooks without a controlling terminal, so the shell records the session's
        // terminal device first and the hooks write there.
        Some("claude") => format!(
            "export WORKI_TTY=\"$(tty)\"; exec {} --settings {}",
            profile.command,
            shell_quote(&claude_status_hooks())
        ),
        _ => format!("exec {}", profile.command),
    }
}

/// Extra Claude Code settings (passed with `--settings`, so the user's own settings files stay
/// untouched) whose hooks print worki's status marker to the session's terminal (`$WORKI_TTY`),
/// where the PTY reader picks it up. Going through the terminal works the same for every
/// isolation: as the user, as an agent user, inside a sandbox. Inside tmux (kept-alive remote
/// sessions) the marker is wrapped so tmux passes it through.
fn claude_status_hooks() -> String {
    let marker = |state: &str| {
        serde_json::json!([{ "hooks": [{
            "type": "command",
            "command": format!(
                "[ -n \"$WORKI_TTY\" ] && {{ if [ -n \"$TMUX\" ]; then printf '\\033Ptmux;\\033\\033]777;worki;{state}\\007\\033\\\\'; else printf '\\033]777;worki;{state}\\007'; fi; }} > \"$WORKI_TTY\" 2>/dev/null || true"
            ),
        }]}])
    };
    serde_json::json!({ "hooks": {
        "UserPromptSubmit": marker("working"),
        "PreToolUse": marker("working"),
        "Stop": marker("waiting"),
        "Notification": marker("waiting"),
    }})
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_command_is_execed() {
        let p = AgentProfile {
            id: "a".into(),
            name: "A".into(),
            command: "aider --verbose".into(),
            install: None,
        };
        let req = session_request(
            Some(&p),
            Path::new("/w"),
            session_env("s1", Path::new("/w")),
            None,
        );
        assert_eq!(req.command.as_deref(), Some("exec aider --verbose"));
        assert!(req.env.contains(&("WORKI_SESSION_ID".into(), "s1".into())));
        assert_eq!(
            session_request(None, Path::new("/w"), vec![], None).command,
            None
        );
    }

    #[test]
    fn claude_gets_status_hooks_that_survive_the_shell() {
        let p = AgentProfile {
            id: "c".into(),
            name: "C".into(),
            command: "claude".into(),
            install: None,
        };
        let command = session_request(Some(&p), Path::new("/w"), vec![], None)
            .command
            .unwrap();
        assert!(command.starts_with("export WORKI_TTY=\"$(tty)\"; exec claude --settings '"));

        let out = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                &command.replacen("exec claude --settings", "printf %s", 1),
            ])
            .output()
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let hook = json["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        let printed = std::process::Command::new("/bin/sh")
            .args(["-c", &hook.replace("> \"$WORKI_TTY\"", "")])
            .env("WORKI_TTY", "/dev/ttys-test")
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert_eq!(printed.stdout, b"\x1b]777;worki;waiting\x07");
        let wrapped = std::process::Command::new("/bin/sh")
            .args(["-c", &hook.replace("> \"$WORKI_TTY\"", "")])
            .env("WORKI_TTY", "/dev/ttys-test")
            .env("TMUX", "/tmp/tmux-1/worki,1,0")
            .output()
            .unwrap();
        assert_eq!(
            wrapped.stdout,
            b"\x1bPtmux;\x1b\x1b]777;worki;waiting\x07\x1b\\"
        );
    }
}
