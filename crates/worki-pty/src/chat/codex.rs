//! Codex's app-server protocol (`codex app-server`): JSON-RPC 2.0 without the `jsonrpc` field,
//! one message per line both ways. worki starts or resumes a thread, runs each message as a turn
//! and answers Codex's approval requests.

use std::collections::HashMap;

use base64::Engine;
use serde_json::{json, Value};

use super::{
    Attachment, ChatEvent, ChatProtocol, ChatSetting, ModelChoice, PermissionAnswer, Question,
    QuestionOption, StateUpdate,
};
use crate::{AgentState, ModelUsage};

/// Codex's approval policies worki offers, as permission modes.
const APPROVAL_POLICIES: [&str; 3] = ["untrusted", "on-request", "never"];

/// Where Codex's thread for each worki session is recorded, under `CODEX_HOME`; shared with
/// Codex running in a terminal.
const SESSIONS_DIR: &str = "worki-sessions";

pub struct CodexOptions {
    pub developer_instructions: Option<String>,
    /// worki already confines the agent, so Codex's own sandbox would only get in the way.
    pub confined: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Asked {
    Initialize,
    Thread,
    Models,
    Turn,
    Other,
}

pub struct CodexProtocol {
    session_id: String,
    options: CodexOptions,
    thread: Option<String>,
    resuming: bool,
    codex_home: Option<String>,
    next_request: u64,
    asked: HashMap<u64, Asked>,
    turn: Option<String>,
    /// Messages sent before the thread exists, sent once it does.
    waiting: Vec<(String, Vec<Value>)>,
    /// Codex's open requests, by the id worki shows, with what answers them.
    open: HashMap<String, OpenRequest>,
    /// File changes Codex announced, by item id, for approvals that only name the item.
    file_changes: HashMap<String, Value>,
    model: Option<String>,
    effort: Option<String>,
    approval_policy: Option<String>,
    model_names: HashMap<String, String>,
    replies: Vec<String>,
    events: Vec<ChatEvent>,
    state: AgentState,
    usage: Option<Vec<ModelUsage>>,
    changed: bool,
}

struct OpenRequest {
    rpc_id: Value,
    kind: RequestKind,
}

enum RequestKind {
    Approval,
    Permissions(Value),
    /// Question ids by question text.
    Questions(HashMap<String, String>),
}

impl CodexProtocol {
    pub fn new(session_id: impl Into<String>, options: CodexOptions) -> Self {
        Self {
            session_id: session_id.into(),
            options,
            thread: None,
            resuming: false,
            codex_home: None,
            next_request: 0,
            asked: HashMap::new(),
            turn: None,
            waiting: Vec::new(),
            open: HashMap::new(),
            file_changes: HashMap::new(),
            model: None,
            effort: None,
            approval_policy: None,
            model_names: HashMap::new(),
            replies: Vec::new(),
            events: Vec::new(),
            state: AgentState::default(),
            usage: None,
            changed: false,
        }
    }

    fn request(&mut self, kind: Asked, method: &str, params: Value) -> String {
        self.next_request += 1;
        self.asked.insert(self.next_request, kind);
        json!({ "method": method, "id": self.next_request, "params": params }).to_string()
    }

    fn sandbox(&self) -> &'static str {
        if self.options.confined {
            "danger-full-access"
        } else {
            "workspace-write"
        }
    }

    fn session(&self) -> ChatEvent {
        ChatEvent::Session {
            model: self.model.clone(),
            permission_mode: self.approval_policy.clone(),
            effort: self.effort.clone(),
        }
    }

    fn update_state(&mut self) {
        self.state.model = self
            .model
            .as_ref()
            .map(|m| self.model_names.get(m).cloned().unwrap_or(m.clone()));
        self.state.effort = self.effort.clone();
        self.state.permission_mode = self.approval_policy.clone();
        self.changed = true;
    }

    fn turn_start(&mut self, id: &str, input: Vec<Value>) -> String {
        let thread = self.thread.clone().unwrap_or_default();
        let mut params = json!({
            "threadId": thread,
            "clientUserMessageId": id,
            "input": input,
        });
        if let Some(model) = &self.model {
            params["model"] = json!(model);
        }
        if let Some(effort) = &self.effort {
            params["effort"] = json!(effort);
        }
        if let Some(policy) = &self.approval_policy {
            params["approvalPolicy"] = json!(policy);
        }
        self.request(Asked::Turn, "turn/start", params)
    }

