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
    let remote = match &remote.host_id {
        Some(id) => {
            let mut r = remote.clone();
            state.projects.ssh_host(id).map_err(err)?.apply(&mut r);
            r
        }
        None => remote,
    };
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
pub fn ssh_hosts(state: State<'_, AppState>) -> CmdResult<Vec<SshHost>> {
    state.projects.ssh_hosts().map_err(err)
}

/// Saves a host profile; its stored password moves along when host, user or port change, and
/// `password`, when given, replaces it.
#[tauri::command]
pub fn save_ssh_host(
    state: State<'_, AppState>,
    host: SshHost,
    password: Option<String>,
) -> CmdResult<SshHost> {
    let before = (!host.id.is_empty())
        .then(|| state.projects.ssh_host(&host.id).ok())
        .flatten()
        .map(|h| h.target());
    let saved = state.projects.save_ssh_host(host).map_err(err)?;
    let after = saved.target();
    if let Some(old) = before.filter(|old| *old != after) {
        if ssh::has_password(&old) && !ssh::has_password(&after) {
            let moved = ssh::password(&old);
            ssh::set_password(&after, moved.as_deref()).map_err(err_chain)?;
            ssh::set_password(&old, None).map_err(err_chain)?;
        }
    }
    if let Some(password) = password.as_deref().filter(|p| !p.is_empty()) {
        ssh::set_password(&after, Some(password)).map_err(err_chain)?;
    }
    Ok(saved)
}

#[tauri::command]
pub fn delete_ssh_host(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.projects.delete_ssh_host(&id).map_err(err)
}

#[tauri::command]
pub fn ssh_password_set(host: String, user: Option<String>, port: Option<u16>) -> bool {
    ssh::has_password(&ssh::SshTarget::new(host, user, port))
}

/// Stores the login password for `remote`, or removes it with `None`.
#[tauri::command]
pub fn set_ssh_password(
    host: String,
    user: Option<String>,
    port: Option<u16>,
    password: Option<String>,
) -> CmdResult<()> {
    let target = ssh::SshTarget::new(host, user, port);
    target.validate().map_err(err_chain)?;
    ssh::set_password(&target, password.as_deref()).map_err(err_chain)
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
    base: Option<String>,
) -> CmdResult<String> {
    state
        .projects
        .file_diff(&worktree_path, &file, base.as_deref())
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn environment_templates(
    state: State<'_, AppState>,
    isolation: Isolation,
) -> CmdResult<Vec<String>> {
    state
        .projects
        .environment_templates(isolation)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn save_environment_template(
    state: State<'_, AppState>,
    project_id: String,
    worktree_path: PathBuf,
    tag: String,
    use_for_project: bool,
) -> CmdResult<Project> {
    state
        .projects
        .save_environment_template(&project_id, &worktree_path, &tag, use_for_project)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn commit_changes(
    state: State<'_, AppState>,
    worktree_path: PathBuf,
    files: Vec<String>,
    message: String,
) -> CmdResult<String> {
    state
        .projects
        .commit(&worktree_path, &files, &message)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn branch_log(
    state: State<'_, AppState>,
    worktree_path: PathBuf,
    base: Option<String>,
) -> CmdResult<Vec<CommitInfo>> {
    state
        .projects
        .branch_log(&worktree_path, base.as_deref())
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn update_from_base(
    state: State<'_, AppState>,
    worktree_path: PathBuf,
    base: String,
) -> CmdResult<BaseUpdate> {
    state
        .projects
        .update_from_base(&worktree_path, &base)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn pull_request(
    state: State<'_, AppState>,
    worktree_path: PathBuf,
) -> CmdResult<Option<PullRequest>> {
    state
        .projects
        .pull_request(&worktree_path)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn create_pull_request(
    state: State<'_, AppState>,
    worktree_path: PathBuf,
    base: String,
    title: String,
    body: String,
    draft: bool,
) -> CmdResult<PullRequest> {
    state
        .projects
        .create_pull_request(&worktree_path, &base, &title, &body, draft)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn compare_base(
    state: State<'_, AppState>,
    worktree_path: PathBuf,
    base: String,
) -> CmdResult<String> {
    state
        .projects
        .compare_base(&worktree_path, &base)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn fetch_remote(state: State<'_, AppState>, worktree_path: PathBuf) -> CmdResult<()> {
    state.projects.fetch(&worktree_path).await.map_err(err)
}

#[tauri::command]
pub async fn push_branch(state: State<'_, AppState>, worktree_path: PathBuf) -> CmdResult<()> {
    state.projects.push(&worktree_path).await.map_err(err)
}

#[tauri::command]
pub async fn pr_review_comments(
    state: State<'_, AppState>,
    worktree_path: PathBuf,
    number: u64,
) -> CmdResult<Vec<PrComment>> {
    state
        .projects
        .pr_review_comments(&worktree_path, number)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn branch_changes(
    state: State<'_, AppState>,
    worktree_path: PathBuf,
    base: String,
) -> CmdResult<BranchChanges> {
    state
        .projects
        .branch_changes(&worktree_path, &base)
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
