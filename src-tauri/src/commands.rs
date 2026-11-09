use std::path::PathBuf;

use tauri::ipc::Channel;
use tauri::State;
use worki_core::service::{CreatedWorktree, SetupScripts};
use worki_core::*;
use worki_isolation::ssh;

use crate::{err, err_chain, AppState, CmdResult};

#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> CmdResult<Vec<Project>> {
    state.projects.list_projects().map_err(err)
}

#[tauri::command]
pub async fn add_project(state: State<'_, AppState>, path: PathBuf) -> CmdResult<Project> {
    state.projects.add_project(&path).await.map_err(err)
}

/// `password`, when given, is stored in the Keychain vault before connecting.
#[tauri::command]
pub async fn add_remote_project(
    state: State<'_, AppState>,
    remote: Remote,
    path: PathBuf,
    password: Option<String>,
) -> CmdResult<Project> {
    remote.target().validate().map_err(err_chain)?;
    if let Some(password) = password.as_deref().filter(|p| !p.is_empty()) {
        ssh::set_password(&remote.target(), Some(password)).map_err(err_chain)?;
    }
    state
        .projects
        .add_remote_project(remote, &path)
        .await
        .map_err(err)
}

/// Keeps a remote project's stored password when its host, user or port change.
#[tauri::command]
pub fn update_project(
    state: State<'_, AppState>,
    id: String,
    patch: ProjectPatch,
) -> CmdResult<Project> {
    let before = state
        .projects
        .project(&id)
        .map_err(err)?
        .remote
        .map(|r| r.target());
    let project = state.projects.update_project(&id, patch).map_err(err)?;
    let after = project.remote.as_ref().map(|r| r.target());
    if let (Some(old), Some(new)) = (before, after) {
        if old != new && ssh::has_password(&old) && !ssh::has_password(&new) {
            let password = ssh::password(&old);
            ssh::set_password(&new, password.as_deref()).map_err(err_chain)?;
            ssh::set_password(&old, None).map_err(err_chain)?;
        }
    }
    Ok(project)
}

#[tauri::command]
pub fn ssh_password_set(remote: Remote) -> bool {
    ssh::has_password(&remote.target())
}

/// Stores the login password for `remote`, or removes it with `None`.
#[tauri::command]
pub fn set_ssh_password(remote: Remote, password: Option<String>) -> CmdResult<()> {
    remote.target().validate().map_err(err_chain)?;
    ssh::set_password(&remote.target(), password.as_deref()).map_err(err_chain)
}

#[tauri::command]
pub async fn remove_project(state: State<'_, AppState>, id: String) -> CmdResult<Option<String>> {
    let project = state.projects.project(&id).map_err(err)?;
    state.close_sessions(|s| project.contains(&s.worktree_path));
    state.projects.remove_project(&id).await.map_err(err)
}

#[tauri::command]
pub fn reorder_projects(state: State<'_, AppState>, ids: Vec<String>) -> CmdResult<()> {
    state.projects.reorder_projects(&ids).map_err(err)
}

#[tauri::command]
pub async fn setup_scripts(state: State<'_, AppState>, id: String) -> CmdResult<SetupScripts> {
    state.projects.setup_scripts(&id).await.map_err(err)
}

#[tauri::command]
pub async fn list_worktrees(
    state: State<'_, AppState>,
    project_id: String,
) -> CmdResult<Vec<Worktree>> {
    state
        .projects
        .list_worktrees(&project_id)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn list_branches(
    state: State<'_, AppState>,
    project_id: String,
) -> CmdResult<Vec<String>> {
    state.projects.list_branches(&project_id).await.map_err(err)
}

#[tauri::command]
pub async fn worktree_status(
    state: State<'_, AppState>,
    path: PathBuf,
) -> CmdResult<WorktreeStatus> {
    state.projects.worktree_status(&path).await.map_err(err)
}

#[tauri::command]
pub async fn file_diff(
    state: State<'_, AppState>,
    worktree_path: PathBuf,
    file: String,
) -> CmdResult<String> {
    state
        .projects
        .file_diff(&worktree_path, &file)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn create_worktree(
    state: State<'_, AppState>,
    project_id: String,
    req: CreateWorktree,
    on_log: Channel<LogLine>,
) -> CmdResult<CreatedWorktree> {
    state
        .projects
        .create_worktree(&project_id, req, move |line| {
            on_log.send(line).ok();
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn remove_worktree(
    state: State<'_, AppState>,
    project_id: String,
    req: RemoveWorktree,
) -> CmdResult<()> {
    state.close_sessions(|s| s.worktree_path == req.path);
    state
        .projects
        .remove_worktree(&project_id, req)
        .await
        .map_err(err)
}

#[tauri::command]
pub fn agent_settings(state: State<'_, AppState>) -> CmdResult<AgentSettings> {
    state.projects.agent_settings().map_err(err)
}

#[tauri::command]
pub fn save_agent_settings(state: State<'_, AppState>, settings: AgentSettings) -> CmdResult<()> {
    state.projects.save_agent_settings(&settings).map_err(err)
}
