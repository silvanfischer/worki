//! Running chat sessions in the [`SessionHost`]: the agent process, its history and activity.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};

use super::{Attachment, ChatEvent, ChatProtocol, ChatSetting, PermissionAnswer};
use crate::activity::Tracker;
use crate::{
    lock, refresh, Activity, AgentState, Io, OnChange, Output, Session, SessionHost, SessionInfo,
    SessionKind, SessionMap, SessionState, REPLAY_BYTES,
};

/// Lines of a chat agent's stderr kept to explain why it stopped.
const STDERR_LINES: usize = 20;
/// How long a chat agent gets to finish after its input closes before it's killed.
const STOP_GRACE: Duration = Duration::from_secs(3);

/// A chat session: the agent process and the [`ChatProtocol`] that talks to it.
pub struct ChatSpec {
    pub id: String,
    pub worktree_path: PathBuf,
    pub profile_id: Option<String>,
    pub title: String,
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub clear_env: bool,
    pub run_as: Option<String>,
    pub cwd: Option<PathBuf>,
    pub protocol: Box<dyn ChatProtocol>,
    /// Earlier messages of the conversation it continues, shown before anything new.
    pub history: Vec<ChatEvent>,
}

pub(crate) struct Chat {
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Child>,
    protocol: Mutex<Box<dyn ChatProtocol>>,
    turn: Mutex<Turn>,
    stderr: Mutex<VecDeque<String>>,
}

/// Where a chat's conversation stands, for its activity.
#[derive(Default)]
struct Turn {
    working: bool,
    /// Open permission requests and what they ask, oldest first.
    pending: Vec<(String, String)>,
}

impl Chat {
    fn send(&self, lines: &[String]) -> Result<()> {
        let mut stdin = lock(&self.stdin);
        let stdin = stdin
            .as_mut()
            .ok_or_else(|| anyhow!("the agent has stopped"))?;
        for line in lines {
            stdin.write_all(line.as_bytes())?;
            stdin.write_all(b"\n")?;
        }
        stdin.flush()?;
        Ok(())
    }

    /// What the agent is doing, and what it asks while it waits.
    pub(crate) fn activity(&self) -> (Activity, Option<String>) {
        let turn = lock(&self.turn);
        match turn.pending.first() {
            Some((_, ask)) => (Activity::Waiting, Some(ask.clone())),
            None if turn.working => (Activity::Working, None),
            None => (Activity::Idle, None),
        }
    }

    /// Closes the agent's input so it can finish on its own, and kills it if it hasn't after a
    /// moment. Killing right away would only stop the program worki started, which inside a
    /// container is `docker exec` or `sbx exec`, not the agent.
    pub(crate) fn stop(self: &Arc<Self>) {
        lock(&self.stdin).take();
        let chat = self.clone();
        std::thread::spawn(move || {
            let deadline = Instant::now() + STOP_GRACE;
            while Instant::now() < deadline {
                if !matches!(lock(&chat.child).try_wait(), Ok(None)) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            lock(&chat.child).kill().ok();
        });
    }
}

impl SessionHost {
    /// Starts an agent that worki talks to over its JSON protocol instead of a terminal. Its
    /// output, attached like a terminal's, is one [`ChatEvent`] as JSON per line.
    pub fn spawn_chat(&self, spec: ChatSpec) -> Result<SessionInfo> {
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .current_dir(spec.cwd.as_ref().unwrap_or(&spec.worktree_path))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if spec.clear_env {
            cmd.env_clear();
        }
        cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn {}", spec.program))?;
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            child.kill().ok();
            return Err(anyhow!("couldn't read the agent's output"));
        };
        let stdin = child.stdin.take();

        let info = SessionInfo {
            id: spec.id,
            kind: SessionKind::Chat,
            worktree_path: spec.worktree_path,
            profile_id: spec.profile_id,
            title: spec.title,
            run_as: spec.run_as,
            resumable: false,
            state: SessionState::Running,
            activity: Activity::Idle,
            attention: None,
            usage: Vec::new(),
            agent: AgentState::default(),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        };
        let mut replay = VecDeque::new();
        push_lines(&mut replay, &event_lines(&spec.history), REPLAY_BYTES);
        let chat = Arc::new(Chat {
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            protocol: Mutex::new(spec.protocol),
            turn: Mutex::default(),
            stderr: Mutex::default(),
        });
        let start = lock(&chat.protocol).start();
        if let Err(e) = chat.send(&start) {
            lock(&chat.child).kill().ok();
            return Err(e.context("starting the agent"));
        }
        let session = Arc::new(Session {
            info: Mutex::new(info.clone()),
            tracker: Mutex::new(Tracker::default()),
            output: Mutex::new(Output {
                replay,
                sinks: Vec::new(),
            }),
            io: Io::Chat(chat.clone()),
        });
        lock(&self.sessions).insert(info.id.clone(), session.clone());

