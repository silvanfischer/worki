mod agents;
mod commands;
mod isolation;
mod openers;
mod sessions;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
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
            sessions::dismiss_attention,
            sessions::close_session,
            sessions::reconnect_session,
            sessions::restore_remote_sessions,
            sessions::restore_agent_sessions,
            commands::save_environment_template,
            commands::environment_templates,
            commands::commit_changes,
            commands::branch_log,
            commands::update_from_base,
            commands::pull_request,
            commands::create_pull_request,
            commands::draft_pull_request,
            commands::push_branch,
            commands::fetch_remote,
            commands::compare_base,
            commands::pr_review_comments,
        ])
        .build(tauri::generate_context!())
        .expect("error while building worki");

    app.run(|app, event| match event {
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" && !STOPPED.load(Ordering::SeqCst) => {
            api.prevent_close();
            quit(app);
        }
        RunEvent::ExitRequested { api, .. } if !STOPPED.load(Ordering::SeqCst) => {
            api.prevent_exit();
            quit(app);
        }
        _ => {}
    });
}

static QUITTING: AtomicBool = AtomicBool::new(false);
static STOPPED: AtomicBool = AtomicBool::new(false);

/// Keeps the window open while sessions end and environments stop, so the UI can show that
/// quitting is in progress, then exits.
fn quit(app: &AppHandle) {
    if QUITTING.swap(true, Ordering::SeqCst) {
        return;
    }
    app.emit("app-quitting", ()).ok();
    let app = app.clone();
    std::thread::spawn(move || {
        stop_everything(&app.state::<AppState>());
        STOPPED.store(true, Ordering::SeqCst);
        app.exit(0);
    });
}

fn stop_everything(state: &AppState) {
    state.sessions.kill_all();
    let stop = state
        .projects
        .isolation_settings()
        .is_ok_and(|s| s.stop_on_quit);
    if stop {
        for isolation in state.projects.isolations_in_use().unwrap_or_default() {
            if let Err(e) = isolation.backend().shutdown() {
                tracing::warn!("stopping {}: {e:#}", isolation.label());
            }
        }
    }
}
