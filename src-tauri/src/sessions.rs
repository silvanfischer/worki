use std::path::{Path, PathBuf};

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;
use worki_core::launch::{session_env, session_request};
use worki_core::{Host, Isolation, Project};
use worki_isolation::ssh::{Ssh, SshTarget};
use worki_isolation::SessionMeta;
use worki_pty::{SessionInfo, SessionState, SpawnSpec};

use crate::{blocking, err, AppState, CmdResult};

#[tauri::command]
pub fn list_sessions(state: State<'_, AppState>) -> Vec<SessionInfo> {
    state.sessions.list()
}

/// `profile_id: None` opens a plain shell. Runs inside the project's isolation.
#[tauri::command]
pub async fn spawn_session(
    state: State<'_, AppState>,
    project_id: String,
    worktree_path: PathBuf,
    profile_id: Option<String>,
    cols: u16,
    rows: u16,
) -> CmdResult<SessionInfo> {
    let projects = &state.projects;
    let project = projects.project(&project_id).map_err(err)?;
    let settings = projects.agent_settings().map_err(err)?;
    let profile = profile_id
        .as_deref()
        .map(|id| {
            settings
                .profile(id)
                .cloned()
                .ok_or_else(|| format!("unknown agent profile {id}"))
        })
        .transpose()?;

    let isolation = projects
        .isolation_for_worktree(&project, &worktree_path)
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
        .grant_access(&project, Some(&worktree_path))
        .await
        .map_err(err)?;

    let id = uuid::Uuid::new_v4().to_string();
    let title = profile
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Shell".into());
    let mut env = session_env(&id, &worktree_path);
    env.extend(projects.secret_env(&isolation).map_err(err)?);
    let meta = SessionMeta {
        id: id.clone(),
        title: title.clone(),
        profile_id: profile_id.clone(),
    };
    let req = session_request(profile.as_ref(), &worktree_path, env, Some(meta));
    let backend_isolation = isolation.clone();
    let launch = blocking(move || backend_isolation.backend().launch(&req)).await?;

    let remote = project.remote.is_some();
    state
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
            resumable: matches!(
                isolation,
                Isolation::Ssh {
                    keep_alive: true,
                    ..
                }
            ),
            cols,
            rows,
        })
        .map_err(err)
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
                worktree_path: remote.worktree,
                profile_id: remote.profile_id,
                title: remote.title,
                run_as: None,
                resumable: true,
                state: SessionState::Running,
                activity: worki_pty::Activity::Idle,
                attention: None,
                usage: Vec::new(),
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
pub fn kill_session(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    end_remote(&state, &id);
    state.sessions.kill(&id).map_err(err)
}

#[tauri::command]
pub fn close_session(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    end_remote(&state, &id);
    state.sessions.close(&id).map_err(err)
}
