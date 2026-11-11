use std::path::Path;

use worki_isolation::ssh::shell_quote;
use worki_isolation::{Isolation, LaunchRequest, SessionMeta};

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
        stdin: false,
        env,
        session,
    }
}

/// Agents worki can run as a chat, by the JSON protocol they speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatAgent {
    Claude,
}

impl ChatAgent {
    pub fn of(profile: &AgentProfile) -> Option<Self> {
        match profile.executable()? {
            "claude" => Some(Self::Claude),
            _ => None,
        }
    }
}

/// Runs the agent without a terminal, talking its JSON protocol over stdin and stdout, in the
/// conversation named `session_id` (resumed once the agent saved it). `None` for agents that
/// can't run as a chat.
pub fn chat_request(
    profile: &AgentProfile,
    cwd: &Path,
    env: Vec<(String, String)>,
    session_id: &str,
    confined: bool,
) -> Option<(ChatAgent, LaunchRequest)> {
    let agent = ChatAgent::of(profile)?;
    let command = match agent {
        ChatAgent::Claude => {
            let sandbox = if confined {
                format!(
                    " --settings {}",
                    shell_quote(r#"{"sandbox":{"enabled":false}}"#)
                )
            } else {
                String::new()
            };
            format!(
                "{} exec {} -p --input-format stream-json --output-format stream-json --verbose \
--include-partial-messages --replay-user-messages --permission-prompt-tool stdio --append-system-prompt {}{sandbox} \"$@\"",
                claude_conversation(session_id),
                profile.command,
                shell_quote(CHAT_PROMPT),
            )
        }
    };
    Some((
        agent,
        LaunchRequest {
            cwd: cwd.into(),
            command: Some(command),
            interactive: false,
            stdin: true,
            env,
            session: None,
        },
    ))
}

/// A command for the user's own terminal that continues Claude Code's conversation `session_id`
/// where it ran: in the worktree, as the agent user, inside the sandbox or container, or on the
/// SSH host, since that's where Claude Code saved it.
pub fn continue_command(isolation: &Isolation, worktree: &Path, session_id: &str) -> String {
    let claude = format!("claude --resume {}", shell_quote(session_id));
    let dir = shell_quote(&worktree.to_string_lossy());
    let in_dir = format!("cd {dir} && {claude}");
    match isolation {
        Isolation::None => in_dir,
        Isolation::MacUser { user } => format!(
            "sudo -u {} -H /bin/zsh -lc {}",
            shell_quote(user),
            shell_quote(&in_dir)
        ),
        Isolation::DockerSandbox { .. } | Isolation::Docker { .. } | Isolation::Podman { .. } => {
            let engine = match isolation {
                Isolation::DockerSandbox { .. } => "sbx",
                Isolation::Docker { .. } => "docker",
                _ => "podman",
            };
            format!(
                "{engine} exec -it -w {dir} -e IS_SANDBOX=1 {} bash -lc {}",
                worki_isolation::sandbox_name(worktree),
                shell_quote(&claude)
            )
        }
        Isolation::Ssh {
            host, user, port, ..
        } => {
            let port = port.map(|p| format!(" -p {p}")).unwrap_or_default();
            let target = match user {
                Some(user) => format!("{user}@{host}"),
                None => host.clone(),
            };
            format!(
                "ssh -t{port} {} {}",
                shell_quote(&target),
                shell_quote(&in_dir)
            )
        }
    }
}

/// Prints the transcript the agent saved for the conversation `session_id` (nothing when it has
/// none), run where the agent runs so it finds the agent's own files.
pub fn chat_history_command(agent: ChatAgent, session_id: &str) -> String {
    match agent {
        ChatAgent::Claude => format!(
            "cat \"${{CLAUDE_CONFIG_DIR:-$HOME/.claude}}\"/projects/*/{}.jsonl 2>/dev/null; true",
            shell_quote(session_id)
        ),
    }
}

/// Tells a chat agent what worki's chat view can show, and how to make it fit in.
const CHAT_PROMPT: &str = "You are running in worki's chat view. It renders Markdown and draws \
visuals from these code blocks; use one when it makes something clearer, not as decoration.
- ```mermaid for flowcharts, sequence, state and class diagrams. Keep labels short; worki \
styles the diagram to match its theme.
- ```html for charts, dashboards, comparisons or anything interactive: a self-contained \
fragment (no <html> or <head>) with inline <style> and <script>; scripts may load libraries \
from cdn.jsdelivr.net or cdnjs.cloudflare.com. It runs in a sandboxed frame that already sets \
the font and these CSS variables for worki's light or dark theme: --fg, --muted, --faint, --line, \
--panel, --elevated, --hover, --accent, --added, --removed, --warn. Use them instead of fixed \
colors, keep the background transparent, and use rounded corners and generous spacing.
- ```svg for static drawings: give it a viewBox, rounded shapes, 13-14px system-ui text and a \
light palette (it is shown on white).";

/// Writes commit messages and pull requests when the user hasn't set their own command.
pub const DEFAULT_WRITER_COMMAND: &str =
    "claude --model haiku {fork} --no-session-persistence -p {prompt}";

/// Shell code running the user's writer `template` once without a terminal: `{prompt}` becomes
/// the prompt in `$WORKI_PROMPT`, and `{fork}` becomes `--resume <fork> --fork-session` once
/// Claude has saved that session's conversation, and nothing otherwise.
pub fn writer_command(template: &str, fork: Option<&str>) -> String {
    let prelude = match fork {
        Some(fork) => {
            let fork = shell_quote(fork);
            format!(
                "if {}; then set -- --resume {fork} --fork-session; else set --; fi;",
                claude_saved(&fork)
            )
        }
        None => "set --;".into(),
    };
    let command = template
        .replace("{fork}", "\"$@\"")
        .replace("{prompt}", "\"$WORKI_PROMPT\"");
    format!("{prelude} exec {command}")
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
        Some(codex @ "codex") => {
            // Codex only reads its own login, not the key in the environment: sign the isolated
            // environment in with the stored key once.
            let login = if confined {
                format!(
                    "if [ -n \"$OPENAI_API_KEY\" ] && ! {codex} login status >/dev/null 2>&1; then \
printenv OPENAI_API_KEY | {codex} login --with-api-key >/dev/null; fi; "
                )
            } else {
                String::new()
            };
            let resume = session_id
                .map(|id| {
                    format!(
                        "f=\"{CODEX_SESSIONS}\"/{}; if [ -s \"$f\" ]; then set -- resume \"$(cat \"$f\")\"; else set --; fi; ",
                        shell_quote(id)
                    )
                })
                .unwrap_or_default();
            let notify = shell_quote(&format!(
                "notify=[\"sh\",\"-c\",{},\"worki\"]",
                toml_string(CODEX_NOTIFY)
            ));
            format!(
                "export WORKI_TTY=\"$(tty)\"; {resume}{login}exec {} \"$@\" -c {notify}{prompt}",
                profile.command
            )
        }
        _ => format!("exec {}{prompt}", profile.command),
    }
}

/// Where Codex's conversation id is kept per worki session, so a restored session resumes it.
const CODEX_SESSIONS: &str = "${CODEX_HOME:-$HOME/.codex}/worki-sessions";

/// Codex's `notify` program, called with a JSON payload after each turn and when it waits for an
/// approval: it records the conversation (`thread-id`) for resuming and tells worki the agent
/// needs the user, as a notification since Codex reports no start of work.
const CODEX_NOTIFY: &str = r#"p="$1"
d="${CODEX_HOME:-$HOME/.codex}/worki-sessions"
id=$(printf '%s' "$p" | sed -n 's/.*"thread-id" *: *"\([^"]*\)".*/\1/p')
if [ -n "$id" ] && [ -n "$WORKI_SESSION_ID" ]; then mkdir -p "$d" && printf '%s' "$id" > "$d/$WORKI_SESSION_ID"; fi
case "$p" in
  *approval-requested*) m="Codex needs your approval" ;;
  *plan-mode-prompt*) m="Codex has a plan for you" ;;
  *agent-turn-complete*) m="Codex finished" ;;
  *) exit 0 ;;
esac
[ -n "$WORKI_TTY" ] || exit 0
if [ -n "$TMUX" ]; then printf '\033Ptmux;\033\033]777;notify;Codex;%s\007\033\\' "$m"; else printf '\033]777;notify;Codex;%s\007' "$m"; fi > "$WORKI_TTY" 2>/dev/null
exit 0"#;

/// A TOML basic string.
fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Names Claude's conversation after the worki session, so starting the session again (after
/// worki restarted) resumes it once Claude has saved a transcript for it.
fn claude_conversation(id: &str) -> String {
    let id = shell_quote(id);
    format!(
        "if {}; then set -- --resume {id}; else set -- --session-id {id}; fi;",
        claude_saved(&id)
    )
}

/// Shell test for a saved Claude transcript; `id` is already shell-quoted.
fn claude_saved(id: &str) -> String {
    format!("ls \"${{CLAUDE_CONFIG_DIR:-$HOME/.claude}}\"/projects/*/{id}.jsonl >/dev/null 2>&1")
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
    // The permission mode only reaches hooks; prompt and tool hooks pass it on before working.
    let with_mode = serde_json::json!([{ "hooks": [hook(format!(
        "m=$(sed -n 's/.*\"permission_mode\" *: *\"\\([^\"]*\\)\".*/\\1/p' | head -n 1); [ -n \"$m\" ] && s=\"mode;$m\" && {{ {print}; }}; s=working; {print}"
    ))] }]);
    let mut settings = serde_json::json!({ "hooks": {
        "UserPromptSubmit": with_mode.clone(),
        "PreToolUse": before_tool,
        // Fires as the permission prompt appears; Notification only follows about 6 s later.
        "PermissionRequest": marker("waiting"),
        "PostToolUse": with_mode,
        "Stop": stop,
        "Notification": marker("waiting"),
    }, "statusLine": { "type": "command", "command": STATUS_LINE }});
    if confined {
        settings["sandbox"] = serde_json::json!({ "enabled": false });
    }
    settings.to_string()
}

/// Claude's status line: reports model, effort, context and the like as a `state` marker, then
/// shows the user's own status line from their Claude settings, if they have one. Plain awk, so
/// it runs wherever the agent does.
const STATUS_LINE: &str = r#"in=$(cat); st=$(printf '%s' "$in" | awk 'BEGIN { RS = "\001" }
function grab(re, key,  v) { if (!match($0, re)) return ""; v = substr($0, RSTART, RLENGTH); sub("^.*\"" key "\" *: *", "", v); gsub(/[";\007\033]/, "", v); return v }
{ printf "model=%s;effort=%s;fast=%s;thinking=%s;context=%s;contextSize=%s;cost=%s;style=%s;version=%s",
  grab("\"model\" *: *[{][^}]*\"display_name\" *: *\"[^\"]*\"", "display_name"),
  grab("\"effort\" *: *[{][^}]*\"level\" *: *\"[^\"]*\"", "level"),
  grab("\"fast_mode\" *: *[a-z]+", "fast_mode"),
  grab("\"thinking\" *: *[{][^}]*\"enabled\" *: *[a-z]+", "enabled"),
  grab("\"used_percentage\" *: *[0-9.]+", "used_percentage"),
  grab("\"context_window_size\" *: *[0-9]+", "context_window_size"),
  grab("\"total_cost_usd\" *: *[0-9.]+", "total_cost_usd"),
  grab("\"output_style\" *: *[{][^}]*\"name\" *: *\"[^\"]*\"", "name"),
  grab("\"version\" *: *\"[^\"]*\"", "version") }'); [ -n "$WORKI_TTY" ] && { if [ -n "$TMUX" ]; then printf '\033Ptmux;\033\033]777;worki;state;%s\007\033\\' "$st"; else printf '\033]777;worki;state;%s\007' "$st"; fi; } > "$WORKI_TTY" 2>/dev/null; c=$(awk 'f && /"command" *:/ || /"statusLine"/ && /"command" *:/ { sub(/.*"command" *: *"/, ""); sub(/" *,? *}?,? *$/, ""); gsub(/\\"/, "\""); print; exit } /"statusLine"/ { f = 1 }' "${CLAUDE_CONFIG_DIR:-$HOME/.claude}/settings.json" 2>/dev/null); [ -n "$c" ] && printf '%s' "$in" | sh -c "$c"; true"#;

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
    fn writer_command_forks_a_saved_conversation() {
        let home = tempfile::tempdir().unwrap();
        let run = |template: &str, fork: Option<&str>| {
            let command =
                super::writer_command(template, fork).replacen("exec claude", "printf '%s|'", 1);
            let out = std::process::Command::new("/bin/sh")
                .args(["-c", &command])
                .env("HOME", home.path())
                .env_remove("CLAUDE_CONFIG_DIR")
                .env("WORKI_PROMPT", "say hi")
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap()
        };
        let default = super::DEFAULT_WRITER_COMMAND;
        let fresh = "--model|haiku|--no-session-persistence|-p|say hi|";
        assert_eq!(run(default, None), fresh);
        assert_eq!(run(default, Some("abc")), fresh);
        let project = home.path().join(".claude/projects/-w");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("abc.jsonl"), "").unwrap();
        assert_eq!(
            run(default, Some("abc")),
            "--model|haiku|--resume|abc|--fork-session|--no-session-persistence|-p|say hi|"
        );
        assert_eq!(run("claude -p {prompt}", Some("abc")), "-p|say hi|");
    }

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

    #[test]
    fn prompt_hook_reports_the_permission_mode() {
        let settings: serde_json::Value = serde_json::from_str(&claude_settings(false)).unwrap();
        let command = settings["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", &command.replace("> \"$WORKI_TTY\"", "")])
            .env("WORKI_TTY", "/dev/ttys-test")
            .env_remove("TMUX")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"session_id":"s","permission_mode":"acceptEdits","prompt":"hi"}"#)
            .unwrap();
        assert_eq!(
            child.wait_with_output().unwrap().stdout,
            b"\x1b]777;worki;mode;acceptEdits\x07\x1b]777;worki;working\x07"
        );
    }

    #[test]
    fn status_line_reports_the_state_and_runs_the_users_status_line() {
        let dir = tempfile::tempdir().unwrap();
        let tty = dir.path().join("tty");
        std::fs::write(
            dir.path().join("settings.json"),
            "{\n  \"statusLine\": {\n    \"type\": \"command\",\n    \"command\": \"printf mine\"\n  }\n}\n",
        )
        .unwrap();
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", STATUS_LINE])
            .env("WORKI_TTY", &tty)
            .env("CLAUDE_CONFIG_DIR", dir.path())
            .env_remove("TMUX")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                br#"{"session_id":"s","model":{"id":"claude-opus-5-5","display_name":"Opus 5.5"},"version":"2.1.319","output_style":{"name":"default"},"cost":{"total_cost_usd":1.25,"total_lines_added":3},"context_window":{"total_input_tokens":5,"context_window_size":200000,"used_percentage":34.5,"current_usage":{"input_tokens":1}},"fast_mode":false,"effort":{"level":"high"},"thinking":{"enabled":true}}"#,
            )
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "mine");
        assert_eq!(
            std::fs::read_to_string(&tty).unwrap(),
            "\x1b]777;worki;state;model=Opus 5.5;effort=high;fast=false;thinking=true;context=34.5;contextSize=200000;cost=1.25;style=default;version=2.1.319\x07"
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
    fn continues_the_conversation_where_it_ran() {
        let wt = Path::new("/w/fix login");
        let in_dir = "cd '/w/fix login' && claude --resume 'abc'";
        assert_eq!(continue_command(&Isolation::None, wt, "abc"), in_dir);
        assert_eq!(
            continue_command(
                &Isolation::MacUser {
                    user: "_worki".into()
                },
                wt,
                "abc"
            ),
            format!("sudo -u '_worki' -H /bin/zsh -lc {}", shell_quote(in_dir))
        );
        let sbx = continue_command(
            &Isolation::DockerSandbox {
                image: "claude".into(),
                template: None,
            },
            wt,
            "abc",
        );
        assert!(sbx.starts_with("sbx exec -it -w '/w/fix login' -e IS_SANDBOX=1 worki-fix-login-"));
        assert!(sbx.ends_with(&format!(
            " bash -lc {}",
            shell_quote("claude --resume 'abc'")
        )));
        assert_eq!(
            continue_command(
                &Isolation::Ssh {
                    host: "dev".into(),
                    user: Some("me".into()),
                    port: Some(2222),
                    keep_alive: true,
                },
                wt,
                "abc"
            ),
            format!("ssh -t -p 2222 'me@dev' {}", shell_quote(in_dir))
        );
    }

    #[test]
    fn chat_history_reads_the_saved_transcript() {
        let home = tempfile::tempdir().unwrap();
        let run = || {
            let out = std::process::Command::new("/bin/sh")
                .args(["-c", &chat_history_command(ChatAgent::Claude, "abc")])
                .env("HOME", home.path())
                .env_remove("CLAUDE_CONFIG_DIR")
                .output()
                .unwrap();
            (out.status.success(), String::from_utf8(out.stdout).unwrap())
        };
        assert_eq!(run(), (true, String::new()));
        let project = home.path().join(".claude/projects/-w");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("abc.jsonl"), "{\"type\":\"user\"}\n").unwrap();
        assert_eq!(run(), (true, "{\"type\":\"user\"}\n".into()));
    }

    #[test]
    fn codex_notify_is_valid_toml() {
        let parsed: toml::Table =
            toml::from_str(&format!("v = {}", super::toml_string(super::CODEX_NOTIFY))).unwrap();
        assert_eq!(parsed["v"].as_str(), Some(super::CODEX_NOTIFY));
    }

    #[test]
    fn codex_notify_records_the_conversation_and_tells_worki() {
        let home = tempfile::tempdir().unwrap();
        let tty = home.path().join("tty");
        std::fs::write(&tty, "").unwrap();
        let payload = r#"{"type":"agent-turn-complete","thread-id":"01a1cc36-bd4a-79e2-b9ee-bae4986f7c77","turn-id":"1","cwd":"/w","input-messages":["hi"],"last-assistant-message":"ok"}"#;
        let ok = std::process::Command::new("/bin/sh")
            .args(["-c", super::CODEX_NOTIFY, "worki", payload])
            .env("HOME", home.path())
            .env_remove("CODEX_HOME")
            .env_remove("TMUX")
            .env("WORKI_SESSION_ID", "s1")
            .env("WORKI_TTY", &tty)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        assert_eq!(
            std::fs::read_to_string(home.path().join(".codex/worki-sessions/s1")).unwrap(),
            "01a1cc36-bd4a-79e2-b9ee-bae4986f7c77"
        );
        assert_eq!(
            std::fs::read_to_string(&tty).unwrap(),
            "\x1b]777;notify;Codex;Codex finished\x07"
        );
    }

    #[test]
    fn codex_resumes_its_conversation_and_signs_in_when_isolated() {
        let profile = AgentProfile {
            id: "codex".into(),
            name: "Codex".into(),
            command: "codex".into(),
            install: None,
        };
        let home = tempfile::tempdir().unwrap();
        let log = home.path().join("log");
        let fake = format!(
            "#!/bin/sh\nif [ \"$1 $2\" = \"login status\" ]; then [ -f {0}.login ]; exit $?; fi\n\
if [ \"$1\" = login ]; then cat > {0}.login; echo login >> {0}; exit 0; fi\n\
case \"$1\" in resume) echo \"resume $2\" ;; -c) echo new ;; esac >> {0}\n",
            log.display()
        );
        std::fs::write(home.path().join("codex"), fake).unwrap();
        std::fs::set_permissions(
            home.path().join("codex"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        let run = |confined: bool| {
            let command = super::agent_command(&profile, Some("s1"), confined, None);
            std::process::Command::new("/bin/sh")
                .args(["-c", &command])
                .env("PATH", format!("{}:/usr/bin:/bin", home.path().display()))
                .env("HOME", home.path())
                .env_remove("CODEX_HOME")
                .env("OPENAI_API_KEY", "sk-test")
                .status()
                .unwrap()
                .success()
        };
        assert!(run(false));
        assert!(run(true));
        let sessions = home.path().join(".codex/worki-sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("s1"), "01a1cc36").unwrap();
        assert!(run(true));
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "new\nlogin\nnew\nresume 01a1cc36\n"
        );
        assert_eq!(
            std::fs::read_to_string(log.with_extension("login")).unwrap(),
            "sk-test\n"
        );
    }

    #[test]
    fn chat_runs_claude_in_its_json_mode() {
        let profile = |command: &str| AgentProfile {
            id: "c".into(),
            name: "C".into(),
            command: command.into(),
            install: None,
        };
        assert!(chat_request(&profile("codex"), Path::new("/w"), vec![], "abc", false).is_none());
        let (agent, req) = chat_request(
            &profile("claude --model opus"),
            Path::new("/w"),
            vec![],
            "abc",
            true,
        )
        .unwrap();
        assert_eq!(agent, ChatAgent::Claude);
        assert!(req.stdin && !req.interactive);
        let home = tempfile::tempdir().unwrap();
        let out = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                &req.command
                    .unwrap()
                    .replacen("exec claude", "printf '%s|'", 1),
            ])
            .env("HOME", home.path())
            .env_remove("CLAUDE_CONFIG_DIR")
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "--model|opus|-p|--input-format|stream-json|--output-format|stream-json|--verbose|\
--include-partial-messages|--replay-user-messages|--permission-prompt-tool|stdio|--append-system-prompt|PROMPT|\
--settings|{\"sandbox\":{\"enabled\":false}}|--session-id|abc|"
                .replace("PROMPT", CHAT_PROMPT)
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
