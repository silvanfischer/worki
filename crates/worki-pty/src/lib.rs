mod activity;

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub use activity::{Activity, ModelUsage};
use activity::{Scanner, Signal, Tracker};
use anyhow::{anyhow, Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;

const REPLAY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub id: String,
    pub worktree_path: PathBuf,
    pub profile_id: Option<String>,
    pub title: String,
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Start from an empty environment instead of inheriting the app's.
    pub clear_env: bool,
    /// Who the process runs as, for display; `None` means the current user.
    pub run_as: Option<String>,
    /// Working directory of the local process when it differs from the worktree (e.g. the
    /// worktree is on another machine).
    pub cwd: Option<PathBuf>,
    /// The session keeps running elsewhere when this process ends and can be attached again.
    pub resumable: bool,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionState {
    Running,
    Exited { code: u32 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub worktree_path: PathBuf,
    pub profile_id: Option<String>,
    pub title: String,
    pub run_as: Option<String>,
    pub resumable: bool,
    pub state: SessionState,
    /// What the process is doing, while it runs.
    pub activity: Activity,
    /// Text of the last desktop-notification escape, while the agent waits.
    pub attention: Option<String>,
    /// Tokens the agent's current conversation used, per model; empty until it reports any.
    pub usage: Vec<ModelUsage>,
    pub created_at: u64,
}

/// Receives output bytes; returning `false` detaches it.
pub type Sink = Box<dyn FnMut(&[u8]) -> bool + Send>;
pub type OnChange = Arc<dyn Fn(&SessionInfo) + Send + Sync>;

struct Output {
    replay: VecDeque<u8>,
    sinks: Vec<(u64, Sink)>,
}

struct Session {
    info: Mutex<SessionInfo>,
    tracker: Mutex<Tracker>,
    output: Mutex<Output>,
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

type SessionMap = Mutex<HashMap<String, Arc<Session>>>;

pub struct SessionHost {
    sessions: Arc<SessionMap>,
    next_sink: AtomicU64,
    on_change: OnChange,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl SessionHost {
    pub fn new(on_change: OnChange) -> Self {
        let sessions = Arc::<SessionMap>::default();
        spawn_ticker(Arc::downgrade(&sessions), on_change.clone());
        Self {
            sessions,
            next_sink: AtomicU64::new(0),
            on_change,
        }
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let mut out: Vec<_> = lock(&self.sessions)
            .values()
            .map(|s| lock(&s.info).clone())
            .collect();
        out.sort_by_key(|s| s.created_at);
        out
    }

    pub fn spawn(&self, spec: SpawnSpec) -> Result<SessionInfo> {
        let pair = native_pty_system()
            .openpty(pty_size(spec.cols, spec.rows))
            .context("open pty")?;

        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        cmd.cwd(spec.cwd.as_ref().unwrap_or(&spec.worktree_path));
        if spec.clear_env {
            cmd.env_clear();
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("spawn {}", spec.program))?;
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let info = SessionInfo {
            id: spec.id,
            worktree_path: spec.worktree_path,
            profile_id: spec.profile_id,
            title: spec.title,
            run_as: spec.run_as,
            resumable: spec.resumable,
            state: SessionState::Running,
            activity: Activity::Idle,
            attention: None,
            usage: Vec::new(),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        };
        let session = Arc::new(Session {
            info: Mutex::new(info.clone()),
            tracker: Mutex::new(Tracker::default()),
            output: Mutex::new(Output {
                replay: VecDeque::new(),
                sinks: Vec::new(),
            }),
            writer: Mutex::new(writer),
            master: Mutex::new(pair.master),
            killer: Mutex::new(child.clone_killer()),
        });
        lock(&self.sessions).insert(info.id.clone(), session.clone());

        let reader_session = session.clone();
        let reader_change = self.on_change.clone();
        std::thread::Builder::new()
            .name(format!("pty-read-{}", &info.id[..8]))
            .spawn(move || pump_output(reader, &reader_session, &reader_change))?;

        let on_change = self.on_change.clone();
        let sessions = self.sessions.clone();
        std::thread::Builder::new()
            .name(format!("pty-wait-{}", &info.id[..8]))
            .spawn(move || {
                let code = child.wait().map(|s| s.exit_code()).unwrap_or(1);
                let info = {
                    let mut info = lock(&session.info);
                    info.state = SessionState::Exited { code };
                    info.activity = Activity::Idle;
                    info.attention = None;
                    info.clone()
                };
                if lock(&sessions).contains_key(&info.id) {
                    on_change(&info);
                }
            })?;

        (self.on_change)(&info);
        Ok(info)
    }

    /// Sends the replay buffer to `sink`, then streams live output to it. Both happen under one
    /// lock so no bytes are lost or duplicated between the snapshot and the live stream.
    pub fn attach(&self, id: &str, mut sink: Sink) -> Result<u64> {
        let session = self.get(id)?;
        let sink_id = self.next_sink.fetch_add(1, Ordering::Relaxed) + 1;
        let mut out = lock(&session.output);
        let (a, b) = out.replay.as_slices();
        let mut snapshot = Vec::with_capacity(a.len() + b.len());
        snapshot.extend_from_slice(a);
        snapshot.extend_from_slice(b);
        if snapshot.is_empty() || sink(&snapshot) {
            out.sinks.push((sink_id, sink));
        }
        Ok(sink_id)
    }

    pub fn detach(&self, id: &str, sink_id: u64) {
        if let Ok(session) = self.get(id) {
            lock(&session.output)
                .sinks
                .retain(|(sid, _)| *sid != sink_id);
        }
    }

    pub fn write(&self, id: &str, data: &[u8]) -> Result<()> {
        let session = self.get(id)?;
        {
            let mut writer = lock(&session.writer);
            writer.write_all(data)?;
            writer.flush()?;
        }
        lock(&session.tracker).on_input(Instant::now());
        refresh(&session, &self.on_change);
        Ok(())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        let session = self.get(id)?;
        lock(&session.master).resize(pty_size(cols, rows))?;
        Ok(())
    }

    pub fn kill(&self, id: &str) -> Result<()> {
        let session = self.get(id)?;
        if lock(&session.info).state == SessionState::Running {
            lock(&session.killer).kill().ok();
        }
        Ok(())
    }

    /// Kills the process if needed and forgets the session.
    pub fn close(&self, id: &str) -> Result<()> {
        self.kill(id)?;
        lock(&self.sessions).remove(id);
        Ok(())
    }

    pub fn kill_all(&self) {
        for session in lock(&self.sessions).values() {
            lock(&session.killer).kill().ok();
        }
    }

    fn get(&self, id: &str) -> Result<Arc<Session>> {
        lock(&self.sessions)
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("no session {id}"))
    }
}

fn pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Feeds the PTY's output to the replay buffer, the attached sinks and the activity tracker until
/// the process closes it.
fn pump_output(mut reader: Box<dyn Read + Send>, session: &Session, on_change: &OnChange) {
    let mut buf = vec![0u8; 64 * 1024];
    let mut scanner = Scanner::default();
    let mut signals = Vec::new();
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let chunk = &buf[..n];
        {
            let mut out = lock(&session.output);
            push_replay(&mut out.replay, chunk, REPLAY_BYTES);
            out.sinks.retain_mut(|(_, sink)| sink(chunk));
        }
        signals.clear();
        scanner.feed(chunk, &mut signals);
        lock(&session.tracker).on_output(Instant::now(), &signals);
        if let Some(usage) = signals.iter().rev().find_map(|s| match s {
            Signal::Usage(u) => Some(u.clone()),
            _ => None,
        }) {
            let info = {
                let mut info = lock(&session.info);
                info.usage = usage;
                info.clone()
            };
            on_change(&info);
        }
        refresh(session, on_change);
    }
}

/// Recomputes a running session's activity and reports it when it changed.
fn refresh(session: &Session, on_change: &OnChange) {
    let (activity, attention) = {
        let tracker = lock(&session.tracker);
        (tracker.activity(Instant::now()), tracker.message())
    };
    let info = {
        let mut info = lock(&session.info);
        if info.state != SessionState::Running
            || (info.activity == activity && info.attention == attention)
        {
            return;
        }
        info.activity = activity;
        info.attention = attention;
        info.clone()
    };
    on_change(&info);
}

/// Output-based activity decays over time (working → idle) without new output, so it is
/// re-evaluated periodically. Stops when the host is dropped.
fn spawn_ticker(sessions: Weak<SessionMap>, on_change: OnChange) {
    std::thread::Builder::new()
        .name("pty-activity".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(500));
            let Some(sessions) = sessions.upgrade() else {
                break;
            };
            let all: Vec<Arc<Session>> = lock(&sessions).values().cloned().collect();
            drop(sessions);
            for session in all {
                refresh(&session, &on_change);
            }
        })
        .ok();
}

