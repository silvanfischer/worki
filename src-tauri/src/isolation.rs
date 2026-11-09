use tauri::ipc::Channel;
use tauri::State;
use worki_core::service::ProjectService;
use worki_core::{secrets, Isolation, IsolationSettings, LogLine};
use worki_isolation::{Check, Fix, Workspace};

use crate::{blocking, err, err_chain, AppState, CmdResult};

fn workspace(state: &AppState, project_id: Option<&str>) -> CmdResult<Option<Workspace>> {
    project_id
        .map(|id| {
            state
                .projects
                .project(id)
                .map(|p| ProjectService::workspace(&p, None))
                .map_err(err)
        })
        .transpose()
}

#[tauri::command]
pub fn isolation_settings(state: State<'_, AppState>) -> CmdResult<IsolationSettings> {
    state.projects.isolation_settings().map_err(err)
}

#[tauri::command]
pub fn save_isolation_settings(
    state: State<'_, AppState>,
    settings: IsolationSettings,
) -> CmdResult<()> {
    settings.default.validate().map_err(err)?;
    state
        .projects
        .save_isolation_settings(&settings)
        .map_err(err)
}

/// Health checks for an isolation mechanism, optionally against one project's files.
#[tauri::command]
pub async fn isolation_doctor(
    state: State<'_, AppState>,
    isolation: Isolation,
    project_id: Option<String>,
) -> CmdResult<Vec<Check>> {
    isolation.validate().map_err(err)?;
    let ws = workspace(&state, project_id.as_deref())?;
    tauri::async_runtime::spawn_blocking(move || isolation.backend().doctor(ws.as_ref()))
        .await
        .map_err(err)
}

