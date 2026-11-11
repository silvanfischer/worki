use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Manager, State};
use worki_core::launch::{
    chat_history_command, chat_request, continue_command, session_env, session_request, ChatAgent,
};
use worki_core::{AgentProfile, Host, Isolation, Project, SavedSession};
use worki_isolation::ssh::{Ssh, SshTarget};
use worki_isolation::SessionMeta;
use worki_pty::chat::{Attachment, ChatProtocol, ChatSetting, ClaudeProtocol, PermissionAnswer};
use worki_pty::{ChatSpec, SessionInfo, SessionState, SpawnSpec};

use crate::{blocking, err, AppState, CmdResult};

#[tauri::command]
pub fn list_sessions(state: State<'_, AppState>) -> Vec<SessionInfo> {
    state.sessions.list()
}

/// `profile_id: None` opens a plain shell. Runs inside the project's isolation. `resume_id`
/// reuses a closed session's id, so a Claude session continues its conversation.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn spawn_session(
    state: State<'_, AppState>,
    project_id: String,
    worktree_path: PathBuf,
    profile_id: Option<String>,
    resume_id: Option<String>,
    prompt: Option<String>,
    cols: u16,
    rows: u16,
) -> CmdResult<SessionInfo> {
    if let Some(id) = &resume_id {
        if state.sessions.list().iter().any(|s| &s.id == id) {
            return Err(format!("session {id} is still open"));
        }
    }
    start_session(
        &state,
        project_id,
        worktree_path,
        profile_id,
        resume_id,
        prompt,
        cols,
        rows,
    )
    .await
}

/// Starts a session, under `id` when it is one worki ran before and should resume.
#[allow(clippy::too_many_arguments)]
async fn start_session(
    state: &AppState,
    project_id: String,
    worktree_path: PathBuf,
    profile_id: Option<String>,
    id: Option<String>,
    prompt: Option<String>,
    cols: u16,
    rows: u16,
) -> CmdResult<SessionInfo> {
    let projects = &state.projects;
    let Prepared {
        project,
        profile,
        isolation,
        id,
        title,
        env,
        confined,
    } = prepare(
        state,
        &project_id,
        &worktree_path,
        profile_id.as_deref(),
        id,
    )
    .await?;
    let meta = SessionMeta {
        id: id.clone(),
        title: title.clone(),
        profile_id: profile_id.clone(),
    };
    let req = session_request(
        profile.as_ref(),
        &worktree_path,
        env,
        Some(meta),
        confined,
        prompt.as_deref(),
    );
    let backend_isolation = isolation.clone();
    let launch = blocking(move || backend_isolation.backend().launch(&req)).await?;

    let remote = project.remote.is_some();
    let resumable = matches!(
        isolation,
        Isolation::Ssh {
            keep_alive: true,
            ..
        }
    );
    let saved = match (&profile, &profile_id) {
        (Some(p), Some(profile_id)) if p.executable() == Some("claude") && !resumable => {
            Some(SavedSession {
                id: id.clone(),
                project_id: project.id.clone(),
                worktree_path: worktree_path.clone(),
                profile_id: profile_id.clone(),
                chat: false,
            })
        }
        _ => None,
    };
    let info = state
        .sessions
        .spawn(SpawnSpec {
            id,
            worktree_path,
            profile_id,
            title,
            program: launch.program,
            args: launch.args,
            env: launch.env,
            clear_env: launch.clear_env,
            run_as: (isolation != Isolation::None).then(|| isolation.label()),
            cwd: remote.then_some(launch.cwd),
            resumable,
            cols,
            rows,
        })
        .map_err(err)?;
    if let Some(saved) = saved {
        if let Err(e) = projects.save_session(saved) {
            tracing::warn!("saving session {}: {e:#}", info.id);
        }
    }
    Ok(info)
}

/// What starting any session in a worktree needs: the agent, the isolation (checked, with the
/// agent installed and access granted) and the environment.
struct Prepared {
    project: Project,
    profile: Option<AgentProfile>,
    isolation: Isolation,
    id: String,
    title: String,
    env: Vec<(String, String)>,
    confined: bool,
}

