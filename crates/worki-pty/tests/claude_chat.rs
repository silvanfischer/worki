//! Runs Claude Code as a chat for real. Needs `claude` on PATH, signed in, and network:
//! `cargo test -p worki-pty --test claude_chat -- --ignored --nocapture`

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use worki_pty::chat::{ClaudeProtocol, PermissionAnswer};
use worki_pty::{ChatSpec, SessionHost, SessionInfo};

fn events(rx: &mpsc::Receiver<Vec<u8>>, until: &str) -> Vec<Value> {
    let mut text = String::new();
    loop {
        let chunk = rx
            .recv_timeout(Duration::from_secs(120))
            .unwrap_or_else(|_| panic!("timed out waiting for {until}; got:\n{text}"));
        text.push_str(&String::from_utf8_lossy(&chunk));
        let all: Vec<Value> = text
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        if all.iter().any(|e| e["type"] == until) {
            return all;
        }
    }
}

#[test]
#[ignore]
fn chats_with_claude_code() {
    let dir = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let host = SessionHost::new(Arc::new(|_: &SessionInfo| {}));
    let command =
        "exec claude -p --input-format stream-json --output-format stream-json --verbose \
         --include-partial-messages --permission-prompt-tool stdio --model haiku \
         --settings '{\"sandbox\":{\"enabled\":false}}' --session-id \"$WORKI_SESSION_ID\"";
    host.spawn_chat(ChatSpec {
        id: id.clone(),
        worktree_path: dir.path().into(),
        profile_id: None,
        title: "Claude".into(),
        program: "/bin/zsh".into(),
        args: vec!["-l".into(), "-c".into(), command.into()],
        env: vec![("WORKI_SESSION_ID".into(), id.clone())],
        clear_env: false,
        run_as: None,
        cwd: None,
        protocol: Box::new(ClaudeProtocol::new(id.clone())),
        history: Vec::new(),
    })
    .unwrap();
    let (tx, rx) = mpsc::channel();
    host.attach(&id, Box::new(move |b: &[u8]| tx.send(b.to_vec()).is_ok()))
        .unwrap();

    host.send_message(
        &id,
        "Create a file named a.txt containing hi with the Write tool, then reply with just: done",
        &[],
    )
    .unwrap();
    let seen = events(&rx, "permission");
    let request = seen.iter().find(|e| e["type"] == "permission").unwrap();
    println!("permission: {request}");
    assert_eq!(request["tool"], "Write");
    host.answer_permission(
        &id,
        request["requestId"].as_str().unwrap(),
        &PermissionAnswer::Allow,
    )
    .unwrap();
    let seen = events(&rx, "turnEnded");
    assert!(dir.path().join("a.txt").exists());
    assert!(seen.iter().any(|e| e["type"] == "text"));
    assert!(seen.iter().any(|e| e["type"] == "toolResult"));

    host.send_message(&id, "Use the AskUserQuestion tool to ask me whether I prefer red or blue, then reply with just the color I picked.", &[])
    .unwrap();
    let seen = events(&rx, "permission");
    let request = seen
        .iter()
        .rev()
        .find(|e| e["type"] == "permission")
        .unwrap();
    println!("question: {request}");
    let question = request["questions"][0]["question"]
        .as_str()
        .unwrap()
        .to_owned();
    let answers = BTreeMap::from([(question, "Blue".to_string())]);
    host.answer_permission(
        &id,
        request["requestId"].as_str().unwrap(),
        &PermissionAnswer::Answers { answers },
    )
    .unwrap();
    let seen = events(&rx, "turnEnded");
    let last_text = seen.iter().rev().find(|e| e["type"] == "text").unwrap();
    println!("answer: {last_text}");
    assert!(last_text["text"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("blue"));

    host.send_message(
        &id,
        "Count from 1 to 400, one number per line, nothing else.",
        &[],
    )
    .unwrap();
    events(&rx, "textDelta");
    host.interrupt(&id).unwrap();
    let seen = events(&rx, "turnEnded");
    let ended = seen
        .iter()
        .rev()
        .find(|e| e["type"] == "turnEnded")
        .unwrap();
    println!("interrupted: {ended}");

    host.kill(&id).unwrap();
}
