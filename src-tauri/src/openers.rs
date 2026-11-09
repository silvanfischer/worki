use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use tauri::State;
use worki_core::openers::{self, OpenerApp, OpenerSettings};
use worki_core::Host;
use worki_isolation::ssh::{shell_quote, SshTarget};

use crate::{err, AppState, CmdResult};

#[tauri::command]
pub fn opener_settings(state: State<'_, AppState>) -> CmdResult<OpenerSettings> {
    state.projects.opener_settings().map_err(err)
}

#[tauri::command]
pub fn save_opener_settings(state: State<'_, AppState>, settings: OpenerSettings) -> CmdResult<()> {
    state.projects.save_opener_settings(&settings).map_err(err)
}

/// An entry for a `.app` bundle the user picked.
#[tauri::command]
pub fn opener_app_from_path(path: PathBuf) -> CmdResult<OpenerApp> {
    if path.extension().is_none_or(|e| e != "app") || !path.is_dir() {
        return Err(format!("{} is not an app", path.display()));
    }
    Ok(openers::app_from_path(&path))
}

/// Opens `target` (a worktree) in the app at `app`, as the human user. Worktrees on another
/// machine open through the app's own SSH support.
#[tauri::command]
pub fn open_in_app(state: State<'_, AppState>, app: PathBuf, target: PathBuf) -> CmdResult<()> {
    if let Host::Ssh(host) = state.projects.host_for_path(&target).map_err(err)? {
        return open_remote(&app, &host, &target);
    }
    let status = Command::new("/usr/bin/open")
        .arg("-a")
        .arg(&app)
        .arg(&target)
        .status()
        .map_err(err)?;
    if !status.success() {
        return Err(format!(
            "could not open {} in {}",
            target.display(),
            openers::app_name(&app)
        ));
    }
    Ok(())
}

fn open_remote(app: &Path, ssh: &SshTarget, target: &Path) -> CmdResult<()> {
    let name = openers::app_name(app);
    let host = ssh.display();
    let host = host.as_str();
    let target_str = target.to_string_lossy();
    let vscode_cli = [
        "Contents/Resources/app/bin/code",
        "Contents/Resources/app/bin/cursor",
    ]
    .iter()
    .map(|p| app.join(p))
    .find(|p| p.is_file());
    let status = if let Some(cli) = vscode_cli {
        Command::new(cli)
            .arg("--remote")
            .arg(format!("ssh-remote+{host}"))
            .arg(target)
            .status()
    } else if app.join("Contents/MacOS/cli").is_file() && name.starts_with("Zed") {
        Command::new(app.join("Contents/MacOS/cli"))
            .arg(format!("ssh://{host}{target_str}"))
            .status()
    } else if matches!(name.as_str(), "Terminal" | "iTerm") {
        let script = write_ssh_script(ssh, &target_str).map_err(err)?;
        Command::new("/usr/bin/open")
            .arg("-a")
            .arg(app)
            .arg(&script)
            .status()
    } else {
        return Err(format!(
            "{name} can't open worktrees on {host}. Use VS Code, Cursor, Zed, Terminal or iTerm for remote projects."
        ));
    };
    if !status.map_err(err)?.success() {
        return Err(format!("could not open {target_str} on {host} in {name}"));
    }
    Ok(())
}

/// A self-deleting `.command` file that opens a login shell in `target` over SSH.
fn write_ssh_script(ssh: &SshTarget, target: &str) -> std::io::Result<PathBuf> {
    let script =
        std::env::temp_dir().join(format!("worki-{}.command", uuid::Uuid::new_v4().simple()));
    let inner = format!(
        "cd {} && exec \"${{SHELL:-/bin/sh}}\" -l",
        shell_quote(target)
    );
    let mut args = String::new();
    if let Some(port) = ssh.port {
        args.push_str(&format!("-p {port} "));
    }
    if let Some(user) = &ssh.user {
        args.push_str(&format!("-l {} ", shell_quote(user)));
    }
    let body = format!(
        "#!/bin/sh\nrm -f \"$0\"\nexec /usr/bin/ssh -t {args}-- {} {}\n",
        shell_quote(&ssh.host),
        shell_quote(&inner)
    );
    std::fs::write(&script, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(script)
}

static ICONS: Mutex<BTreeMap<PathBuf, Option<String>>> = Mutex::new(BTreeMap::new());

/// The app's icon as a PNG data URL, or `None` when it has no `.icns` file (icons only in an
/// asset catalog). Converted once with `sips` and cached for the process.
#[tauri::command]
pub async fn app_icon(app: PathBuf) -> Option<String> {
    if let Some(cached) = ICONS.lock().ok()?.get(&app) {
        return cached.clone();
    }
    let icon = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        move || render_icon(&app)
    })
    .await
    .ok()
    .flatten();
    ICONS.lock().ok()?.insert(app, icon.clone());
    icon
}

fn render_icon(app: &Path) -> Option<String> {
    let out = Command::new("/usr/bin/defaults")
        .arg("read")
        .arg(app.join("Contents/Info"))
        .arg("CFBundleIconFile")
        .output()
        .ok()?;
    let mut name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if name.is_empty() {
        return None;
    }
    if !name.ends_with(".icns") {
        name.push_str(".icns");
    }
    let icns = app.join("Contents/Resources").join(name);
    let png =
        std::env::temp_dir().join(format!("worki-icon-{}.png", uuid::Uuid::new_v4().simple()));
    let ok = Command::new("/usr/bin/sips")
        .args(["-s", "format", "png", "-Z", "64"])
        .arg(&icns)
        .arg("--out")
        .arg(&png)
        .output()
        .is_ok_and(|o| o.status.success());
    let bytes = ok.then(|| std::fs::read(&png).ok()).flatten();
    std::fs::remove_file(&png).ok();
    Some(format!("data:image/png;base64,{}", base64(&bytes?)))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
