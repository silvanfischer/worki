use std::process::Stdio;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use worki_isolation::Launch;

use crate::model::LogLine;
use crate::{Error, Result};

/// Runs a non-interactive launch (setup script, installer), streaming each output line to `log`.
pub async fn run_logged(
    launch: Launch,
    log: impl Fn(LogLine) + Send + Sync + Clone + 'static,
) -> Result<()> {
    let mut cmd = Command::new(&launch.program);
    cmd.args(&launch.args).current_dir(&launch.cwd);
    if launch.clear_env {
        cmd.env_clear();
    }
    let mut child = cmd
        .envs(launch.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let out = pump(child.stdout.take(), log.clone(), LogLine::Stdout);
    let err = pump(child.stderr.take(), log, LogLine::Stderr);
    let (status, _, _) = tokio::join!(child.wait(), out, err);
    let status = status?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::CommandFailed(status.code()))
    }
}

/// Runs a non-interactive launch and returns its output. On failure, the error carries the end
/// of what it printed.
pub async fn run_captured(launch: Launch) -> Result<String> {
    let out = Arc::new(Mutex::new(Vec::new()));
    let err = Arc::new(Mutex::new(Vec::new()));
    let (o, e) = (out.clone(), err.clone());
    let result = run_logged(launch, move |line| match line {
        LogLine::Stdout(l) => o.lock().unwrap().push(l),
        LogLine::Stderr(l) => e.lock().unwrap().push(l),
        LogLine::Info(_) => {}
    })
    .await;
    let out = out.lock().unwrap().join("\n");
    match result {
        Ok(()) => Ok(out),
        Err(Error::CommandFailed(code)) => {
            let err = err.lock().unwrap();
            let lines = if err.is_empty() {
                out.lines().map(str::to_owned).collect()
            } else {
                err.clone()
            };
            let tail = lines[lines.len().saturating_sub(5)..].join("\n");
            Err(Error::Isolation(if tail.trim().is_empty() {
                format!("the agent stopped with exit code {}", code.unwrap_or(-1))
            } else {
                tail
            }))
        }
        Err(e) => Err(e),
    }
}

async fn pump<R: AsyncRead + Unpin>(
    reader: Option<R>,
    log: impl Fn(LogLine),
    wrap: fn(String) -> LogLine,
) {
    let Some(reader) = reader else { return };
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        log(wrap(line));
    }
}
