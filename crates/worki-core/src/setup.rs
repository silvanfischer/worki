use std::process::Stdio;

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
