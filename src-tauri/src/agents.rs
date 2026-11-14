use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{Emitter, State};
use worki_core::agents;
use worki_core::{secrets, Isolation, LogLine};
use worki_isolation::{host_which, Backend, LaunchRequest};

use crate::sessions::events;
use crate::{blocking, err, AppState, CmdResult};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    profile_id: String,
    name: String,
    executable: String,
    /// Where the agent resolves inside the isolation.
    env_path: Option<String>,
    /// Where it resolves for the human user.
    host_path: Option<String>,
    /// New environments have it from their image, without installing.
    in_base_image: bool,
    /// The human's own installation can be reused inside.
    can_share: bool,
    auth: Option<AuthStatus>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    /// Env var the sign-in token is passed as.
    secret: String,
    /// A token is stored for this isolation.
    set: bool,
}

/// Per-agent availability inside `isolation`, with what the human has that could be reused.
#[tauri::command]
pub async fn agent_statuses(
    state: State<'_, AppState>,
    isolation: Isolation,
) -> CmdResult<Vec<AgentStatus>> {
    isolation.validate().map_err(err)?;
    let profiles = state.projects.agent_settings().map_err(err)?.profiles;
    let scope = isolation.scope_key();
    blocking(move || {
        let backend = isolation.backend();
        let caps = backend.capabilities();
        let instances = backend.instances().unwrap_or_default();
        // Where the agent resolves: the backend itself (user account, sandbox image), else in
        // every environment instance (each sandbox).
        let env_path = |executable: &str| -> Option<String> {
            if let Some(path) = backend.which(executable) {
                return Some(path);
            }
            match instances.len() {
                0 => None,
                1 => instances[0].which(executable),
                n => instances
                    .iter()
                    .all(|i| i.which(executable).is_some())
                    .then(|| format!("installed in all {n} sandboxes")),
            }
        };
        Ok(profiles
            .into_iter()
            .filter_map(|p| {
                let executable = p.executable()?.to_owned();
                let kit = agents::kit(&executable);
                Some(AgentStatus {
                    env_path: env_path(&executable),
                    host_path: host_which(&executable),
                    in_base_image: isolation.image_ships(&executable),
                    can_share: caps.share_host_install,
                    auth: kit.and_then(|k| k.sign_in).map(|sign_in| AuthStatus {
                        set: secrets::is_set(&scope, sign_in.secret),
                        secret: sign_in.secret.to_owned(),
                    }),
                    profile_id: p.id,
                    name: p.name,
                    executable,
                })
            })
            .collect())
    })
    .await
}

/// Reuses the human's installation of `executable` inside `isolation`.
#[tauri::command]
pub async fn share_agent(isolation: Isolation, executable: String) -> CmdResult<String> {
    isolation.validate().map_err(err)?;
    blocking(move || agents::share_install(isolation.backend().as_ref(), &executable)).await
}

/// Creates a sign-in token for agents without sharing the human's own login: `claude setup-token`
/// (the user approves in the browser) bound to their subscription, stored as a secret for
/// `isolation`. macOS keeps interactive logins only in the Keychain, which agent users lack.
#[tauri::command]
pub async fn sign_in_agent(
    state: State<'_, AppState>,
    isolation: Isolation,
    executable: String,
    on_log: Channel<LogLine>,
) -> CmdResult<()> {
    let secret = agents::kit(&executable)
        .and_then(|k| k.sign_in)
        .map(|s| s.secret)
        .ok_or_else(|| format!("worki can't sign in {executable}"))?;
    let log = move |line: LogLine| {
        on_log.send(line).ok();
    };
    log(LogLine::Info(
        "Running `claude setup-token` — approve the request in your browser.".into(),
    ));
    let value = blocking(move || {
        let claude = host_which("claude")
            .ok_or_else(|| anyhow::anyhow!("`claude` is not installed for you"))?;
        let mut output = String::new();
        let mut pending = String::new();
        let code = worki_pty::run_captured(
            &claude,
            &["setup-token".into()],
            &std::env::temp_dir(),
            1000,
            Duration::from_secs(600),
            |bytes| {
                let text = String::from_utf8_lossy(bytes);
                output.push_str(&text);
                pending.push_str(&agents::strip_ansi(&text));
                while let Some(i) = pending.find('\n') {
                    let line: String = pending.drain(..=i).collect();
                    let line = line.trim();
                    if !line.is_empty() && !line.contains("sk-ant-") {
                        log(LogLine::Stdout(line.to_owned()));
                    }
                }
            },
        )?;
        match (agents::find_claude_token(&output), code) {
            (Some(token), _) => Ok(token),
            (None, None) => anyhow::bail!("Timed out waiting for the browser sign-in."),
            (None, Some(code)) => {
                anyhow::bail!("`claude setup-token` exited ({code}) without printing a token.")
            }
        }
    })
    .await?;

    let scope = isolation.scope_key();
    secrets::set(&scope, secret, &value).map_err(err)?;
    state
        .projects
        .update_agent_settings(|s| s.add_secret(&scope, secret))
        .map_err(err)
}