async fn prepare(
    state: &AppState,
    project_id: &str,
    worktree_path: &Path,
    profile_id: Option<&str>,
    id: Option<String>,
) -> CmdResult<Prepared> {
    let projects = &state.projects;
    let project = projects.project(project_id).map_err(err)?;
    let settings = projects.agent_settings().map_err(err)?;
    let profile = profile_id
        .map(|id| {
            settings
                .profile(id)
                .cloned()
                .ok_or_else(|| format!("unknown agent profile {id}"))
        })
        .transpose()?;

    let isolation = projects
        .isolation_for_worktree(&project, worktree_path)
        .map_err(err)?;
    isolation.validate().map_err(err)?;

    if isolation != Isolation::None {
        let check = isolation.clone();
        let executable = profile
            .as_ref()
            .and_then(|p| p.executable())
            .map(str::to_owned);
        let exe = executable.clone();
        let found = tauri::async_runtime::spawn_blocking(move || {
            let backend = check.backend();
            backend.ready()?;
            Ok::<_, anyhow::Error>(exe.map(|e| backend.which(&e).is_some()).unwrap_or(true))
        })
        .await
        .map_err(err)?
        .map_err(err)?;
        if !found {
            let executable = executable.unwrap_or_default();
            let label = isolation.label();
            return Err(if project.remote.is_some() {
                format!(
                    "`{executable}` isn't installed on {label}. Open the project settings and use “Install” under Agents."
                )
            } else {
                format!(
                    "`{executable}` isn't installed for {label}. Open Isolation and use “Use my installation” or “Install” under Agents."
                )
            });
        }
    }

    projects
        .grant_access(&project, Some(worktree_path))
        .await
        .map_err(err)?;

    let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let title = profile
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Shell".into());
    let mut env = session_env(&id, worktree_path);
    env.extend(projects.secret_env(&isolation).map_err(err)?);
    let confined = !matches!(isolation, Isolation::None | Isolation::Ssh { .. });
    Ok(Prepared {
        project,
        profile,
        isolation,
        id,
        title,
        env,
        confined,
    })
}

/// Starts the Claude sessions that were open when worki last quit again, all at once, resuming
/// their conversations.
#[tauri::command]
pub async fn restore_agent_sessions(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<RestoreReport> {
    static RESTORED: AtomicBool = AtomicBool::new(false);
    if RESTORED.swap(true, Ordering::SeqCst) {
        return Ok(RestoreReport::default());
    }
    let saved = state.projects.saved_sessions().map_err(err)?;
    let running = state.sessions.list();
    let mut tasks = Vec::new();
    for s in saved {
        if running.iter().any(|r| r.id == s.id) {
            continue;
        }
        let gone = match state.projects.project(&s.project_id) {
            Ok(p) => p.remote.is_none() && !s.worktree_path.is_dir(),
            Err(_) => true,
        };
        if gone {
            state.projects.forget_session(&s.id).ok();
            continue;
        }
        let app = app.clone();
        tasks.push(tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            let id = s.id.clone();
            let started = if s.chat {
                start_chat(
                    &state,
                    s.project_id,
                    s.worktree_path,
                    s.profile_id,
                    Some(s.id),
                )
                .await
            } else {
                start_session(
                    &state,
                    s.project_id,
                    s.worktree_path,
                    Some(s.profile_id),
                    Some(s.id),
                    None,
                    120,
                    32,
                )
                .await
            };
            started
                .map(drop)
                .inspect_err(|e| tracing::warn!("resuming session {id}: {e}"))
        }));
    }
    let mut report = RestoreReport::default();
    for task in tasks {
        match task.await.map_err(err).and_then(|r| r) {
            Ok(()) => report.started += 1,
            Err(e) => report.failed.push(e),
        }
    }
    Ok(report)
}

#[derive(Default, serde::Serialize)]
pub struct RestoreReport {
    started: usize,
    failed: Vec<String>,
}

/// The kept-alive remote session behind `info`, if it has one.
fn remote_of(state: &AppState, info: &SessionInfo) -> Option<Ssh> {
    if !info.resumable {
        return None;
    }
    match state.projects.host_for_path(&info.worktree_path).ok()? {
        Host::Ssh(target) => Some(Ssh::new(target, true)),
        Host::Local => None,
    }
}

fn session_info(state: &AppState, id: &str) -> CmdResult<SessionInfo> {
    state
        .sessions
        .list()
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("no session {id}"))
}