        let short = &info.id[..8.min(info.id.len())];
        let (reader_session, on_change) = (session.clone(), self.on_change.clone());
        std::thread::Builder::new()
            .name(format!("chat-read-{short}"))
            .spawn(move || pump_chat(stdout, &reader_session, &on_change))?;

        let stderr_chat = chat.clone();
        std::thread::Builder::new()
            .name(format!("chat-err-{short}"))
            .spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(|l| l.ok()) {
                    let mut lines = lock(&stderr_chat.stderr);
                    lines.push_back(line);
                    if lines.len() > STDERR_LINES {
                        lines.pop_front();
                    }
                }
            })?;

        let (on_change, sessions) = (self.on_change.clone(), self.sessions.clone());
        std::thread::Builder::new()
            .name(format!("chat-wait-{short}"))
            .spawn(move || wait_chat(&session, &sessions, &on_change))?;

        (self.on_change)(&info);
        Ok(info)
    }

    /// Sends the user's message, with anything attached, to a chat session.
    pub fn send_message(&self, id: &str, text: &str, attachments: &[Attachment]) -> Result<()> {
        let session = self.get(id)?;
        let chat = session.io.chat().context("not a chat session")?;
        let message_id = uuid::Uuid::new_v4().to_string();
        let lines = lock(&chat.protocol).message(&message_id, text, attachments);
        emit(
            &session,
            &self.on_change,
            &[ChatEvent::User {
                id: Some(message_id),
                text: text.into(),
                attachments: attachments.iter().map(|a| a.name().to_owned()).collect(),
            }],
        );
        chat.send(&lines)
    }

    /// Stops jobs the chat agent runs in the background; a `BackgroundTasks` event then lists the
    /// jobs left.
    pub fn stop_tasks(&self, id: &str, task_ids: &[String]) -> Result<()> {
        self.send_to_agent(id, |protocol| {
            let mut lines = Vec::new();
            for task_id in task_ids {
                lines.extend(
                    protocol
                        .stop_task(task_id)
                        .ok_or_else(|| anyhow!("this agent can't stop background jobs"))?,
                );
            }
            Ok(lines)
        })
    }

    /// Moves the chat agent's foreground commands to the background; they keep running and
    /// the agent reads its queued messages.
    pub fn background(&self, id: &str) -> Result<()> {
        self.send_to_agent(id, |protocol| {
            protocol
                .background()
                .ok_or_else(|| anyhow!("this agent can't move commands to the background"))
        })
    }

    /// Answers a chat agent's permission request or questions.
    pub fn answer_permission(
        &self,
        id: &str,
        request_id: &str,
        answer: &PermissionAnswer,
    ) -> Result<()> {
        let session = self.get(id)?;
        let chat = session.io.chat().context("not a chat session")?;
        let lines = lock(&chat.protocol)
            .answer(request_id, answer)
            .ok_or_else(|| anyhow!("the agent isn't waiting for {request_id} anymore"))?;
        // Recorded before it's sent, so the agent's reply can't come before it in the history.
        let allowed = !matches!(answer, PermissionAnswer::Deny { .. });
        emit(
            &session,
            &self.on_change,
            &[ChatEvent::PermissionResolved {
                request_id: request_id.into(),
                allowed,
            }],
        );
        chat.send(&lines)
    }

    /// Switches a chat agent's model, permission mode or effort; the agent confirms it with a
    /// `Session` event.
    pub fn change_setting(&self, id: &str, setting: &ChatSetting) -> Result<()> {
        self.send_to_agent(id, |protocol| {
            protocol
                .set(setting)
                .ok_or_else(|| anyhow!("this agent can't change that setting"))
        })
    }

    /// Stops what a chat agent is doing; it waits for the next message afterwards.
    pub fn interrupt(&self, id: &str) -> Result<()> {
        self.send_to_agent(id, |protocol| Ok(protocol.interrupt()))
    }

    /// Sends the lines `request` gets from the protocol of chat session `id` to its agent.
    fn send_to_agent(
        &self,
        id: &str,
        request: impl FnOnce(&mut dyn ChatProtocol) -> Result<Vec<String>>,
    ) -> Result<()> {
        let session = self.get(id)?;
        let chat = session.io.chat().context("not a chat session")?;
        let (lines, events) = {
            let mut protocol = lock(&chat.protocol);
            let lines = request(protocol.as_mut())?;
            (lines, protocol.take_events())
        };
        emit(&session, &self.on_change, &events);
        chat.send(&lines)
    }
}

