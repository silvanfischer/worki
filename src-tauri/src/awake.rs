use std::process::{Child, Command};
use std::sync::Mutex;

use crate::CmdResult;

static CAFFEINATE: Mutex<Option<Child>> = Mutex::new(None);

/// Keeps the Mac from idle sleep while on, through `caffeinate`, which also ends with worki.
#[tauri::command]
pub fn keep_awake(on: bool) -> CmdResult<()> {
    let mut held = CAFFEINATE.lock().unwrap_or_else(|e| e.into_inner());
    let running = held
        .as_mut()
        .is_some_and(|child| matches!(child.try_wait(), Ok(None)));
    if on && running {
        return Ok(());
    }
    if let Some(mut child) = held.take() {
        child.kill().ok();
        child.wait().ok();
    }
    if on {
        let child = Command::new("/usr/bin/caffeinate")
            .args(["-i", "-w", &std::process::id().to_string()])
            .spawn()
            .map_err(|e| format!("couldn't start caffeinate: {e}"))?;
        *held = Some(child);
    }
    Ok(())
}