fn attach_spec(
    host: &Ssh,
    project: &Project,
    info: SessionInfo,
    cols: u16,
    rows: u16,
) -> SpawnSpec {
    let launch = host.attach_launch(&info.id);
    SpawnSpec {
        id: info.id,
        worktree_path: info.worktree_path,
        profile_id: info.profile_id,
        title: info.title,
        program: launch.program,
        args: launch.args,
        env: launch.env,
        clear_env: false,
        run_as: project.remote.as_ref().map(|r| r.target().display()),
        cwd: Some(launch.cwd),
        resumable: true,
        cols,
        rows,
    }
}

/// Attaches again to a kept-alive remote session after its connection ended.
#[tauri::command]
pub async fn reconnect_session(
    state: State<'_, AppState>,
    id: String,
    cols: u16,
    rows: u16,
) -> CmdResult<SessionInfo> {
    let info = session_info(&state, &id)?;
    let host = remote_of(&state, &info).ok_or("this session can't be reconnected")?;
    let project = project_for(&state, &info.worktree_path)?;
    let (host, exists) = tauri::async_runtime::spawn_blocking(move || {
        let exists = host.session_exists(&id);
        (host, exists)
    })
    .await
    .map_err(err)?;
    if !exists {
        return Err("The session has ended on the host.".into());
    }
    state
        .sessions
        .spawn(attach_spec(&host, &project, info, cols, rows))
        .map_err(err)
}

fn project_for(state: &AppState, path: &Path) -> CmdResult<Project> {
    state
        .projects
        .list_projects()
        .map_err(err)?
        .into_iter()
        .find(|p| p.contains(path))
        .ok_or_else(|| format!("no project contains {}", path.display()))
}

/// Picks up sessions still running on remote hosts (after worki restarted or lost the
/// connection) and attaches to them. Returns how many were attached.
#[tauri::command]
pub async fn restore_remote_sessions(state: State<'_, AppState>) -> CmdResult<usize> {
    let projects: Vec<Project> = state
        .projects
        .list_projects()
        .map_err(err)?
        .into_iter()
        .filter(|p| p.remote.as_ref().is_some_and(|r| r.keep_alive))
        .collect();
    let mut hosts: Vec<SshTarget> = projects
        .iter()
        .filter_map(|p| p.remote.as_ref().map(|r| r.target()))
        .collect();
    hosts.sort_by_key(|t| t.display());
    hosts.dedup();

    let mut attached = 0;
    for target in hosts {
        let host_name = target.display();
        let host = Ssh::new(target.clone(), true);
        let (host, found) = match tauri::async_runtime::spawn_blocking(move || {
            let found = host.sessions();
            (host, found)
        })
        .await
        {
            Ok((host, Ok(found))) => (host, found),
            Ok((_, Err(e))) => {
                tracing::warn!("listing sessions on {host_name}: {e:#}");
                continue;
            }
            Err(e) => return Err(err(e)),
        };
        let current = state.sessions.list();
        for remote in found {
            if current
                .iter()
                .any(|s| s.id == remote.id && s.state == SessionState::Running)
            {
                continue;
            }
            let Some(project) = projects.iter().find(|p| {
                p.remote.as_ref().is_some_and(|r| r.target() == target)
                    && p.contains(&remote.worktree)
            }) else {
                continue;
            };
            let info = SessionInfo {
                id: remote.id,
                kind: worki_pty::SessionKind::Terminal,
                worktree_path: remote.worktree,
                profile_id: remote.profile_id,
                title: remote.title,
                run_as: None,
                resumable: true,
                state: SessionState::Running,
                activity: worki_pty::Activity::Idle,
                attention: None,
                usage: Vec::new(),
                agent: Default::default(),
                created_at: 0,
            };
            if state
                .sessions
                .spawn(attach_spec(&host, project, info, 120, 32))
                .is_ok()
            {
                attached += 1;
            }
        }
    }
    Ok(attached)
}

/// Ends the session's remote side too, for kept-alive sessions.
fn end_remote(state: &AppState, id: &str) {
    let Some(host) = session_info(state, id)
        .ok()
        .and_then(|info| remote_of(state, &info))
    else {
        return;
    };
    let id = id.to_owned();
    std::thread::spawn(move || {
        if let Err(e) = host.kill_session(&id) {
            tracing::warn!("ending remote session {id}: {e:#}");
        }
    });
}