    fn response(&mut self, id: u64, msg: &Value) -> Vec<ChatEvent> {
        let Some(kind) = self.asked.remove(&id) else {
            return Vec::new();
        };
        let result = &msg["result"];
        if let Some(error) = msg["error"]["message"].as_str() {
            return match kind {
                Asked::Thread if self.resuming => {
                    // The thread is gone (or never saved): start a new one.
                    self.resuming = false;
                    self.thread = None;
                    let start = self.thread_start();
                    self.replies.push(start);
                    vec![ChatEvent::Notice {
                        text: format!("Couldn't continue the earlier conversation: {error}"),
                    }]
                }
                Asked::Turn => {
                    vec![ChatEvent::TurnEnded {
                        ok: false,
                        interrupted: false,
                        error: Some(error.to_owned()),
                    }]
                }
                Asked::Thread => vec![ChatEvent::Notice {
                    text: format!("Codex couldn't start the conversation: {error}"),
                }],
                Asked::Initialize | Asked::Models | Asked::Other => Vec::new(),
            };
        }
        match kind {
            Asked::Initialize => {
                self.codex_home = result["codexHome"].as_str().map(str::to_owned);
                self.state.version = result["userAgent"]
                    .as_str()
                    .and_then(|ua| ua.split_whitespace().next())
                    .and_then(|first| first.split('/').nth(1))
                    .map(str::to_owned);
                self.remember_thread();
                Vec::new()
            }
            Asked::Thread => {
                let thread = &result["thread"];
                self.thread = thread["id"].as_str().map(str::to_owned);
                let given = |key: &str| result[key].as_str().map(str::to_owned);
                self.model = self.model.take().or_else(|| given("model"));
                self.effort = self.effort.take().or_else(|| given("reasoningEffort"));
                self.approval_policy = self
                    .approval_policy
                    .take()
                    .or_else(|| given("approvalPolicy"));
                self.update_state();
                self.remember_thread();
                let mut events = Vec::new();
                if std::mem::take(&mut self.resuming) {
                    events.extend(past_turns(&thread["turns"]));
                    if !events.is_empty() {
                        events.push(ChatEvent::Notice {
                            text: "Continued from an earlier session".into(),
                        });
                        // History isn't a turn the agent is working on.
                        events.push(ChatEvent::TurnEnded {
                            ok: true,
                            interrupted: false,
                            error: None,
                        });
                    }
                }
                events.push(self.session());
                for (id, input) in std::mem::take(&mut self.waiting) {
                    let line = self.turn_start(&id, input);
                    self.replies.push(line);
                }
                events
            }
            Asked::Models => {
                let models: Vec<ModelChoice> = result["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|m| m["hidden"] != true)
                    .map(|m| ModelChoice {
                        value: m["id"].as_str().unwrap_or_default().to_owned(),
                        model: m["model"].as_str().map(str::to_owned),
                        label: m["displayName"].as_str().unwrap_or_default().to_owned(),
                        description: m["description"]
                            .as_str()
                            .filter(|d| !d.is_empty())
                            .map(str::to_owned),
                        effort_levels: m["supportedReasoningEfforts"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|e| e["reasoningEffort"].as_str().map(str::to_owned))
                            .collect(),
                    })
                    .filter(|m| !m.value.is_empty())
                    .collect();
                for m in &models {
                    self.model_names.insert(m.value.clone(), m.label.clone());
                }
                self.update_state();
                vec![ChatEvent::Options {
                    models,
                    permission_modes: APPROVAL_POLICIES.map(String::from).to_vec(),
                }]
            }
            Asked::Turn => {
                if let Some(turn) = result["turn"]["id"].as_str() {
                    self.turn = Some(turn.to_owned());
                }
                Vec::new()
            }
            Asked::Other => Vec::new(),
        }
    }

    fn thread_start(&mut self) -> String {
        let mut params = json!({ "sandbox": self.sandbox() });
        if let Some(instructions) = &self.options.developer_instructions {
            params["developerInstructions"] = json!(instructions);
        }
        self.request(Asked::Thread, "thread/start", params)
    }

    /// Records the session's thread where Codex keeps its files, so worki resumes it next time.
    fn remember_thread(&mut self) {
        let (Some(home), Some(thread)) = (self.codex_home.clone(), self.thread.clone()) else {
            return;
        };
        let dir = format!("{}/{SESSIONS_DIR}", home.trim_end_matches('/'));
        let mkdir = self.request(
            Asked::Other,
            "fs/createDirectory",
            json!({ "path": dir, "recursive": true }),
        );
        let write = self.request(
            Asked::Other,
            "fs/writeFile",
            json!({
                "path": format!("{dir}/{}", self.session_id),
                "dataBase64": base64::engine::general_purpose::STANDARD.encode(thread),
            }),
        );
        self.replies.extend([mkdir, write]);
    }

    fn notification(&mut self, method: &str, params: &Value) -> Vec<ChatEvent> {
        match method {
            "turn/started" => {
                self.turn = params["turn"]["id"].as_str().map(str::to_owned);
                vec![ChatEvent::Status {
                    status: "requesting".into(),
                    tool: None,
                }]
            }
            "turn/completed" => {
                self.turn = None;
                let turn = &params["turn"];
                let status = turn["status"].as_str().unwrap_or_default();
                let interrupted = status == "interrupted";
                let ok = status == "completed";
                let error = (!ok && !interrupted).then(|| {
                    turn["error"]["message"]
                        .as_str()
                        .unwrap_or("The agent stopped with an error")
                        .to_owned()
                });
                vec![ChatEvent::TurnEnded {
                    ok,
                    interrupted,
                    error,
                }]
            }
            "item/started" => self.item_started(&params["item"]),
            "item/completed" => item_events(&params["item"], true),
            "item/agentMessage/delta" => vec![ChatEvent::TextDelta {
                id: params["itemId"].as_str().unwrap_or_default().to_owned(),
                text: params["delta"].as_str().unwrap_or_default().to_owned(),
                parent: None,
            }],
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                vec![ChatEvent::Status {
                    status: "thinking".into(),
                    tool: None,
                }]
            }
            "serverRequest/resolved" => {
                let id = rpc_key(&params["requestId"]);
                match self.open.remove(&id) {
                    Some(_) => vec![ChatEvent::PermissionResolved {
                        request_id: id,
                        allowed: false,
                        elsewhere: false,
                    }],
                    None => Vec::new(),
                }
            }
            "thread/tokenUsage/updated" => {
                self.token_usage(&params["tokenUsage"]);
                Vec::new()
            }
            "error" if params["willRetry"] == true => vec![ChatEvent::Notice {
                text: format!(
                    "{} Retrying…",
                    params["error"]["message"]
                        .as_str()
                        .unwrap_or("Codex hit an error.")
                ),
            }],
            "model/rerouted" => {
                self.model = params["toModel"]
                    .as_str()
                    .or(params["model"].as_str())
                    .map(str::to_owned)
                    .or(self.model.take());
                self.update_state();
                vec![self.session()]
            }
            _ => Vec::new(),
        }
    }