/// Reads a chat agent's output line by line into events, answering what the protocol replies
/// to and reporting the agent's state.
fn pump_chat(stdout: impl Read, session: &Session, on_change: &OnChange) {
    let chat = session.io.chat().expect("chat session");
    for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
        let (events, replies, state) = {
            let mut protocol = lock(&chat.protocol);
            let events = protocol.parse(&line);
            (events, protocol.take_replies(), protocol.take_state())
        };
        if !replies.is_empty() {
            chat.send(&replies).ok();
        }
        if let Some(update) = state {
            let info = {
                let mut info = lock(&session.info);
                if let Some(agent) = update.agent {
                    info.agent = agent;
                }
                if let Some(usage) = update.usage {
                    info.usage = usage;
                }
                info.clone()
            };
            on_change(&info);
        }
        emit(session, on_change, &events);
    }
}

fn wait_chat(session: &Arc<Session>, sessions: &SessionMap, on_change: &OnChange) {
    let chat = session.io.chat().expect("chat session");
    let code = loop {
        if let Ok(Some(status)) = lock(&chat.child).try_wait() {
            break status.code().map_or(1, |c| c as u32);
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    lock(&chat.stdin).take();
    // Give the stderr reader a moment to collect the last lines.
    std::thread::sleep(Duration::from_millis(100));
    let error = (code != 0).then(|| {
        let lines = lock(&chat.stderr);
        lines.iter().cloned().collect::<Vec<_>>().join("\n")
    });
    emit(session, on_change, &[ChatEvent::Exited { code, error }]);
    let info = {
        let mut info = lock(&session.info);
        info.state = SessionState::Exited { code };
        info.activity = Activity::Idle;
        info.attention = None;
        info.clone()
    };
    if lock(sessions).contains_key(&info.id) {
        on_change(&info);
    }
}

/// Adds chat events to the session's history, sends them to attached sinks and updates its
/// activity.
fn emit(session: &Session, on_change: &OnChange, events: &[ChatEvent]) {
    if events.is_empty() {
        return;
    }
    let chat = session.io.chat().expect("chat session");
    {
        let mut turn = lock(&chat.turn);
        for event in events {
            match event {
                ChatEvent::User { .. } | ChatEvent::UserRead { .. } => turn.working = true,
                ChatEvent::Permission {
                    request_id,
                    tool,
                    questions,
                    ..
                } => {
                    let ask = questions
                        .first()
                        .map(|q| q.question.clone())
                        .unwrap_or_else(|| format!("Allow {tool}?"));
                    turn.pending.push((request_id.clone(), ask));
                }
                ChatEvent::PermissionResolved { request_id, .. } => {
                    turn.pending.retain(|(id, _)| id != request_id);
                }
                ChatEvent::TurnEnded { .. } | ChatEvent::Exited { .. } => {
                    turn.working = false;
                    turn.pending.clear();
                }
                _ => {}
            }
        }
    }
    let bytes = event_lines(events);
    {
        let mut out = lock(&session.output);
        push_lines(&mut out.replay, &bytes, REPLAY_BYTES);
        out.sinks.retain_mut(|(_, sink)| sink(&bytes));
    }
    refresh(session, on_change);
}

/// One JSON line per event, stamped with when it was recorded (`at`, Unix milliseconds), so
/// timers stay right when the history is replayed later.
fn event_lines(events: &[ChatEvent]) -> Vec<u8> {
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let mut bytes = Vec::new();
    for event in events {
        let Ok(mut json) = serde_json::to_value(event) else {
            continue;
        };
        json["at"] = at.into();
        bytes.extend_from_slice(json.to_string().as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

/// Appends to the replay buffer, always cutting at a line end however long the line, since chat
/// history is whole JSON lines (a terminal's replay only looks a little way for one).
fn push_lines(replay: &mut VecDeque<u8>, data: &[u8], limit: usize) {
    replay.extend(data);
    if replay.len() <= limit {
        return;
    }
    let excess = replay.len() - limit;
    let cut = replay
        .iter()
        .skip(excess)
        .position(|&b| b == b'\n')
        .map_or(replay.len(), |nl| excess + nl + 1);
    replay.drain(..cut);
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::Sink;

    fn channel_sink() -> (Sink, mpsc::Receiver<Vec<u8>>) {
        let (tx, rx) = mpsc::channel();
        (Box::new(move |b: &[u8]| tx.send(b.to_vec()).is_ok()), rx)
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

    /// Talks to `cat`: every line sent comes back and is read as the agent's answer.
    struct Echo;

    impl ChatProtocol for Echo {
        fn start(&mut self) -> Vec<String> {
            vec!["ready".into()]
        }
        fn message(&mut self, _: &str, text: &str, _: &[Attachment]) -> Vec<String> {
            vec![format!("say {text}"), "ask".into()]
        }
        fn answer(&mut self, request_id: &str, _: &PermissionAnswer) -> Option<Vec<String>> {
            (request_id == "r1").then(|| vec!["done".into()])
        }
        fn interrupt(&mut self) -> Vec<String> {
            Vec::new()
        }
        fn set(&mut self, _: &ChatSetting) -> Option<Vec<String>> {
            None
        }
        fn parse(&mut self, line: &str) -> Vec<ChatEvent> {
            match line {
                "ask" => vec![ChatEvent::Permission {
                    request_id: "r1".into(),
                    tool: "Write".into(),
                    input: serde_json::Value::Null,
                    questions: Vec::new(),
                }],
                "done" => vec![ChatEvent::TurnEnded {
                    ok: true,
                    interrupted: false,
                    error: None,
                }],
                _ => match line.strip_prefix("say ") {
                    Some(text) => vec![ChatEvent::Text {
                        id: "t".into(),
                        text: text.into(),
                        parent: None,
                    }],
                    None => Vec::new(),
                },
            }
        }
    }

    fn spec(program: &str, args: &[&str]) -> ChatSpec {
        ChatSpec {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: std::env::temp_dir(),
            profile_id: None,
            title: "chat".into(),
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            env: Vec::new(),
            clear_env: false,
            run_as: None,
            cwd: None,
            protocol: Box::new(Echo),
            history: vec![ChatEvent::Notice {
                text: "before".into(),
            }],
        }
    }

    #[test]
    fn runs_a_chat_over_the_agents_json_protocol() {
        let (tx, rx) = mpsc::channel();
        let host = SessionHost::new(Arc::new(move |info: &SessionInfo| {
            tx.send((info.activity, info.attention.clone(), info.state.clone()))
                .ok();
        }));
        let info = host.spawn_chat(spec("/bin/cat", &[])).unwrap();
        assert_eq!(info.kind, SessionKind::Chat);
        assert!(host.write(&info.id, b"x").is_err());

        let wait_for = |want: Activity| loop {
            let (activity, attention, _) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            if activity == want {
                break attention;
            }
        };
        host.send_message(&info.id, "hi", &[]).unwrap();
        assert_eq!(wait_for(Activity::Waiting).as_deref(), Some("Allow Write?"));
        host.answer_permission(&info.id, "r1", &PermissionAnswer::Allow)
            .unwrap();
        assert!(host
            .answer_permission(&info.id, "r9", &PermissionAnswer::Allow)
            .is_err());
        wait_for(Activity::Idle);

        let (sink, lines) = channel_sink();
        host.attach(&info.id, sink).unwrap();
        let history = collect_until(&lines, "turnEnded");
        let types: Vec<String> = history
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["type"].to_string())
            .collect();
        assert_eq!(
            types,
            [
                "\"notice\"",
                "\"user\"",
                "\"text\"",
                "\"permission\"",
                "\"permissionResolved\"",
                "\"turnEnded\""
            ]
        );

        // `cat` ends once its input closes, without being killed.
        host.kill(&info.id).unwrap();
        loop {
            let (_, _, state) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            if let SessionState::Exited { code } = state {
                assert_eq!(code, 0);
                break;
            }
        }
        assert!(collect_until(&lines, "exited").contains("\"code\""));
    }

    #[test]
    fn kills_an_agent_that_ignores_its_closed_input() {
        let (tx, rx) = mpsc::channel();
        let host = SessionHost::new(Arc::new(move |info: &SessionInfo| {
            tx.send(info.state.clone()).ok();
        }));
        let info = host
            .spawn_chat(spec("/bin/sh", &["-c", "exec sleep 60"]))
            .unwrap();
        let started = Instant::now();
        host.kill(&info.id).unwrap();
        loop {
            if let SessionState::Exited { code } = rx.recv_timeout(Duration::from_secs(10)).unwrap()
            {
                assert_ne!(code, 0);
                break;
            }
        }
        assert!(started.elapsed() >= STOP_GRACE);
    }

    #[test]
    fn reports_an_agent_that_fails_to_start() {
        let host = SessionHost::new(Arc::new(|_: &SessionInfo| {}));
        assert!(host.spawn_chat(spec("/nonexistent/agent", &[])).is_err());
        assert!(host.list().is_empty());
    }

    #[test]
    fn keeps_chat_history_in_whole_lines() {
        let mut replay = VecDeque::new();
        push_lines(&mut replay, b"aaaaaaaaaa\nbb\n", 6);
        assert_eq!(replay.iter().copied().collect::<Vec<_>>(), b"bb\n");
        push_lines(&mut replay, b"cccccccccc\n", 6);
        assert!(replay.is_empty());
    }
}