/// Starts an agent as a chat instead of in a terminal. Its output (see `attach_session`) is one
/// chat event as JSON per line. `resume_id` continues a closed chat's conversation.
#[tauri::command]
pub async fn spawn_chat_session(
    state: State<'_, AppState>,
    project_id: String,
    worktree_path: PathBuf,
    profile_id: String,
    resume_id: Option<String>,
) -> CmdResult<SessionInfo> {
    if let Some(id) = &resume_id {
        if state.sessions.list().iter().any(|s| &s.id == id) {
            return Err(format!("session {id} is still open"));
        }
    }
    start_chat(&state, project_id, worktree_path, profile_id, resume_id).await
}

/// Starts a chat, under `id` when it continues a conversation worki ran before.
async fn start_chat(
    state: &AppState,
    project_id: String,
    worktree_path: PathBuf,
    profile_id: String,
    id: Option<String>,
) -> CmdResult<SessionInfo> {
    let resuming = id.is_some();
    let Prepared {
        project,
        profile,
        isolation,
        id,
        title,
        env,
        confined,
    } = prepare(state, &project_id, &worktree_path, Some(&profile_id), id).await?;
    let profile = profile.ok_or("pick an agent to chat with")?;
    let (agent, req) =
        chat_request(&profile, &worktree_path, env, &id, confined).ok_or_else(|| {
            format!(
                "{} can't run as a chat yet; start it in a terminal",
                profile.name
            )
        })?;
    let backend_isolation = isolation.clone();
    let launch = blocking(move || backend_isolation.backend().launch(&req)).await?;
    let mut protocol: Box<dyn ChatProtocol> = match agent {
        ChatAgent::Claude => Box::new(ClaudeProtocol::new(id.clone())),
    };
    let history = if resuming {
        let transcript = read_transcript(&isolation, &worktree_path, agent, &id).await;
        protocol.history(&transcript)
    } else {
        Vec::new()
    };
    let info = state
        .sessions
        .spawn_chat(ChatSpec {
            id: id.clone(),
            worktree_path: worktree_path.clone(),
            profile_id: Some(profile_id.clone()),
            title,
            program: launch.program,
            args: launch.args,
            env: launch.env,
            clear_env: launch.clear_env,
            run_as: (isolation != Isolation::None).then(|| isolation.label()),
            cwd: project.remote.is_some().then_some(launch.cwd),
            protocol,
            history,
        })
        .map_err(err)?;
    let saved = SavedSession {
        id,
        project_id: project.id,
        worktree_path,
        profile_id,
        chat: true,
    };
    if let Err(e) = state.projects.save_session(saved) {
        tracing::warn!("saving session {}: {e:#}", info.id);
    }
    Ok(info)
}

/// The command that continues the agent session `id` in the user's own terminal, in the
/// isolation it ran in (see `continue_command`).
#[tauri::command]
pub fn session_continue_command(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    let (worktree, profile_id) = match state.sessions.list().into_iter().find(|s| s.id == id) {
        Some(info) => (info.worktree_path, info.profile_id),
        None => {
            let saved = state.projects.saved_sessions().map_err(err)?;
            let s = saved
                .into_iter()
                .find(|s| s.id == id)
                .ok_or_else(|| format!("no session {id}"))?;
            (s.worktree_path, Some(s.profile_id))
        }
    };
    let projects = &state.projects;
    let profile_id = profile_id.ok_or("a shell has no conversation")?;
    let profile = projects
        .agent_settings()
        .map_err(err)?
        .profile(&profile_id)
        .cloned()
        .ok_or_else(|| format!("unknown agent profile {profile_id}"))?;
    if ChatAgent::of(&profile).is_none() {
        return Err(format!(
            "{} conversations can't be continued outside worki",
            profile.name
        ));
    }
    let project = projects
        .list_projects()
        .map_err(err)?
        .into_iter()
        .find(|p| p.contains(&worktree))
        .ok_or("the session's project is gone")?;
    let isolation = projects
        .isolation_for_worktree(&project, &worktree)
        .map_err(err)?;
    Ok(continue_command(&isolation, &worktree, &id))
}