    fn item_started(&mut self, item: &Value) -> Vec<ChatEvent> {
        let id = item["id"].as_str().unwrap_or_default().to_owned();
        match item["type"].as_str() {
            Some("userMessage") => match item["clientId"].as_str() {
                Some(client) => vec![ChatEvent::UserRead {
                    id: client.to_owned(),
                }],
                None => Vec::new(),
            },
            Some("agentMessage") => vec![ChatEvent::Status {
                status: "writing".into(),
                tool: None,
            }],
            Some("reasoning") => vec![ChatEvent::Status {
                status: "thinking".into(),
                tool: None,
            }],
            Some("fileChange") => {
                self.file_changes.insert(id, item.clone());
                item_events(item, false)
            }
            _ => item_events(item, false),
        }
    }

    fn token_usage(&mut self, usage: &Value) {
        let total = &usage["total"];
        let count = |u: &Value, key: &str| u[key].as_u64().unwrap_or(0);
        if let Some(model) = self.model.clone() {
            let cached = count(total, "cachedInputTokens");
            self.usage = Some(vec![ModelUsage {
                model,
                input: count(total, "inputTokens").saturating_sub(cached),
                output: count(total, "outputTokens"),
                cache_write: count(total, "cacheWriteInputTokens"),
                cache_write_1h: 0,
                cache_read: cached,
            }]);
        }
        if let Some(window) = usage["modelContextWindow"].as_u64().filter(|w| *w > 0) {
            let used = count(&usage["last"], "inputTokens");
            self.state.context_size = Some(window);
            self.state.context_used = Some((used as f64 / window as f64 * 1000.0).round() / 10.0);
        }
        self.changed = true;
    }

