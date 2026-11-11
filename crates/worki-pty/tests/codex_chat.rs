//! Runs Codex as a chat for real. Needs `codex` on PATH, signed in, and network:
//! `cargo test -p worki-pty --test codex_chat -- --ignored --nocapture`

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use worki_pty::chat::{ChatProtocol, ChatSetting, CodexOptions, CodexProtocol, PermissionAnswer};
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

fn spawn(host: &SessionHost, dir: &std::path::Path, id: &str, protocol: CodexProtocol) {
    host.spawn_chat(ChatSpec {
        id: id.into(),
        worktree_path: dir.into(),
        profile_id: None,
        title: "Codex".into(),
        program: "/bin/zsh".into(),
        args: vec!["-l".into(), "-c".into(), "exec codex app-server".into()],
        env: Vec::new(),
        clear_env: false,
        run_as: None,
        cwd: None,
        protocol: Box::new(protocol),
        history: Vec::new(),
    })
    .unwrap();
}

fn options() -> CodexOptions {
    CodexOptions {
        developer_instructions: None,
        confined: false,
    }
}

#[test]
#[ignore]
fn chats_with_codex() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let host = SessionHost::new(Arc::new(|_: &SessionInfo| {}));
    spawn(
        &host,
        dir.path(),
        &id,
        CodexProtocol::new(id.clone(), options()),
    );
    let (tx, rx) = mpsc::channel();
    host.attach(&id, Box::new(move |b: &[u8]| tx.send(b.to_vec()).is_ok()))
        .unwrap();
    let seen = events(&rx, "options");
    println!(
        "options: {}",
        seen.iter().find(|e| e["type"] == "options").unwrap()
    );

    host.change_setting(&id, &ChatSetting::PermissionMode("untrusted".into()))
        .unwrap();
    host.send_message(
        &id,
        "Run the shell command `printf hi > b.txt`, then reply with just: done",
        &[],
    )
    .unwrap();
    let seen = events(&rx, "permission");
    let request = seen.iter().find(|e| e["type"] == "permission").unwrap();
    println!("permission: {request}");
    assert!(seen.iter().any(|e| e["type"] == "userRead"));
    host.answer_permission(
        &id,
        request["requestId"].as_str().unwrap(),
        &PermissionAnswer::Allow,
    )
    .unwrap();
    let seen = events(&rx, "turnEnded");
    assert!(dir.path().join("b.txt").exists());
    assert!(seen.iter().any(|e| e["type"] == "text"));
    assert!(seen.iter().any(|e| e["type"] == "toolResult"));

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

    let home = std::env::var("CODEX_HOME")
        .unwrap_or_else(|_| format!("{}/.codex", std::env::var("HOME").unwrap()));
    let recorded = format!("{home}/worki-sessions/{id}");
    let thread = std::fs::read_to_string(&recorded).expect("the thread wasn't recorded");
    let mut resumed = CodexProtocol::new(id.clone(), options());
    resumed.history(&thread);
    spawn(&host, dir.path(), &id, resumed);
    let (tx, rx) = mpsc::channel();
    host.attach(&id, Box::new(move |b: &[u8]| tx.send(b.to_vec()).is_ok()))
        .unwrap();
    let seen = events(&rx, "notice");
    assert!(seen
        .iter()
        .any(|e| e["type"] == "user" && e["text"].as_str().unwrap().contains("b.txt")));
    host.send_message(
        &id,
        "Which file did you create? Reply with just its name.",
        &[],
    )
    .unwrap();
    let seen = events(&rx, "turnEnded");
    let last_text = seen.iter().rev().find(|e| e["type"] == "text").unwrap();
    println!("after resuming: {last_text}");
    assert!(last_text["text"].as_str().unwrap().contains("b.txt"));
    host.kill(&id).unwrap();
    std::fs::remove_file(recorded).ok();
}