/// Installs the agent whose executable is `command` inside `isolation`.
#[tauri::command]
pub async fn install_agent(
    state: State<'_, AppState>,
    isolation: Isolation,
    command: String,
    on_log: Channel<LogLine>,
) -> CmdResult<()> {
    isolation.validate().map_err(err)?;
    state
        .projects
        .install_agent(&isolation, &command, move |line| {
            on_log.send(line).ok();
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn isolation_fix(
    state: State<'_, AppState>,
    isolation: Isolation,
    fix: Fix,
    project_id: Option<String>,
) -> CmdResult<()> {
    isolation.validate().map_err(err)?;
    // Without a project, re-confine every project using this isolation; that also removes
    // broader grants made by older versions.
    let workspaces: Vec<Workspace> = match workspace(&state, project_id.as_deref())? {
        Some(ws) => vec![ws],
        None => {
            let projects = state.projects.list_projects().map_err(err)?;
            let mut out = Vec::new();
            for p in projects {
                if state.projects.isolation_for(&p).map_err(err)? == isolation {
                    out.push(ProjectService::workspace(&p, None));
                }
            }
            out
        }
    };
    blocking(move || {
        let backend = isolation.backend();
        match fix {
            Fix::Provision => backend.provision(),
            Fix::Prepare => backend.prepare(),
            Fix::GrantAccess => workspaces
                .iter()
                .try_for_each(|ws| backend.grant_access(ws)),
            Fix::BuildImage => Err(anyhow::anyhow!(
                "images are built with isolation_build_image"
            )),
        }
    })
    .await
}

/// Removes an isolated environment: stops its sessions, undoes its grants and setup, and points
/// settings that used it back to no isolation.
#[tauri::command]
pub async fn isolation_remove(state: State<'_, AppState>, isolation: Isolation) -> CmdResult<()> {
    isolation.validate().map_err(err)?;
    if isolation == Isolation::None {
        return Ok(());
    }
    let label = isolation.label();
    state.close_sessions(|s| s.run_as.as_deref() == Some(label.as_str()));

    let projects = state.projects.list_projects().map_err(err)?;
    let workspaces: Vec<Workspace> = projects
        .iter()
        .map(|p| ProjectService::workspace(p, None))
        .collect();
    let backend_isolation = isolation.clone();
    let (removed, gone) = tauri::async_runtime::spawn_blocking(move || {
        let backend = backend_isolation.backend();
        let removed = backend.remove(&workspaces).map_err(err_chain);
        (removed, !backend.exists())
    })
    .await
    .map_err(err)?;
    // Settings move off this isolation once the account is gone, even if a later step failed.
    if let Err(e) = &removed {
        if !gone {
            return Err(e.clone());
        }
    }

    let mut settings = state.projects.isolation_settings().map_err(err)?;
    if settings.default == isolation {
        settings.default = Isolation::None;
        state
            .projects
            .save_isolation_settings(&settings)
            .map_err(err)?;
    }
    for p in projects {
        if p.isolation.resolve(&Isolation::None) == isolation {
            let patch = worki_core::ProjectPatch {
                isolation: Some(worki_core::ProjectIsolation::Inherit),
                ..Default::default()
            };
            state.projects.update_project(&p.id, patch).map_err(err)?;
        }
    }
    removed
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeIsolation {
    /// The worktree's own choice; `inherit` follows the project default.
    choice: worki_core::ProjectIsolation,
    /// What it runs in after resolving defaults.
    effective: Isolation,
}

#[tauri::command]
pub fn worktree_isolation(
    state: State<'_, AppState>,
    project_id: String,
    worktree_path: std::path::PathBuf,
) -> CmdResult<WorktreeIsolation> {
    let project = state.projects.project(&project_id).map_err(err)?;
    Ok(WorktreeIsolation {
        choice: state
            .projects
            .worktree_isolation_choice(&worktree_path)
            .map_err(err)?,
        effective: state
            .projects
            .isolation_for_worktree(&project, &worktree_path)
            .map_err(err)?,
    })
}

/// Changes a worktree's isolation: its sessions are closed and its old environment removed.
#[tauri::command]
pub async fn set_worktree_isolation(
    state: State<'_, AppState>,
    project_id: String,
    worktree_path: std::path::PathBuf,
    choice: worki_core::ProjectIsolation,
) -> CmdResult<Isolation> {
    state.close_sessions(|s| s.worktree_path == worktree_path);
    state
        .projects
        .set_worktree_isolation(&project_id, &worktree_path, choice)
        .await
        .map_err(err)
}

static BUILDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Builds the isolation's image, streaming the build output. One build at a time.
#[tauri::command]
pub async fn isolation_build_image(
    isolation: Isolation,
    on_log: Channel<LogLine>,
) -> CmdResult<()> {
    use std::sync::atomic::Ordering;
    isolation.validate().map_err(err)?;
    if BUILDING.swap(true, Ordering::SeqCst) {
        return Err("An image is already being built; wait for it to finish.".into());
    }
    let result = blocking(move || {
        isolation.backend().build_image(&mut |line| {
            on_log.send(LogLine::Stdout(line)).ok();
        })
    })
    .await;
    BUILDING.store(false, Ordering::SeqCst);
    result
}

#[tauri::command]
pub fn secret_status(scope: String, names: Vec<String>) -> Vec<(String, bool)> {
    names
        .into_iter()
        .map(|n| (n.clone(), secrets::is_set(&scope, &n)))
        .collect()
}

/// Stores a secret for agents in `scope` and lists it there.
#[tauri::command]
pub fn set_secret(
    state: State<'_, AppState>,
    scope: String,
    name: String,
    value: String,
) -> CmdResult<()> {
    secrets::set(&scope, &name, &value).map_err(err)?;
    state
        .projects
        .update_agent_settings(|s| s.add_secret(&scope, &name))
        .map_err(err)
}

#[tauri::command]
pub fn delete_secret(state: State<'_, AppState>, scope: String, name: String) -> CmdResult<()> {
    secrets::delete(&scope, &name).map_err(err)?;
    state
        .projects
        .update_agent_settings(|s| s.remove_secret(&scope, &name))
        .map_err(err)
}
