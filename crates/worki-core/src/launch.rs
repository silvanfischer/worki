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
/// `prompt` is handed to the agent as its first message.
pub fn session_request(
    profile: Option<&AgentProfile>,
    cwd: &Path,
    env: Vec<(String, String)>,
    session: Option<SessionMeta>,
    confined: bool,
    prompt: Option<&str>,
) -> LaunchRequest {
    LaunchRequest {
        cwd: cwd.into(),
        command: profile
            .map(|p| agent_command(p, session.as_ref().map(|s| s.id.as_str()), confined, prompt)),
        interactive: true,
        env,
        session,
    }
}

/// `confined`: worki's isolation already keeps the agent in its worktree, so Claude Code's own
/// sandbox is turned off; it only got in the way there (e.g. it hides the system CA bundle).
fn agent_command(
    profile: &AgentProfile,
    session_id: Option<&str>,
    confined: bool,
    prompt: Option<&str>,
) -> String {
    let prompt = prompt
        .filter(|p| !p.trim().is_empty())
        .map(|p| format!(" {}", shell_quote(p)))
        .unwrap_or_default();
    match profile.executable() {
        // Claude runs hooks without a controlling terminal, so the shell records the session's
        // terminal device first and the hooks write there.
        Some("claude") => {
            let hooks = shell_quote(&claude_settings(confined));
            match session_id {
                Some(id) => format!(
                    "export WORKI_TTY=\"$(tty)\"; {} exec {} --settings {hooks} \"$@\"{prompt}",
                    claude_conversation(id),
                    profile.command,
                ),
                None => format!(
                    "export WORKI_TTY=\"$(tty)\"; exec {} --settings {hooks}{prompt}",
                    profile.command
                ),
            }
        }
        _ => format!("exec {}{prompt}", profile.command),
    }
}

/// Names Claude's conversation after the worki session, so starting the session again (after
/// worki restarted) resumes it once Claude has saved a transcript for it.
fn claude_conversation(id: &str) -> String {
    let id = shell_quote(id);
    format!(
        "if ls \"${{CLAUDE_CONFIG_DIR:-$HOME/.claude}}\"/projects/*/{id}.jsonl >/dev/null 2>&1; then set -- --resume {id}; else set -- --session-id {id}; fi;"
    )
}

/// Extra Claude Code settings (passed with `--settings`, so the user's own settings files stay
/// untouched) whose hooks print worki's status marker to the session's terminal (`$WORKI_TTY`),
/// where the PTY reader picks it up. Going through the terminal works the same for every
/// isolation: as the user, as an agent user, inside a sandbox. Inside tmux (kept-alive remote
/// sessions) the marker is wrapped so tmux passes it through.
fn claude_settings(confined: bool) -> String {
    // `$s` holds the state; the marker is wrapped for tmux when the session runs inside it.
    let print = "[ -n \"$WORKI_TTY\" ] && { if [ -n \"$TMUX\" ]; then printf '\\033Ptmux;\\033\\033]777;worki;%s\\007\\033\\\\' \"$s\"; else printf '\\033]777;worki;%s\\007' \"$s\"; fi; } > \"$WORKI_TTY\" 2>/dev/null || true";
    let hook = |command: String| serde_json::json!({ "type": "command", "command": command });
    let marker =
        |state: &str| serde_json::json!([{ "hooks": [hook(format!("s={state}; {print}"))] }]);
    // A question to the user arrives as a tool call; it waits for an answer rather than working.
    let before_tool = serde_json::json!([{ "hooks": [hook(format!(
        "s=working; grep -q '\"tool_name\" *: *\"AskUserQuestion\"' && s=waiting; {print}"
    ))] }]);
    let mut stop = marker("waiting");
    stop[0]["hooks"]
        .as_array_mut()
        .expect("marker hooks")
        .push(hook(USAGE_HOOK.into()));
    let mut settings = serde_json::json!({ "hooks": {
        "UserPromptSubmit": marker("working"),
        "PreToolUse": before_tool,
        // Fires as the permission prompt appears; Notification only follows about 6 s later.
        "PermissionRequest": marker("waiting"),
        "PostToolUse": marker("working"),
        "Stop": stop,
        "Notification": marker("waiting"),
    }});
    if confined {
        settings["sandbox"] = serde_json::json!({ "enabled": false });
    }
    settings.to_string()
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
            false,
            None,
        );
        assert_eq!(req.command.as_deref(), Some("exec aider --verbose"));
        let with_prompt = session_request(
            Some(&p),
            Path::new("/w"),
            vec![],
            None,
            false,
            Some("fix it's tests"),
        );
        let out = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                &with_prompt
                    .command
                    .unwrap()
                    .replacen("exec aider", "printf '%s|'", 1),
            ])
            .output()
            .unwrap();
        assert_eq!(out.stdout, b"--verbose|fix it's tests|");
        assert!(req.env.contains(&("WORKI_SESSION_ID".into(), "s1".into())));
        assert_eq!(
            session_request(None, Path::new("/w"), vec![], None, false, None).command,
            None
        );
    }

    #[test]
    fn claude_resumes_its_conversation_once_saved() {
        let p = AgentProfile {
            id: "c".into(),
            name: "C".into(),
            command: "claude".into(),
            install: None,
        };
        let meta = SessionMeta {
            id: "abc".into(),
            title: "C".into(),
            profile_id: None,
        };
        let command = session_request(Some(&p), Path::new("/w"), vec![], Some(meta), false, None)
            .command
            .unwrap();
        let run = |home: &Path| {
            let out = std::process::Command::new("/bin/sh")
                .args([
                    "-c",
                    &command.replacen("exec claude --settings", "printf '%s ' x", 1),
                ])
                .env("HOME", home)
                .env_remove("CLAUDE_CONFIG_DIR")
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap()
        };
        let home = tempfile::tempdir().unwrap();
        assert!(run(home.path()).ends_with("--session-id abc "));
        let project = home.path().join(".claude/projects/-w");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("abc.jsonl"), "").unwrap();
        assert!(run(home.path()).ends_with("--resume abc "));
    }

    #[test]
    fn claude_gets_status_hooks_that_survive_the_shell() {
        let p = AgentProfile {
            id: "c".into(),
            name: "C".into(),
            command: "claude".into(),
            install: None,
        };
        let command = session_request(Some(&p), Path::new("/w"), vec![], None, false, None)
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
        assert!(json.get("sandbox").is_none());
        let confined: serde_json::Value =
            serde_json::from_str(&super::claude_settings(true)).unwrap();
        assert_eq!(confined["sandbox"]["enabled"], false);
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

    #[test]
    fn tool_calls_mark_working_except_questions() {
        use std::io::Write;
        let settings: serde_json::Value =
            serde_json::from_str(&super::claude_settings(false)).unwrap();
        let hooks = &settings["hooks"];
        assert!(hooks["PermissionRequest"].is_array() && hooks["PostToolUse"].is_array());
        let hook = hooks["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        let run = |input: &str| {
            let mut child = std::process::Command::new("/bin/sh")
                .args(["-c", &hook.replace("> \"$WORKI_TTY\"", "")])
                .env("WORKI_TTY", "/dev/ttys-test")
                .env_remove("TMUX")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
            child.wait_with_output().unwrap().stdout
        };
        assert_eq!(
            run(r#"{"tool_name":"Bash"}"#),
            b"\x1b]777;worki;working\x07"
        );
        assert_eq!(
            run(r#"{"tool_name": "AskUserQuestion"}"#),
            b"\x1b]777;worki;waiting\x07"
        );
    }
}
