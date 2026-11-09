mod agents;
mod commands;
mod isolation;
mod openers;
mod sessions;

use std::sync::Arc;

use tauri::{Emitter, Manager, RunEvent};
use worki_core::ProjectService;
use worki_pty::{SessionHost, SessionInfo};

pub struct AppState {
    pub projects: Arc<ProjectService>,
    pub sessions: Arc<SessionHost>,
}

impl AppState {
    fn close_sessions(&self, matches: impl Fn(&SessionInfo) -> bool) {
        for session in self.sessions.list().iter().filter(|s| matches(s)) {
            self.sessions.close(&session.id).ok();
        }
    }
}

type CmdResult<T> = std::result::Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn err_chain(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// Runs blocking work off the async runtime; errors keep their whole context chain.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(err)?
        .map_err(err_chain)
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let projects = ProjectService::open_default()?;
            let handle = app.handle().clone();
            let sessions = SessionHost::new(Arc::new(move |info: &SessionInfo| {
                handle.emit("session-changed", info).ok();
            }));
            app.manage(AppState {
                projects: Arc::new(projects),
                sessions: Arc::new(sessions),
            });

            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                use window_vibrancy::{
                    apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState,
                };
                apply_vibrancy(
                    &window,
                    NSVisualEffectMaterial::Sidebar,
                    Some(NSVisualEffectState::FollowsWindowActiveState),
                    None,
                )
                .ok();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_projects,
            commands::add_project,
            commands::add_remote_project,
            commands::ssh_password_set,
            commands::set_ssh_password,
            commands::update_project,
            commands::remove_project,
            commands::reorder_projects,
            commands::setup_scripts,
            commands::list_worktrees,
            commands::list_branches,
            commands::worktree_status,
            commands::branch_changes,
            commands::ssh_hosts,
            commands::save_ssh_host,
            commands::delete_ssh_host,
            commands::file_diff,
            openers::opener_settings,
            openers::save_opener_settings,
            openers::opener_app_from_path,
            openers::open_in_app,
            openers::app_icon,
            commands::create_worktree,
            commands::remove_worktree,
            commands::agent_settings,
            commands::save_agent_settings,
            isolation::isolation_settings,
            isolation::save_isolation_settings,
            isolation::isolation_doctor,
            isolation::isolation_fix,
            isolation::install_agent,
            isolation::isolation_remove,
            isolation::isolation_build_image,
            isolation::worktree_isolation,
            isolation::set_worktree_isolation,
            agents::agent_statuses,
            agents::share_agent,
            agents::copy_agent_config,
            agents::sign_in_agent,
            isolation::secret_status,
            isolation::set_secret,
            isolation::delete_secret,
            sessions::list_sessions,
            sessions::spawn_session,
            sessions::attach_session,
            sessions::detach_session,
            sessions::write_session,
            sessions::resize_session,
            sessions::kill_session,
            sessions::close_session,
            sessions::reconnect_session,
            sessions::restore_remote_sessions,
        ])
        .build(tauri::generate_context!())
        .expect("error while building worki");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            let state = app.state::<AppState>();
            state.sessions.kill_all();
            // Pause per-worktree environments (Docker sandboxes); the next session resumes them.
            for isolation in state.projects.isolations_in_use().unwrap_or_default() {
                if let Err(e) = isolation.backend().shutdown() {
                    tracing::warn!("stopping {}: {e:#}", isolation.label());
                }
            }
        }
    });
}
