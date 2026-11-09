use std::time::Duration;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;
use worki_core::agents;
use worki_core::{secrets, Isolation, LogLine};
use worki_isolation::host_which;

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
    can_install: bool,
    /// The human's own installation can be reused inside.
    can_share: bool,
    /// worki knows this agent's config layout and the human has some config to copy.
    has_config: bool,
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
                    can_install: caps.install
                        && p.install.as_deref().is_some_and(|s| !s.trim().is_empty()),
                    can_share: caps.share_host_install,
                    has_config: caps.copy_config
                        && kit.is_some_and(|k| !agents::host_config_paths(k).is_empty()),
                    auth: kit.and_then(|k| k.token_secret).map(|secret| AuthStatus {
                        set: secrets::is_set(&scope, secret),
                        secret: secret.to_owned(),
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

/// Copies the human's config for `executable` into every environment of `isolation` and
/// remembers it for environments created later; returns what was copied.
#[tauri::command]
pub async fn copy_agent_config(
    state: State<'_, AppState>,
    isolation: Isolation,
    executable: String,
) -> CmdResult<agents::CopiedConfig> {
    isolation.validate().map_err(err)?;
    let scope = isolation.scope_key();
    let exe = executable.clone();
    let copied = blocking(move || {
        let kit = agents::kit(&exe)
            .ok_or_else(|| anyhow::anyhow!("worki doesn't know {exe}'s config"))?;
        let mut result = agents::CopiedConfig::default();
        for instance in isolation.backend().instances()? {
            result = agents::copy_config(instance.as_ref(), kit)?;
        }
        Ok(result)
    })
    .await?;
    state
        .projects
        .update_agent_settings(|s| s.remember_setup(&scope, &executable, false))
        .map_err(err)?;
    Ok(copied)
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
        .and_then(|k| k.token_secret)
        .ok_or_else(|| {
            format!("worki can't sign in {executable}; add its API key as a secret instead")
        })?;
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