/// Shown in the worktree while Codex waits for its device-code sign-in.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSignIn {
    worktree_path: PathBuf,
    agent: String,
    url: String,
    code: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SignInEnded {
    worktree_path: PathBuf,
}

/// Signs Codex in where `worktree`'s agents run, when it isn't yet: Codex's device-code sign-in,
/// whose link and code the worktree shows until it's approved in the browser. The login stays in
/// that environment.
pub async fn sign_in_codex(
    state: &AppState,
    instance: Arc<dyn Backend>,
    worktree: &Path,
    agent: &str,
) -> CmdResult<()> {
    let request = |command: &str, interactive: bool| LaunchRequest {
        cwd: worktree.to_owned(),
        command: Some(command.into()),
        interactive,
        stdin: false,
        env: Vec::new(),
        session: None,
    };
    let (status, sign_in) = (
        request("codex login status", false),
        request("codex login --device-auth", true),
    );
    let app = state.app.clone();
    let (worktree_path, agent) = (worktree.to_owned(), agent.to_owned());
    blocking(move || {
        let mut status = instance.launch(&status)?.command();
        if status
            .stdin(std::process::Stdio::null())
            .output()?
            .status
            .success()
        {
            return Ok(());
        }
        let launch = instance.launch(&sign_in)?;
        let mut output = String::new();
        let mut shown = false;
        let code = worki_pty::run_captured(
            &launch.program,
            &launch.args,
            &launch.cwd,
            120,
            Duration::from_secs(15 * 60),
            |bytes| {
                output.push_str(&agents::strip_ansi(&String::from_utf8_lossy(bytes)));
                if let (false, Some((url, code))) = (shown, device_code(&output)) {
                    shown = true;
                    let event = AgentSignIn {
                        worktree_path: worktree_path.clone(),
                        agent: agent.clone(),
                        url,
                        code,
                    };
                    app.emit(events::AGENT_SIGN_IN, event).ok();
                }
            },
        );
        app.emit(events::AGENT_SIGNED_IN, SignInEnded { worktree_path })
            .ok();
        match code? {
            Some(0) => Ok(()),
            None => anyhow::bail!("Codex sign-in expired; start Codex again for a new code."),
            Some(_) => anyhow::bail!("Codex sign-in didn't finish: {}", output.trim()),
        }
    })
    .await
}

/// The link and one-time code in Codex's device-code sign-in output.
fn device_code(output: &str) -> Option<(String, String)> {
    let words: Vec<&str> = output.split_whitespace().collect();
    let url = words.iter().find(|w| w.starts_with("https://"))?;
    let code = words.iter().find(|w| {
        w.split_once('-').is_some_and(|(a, b)| {
            [a, b].iter().all(|part| {
                (4..=6).contains(&part.len())
                    && part
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            })
        })
    })?;
    Some((url.to_string(), code.to_string()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_device_code_from_codex_output() {
        let output = "Follow these steps to sign in with ChatGPT using device code authorization:\n\n\
1. Open this link in your browser and sign in to your account\n   https://auth.openai.com/codex/device\n\n\
2. Enter this one-time code (expires in 15 minutes)\n   CWPX-HIAOL\n";
        assert_eq!(
            super::device_code(output),
            Some((
                "https://auth.openai.com/codex/device".into(),
                "CWPX-HIAOL".into()
            ))
        );
        assert_eq!(super::device_code("1. Open this link"), None);
    }
}
