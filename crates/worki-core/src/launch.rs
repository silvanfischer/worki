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
    let mut stop = marker("waiting");
    stop[0]["hooks"]
        .as_array_mut()
        .expect("marker hooks")
        .push(serde_json::json!({ "type": "command", "command": USAGE_HOOK }));
    serde_json::json!({ "hooks": {
        "UserPromptSubmit": marker("working"),
        "PreToolUse": marker("working"),
        "Stop": stop,
        "Notification": marker("waiting"),
    }})
    .to_string()
}

/// Adds up the token usage in the conversation's transcript (path from the hook's JSON input)
/// per model and prints it as a `usage` marker. A message is written as several transcript lines
/// repeating its usage, so each message id counts once. Plain sed and awk, so it runs wherever
/// the agent does.
const USAGE_HOOK: &str = r#"t=$(sed -n 's/.*"transcript_path" *: *"\([^"]*\)".*/\1/p' | head -n 1); [ -n "$WORKI_TTY" ] && [ -f "$t" ] && u=$(awk '
function num(k,  r) { if (match($0, "\"" k "\":[0-9]+")) { r = substr($0, RSTART, RLENGTH); sub(/.*:/, "", r); return r + 0 } return 0 }
/"type":"assistant"/ && /"usage":/ {
  id = ""; if (match($0, /"id":"msg_[^"]*"/)) id = substr($0, RSTART, RLENGTH)
  if (id != "") { if (id in seen) next; seen[id] = 1 }
  m = ""; if (match($0, /"model":"[^"]*"/)) m = substr($0, RSTART + 9, RLENGTH - 10)
  if (m == "" || m ~ /[<:,;]/) next
  i[m] += num("input_tokens"); o[m] += num("output_tokens"); w[m] += num("cache_creation_input_tokens")
  r[m] += num("cache_read_input_tokens"); h[m] += num("ephemeral_1h_input_tokens")
}
END { s = ""; for (m in o) s = s (s == "" ? "" : ",") m ":" i[m] ":" o[m] ":" w[m] ":" r[m] ":" h[m]; printf "%s", s }' "$t") && [ -n "$u" ] && { if [ -n "$TMUX" ]; then printf '\033Ptmux;\033\033]777;worki;usage;%s\007\033\\' "$u"; else printf '\033]777;worki;usage;%s\007' "$u"; fi; } > "$WORKI_TTY" 2>/dev/null || true"#;

#[cfg(test)]
mod tests {
    #[test]
    fn usage_hook_sums_the_transcript_per_model() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("t.jsonl");
        let line = |id: &str, model: &str, out: u32| {
            format!(
                r#"{{"type":"assistant","message":{{"id":"{id}","model":"{model}","usage":{{"input_tokens":3,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,"cache_creation":{{"ephemeral_5m_input_tokens":60,"ephemeral_1h_input_tokens":40}},"output_tokens":{out}}}}}}}"#
            )
        };
        std::fs::write(
            &transcript,
            [
                r#"{"type":"user","message":{"content":"hi"}}"#.to_owned(),
                line("msg_1", "claude-opus-5-5", 50),
                line("msg_1", "claude-opus-5-5", 50),
                line("msg_2", "claude-opus-5-5", 7),
                line("msg_3", "<synthetic>", 0),
            ]
            .join("\n"),
        )
        .unwrap();
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", &USAGE_HOOK.replace("> \"$WORKI_TTY\"", "")])
            .env("WORKI_TTY", "/dev/ttys-test")
            .env_remove("TMUX")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        write!(
            child.stdin.take().unwrap(),
            r#"{{"session_id":"s","transcript_path":"{}","hook_event_name":"Stop"}}"#,
            transcript.display()
        )
        .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "\x1b]777;worki;usage;claude-opus-5-5:6:57:200:2000:80\x07"
        );
    }

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