/// Runs a program in a throwaway PTY (for tools that insist on a terminal), feeding all output to
/// `on_output`. Returns the exit code, or `None` if it was killed after `timeout`.
pub fn run_captured(
    program: &str,
    args: &[String],
    cwd: &Path,
    cols: u16,
    timeout: Duration,
    mut on_output: impl FnMut(&[u8]),
) -> Result<Option<u32>> {
    let pair = native_pty_system().openpty(PtySize {
        rows: 50,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut cmd = CommandBuilder::new(program);
    cmd.args(args);
    cmd.cwd(cwd);
    cmd.env("TERM", "xterm-256color");
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .with_context(|| format!("spawn {program}"))?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader()?;

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(chunk) => on_output(&chunk),
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                child.kill().ok();
                child.wait().ok();
                return Ok(None);
            }
        }
    }
    Ok(Some(child.wait()?.exit_code()))
}

/// Appends to the ring buffer; when trimming, cuts at the next newline so replay doesn't start
/// in the middle of an escape sequence or multi-byte character.
fn push_replay(replay: &mut VecDeque<u8>, data: &[u8], limit: usize) {
    replay.extend(data);
    if replay.len() <= limit {
        return;
    }
    let mut excess = replay.len() - limit;
    if let Some(nl) = replay
        .iter()
        .skip(excess)
        .take(4096)
        .position(|&b| b == b'\n')
    {
        excess += nl + 1;
    }
    replay.drain(..excess);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(program: &str, args: &[&str]) -> SpawnSpec {
        SpawnSpec {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: std::env::temp_dir(),
            profile_id: None,
            title: "test".into(),
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: vec![("WORKI_TEST".into(), "yes".into())],
            clear_env: true,
            run_as: None,
            cwd: None,
            resumable: false,
            cols: 80,
            rows: 24,
        }
    }

    fn collect_until(rx: &mpsc::Receiver<Vec<u8>>, needle: &str) -> String {
        let mut out = String::new();
        while !out.contains(needle) {
            let chunk = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("timed out waiting for output");
            out.push_str(&String::from_utf8_lossy(&chunk));
        }
        out
    }

    fn channel_sink() -> (Sink, mpsc::Receiver<Vec<u8>>) {
        let (tx, rx) = mpsc::channel();
        (Box::new(move |b: &[u8]| tx.send(b.to_vec()).is_ok()), rx)
    }

    #[test]
    fn replays_output_and_reports_exit() {
        let (exit_tx, exit_rx) = mpsc::channel();
        let host = SessionHost::new(Arc::new(move |info: &SessionInfo| {
            if let SessionState::Exited { code } = info.state {
                exit_tx.send(code).ok();
            }
        }));
        let info = host
            .spawn(spec("/bin/sh", &["-c", "echo hello-$WORKI_TEST; exit 3"]))
            .unwrap();
        assert_eq!(exit_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 3);

        let (sink, rx) = channel_sink();
        host.attach(&info.id, sink).unwrap();
        collect_until(&rx, "hello-yes");
        assert_eq!(host.list()[0].state, SessionState::Exited { code: 3 });

        host.close(&info.id).unwrap();
        assert!(host.list().is_empty());
    }

    #[test]
    fn streams_input_and_output() {
        let host = SessionHost::new(Arc::new(|_: &SessionInfo| {}));
        let info = host.spawn(spec("/bin/cat", &[])).unwrap();
        let (sink, rx) = channel_sink();
        let sink_id = host.attach(&info.id, sink).unwrap();
        host.resize(&info.id, 120, 40).unwrap();
        host.write(&info.id, b"ping\n").unwrap();
        collect_until(&rx, "ping");
        host.detach(&info.id, sink_id);
        host.kill(&info.id).unwrap();
    }

    #[test]
    fn captures_output_of_short_program() {
        let mut out = Vec::new();
        let code = run_captured(
            "/bin/sh",
            &["-c".into(), "printf 'tok-123'; exit 4".into()],
            &std::env::temp_dir(),
            200,
            Duration::from_secs(5),
            |b| out.extend_from_slice(b),
        )
        .unwrap();
        assert_eq!(code, Some(4));
        assert!(String::from_utf8_lossy(&out).contains("tok-123"));
    }

    #[test]
    fn reports_activity_changes() {
        let (tx, rx) = mpsc::channel();
        let host = SessionHost::new(Arc::new(move |info: &SessionInfo| {
            tx.send((info.activity, info.attention.clone())).ok();
        }));
        let info = host
            .spawn(spec(
                "/bin/sh",
                &[
                    "-c",
                    "printf 'work'; sleep 2; printf '\\033]9;Ready for review\\007'; sleep 5",
                ],
            ))
            .unwrap();
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            if let Ok(change) = rx.recv_timeout(Duration::from_millis(200)) {
                seen.push(change.clone());
                if change.0 == Activity::Waiting {
                    break;
                }
            }
        }
        assert!(
            seen.iter().any(|(a, _)| *a == Activity::Working),
            "{seen:?}"
        );
        assert!(seen.iter().any(|(a, _)| *a == Activity::Idle), "{seen:?}");
        assert_eq!(
            seen.last(),
            Some(&(Activity::Waiting, Some("Ready for review".into()))),
            "{seen:?}"
        );
        host.write(&info.id, b"ok\n").unwrap();
        let after = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_ne!(after.0, Activity::Waiting);
        host.close(&info.id).unwrap();
    }

    #[test]
    fn trims_replay_at_line_boundary() {
        let mut replay = VecDeque::new();
        push_replay(&mut replay, b"aaaa\nbbbb\ncccc\n", 8);
        assert_eq!(replay.iter().copied().collect::<Vec<_>>(), b"cccc\n");
    }
}