    fn server_request(&mut self, msg: &Value) -> Vec<ChatEvent> {
        let rpc_id = msg["id"].clone();
        let request_id = rpc_key(&rpc_id);
        let params = &msg["params"];
        let (tool, input, kind, questions) = match msg["method"].as_str() {
            Some("item/commandExecution/requestApproval") => (
                "Bash".to_owned(),
                json!({
                    "command": command_text(params),
                    "description": params["reason"],
                }),
                RequestKind::Approval,
                Vec::new(),
            ),
            Some("item/fileChange/requestApproval") => {
                let item = params["itemId"]
                    .as_str()
                    .and_then(|id| self.file_changes.get(id))
                    .cloned()
                    .unwrap_or_default();
                (
                    "Edit".to_owned(),
                    json!({
                        "file_path": item["changes"][0]["path"],
                        "diff": item["changes"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|c| c["diff"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        "description": params["reason"],
                    }),
                    RequestKind::Approval,
                    Vec::new(),
                )
            }
            Some("item/permissions/requestApproval") => (
                "Permissions".to_owned(),
                json!({ "reason": params["reason"], "permissions": params["permissions"] }),
                RequestKind::Permissions(params["permissions"].clone()),
                Vec::new(),
            ),
            Some("item/tool/requestUserInput") => {
                let asked = params["questions"].as_array().cloned().unwrap_or_default();
                let ids = asked
                    .iter()
                    .map(|q| {
                        (
                            q["question"].as_str().unwrap_or_default().to_owned(),
                            q["id"].as_str().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect();
                let questions = asked
                    .iter()
                    .map(|q| Question {
                        question: q["question"].as_str().unwrap_or_default().to_owned(),
                        header: q["header"].as_str().map(str::to_owned),
                        options: q["options"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|o| QuestionOption {
                                label: o["label"].as_str().unwrap_or_default().to_owned(),
                                description: o["description"].as_str().map(str::to_owned),
                            })
                            .collect(),
                        multi_select: false,
                    })
                    .collect();
                (
                    "AskUserQuestion".to_owned(),
                    params.clone(),
                    RequestKind::Questions(ids),
                    questions,
                )
            }
            _ => {
                self.replies.push(
                    json!({
                        "id": rpc_id,
                        "error": { "code": -32601, "message": "worki doesn't handle this request" },
                    })
                    .to_string(),
                );
                return Vec::new();
            }
        };
        self.open
            .insert(request_id.clone(), OpenRequest { rpc_id, kind });
        vec![ChatEvent::Permission {
            request_id,
            tool,
            input,
            questions,
        }]
    }
}

impl ChatProtocol for CodexProtocol {
    fn start(&mut self) -> Vec<String> {
        let initialize = self.request(
            Asked::Initialize,
            "initialize",
            json!({
                "clientInfo": { "name": "worki", "title": "worki", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": null,
            }),
        );
        let initialized = json!({ "method": "initialized" }).to_string();
        let thread = match self.thread.clone() {
            Some(thread) => {
                self.resuming = true;
                let mut params = json!({ "threadId": thread, "sandbox": self.sandbox() });
                if let Some(instructions) = &self.options.developer_instructions {
                    params["developerInstructions"] = json!(instructions);
                }
                self.request(Asked::Thread, "thread/resume", params)
            }
            None => self.thread_start(),
        };
        let models = self.request(Asked::Models, "model/list", json!({}));
        vec![initialize, initialized, thread, models]
    }

    fn message(&mut self, id: &str, text: &str, attachments: &[Attachment]) -> Vec<String> {
        let mut input: Vec<Value> = attachments
            .iter()
            .map(|a| match a {
                Attachment::Image {
                    media_type, data, ..
                } => json!({ "type": "image", "url": format!("data:{media_type};base64,{data}") }),
                Attachment::Text { name, text } => json!({
                    "type": "text",
                    "text": format!("<attached-file name=\"{name}\">\n{text}\n</attached-file>"),
                    "text_elements": [],
                }),
            })
            .collect();
        if !text.trim().is_empty() {
            input.push(json!({ "type": "text", "text": text, "text_elements": [] }));
        }
        if self.thread.is_none() {
            self.waiting.push((id.to_owned(), input));
            return Vec::new();
        }
        match self.turn.clone() {
            Some(turn) => {
                let params = json!({
                    "threadId": self.thread,
                    "clientUserMessageId": id,
                    "input": input,
                    "expectedTurnId": turn,
                });
                vec![self.request(Asked::Other, "turn/steer", params)]
            }
            None => vec![self.turn_start(id, input)],
        }
    }

    fn answer(&mut self, request_id: &str, answer: &PermissionAnswer) -> Option<Vec<String>> {
        let open = self.open.remove(request_id)?;
        let result = match (&open.kind, answer) {
            (RequestKind::Approval, PermissionAnswer::Deny { .. }) => {
                json!({ "decision": "decline" })
            }
            (RequestKind::Approval, _) => json!({ "decision": "accept" }),
            (RequestKind::Permissions(_), PermissionAnswer::Deny { .. }) => {
                json!({ "permissions": {}, "scope": "turn" })
            }
            (RequestKind::Permissions(asked), _) => {
                json!({ "permissions": asked, "scope": "turn" })
            }
            (RequestKind::Questions(ids), PermissionAnswer::Answers { answers }) => {
                let answers: serde_json::Map<String, Value> = answers
                    .iter()
                    .map(|(question, answer)| {
                        let id = ids.get(question).cloned().unwrap_or(question.clone());
                        (id, json!({ "answers": [answer] }))
                    })
                    .collect();
                json!({ "answers": answers })
            }
            (RequestKind::Questions(_), _) => json!({ "answers": {} }),
        };
        Some(vec![
            json!({ "id": open.rpc_id, "result": result }).to_string()
        ])
    }

    fn interrupt(&mut self) -> Vec<String> {
        match (self.thread.clone(), self.turn.clone()) {
            (Some(thread), Some(turn)) => vec![self.request(
                Asked::Other,
                "turn/interrupt",
                json!({ "threadId": thread, "turnId": turn }),
            )],
            _ => Vec::new(),
        }
    }

    /// Codex takes these with the next turn, so they're applied here and confirmed at once.
    fn set(&mut self, setting: &ChatSetting) -> Option<Vec<String>> {
        match setting {
            ChatSetting::Model(model) => self.model = Some(model.clone()),
            ChatSetting::Effort(effort) => self.effort = Some(effort.clone()),
            ChatSetting::PermissionMode(mode) => {
                if !APPROVAL_POLICIES.contains(&mode.as_str()) {
                    return None;
                }
                self.approval_policy = Some(mode.clone());
            }
            ChatSetting::RemoteControl(_) => return None,
        }
        self.update_state();
        let session = self.session();
        self.events.push(session);
        Some(Vec::new())
    }

    fn parse(&mut self, line: &str) -> Vec<ChatEvent> {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        match (msg["method"].as_str(), msg.get("id")) {
            (Some(_), Some(_)) => self.server_request(&msg),
            (Some(method), None) => self.notification(method, &msg["params"]),
            (None, Some(id)) => match id.as_u64() {
                Some(id) => self.response(id, &msg),
                None => Vec::new(),
            },
            (None, None) => Vec::new(),
        }
    }

    fn take_replies(&mut self) -> Vec<String> {
        std::mem::take(&mut self.replies)
    }

    fn take_events(&mut self) -> Vec<ChatEvent> {
        std::mem::take(&mut self.events)
    }

    /// The "transcript" is the id of the Codex thread worki recorded for this session; the
    /// conversation itself comes with resuming the thread.
    fn history(&mut self, transcript: &str) -> Vec<ChatEvent> {
        let thread = transcript.trim();
        if !thread.is_empty() {
            self.thread = Some(thread.to_owned());
        }
        Vec::new()
    }

    fn take_state(&mut self) -> Option<StateUpdate> {
        if !std::mem::take(&mut self.changed) {
            return None;
        }
        Some(StateUpdate {
            agent: Some(self.state.clone()),
            usage: self.usage.take(),
        })
    }
}

/// JSON-RPC ids may be numbers or strings; worki keys requests by their text.
fn rpc_key(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The command as the agent wrote it, without the shell Codex wraps it in.
fn command_text(item: &Value) -> String {
    let actions = item["commandActions"].as_array();
    match actions.map(Vec::as_slice) {
        Some([only]) => only["command"].as_str().map(str::to_owned),
        _ => None,
    }
    .or_else(|| item["command"].as_str().map(str::to_owned))
    .unwrap_or_default()
}

/// What an item adds to the conversation; `done` once Codex completed it.
fn item_events(item: &Value, done: bool) -> Vec<ChatEvent> {
    let id = item["id"].as_str().unwrap_or_default().to_owned();
    let call = |name: &str, input: Value| ChatEvent::ToolCall {
        id: id.clone(),
        name: name.to_owned(),
        input,
        parent: None,
    };
    let result = |output: String, is_error: bool| ChatEvent::ToolResult {
        id: id.clone(),
        output,
        is_error,
        parent: None,
    };
    let failed = matches!(item["status"].as_str(), Some("failed" | "declined"));
    match item["type"].as_str() {
        Some("agentMessage") if done => vec![ChatEvent::Text {
            id: id.clone(),
            text: item["text"].as_str().unwrap_or_default().to_owned(),
            parent: None,
        }],
        Some("commandExecution") => {
            let mut events = vec![call("Bash", json!({ "command": command_text(item) }))];
            if done {
                let exit = item["exitCode"].as_i64();
                let mut output = item["aggregatedOutput"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                if item["status"] == "declined" {
                    output = "Declined".into();
                }
                events.push(result(output, failed || exit.is_some_and(|c| c != 0)));
            }
            events
        }
        Some("fileChange") => {
            let changes = item["changes"].as_array().cloned().unwrap_or_default();
            let mut events = vec![call(
                "Edit",
                json!({
                    "file_path": changes.first().map(|c| c["path"].clone()),
                    "diff": changes
                        .iter()
                        .filter_map(|c| c["diff"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                }),
            )];
            if done {
                let output = if failed {
                    "The change wasn't applied"
                } else {
                    ""
                };
                events.push(result(output.into(), failed));
            }
            events
        }
        Some("mcpToolCall") => {
            let name = format!(
                "{}.{}",
                item["server"].as_str().unwrap_or_default(),
                item["tool"].as_str().unwrap_or_default()
            );
            let mut events = vec![call(&name, item["arguments"].clone())];
            if done {
                let output = match item["error"]["message"].as_str() {
                    Some(error) => error.to_owned(),
                    None => texts(&item["result"]["content"]),
                };
                events.push(result(output, failed || !item["error"].is_null()));
            }
            events
        }
        Some("dynamicToolCall") => {
            let mut events = vec![call(
                item["tool"].as_str().unwrap_or_default(),
                item["arguments"].clone(),
            )];
            if done {
                let output = texts(&item["contentItems"]);
                events.push(result(output, failed || item["success"] == false));
            }
            events
        }
        Some("webSearch") => {
            let mut events = vec![call("WebSearch", json!({ "query": item["query"] }))];
            if done {
                events.push(result(String::new(), false));
            }
            events
        }
        _ => Vec::new(),
    }
}

/// The `text` of every block in `blocks` that has one, one per line.
fn texts(blocks: &Value) -> String {
    blocks
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The conversation of a resumed thread, from the turns Codex returns with it.
fn past_turns(turns: &Value) -> Vec<ChatEvent> {
    let mut events = Vec::new();
    for item in turns
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|t| t["items"].as_array().cloned().unwrap_or_default())
    {
        if item["type"] == "userMessage" {
            let content = item["content"].as_array().cloned().unwrap_or_default();
            let text = content
                .iter()
                .filter(|c| c["type"] == "text")
                .filter_map(|c| c["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let attachments = content
                .iter()
                .filter(|c| c["type"] != "text")
                .map(|c| c["type"].as_str().unwrap_or("attachment").to_owned())
                .collect();
            events.push(ChatEvent::User {
                id: None,
                text,
                attachments,
            });
        } else {
            events.extend(item_events(&item, true));
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD: &str = "01a1cc4b-22b1-78f0-a7a9-10d3e94af059";

    fn protocol() -> CodexProtocol {
        CodexProtocol::new(
            "worki-1",
            CodexOptions {
                developer_instructions: Some("be brief".into()),
                confined: false,
            },
        )
    }

    fn sent(lines: &[String]) -> Vec<Value> {
        lines
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn parse_all(p: &mut CodexProtocol, lines: &[&str]) -> Vec<ChatEvent> {
        lines.iter().flat_map(|l| p.parse(l)).collect()
    }

    /// Lines Codex 0.167 printed for a turn that read a file and changed it.
    const TURN: &[&str] = &[
        r#"{"id": 1, "result": {"userAgent": "worki/0.167.1 (Mac OS 26.6.2; arm64) claude-desktop (worki; 0)", "codexHome": "/Users/me/.codex", "platformFamily": "unix", "platformOs": "macos"}}"#,
        r#"{"id": 2, "result": {"thread": {"id": "01a1cc4b-22b1-78f0-a7a9-10d3e94af059", "turns": []}, "model": "gpt-6-luna", "modelProvider": "openai", "approvalPolicy": "untrusted", "sandbox": {"type": "workspaceWrite"}, "reasoningEffort": null}}"#,
        r#"{"id": 3, "result": {"data": [{"id": "gpt-6-luna", "model": "gpt-6-luna", "displayName": "GPT-6 Luna", "description": "Fast", "hidden": false, "supportedReasoningEfforts": [{"reasoningEffort": "low", "description": ""}, {"reasoningEffort": "high", "description": ""}], "isDefault": true}, {"id": "secret", "displayName": "Secret", "hidden": true, "supportedReasoningEfforts": []}], "nextCursor": null}}"#,
    ];

    fn started() -> CodexProtocol {
        let mut p = protocol();
        p.start();
        parse_all(&mut p, TURN);
        p.take_replies();
        p.take_state();
        p
    }

    #[test]
    fn starts_a_thread_and_records_it_for_resuming() {
        let mut p = protocol();
        let start = sent(&p.start());
        assert_eq!(start[0]["method"], "initialize");
        assert_eq!(start[1], json!({ "method": "initialized" }));
        assert_eq!(start[2]["method"], "thread/start");
        assert_eq!(start[2]["params"]["sandbox"], "workspace-write");
        assert_eq!(start[2]["params"]["developerInstructions"], "be brief");
        assert_eq!(start[3]["method"], "model/list");

        let events = parse_all(&mut p, TURN);
        assert!(events.contains(&ChatEvent::Session {
            model: Some("gpt-6-luna".into()),
            permission_mode: Some("untrusted".into()),
            effort: None,
        }));
        let Some(ChatEvent::Options {
            models,
            permission_modes,
        }) = events.last()
        else {
            panic!("no options: {events:?}")
        };
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].label, "GPT-6 Luna");
        assert_eq!(models[0].effort_levels, ["low", "high"]);
        assert_eq!(permission_modes, &APPROVAL_POLICIES.map(String::from));

        let replies = sent(&p.take_replies());
        assert_eq!(replies[0]["method"], "fs/createDirectory");
        assert_eq!(
            replies[0]["params"]["path"],
            "/Users/me/.codex/worki-sessions"
        );
        assert_eq!(replies[1]["method"], "fs/writeFile");
        assert_eq!(
            replies[1]["params"]["path"],
            "/Users/me/.codex/worki-sessions/worki-1"
        );
        assert_eq!(
            replies[1]["params"]["dataBase64"],
            base64::engine::general_purpose::STANDARD.encode(THREAD)
        );

        let state = p.take_state().unwrap().agent.unwrap();
        assert_eq!(state.model.as_deref(), Some("GPT-6 Luna"));
        assert_eq!(state.version.as_deref(), Some("0.167.1"));
    }

    #[test]
    fn confined_sessions_turn_codexs_sandbox_off() {
        let mut p = CodexProtocol::new(
            "s",
            CodexOptions {
                developer_instructions: None,
                confined: true,
            },
        );
        let start = sent(&p.start());
        assert_eq!(start[2]["params"]["sandbox"], "danger-full-access");
        assert!(start[2]["params"].get("developerInstructions").is_none());
    }

    #[test]
    fn holds_messages_until_the_thread_exists() {
        let mut p = protocol();
        p.start();
        assert!(p.message("m1", "hi", &[]).is_empty());
        parse_all(&mut p, &TURN[..2]);
        let replies = sent(&p.take_replies());
        let turn = replies
            .iter()
            .find(|r| r["method"] == "turn/start")
            .unwrap();
        assert_eq!(turn["params"]["threadId"], THREAD);
        assert_eq!(turn["params"]["clientUserMessageId"], "m1");
        assert_eq!(turn["params"]["input"][0]["text"], "hi");
    }

    #[test]
    fn reads_a_turn_with_commands_and_streamed_text() {
        let mut p = started();
        let turn = sent(&p.message("m1", "change a.txt", &[]));
        assert_eq!(turn[0]["method"], "turn/start");
        let id = turn[0]["id"].as_u64().unwrap();
        let events = parse_all(
            &mut p,
            &[
                &format!(
                    r#"{{"id": {id}, "result": {{"turn": {{"id": "t1", "status": "inProgress"}}}}}}"#
                ),
                r#"{"method": "turn/started", "params": {"threadId": "x", "turn": {"id": "t1"}}}"#,
                r#"{"method": "item/started", "params": {"item": {"type": "userMessage", "id": "u1", "clientId": "m1", "content": []}}}"#,
                r#"{"method": "item/started", "params": {"item": {"type": "agentMessage", "id": "msg1", "text": ""}}}"#,
                r#"{"method": "item/agentMessage/delta", "params": {"itemId": "msg1", "delta": "I'll"}}"#,
                r#"{"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": "msg1", "text": "I'll do it."}}}"#,
                r#"{"method": "item/started", "params": {"item": {"type": "commandExecution", "id": "exec1", "command": "/bin/bash -lc 'cat a.txt'", "status": "inProgress", "commandActions": [{"type": "read", "command": "cat a.txt", "name": "a.txt", "path": "/w/a.txt"}], "aggregatedOutput": null, "exitCode": null}}}"#,
                r#"{"method": "item/completed", "params": {"item": {"type": "commandExecution", "id": "exec1", "command": "/bin/bash -lc 'cat a.txt'", "status": "completed", "commandActions": [{"type": "read", "command": "cat a.txt", "name": "a.txt", "path": "/w/a.txt"}], "aggregatedOutput": "hello\n", "exitCode": 0}}}"#,
                r#"{"method": "item/completed", "params": {"item": {"type": "fileChange", "id": "fc1", "status": "completed", "changes": [{"path": "/w/a.txt", "kind": {"type": "update", "move_path": null}, "diff": "-hello\n+hi\n"}]}}}"#,
                r#"{"method": "thread/tokenUsage/updated", "params": {"tokenUsage": {"total": {"inputTokens": 1000, "cachedInputTokens": 800, "cacheWriteInputTokens": 0, "outputTokens": 50}, "last": {"inputTokens": 500}, "modelContextWindow": 10000}}}"#,
                r#"{"method": "turn/completed", "params": {"turn": {"id": "t1", "status": "completed", "error": null}}}"#,
            ],
        );
        let status = |s: &str| ChatEvent::Status {
            status: s.into(),
            tool: None,
        };
        assert_eq!(
            events,
            vec![
                status("requesting"),
                ChatEvent::UserRead { id: "m1".into() },
                status("writing"),
                ChatEvent::TextDelta {
                    id: "msg1".into(),
                    text: "I'll".into(),
                    parent: None
                },
                ChatEvent::Text {
                    id: "msg1".into(),
                    text: "I'll do it.".into(),
                    parent: None
                },
                ChatEvent::ToolCall {
                    id: "exec1".into(),
                    name: "Bash".into(),
                    input: json!({ "command": "cat a.txt" }),
                    parent: None
                },
                ChatEvent::ToolCall {
                    id: "exec1".into(),
                    name: "Bash".into(),
                    input: json!({ "command": "cat a.txt" }),
                    parent: None
                },
                ChatEvent::ToolResult {
                    id: "exec1".into(),
                    output: "hello\n".into(),
                    is_error: false,
                    parent: None
                },
                ChatEvent::ToolCall {
                    id: "fc1".into(),
                    name: "Edit".into(),
                    input: json!({ "file_path": "/w/a.txt", "diff": "-hello\n+hi\n" }),
                    parent: None
                },
                ChatEvent::ToolResult {
                    id: "fc1".into(),
                    output: String::new(),
                    is_error: false,
                    parent: None
                },
                ChatEvent::TurnEnded {
                    ok: true,
                    interrupted: false,
                    error: None
                },
            ]
        );
        let state = p.take_state().unwrap();
        assert_eq!(state.agent.unwrap().context_used, Some(5.0));
        assert_eq!(
            state.usage.unwrap(),
            vec![ModelUsage {
                model: "gpt-6-luna".into(),
                input: 200,
                output: 50,
                cache_write: 0,
                cache_write_1h: 0,
                cache_read: 800,
            }]
        );

        let steer = sent(&p.message("m2", "more", &[]));
        assert_eq!(steer[0]["method"], "turn/start");
    }

    #[test]
    fn steers_and_interrupts_a_running_turn() {
        let mut p = started();
        assert!(p.interrupt().is_empty());
        parse_all(
            &mut p,
            &[r#"{"method": "turn/started", "params": {"turn": {"id": "t1"}}}"#],
        );
        let steer = sent(&p.message("m2", "also this", &[]));
        assert_eq!(steer[0]["method"], "turn/steer");
        assert_eq!(steer[0]["params"]["expectedTurnId"], "t1");
        let interrupt = sent(&p.interrupt());
        assert_eq!(interrupt[0]["method"], "turn/interrupt");
        assert_eq!(interrupt[0]["params"]["turnId"], "t1");
        let events = parse_all(
            &mut p,
            &[
                r#"{"method": "turn/completed", "params": {"turn": {"id": "t1", "status": "interrupted", "error": null}}}"#,
            ],
        );
        assert_eq!(
            events,
            vec![ChatEvent::TurnEnded {
                ok: false,
                interrupted: true,
                error: None
            }]
        );
    }

    #[test]
    fn asks_to_approve_commands_and_edits() {
        let mut p = started();
        let events = parse_all(
            &mut p,
            &[
                r#"{"method": "item/commandExecution/requestApproval", "id": 0, "params": {"itemId": "exec1", "command": "/bin/bash -lc \"printf 'hi' > a.txt\"", "commandActions": [{"type": "unknown", "command": "printf 'hi' > a.txt"}], "reason": null}}"#,
            ],
        );
        assert_eq!(
            events,
            vec![ChatEvent::Permission {
                request_id: "0".into(),
                tool: "Bash".into(),
                input: json!({ "command": "printf 'hi' > a.txt", "description": null }),
                questions: Vec::new(),
            }]
        );
        assert_eq!(
            sent(&p.answer("0", &PermissionAnswer::Allow).unwrap()),
            vec![json!({ "id": 0, "result": { "decision": "accept" } })]
        );
        assert!(p.answer("0", &PermissionAnswer::Allow).is_none());

        parse_all(
            &mut p,
            &[
                r#"{"method": "item/started", "params": {"item": {"type": "fileChange", "id": "fc1", "status": "inProgress", "changes": [{"path": "/w/a.txt", "diff": "+hi\n"}]}}}"#,
                r#"{"method": "item/fileChange/requestApproval", "id": 1, "params": {"itemId": "fc1", "reason": "outside the workspace"}}"#,
            ],
        );
        assert_eq!(
            sent(
                &p.answer("1", &PermissionAnswer::Deny { message: None })
                    .unwrap()
            ),
            vec![json!({ "id": 1, "result": { "decision": "decline" } })]
        );
    }

    #[test]
    fn answers_questions_by_their_ids() {
        let mut p = started();
        let events = parse_all(
            &mut p,
            &[
                r#"{"method": "item/tool/requestUserInput", "id": 7, "params": {"itemId": "q", "isBlocking": true, "questions": [{"id": "lang", "header": "Language", "question": "Which language?", "isOther": true, "isSecret": false, "options": [{"label": "Rust", "description": "fast"}]}]}}"#,
            ],
        );
        let Some(ChatEvent::Permission { questions, .. }) = events.first() else {
            panic!("{events:?}")
        };
        assert_eq!(questions[0].question, "Which language?");
        assert_eq!(questions[0].options[0].label, "Rust");
        let answers = [("Which language?".to_owned(), "Rust".to_owned())].into();
        assert_eq!(
            sent(
                &p.answer("7", &PermissionAnswer::Answers { answers })
                    .unwrap()
            ),
            vec![json!({ "id": 7, "result": { "answers": { "lang": { "answers": ["Rust"] } } } })]
        );
    }

    #[test]
    fn refuses_requests_it_doesnt_handle_and_reports_resolved_ones() {
        let mut p = started();
        assert!(parse_all(
            &mut p,
            &[r#"{"method": "mcpServer/elicitation/request", "id": 9, "params": {}}"#]
        )
        .is_empty());
        assert_eq!(sent(&p.take_replies())[0]["error"]["code"], -32601);

        parse_all(
            &mut p,
            &[
                r#"{"method": "item/commandExecution/requestApproval", "id": 3, "params": {"command": "ls"}}"#,
            ],
        );
        assert_eq!(
            parse_all(
                &mut p,
                &[r#"{"method": "serverRequest/resolved", "params": {"requestId": 3}}"#]
            ),
            vec![ChatEvent::PermissionResolved {
                request_id: "3".into(),
                allowed: false,
                elsewhere: false,
            }]
        );
    }

    #[test]
    fn applies_settings_to_the_next_turn() {
        let mut p = started();
        assert_eq!(
            p.set(&ChatSetting::Model("gpt-5.6-terra".into())),
            Some(Vec::new())
        );
        p.set(&ChatSetting::Effort("high".into()));
        p.set(&ChatSetting::PermissionMode("never".into()));
        assert!(p.set(&ChatSetting::PermissionMode("plan".into())).is_none());
        assert_eq!(
            p.take_events().last(),
            Some(&ChatEvent::Session {
                model: Some("gpt-5.6-terra".into()),
                permission_mode: Some("never".into()),
                effort: Some("high".into()),
            })
        );
        let turn = sent(&p.message("m", "go", &[]));
        assert_eq!(turn[0]["params"]["model"], "gpt-5.6-terra");
        assert_eq!(turn[0]["params"]["effort"], "high");
        assert_eq!(turn[0]["params"]["approvalPolicy"], "never");
    }

    #[test]
    fn keeps_settings_chosen_before_the_thread_started() {
        let mut p = protocol();
        p.start();
        p.set(&ChatSetting::PermissionMode("never".into()));
        let events = parse_all(&mut p, &TURN[..2]);
        assert!(matches!(
            events.last(),
            Some(ChatEvent::Session { permission_mode: Some(m), model: Some(_), .. }) if m == "never"
        ));
    }

    #[test]
    fn resumes_the_recorded_thread_with_its_conversation() {
        let mut p = protocol();
        assert!(p.history(&format!("{THREAD}\n")).is_empty());
        let start = sent(&p.start());
        assert_eq!(start[2]["method"], "thread/resume");
        assert_eq!(start[2]["params"]["threadId"], THREAD);
        let events = parse_all(
            &mut p,
            &[
                TURN[0],
                r#"{"id": 2, "result": {"thread": {"id": "01a1cc4b-22b1-78f0-a7a9-10d3e94af059", "turns": [{"id": "t1", "items": [{"type": "userMessage", "id": "u1", "clientId": null, "content": [{"type": "text", "text": "Run 'cat a.txt'", "text_elements": []}]}, {"type": "commandExecution", "id": "exec1", "command": "/bin/bash -lc 'cat a.txt'", "status": "completed", "commandActions": [], "aggregatedOutput": "hello\n", "exitCode": 0}, {"type": "agentMessage", "id": "msg1", "text": "It says hello."}]}]}, "model": "gpt-6-luna", "approvalPolicy": "untrusted", "reasoningEffort": "high"}}"#,
            ],
        );
        assert_eq!(
            events[0],
            ChatEvent::User {
                id: None,
                text: "Run 'cat a.txt'".into(),
                attachments: Vec::new()
            }
        );
        assert_eq!(
            events[1],
            ChatEvent::ToolCall {
                id: "exec1".into(),
                name: "Bash".into(),
                input: json!({ "command": "/bin/bash -lc 'cat a.txt'" }),
                parent: None
            }
        );
        assert!(matches!(&events[3], ChatEvent::Text { text, .. } if text == "It says hello."));
        assert!(matches!(&events[4], ChatEvent::Notice { .. }));
        assert!(matches!(&events[5], ChatEvent::TurnEnded { ok: true, .. }));
        assert!(matches!(&events[6], ChatEvent::Session { effort: Some(e), .. } if e == "high"));
    }

    #[test]
    fn starts_over_when_the_thread_cant_be_resumed() {
        let mut p = protocol();
        p.history(THREAD);
        p.start();
        let events = parse_all(
            &mut p,
            &[r#"{"id": 2, "error": {"code": -32600, "message": "no rollout found"}}"#],
        );
        assert!(
            matches!(&events[0], ChatEvent::Notice { text } if text.contains("no rollout found"))
        );
        let replies = sent(&p.take_replies());
        assert_eq!(replies[0]["method"], "thread/start");
    }

    #[test]
    fn sends_attachments_as_input() {
        let mut p = started();
        let lines = sent(&p.message(
            "m",
            "",
            &[
                Attachment::Text {
                    name: "a.md".into(),
                    text: "# A".into(),
                },
                Attachment::Image {
                    name: "s.png".into(),
                    media_type: "image/png".into(),
                    data: "AAAA".into(),
                },
            ],
        ));
        let input = &lines[0]["params"]["input"];
        assert_eq!(input.as_array().unwrap().len(), 2);
        assert!(input[0]["text"].as_str().unwrap().contains("# A"));
        assert_eq!(input[1]["url"], "data:image/png;base64,AAAA");
    }
}