/// The transcript the agent saved for a conversation, read where the agent runs. Empty when
/// there is none or it can't be read; the chat then starts without its earlier messages.
async fn read_transcript(
    isolation: &Isolation,
    worktree: &Path,
    agent: ChatAgent,
    id: &str,
) -> String {
    let req = worki_isolation::LaunchRequest {
        cwd: worktree.into(),
        command: Some(chat_history_command(agent, id)),
        interactive: false,
        stdin: false,
        env: Vec::new(),
        session: None,
    };
    let isolation = isolation.clone();
    let launch = match blocking(move || isolation.backend().launch(&req)).await {
        Ok(launch) => launch,
        Err(e) => {
            tracing::warn!("reading the transcript of {id}: {e}");
            return String::new();
        }
    };
    worki_core::setup::run_captured(launch)
        .await
        .inspect_err(|e| tracing::warn!("reading the transcript of {id}: {e:#}"))
        .unwrap_or_default()
}

/// Something the user attaches: a file worki reads, or an image pasted from the clipboard.
#[derive(serde::Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AttachmentInput {
    File {
        path: PathBuf,
    },
    Image {
        name: String,
        media_type: String,
        data: String,
    },
}

#[tauri::command]
pub async fn send_chat_message(
    state: State<'_, AppState>,
    id: String,
    text: String,
    attachments: Vec<AttachmentInput>,
) -> CmdResult<()> {
    let attachments = blocking(move || {
        attachments
            .into_iter()
            .map(|a| match a {
                AttachmentInput::File { path } => Attachment::from_path(&path),
                AttachmentInput::Image {
                    name,
                    media_type,
                    data,
                } => Attachment::image(name, media_type, data),
            })
            .collect::<anyhow::Result<Vec<_>>>()
    })
    .await?;
    state
        .sessions
        .send_message(&id, &text, &attachments)
        .map_err(err)
}

#[tauri::command]
pub fn answer_chat_permission(
    state: State<'_, AppState>,
    id: String,
    request_id: String,
    answer: PermissionAnswer,
) -> CmdResult<()> {
    state
        .sessions
        .answer_permission(&id, &request_id, &answer)
        .map_err(err)
}

#[tauri::command]
pub fn change_chat_setting(
    state: State<'_, AppState>,
    id: String,
    setting: ChatSetting,
) -> CmdResult<()> {
    state.sessions.change_setting(&id, &setting).map_err(err)
}

#[tauri::command]
pub fn stop_chat_tasks(
    state: State<'_, AppState>,
    id: String,
    task_ids: Vec<String>,
) -> CmdResult<()> {
    state.sessions.stop_tasks(&id, &task_ids).map_err(err)
}

#[tauri::command]
pub fn background_chat_commands(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.sessions.background(&id).map_err(err)
}

#[tauri::command]
pub fn interrupt_chat(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.sessions.interrupt(&id).map_err(err)
}

/// Streams the session's replay buffer followed by live output as raw bytes.
#[tauri::command]
pub fn attach_session(
    state: State<'_, AppState>,
    id: String,
    on_data: Channel<InvokeResponseBody>,
) -> CmdResult<u64> {
    state
        .sessions
        .attach(
            &id,
            Box::new(move |bytes: &[u8]| {
                on_data
                    .send(InvokeResponseBody::Raw(bytes.to_vec()))
                    .is_ok()
            }),
        )
        .map_err(err)
}

#[tauri::command]
pub fn detach_session(state: State<'_, AppState>, id: String, sink_id: u64) {
    state.sessions.detach(&id, sink_id)
}

#[tauri::command]
pub fn write_session(state: State<'_, AppState>, id: String, data: String) -> CmdResult<()> {
    state.sessions.write(&id, data.as_bytes()).map_err(err)
}

#[tauri::command]
pub fn resize_session(
    state: State<'_, AppState>,
    id: String,
    cols: u16,
    rows: u16,
) -> CmdResult<()> {
    state.sessions.resize(&id, cols, rows).map_err(err)
}

#[tauri::command]
pub fn dismiss_attention(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.sessions.dismiss_attention(&id).map_err(err)
}

#[tauri::command]
pub fn kill_session(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    end_remote(&state, &id);
    state.projects.forget_session(&id).ok();
    state.sessions.kill(&id).map_err(err)
}

#[tauri::command]
pub fn close_session(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    end_remote(&state, &id);
    state.projects.forget_session(&id).ok();
    state.sessions.close(&id).map_err(err)
}
