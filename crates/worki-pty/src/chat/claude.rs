//! Claude Code's stream-json protocol (`claude -p --input-format stream-json --output-format
//! stream-json --verbose --include-partial-messages --permission-prompt-tool stdio`): one JSON
//! object per line both ways, with permission requests and interrupts as control messages.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use super::{
    Attachment, BackgroundTask, ChatEvent, ChatProtocol, ChatSetting, ModelChoice,
    PermissionAnswer, Question, StateUpdate,
};

/// Claude Code's permission modes worki offers. Skipping all prompts isn't among them: Claude
/// Code only allows it for sessions started with it.
const PERMISSION_MODES: [&str; 4] = ["default", "acceptEdits", "plan", "auto"];
use crate::{AgentState, ModelUsage, RemoteControl};

#[derive(Default)]
pub struct ClaudeProtocol {
    session_id: String,
    next_request: u64,
    /// worki's control requests waiting for an answer, by request id, with what they ask.
    asked: HashMap<String, Value>,
    effort: Option<String>,
    /// Display names of the models Claude Code offers, by model id.
    model_names: HashMap<String, String>,
    /// The message and content block being streamed, per subagent (`None` for the main agent).
    streaming: HashMap<Option<String>, (String, u64)>,
    /// Open permission requests and the tool input they were asked with.
    open: HashMap<String, Value>,
    /// Messages worki sent that Claude Code hasn't echoed back yet.
    sent: HashSet<String>,
    replies: Vec<String>,
    state: AgentState,
    /// Tokens the main agent's last request sent, which is what fills its context window.
    context_tokens: Option<u64>,
    changed: bool,
    usage: Option<Vec<ModelUsage>>,
    remote_control_at_start: bool,
    remote: Remote,
}

#[derive(Default)]
struct Remote {
    on: bool,
    state: Option<String>,
    url: Option<String>,
}

impl ClaudeProtocol {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            ..Default::default()
        }
    }

    pub fn with_remote_control(mut self, on: bool) -> Self {
        self.remote_control_at_start = on;
        self
    }

    fn remote_control(&mut self, on: bool) -> String {
        self.remote.on = on;
        self.control(json!({ "subtype": "remote_control", "enabled": on }))
    }

    fn report_remote(&mut self) {
        let state = if self.remote.on {
            self.remote.state.as_deref().unwrap_or("connecting")
        } else {
            "off"
        };
        self.state.remote_control = Some(RemoteControl {
            state: state.to_owned(),
            url: self.remote.url.clone().filter(|_| self.remote.on),
        });
        self.changed = true;
    }

    fn control(&mut self, request: Value) -> String {
        self.next_request += 1;
        let id = format!("worki_{}", self.next_request);
        self.asked.insert(id.clone(), request.clone());
        json!({ "type": "control_request", "request_id": id, "request": request }).to_string()
    }

    fn session(&self) -> ChatEvent {
        ChatEvent::Session {
            model: self.state.model.clone(),
            permission_mode: self.state.permission_mode.clone(),
            effort: self.effort.clone(),
        }
    }

    /// Claude Code's answer to one of worki's control requests.
    fn control_response(&mut self, msg: &Value) -> Vec<ChatEvent> {
        let response = &msg["response"];
        let Some(request) = response["request_id"]
            .as_str()
            .and_then(|id| self.asked.remove(id))
        else {
            return Vec::new();
        };
        if response["subtype"] != "success" {
            let error = response["error"].as_str().unwrap_or("the agent refused");
            let what = match request["subtype"].as_str() {
                Some("initialize" | "interrupt") | None => return Vec::new(),
                Some("background_tasks") => "move the command to the background".to_owned(),
                Some("stop_task") => "stop the background job".to_owned(),
                Some("remote_control") if request["enabled"] == true => {
                    self.remote = Remote::default();
                    self.report_remote();
                    "turn on Remote Control".to_owned()
                }
                Some("remote_control") => "turn off Remote Control".to_owned(),
                Some(setting) => format!("change the setting ({setting})"),
            };
            return vec![ChatEvent::Notice {
                text: format!("Couldn't {what}: {error}"),
            }];
        }
        let answer = &response["response"];
        match request["subtype"].as_str() {
            Some("initialize") => {
                let models = answer["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|m| ModelChoice {
                        value: m["value"].as_str().unwrap_or_default().to_owned(),
                        model: m["resolvedModel"].as_str().map(str::to_owned),
                        label: m["displayName"].as_str().unwrap_or_default().to_owned(),
                        description: m["description"].as_str().map(str::to_owned),
                        effort_levels: m["supportedEffortLevels"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|l| l.as_str().map(str::to_owned))
                            .collect(),
                    })
                    .filter(|m: &ModelChoice| !m.value.is_empty())
                    .collect::<Vec<_>>();
                for m in models.iter().filter(|m| m.value != "default") {
                    if let Some(id) = &m.model {
                        self.model_names
                            .entry(id.clone())
                            .or_insert(m.label.clone());
                    }
                }
                // Until the first turn says otherwise, the agent runs its default model.
                if self.state.model.is_none() {
                    self.state.model = models
                        .iter()
                        .find(|m| m.value == "default")
                        .and_then(|m| m.model.clone());
                }
                if let Some(mode) = answer["current_permission_mode"].as_str() {
                    self.state.permission_mode = Some(mode.into());
                }
                self.report_remote();
                vec![
                    ChatEvent::Options {
                        models,
                        permission_modes: PERMISSION_MODES.map(String::from).to_vec(),
                    },
                    self.session(),
                ]
            }
            Some("set_model") => {
                self.state.model = answer["model"]
                    .as_str()
                    .or(request["model"].as_str())
                    .map(str::to_owned);
                self.changed = true;
                vec![self.session()]
            }
            Some("set_permission_mode") => {
                self.state.permission_mode = request["mode"].as_str().map(str::to_owned);
                self.changed = true;
                vec![self.session()]
            }
            Some("apply_flag_settings") => {
                self.effort = request["settings"]["effortLevel"]
                    .as_str()
                    .map(str::to_owned);
                self.state.effort = self.effort.clone();
                self.changed = true;
                vec![self.session()]
            }
            Some("remote_control") if request["enabled"] == true => {
                self.remote.url = answer["session_url"].as_str().map(str::to_owned);
                self.report_remote();
                Vec::new()
            }
            Some("remote_control") => {
                self.remote = Remote::default();
                self.report_remote();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn assistant(&mut self, msg: &Value, parent: Option<String>) -> Vec<ChatEvent> {
        let message = &msg["message"];
        let id = message["id"].as_str().unwrap_or_default();
        let streamed = self
            .streaming
            .get(&parent)
            .filter(|(current, _)| current == id)
            .map(|(_, index)| *index);
        let blocks = message["content"].as_array().cloned().unwrap_or_default();
        blocks
            .iter()
            .enumerate()
            .filter_map(|(i, block)| match block["type"].as_str()? {
                "text" => Some(ChatEvent::Text {
                    id: format!("{id}:{}", streamed.unwrap_or(i as u64)),
                    text: block["text"].as_str().unwrap_or_default().to_owned(),
                    parent: parent.clone(),
                }),
                "tool_use" => Some(ChatEvent::ToolCall {
                    id: block["id"].as_str().unwrap_or_default().to_owned(),
                    name: block["name"].as_str().unwrap_or_default().to_owned(),
                    input: block["input"].clone(),
                    parent: parent.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    fn user(&mut self, msg: &Value, parent: Option<String>) -> Vec<ChatEvent> {
        let content = &msg["message"]["content"];
        if let Some(text) = content.as_str() {
            return notice(text, &parent);
        }
        let blocks = content.as_array().cloned().unwrap_or_default();
        blocks
            .iter()
            .flat_map(|block| match block["type"].as_str() {
                Some("tool_result") => vec![ChatEvent::ToolResult {
                    id: block["tool_use_id"].as_str().unwrap_or_default().to_owned(),
                    output: flatten(&block["content"]),
                    is_error: block["is_error"].as_bool().unwrap_or(false),
                    parent: parent.clone(),
                }],
                Some("text") => notice(block["text"].as_str().unwrap_or_default(), &parent),
                _ => Vec::new(),
            })
            .collect()
    }

    fn stream_event(&mut self, msg: &Value, parent: Option<String>) -> Vec<ChatEvent> {
        let event = &msg["event"];
        match event["type"].as_str() {
            Some("message_start") => {
                let id = event["message"]["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                self.streaming.insert(parent, (id, 0));
            }
            Some("content_block_start") => {
                if let Some(current) = self.streaming.get_mut(&parent) {
                    current.1 = event["index"].as_u64().unwrap_or(0);
                }
                if parent.is_none() {
                    let block = &event["content_block"];
                    let (status, tool) = match block["type"].as_str() {
                        Some("thinking" | "redacted_thinking") => ("thinking", None),
                        Some("text") => ("writing", None),
                        Some("tool_use") => ("tool", block["name"].as_str().map(str::to_owned)),
                        _ => return Vec::new(),
                    };
                    return vec![ChatEvent::Status {
                        status: status.into(),
                        tool,
                    }];
                }
            }
            Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
                let Some((id, _)) = self.streaming.get(&parent) else {
                    return Vec::new();
                };
                return vec![ChatEvent::TextDelta {
                    id: format!("{id}:{}", event["index"].as_u64().unwrap_or(0)),
                    text: event["delta"]["text"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    parent,
                }];
            }
            Some("message_delta") if parent.is_none() => {
                let usage = &event["usage"];
                let tokens = [
                    "input_tokens",
                    "cache_creation_input_tokens",
                    "cache_read_input_tokens",
                ]
                .iter()
                .filter_map(|k| usage[*k].as_u64())
                .sum::<u64>();
                if tokens > 0 {
                    self.context_tokens = Some(tokens);
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn result(&mut self, msg: &Value) -> Vec<ChatEvent> {
        let ok = msg["subtype"] == "success" && !msg["is_error"].as_bool().unwrap_or(false);
        let interrupted = msg["terminal_reason"]
            .as_str()
            .is_some_and(|r| r.starts_with("aborted"));
        let model_usage = msg["modelUsage"].as_object().cloned().unwrap_or_default();
        let count = |u: &Value, key: &str| u[key].as_u64().unwrap_or(0);
        let usage: Vec<ModelUsage> = model_usage
            .iter()
            .map(|(model, u)| ModelUsage {
                model: model.clone(),
                input: count(u, "inputTokens"),
                output: count(u, "outputTokens"),
                cache_write: count(u, "cacheCreationInputTokens"),
                cache_write_1h: count(u, "cacheCreationEphemeral1hInputTokens"),
                cache_read: count(u, "cacheReadInputTokens"),
            })
            .collect();
        let window = self
            .state
            .model
            .as_ref()
            .and_then(|m| model_usage.get(m))
            .and_then(|u| u["contextWindow"].as_u64());
        if let Some(window) = window.filter(|w| *w > 0) {
            self.state.context_size = Some(window);
            self.state.context_used = self
                .context_tokens
                .map(|t| (t as f64 / window as f64 * 1000.0).round() / 10.0);
        }
        self.state.cost_usd = msg["total_cost_usd"].as_f64().or(self.state.cost_usd);
        if !usage.is_empty() {
            self.usage = Some(usage);
        }
        self.changed = true;
        let error = (!ok && !interrupted).then(|| {
            msg["errors"]
                .as_array()
                .and_then(|e| e.first())
                .and_then(Value::as_str)
                .or(msg["result"].as_str())
                .unwrap_or("The agent stopped with an error")
                .to_owned()
        });
        vec![ChatEvent::TurnEnded {
            ok,
            interrupted,
            error,
        }]
    }

    fn control_request(&mut self, msg: &Value) -> Vec<ChatEvent> {
        let request_id = msg["request_id"].as_str().unwrap_or_default().to_owned();
        let request = &msg["request"];
        if request["subtype"] != "can_use_tool" {
            self.replies.push(
                json!({
                    "type": "control_response",
                    "response": {
                        "subtype": "error",
                        "request_id": request_id,
                        "error": "worki doesn't handle this request",
                    },
                })
                .to_string(),
            );
            return Vec::new();
        }
        let tool = request["tool_name"].as_str().unwrap_or_default().to_owned();
        let input = request["input"].clone();
        let questions = if tool == "AskUserQuestion" {
            serde_json::from_value::<Vec<Question>>(input["questions"].clone()).unwrap_or_default()
        } else {
            Vec::new()
        };
        self.open.insert(request_id.clone(), input.clone());
        vec![ChatEvent::Permission {
            request_id,
            tool,
            input,
            questions,
        }]
    }
}

impl ChatProtocol for ClaudeProtocol {
    fn start(&mut self) -> Vec<String> {
        let mut lines = vec![self.control(json!({ "subtype": "initialize", "hooks": null }))];
        if self.remote_control_at_start {
            lines.push(self.remote_control(true));
        }
        lines
    }

    fn message(&mut self, id: &str, text: &str, attachments: &[Attachment]) -> Vec<String> {
        let mut content: Vec<Value> = attachments
            .iter()
            .map(|a| match a {
                Attachment::Image {
                    media_type, data, ..
                } => json!({
                    "type": "image",
                    "source": { "type": "base64", "media_type": media_type, "data": data },
                }),
                Attachment::Text { name, text } => json!({
                    "type": "text",
                    "text": format!("<attached-file name=\"{name}\">\n{text}\n</attached-file>"),
                }),
            })
            .collect();
        self.sent.insert(id.to_owned());
        // The API refuses empty text blocks; a message may be only attachments.
        if !text.trim().is_empty() {
            content.push(json!({ "type": "text", "text": text }));
        }
        vec![json!({
            "type": "user",
            "message": { "role": "user", "content": content },
            "parent_tool_use_id": null,
            "session_id": self.session_id,
            "uuid": id,
        })
        .to_string()]
    }

    fn answer(&mut self, request_id: &str, answer: &PermissionAnswer) -> Option<Vec<String>> {
        let input = self.open.remove(request_id)?;
        let response = match answer {
            PermissionAnswer::Allow => json!({ "behavior": "allow", "updatedInput": input }),
            PermissionAnswer::Deny { message } => json!({
                "behavior": "deny",
                "message": message.as_deref().unwrap_or("The user declined this."),
            }),
            PermissionAnswer::Answers { answers } => {
                let mut input = input;
                if let Some(object) = input.as_object_mut() {
                    object.insert("answers".into(), json!(answers));
                }
                json!({ "behavior": "allow", "updatedInput": input })
            }
        };
        Some(vec![json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": request_id, "response": response },
        })
        .to_string()])
    }

    fn interrupt(&mut self) -> Vec<String> {
        vec![self.control(json!({ "subtype": "interrupt" }))]
    }

    fn background(&mut self) -> Option<Vec<String>> {
        Some(vec![self.control(json!({ "subtype": "background_tasks" }))])
    }

    fn stop_task(&mut self, task_id: &str) -> Option<Vec<String>> {
        Some(vec![self.control(
            json!({ "subtype": "stop_task", "task_id": task_id }),
        )])
    }

    fn set(&mut self, setting: &ChatSetting) -> Option<Vec<String>> {
        let request = match setting {
            ChatSetting::Model(model) => json!({ "subtype": "set_model", "model": model }),
            ChatSetting::PermissionMode(mode) => {
                json!({ "subtype": "set_permission_mode", "mode": mode })
            }
            // Claude Code has no request for effort; it takes it as a setting.
            ChatSetting::Effort(level) => {
                json!({ "subtype": "apply_flag_settings", "settings": { "effortLevel": level } })
            }
            ChatSetting::RemoteControl(on) => {
                let line = self.remote_control(*on);
                if *on {
                    self.report_remote();
                }
                return Some(vec![line]);
            }
        };
        Some(vec![self.control(request)])
    }

    fn parse(&mut self, line: &str) -> Vec<ChatEvent> {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let parent = msg["parent_tool_use_id"].as_str().map(str::to_owned);
        match msg["type"].as_str() {
            Some("system") if msg["subtype"] == "init" => {
                self.state.model = msg["model"].as_str().map(str::to_owned);
                self.state.permission_mode = msg["permissionMode"].as_str().map(str::to_owned);
                self.state.version = msg["claude_code_version"].as_str().map(str::to_owned);
                self.state.output_style = msg["output_style"].as_str().map(str::to_owned);
                self.state.effort = self.effort.clone();
                self.changed = true;
                vec![self.session()]
            }
            Some("system") if msg["subtype"] == "status" => match msg["status"].as_str() {
                Some(status) if !status.is_empty() => vec![ChatEvent::Status {
                    status: status.to_owned(),
                    tool: None,
                }],
                _ => Vec::new(),
            },
            Some("system") if msg["subtype"] == "bridge_state" => {
                self.remote.state = msg["state"].as_str().map(str::to_owned);
                if self.remote.on {
                    self.report_remote();
                }
                Vec::new()
            }
            Some("system") if msg["subtype"] == "background_tasks_changed" => {
                let field = |t: &Value, key: &str| t[key].as_str().unwrap_or_default().to_owned();
                let tasks = msg["tasks"].as_array().into_iter().flatten();
                vec![ChatEvent::BackgroundTasks {
                    tasks: tasks
                        .map(|t| BackgroundTask {
                            id: field(t, "task_id"),
                            kind: field(t, "task_type"),
                            description: field(t, "description"),
                        })
                        .collect(),
                }]
            }
            Some("stream_event") => self.stream_event(&msg, parent),
            Some("assistant") => self.assistant(&msg, parent),
            Some("user") if msg["isReplay"] == true => match msg["uuid"].as_str() {
                Some(id) if self.sent.remove(id) => vec![ChatEvent::UserRead { id: id.to_owned() }],
                // Sent from elsewhere, such as the Claude app through Remote Control.
                _ => past_user_message(&msg["message"]["content"]),
            },
            Some("user") => self.user(&msg, parent),
            Some("result") => self.result(&msg),
            Some("control_request") => self.control_request(&msg),
            Some("control_response") => self.control_response(&msg),
            Some("control_cancel_request") => {
                let request_id = msg["request_id"].as_str().unwrap_or_default().to_owned();
                match self.open.remove(&request_id) {
                    Some(_) => vec![ChatEvent::PermissionResolved {
                        request_id,
                        allowed: false,
                        elsewhere: self.remote.on,
                    }],
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    fn take_replies(&mut self) -> Vec<String> {
        std::mem::take(&mut self.replies)
    }

    /// Reads Claude Code's transcript (`projects/<dir>/<session>.jsonl`): the conversation is in
    /// its `user` and `assistant` lines; subagents' work (`isSidechain`) and what Claude Code adds
    /// on its own (`isMeta`) are left out.
    fn history(&mut self, transcript: &str) -> Vec<ChatEvent> {
        let mut events = Vec::new();
        let mut blocks: HashMap<String, u64> = HashMap::new();
        for line in transcript.lines() {
            let Ok(msg) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if msg["isSidechain"] == true || msg["isMeta"] == true {
                continue;
            }
            let content = &msg["message"]["content"];
            match msg["type"].as_str() {
                Some("assistant") => {
                    let id = msg["message"]["id"].as_str().unwrap_or_default().to_owned();
                    for block in content.as_array().into_iter().flatten() {
                        let n = blocks.entry(id.clone()).or_default();
                        *n += 1;
                        match block["type"].as_str() {
                            Some("text") => events.push(ChatEvent::Text {
                                id: format!("{id}:history{n}"),
                                text: block["text"].as_str().unwrap_or_default().to_owned(),
                                parent: None,
                            }),
                            Some("tool_use") => events.push(ChatEvent::ToolCall {
                                id: block["id"].as_str().unwrap_or_default().to_owned(),
                                name: block["name"].as_str().unwrap_or_default().to_owned(),
                                input: block["input"].clone(),
                                parent: None,
                            }),
                            _ => {}
                        }
                    }
                }
                Some("user") => events.extend(past_user_message(content)),
                _ => {}
            }
        }
        if !events.is_empty() {
            events.push(ChatEvent::Notice {
                text: "Continued from an earlier session".into(),
            });
        }
        events
    }

    fn take_state(&mut self) -> Option<StateUpdate> {
        if !std::mem::take(&mut self.changed) {
            return None;
        }
        let mut agent = self.state.clone();
        agent.model = agent
            .model
            .map(|id| self.model_names.get(&id).cloned().unwrap_or(id));
        Some(StateUpdate {
            agent: Some(agent),
            usage: self.usage.take(),
        })
    }
}

/// A `user` line of the transcript: what the user wrote (with what worki attached to it), tool
/// results, or a notice such as an interruption.
fn past_user_message(content: &Value) -> Vec<ChatEvent> {
    let blocks = match content {
        Value::String(text) => vec![json!({ "type": "text", "text": text })],
        Value::Array(blocks) => blocks.clone(),
        _ => return Vec::new(),
    };
    let mut events = Vec::new();
    let mut text = Vec::new();
    let mut attachments = Vec::new();
    for block in &blocks {
        match block["type"].as_str() {
            Some("tool_result") => events.push(ChatEvent::ToolResult {
                id: block["tool_use_id"].as_str().unwrap_or_default().to_owned(),
                output: flatten(&block["content"]),
                is_error: block["is_error"].as_bool().unwrap_or(false),
                parent: None,
            }),
            Some("image") => attachments.push("image".to_owned()),
            Some("text") => {
                let t = block["text"].as_str().unwrap_or_default();
                if let Some(name) = t
                    .strip_prefix("<attached-file name=\"")
                    .and_then(|rest| rest.split_once('"'))
                {
                    attachments.push(name.0.to_owned());
                } else if t.starts_with("[Request interrupted") {
                    events.extend(notice(t, &None));
                } else if !t.starts_with("<system-reminder>")
                    && !t.starts_with("<command-")
                    && !t.starts_with("<local-command")
                {
                    text.push(t.to_owned());
                }
            }
            _ => {}
        }
    }
    if !text.is_empty() || !attachments.is_empty() {
        events.insert(
            0,
            ChatEvent::User {
                id: None,
                text: text.join("\n\n"),
                attachments,
            },
        );
    }
    events
}

/// Text the agent's side adds to the conversation, such as "[Request interrupted by user]".
/// Subagents' prompts aren't shown.
fn notice(text: &str, parent: &Option<String>) -> Vec<ChatEvent> {
    let text = text.trim();
    if parent.is_some() || text.is_empty() {
        return Vec::new();
    }
    let text = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or(text);
    vec![ChatEvent::Notice { text: text.into() }]
}

/// A tool result's content: a string, or blocks of text and images.
fn flatten(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .map(|b| match b["type"].as_str() {
                Some("text") => b["text"].as_str().unwrap_or_default().to_owned(),
                Some("image") => "[image]".into(),
                _ => String::new(),
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{json, Value};

    use super::*;

    /// Lines as Claude Code 2.1.319 printed them, trimmed to the fields worki reads.
    const TURN: &[&str] = &[
        r#"{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","claude_code_version":"2.1.319","output_style":"default","session_id":"s"}"#,
        r#"{"type":"system","subtype":"status","status":"requesting","session_id":"s"}"#,
        r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_1","model":"claude-haiku-5-5"}},"parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"Bash","input":{}}},"parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\": \"ls\"}"}},"parent_tool_use_id":null}"#,
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}]},"parent_tool_use_id":null}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_1","type":"tool_result","content":"hello.txt","is_error":false}]},"parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_2"}},"parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}},"parent_tool_use_id":null}"#,
        r#"{"type":"assistant","message":{"id":"msg_2","content":[{"type":"thinking","thinking":""}]},"parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"do"}},"parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"ne"}},"parent_tool_use_id":null}"#,
        r#"{"type":"assistant","message":{"id":"msg_2","content":[{"type":"text","text":"done"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":4,"cache_creation_input_tokens":96,"cache_read_input_tokens":900,"output_tokens":5}},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done","total_cost_usd":0.0084,"terminal_reason":"completed","modelUsage":{"claude-haiku-5-5":{"inputTokens":1193,"outputTokens":216,"cacheReadInputTokens":34439,"cacheCreationInputTokens":39011,"cacheCreationEphemeral1hInputTokens":39011,"contextWindow":10000}}}"#,
    ];

    fn parse_all(p: &mut ClaudeProtocol, lines: &[&str]) -> Vec<ChatEvent> {
        lines.iter().flat_map(|l| p.parse(l)).collect()
    }

    #[test]
    fn reads_a_turn_with_a_tool_call_and_streamed_text() {
        let mut p = ClaudeProtocol::new("s");
        let events = parse_all(&mut p, TURN);
        assert_eq!(
            events,
            [
                ChatEvent::Session {
                    model: Some("claude-haiku-5-5".into()),
                    permission_mode: Some("default".into()),
                    effort: None,
                },
                ChatEvent::Status {
                    status: "requesting".into(),
                    tool: None,
                },
                ChatEvent::Status {
                    status: "tool".into(),
                    tool: Some("Bash".into()),
                },
                ChatEvent::ToolCall {
                    id: "toolu_1".into(),
                    name: "Bash".into(),
                    input: json!({ "command": "ls" }),
                    parent: None,
                },
                ChatEvent::ToolResult {
                    id: "toolu_1".into(),
                    output: "hello.txt".into(),
                    is_error: false,
                    parent: None,
                },
                ChatEvent::Status {
                    status: "thinking".into(),
                    tool: None,
                },
                ChatEvent::Status {
                    status: "writing".into(),
                    tool: None,
                },
                ChatEvent::TextDelta {
                    id: "msg_2:1".into(),
                    text: "do".into(),
                    parent: None,
                },
                ChatEvent::TextDelta {
                    id: "msg_2:1".into(),
                    text: "ne".into(),
                    parent: None,
                },
                ChatEvent::Text {
                    id: "msg_2:1".into(),
                    text: "done".into(),
                    parent: None,
                },
                ChatEvent::TurnEnded {
                    ok: true,
                    interrupted: false,
                    error: None,
                },
            ]
        );
        let state = p.take_state().unwrap();
        let agent = state.agent.unwrap();
        assert_eq!(agent.model.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(agent.version.as_deref(), Some("2.1.319"));
        assert_eq!(agent.cost_usd, Some(0.0084));
        assert_eq!(
            (agent.context_used, agent.context_size),
            (Some(10.0), Some(10_000))
        );
        assert_eq!(
            state.usage.unwrap(),
            [ModelUsage {
                model: "claude-haiku-5-5".into(),
                input: 1193,
                output: 216,
                cache_write: 39011,
                cache_write_1h: 39011,
                cache_read: 34439,
            }]
        );
        assert_eq!(p.take_state(), None);
    }

    #[test]
    fn reports_when_a_queued_message_is_read() {
        let mut p = ClaudeProtocol::new("s");
        p.message("u1", "Now run: echo second", &[]);
        let replay = |uuid: &str, text: &str| {
            json!({
                "type": "user",
                "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
                "session_id": "s",
                "parent_tool_use_id": null,
                "uuid": uuid,
                "isReplay": true,
            })
            .to_string()
        };
        assert_eq!(
            p.parse(&replay("u1", "Now run: echo second")),
            [ChatEvent::UserRead { id: "u1".into() }]
        );
        assert_eq!(
            p.parse(&replay("phone-1", "run date in the shell")),
            [ChatEvent::User {
                id: None,
                text: "run date in the shell".into(),
                attachments: Vec::new(),
            }]
        );
    }

    #[test]
    fn stops_a_background_task() {
        let mut p = ClaudeProtocol::new("s");
        let request: Value = serde_json::from_str(&p.stop_task("byn260eu5").unwrap()[0]).unwrap();
        assert_eq!(
            request["request"],
            json!({ "subtype": "stop_task", "task_id": "byn260eu5" })
        );
    }

    #[test]
    fn moves_foreground_commands_to_the_background() {
        let mut p = ClaudeProtocol::new("s");
        let sent = p.message("u1", "run cms tests", &[]);
        assert_eq!(
            serde_json::from_str::<Value>(&sent[0]).unwrap()["uuid"],
            "u1"
        );
        let request: Value = serde_json::from_str(&p.background().unwrap()[0]).unwrap();
        assert_eq!(request["request"], json!({ "subtype": "background_tasks" }));
        let id = request["request_id"].as_str().unwrap();
        let events = p.parse(
            &json!({ "type": "control_response", "response": { "subtype": "error", "request_id": id, "error": "disabled" } })
                .to_string(),
        );
        assert_eq!(
            events,
            [ChatEvent::Notice {
                text: "Couldn't move the command to the background: disabled".into()
            }]
        );
    }

    #[test]
    fn lists_background_tasks() {
        let mut p = ClaudeProtocol::new("s");
        let events = p.parse(
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"b0hq7vzs2","run_id":"r","task_type":"local_bash","description":"Sleep for four seconds"}],"session_id":"s"}"#,
        );
        assert_eq!(
            events,
            [ChatEvent::BackgroundTasks {
                tasks: vec![BackgroundTask {
                    id: "b0hq7vzs2".into(),
                    kind: "local_bash".into(),
                    description: "Sleep for four seconds".into(),
                }],
            }]
        );
        let events = p.parse(
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[],"session_id":"s"}"#,
        );
        assert_eq!(events, [ChatEvent::BackgroundTasks { tasks: vec![] }]);
    }

    #[test]
    fn asks_permission_and_sends_the_answers() {
        let mut p = ClaudeProtocol::new("s");
        let events = p.parse(
            r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Red or blue?","header":"Color","options":[{"label":"Red","description":"Pick red"},{"label":"Blue"}],"multiSelect":false}]},"tool_use_id":"toolu_9"}}"#,
        );
        let [ChatEvent::Permission {
            questions, tool, ..
        }] = &events[..]
        else {
            panic!("{events:?}");
        };
        assert_eq!(tool, "AskUserQuestion");
        assert_eq!(questions[0].options[1].label, "Blue");
        assert_eq!(questions[0].options[1].description, None);

        let answers = BTreeMap::from([("Red or blue?".to_string(), "Blue".to_string())]);
        let reply: Value = serde_json::from_str(
            &p.answer("r1", &PermissionAnswer::Answers { answers })
                .unwrap()[0],
        )
        .unwrap();
        assert_eq!(reply["response"]["request_id"], "r1");
        assert_eq!(reply["response"]["response"]["behavior"], "allow");
        assert_eq!(
            reply["response"]["response"]["updatedInput"]["answers"]["Red or blue?"],
            "Blue"
        );
        assert_eq!(p.answer("r1", &PermissionAnswer::Allow), None);
    }

    #[test]
    fn denies_and_reports_cancelled_requests() {
        let mut p = ClaudeProtocol::new("s");
        let request = |id: &str| {
            format!(
                r#"{{"type":"control_request","request_id":"{id}","request":{{"subtype":"can_use_tool","tool_name":"Write","input":{{"file_path":"/w/a.txt"}}}}}}"#
            )
        };
        p.parse(&request("r1"));
        let reply: Value = serde_json::from_str(
            &p.answer("r1", &PermissionAnswer::Deny { message: None })
                .unwrap()[0],
        )
        .unwrap();
        assert_eq!(reply["response"]["response"]["behavior"], "deny");

        p.parse(&request("r2"));
        assert_eq!(
            p.parse(r#"{"type":"control_cancel_request","request_id":"r2"}"#),
            [ChatEvent::PermissionResolved {
                request_id: "r2".into(),
                allowed: false,
                elsewhere: false,
            }]
        );

        p.set(&ChatSetting::RemoteControl(true));
        p.parse(&request("r3"));
        assert_eq!(
            p.parse(r#"{"type":"control_cancel_request","request_id":"r3"}"#),
            [ChatEvent::PermissionResolved {
                request_id: "r3".into(),
                allowed: false,
                elsewhere: true,
            }]
        );
    }

    #[test]
    fn answers_unknown_control_requests_with_an_error() {
        let mut p = ClaudeProtocol::new("s");
        assert_eq!(
            p.parse(r#"{"type":"control_request","request_id":"h1","request":{"subtype":"hook_callback"}}"#),
            []
        );
        let reply: Value = serde_json::from_str(&p.take_replies()[0]).unwrap();
        assert_eq!(reply["response"]["subtype"], "error");
        assert_eq!(reply["response"]["request_id"], "h1");
    }

    #[test]
    fn reports_an_interrupted_turn() {
        let mut p = ClaudeProtocol::new("s");
        let events = parse_all(
            &mut p,
            &[
                r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]},"parent_tool_use_id":null}"#,
                r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":null,"terminal_reason":"aborted_streaming","errors":["[ede_diagnostic] result_type=user"]}"#,
            ],
        );
        assert_eq!(
            events,
            [
                ChatEvent::Notice {
                    text: "Request interrupted by user".into(),
                },
                ChatEvent::TurnEnded {
                    ok: false,
                    interrupted: true,
                    error: None,
                },
            ]
        );
    }

    #[test]
    fn offers_models_and_switches_settings() {
        let mut p = ClaudeProtocol::new("s");
        let init: Value = serde_json::from_str(&p.start()[0]).unwrap();
        let events = p.parse(
            &json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": init["request_id"],
                    "response": {
                        "current_permission_mode": "default",
                        "models": [
                            { "value": "opus", "resolvedModel": "claude-opus-5-5", "displayName": "Opus 5.5",
                              "description": "For complex work", "supportedEffortLevels": ["low", "high"] },
                            { "value": "haiku", "resolvedModel": "claude-haiku-5-5", "displayName": "Haiku 5.5" },
                            { "value": "default", "resolvedModel": "claude-opus-5-5", "displayName": "Default (recommended)" }
                        ]
                    }
                }
            })
            .to_string(),
        );
        let ChatEvent::Options {
            models,
            permission_modes,
        } = &events[0]
        else {
            panic!("{events:?}");
        };
        assert_eq!(models[0].label, "Opus 5.5");
        assert_eq!(models[0].effort_levels, ["low", "high"]);
        assert!(models[1].effort_levels.is_empty());
        assert!(permission_modes.contains(&"acceptEdits".to_string()));
        // The status bar shows the default model by name before the first turn.
        let state = p.take_state().unwrap().agent.unwrap();
        assert_eq!(state.model.as_deref(), Some("Opus 5.5"));
        assert_eq!(state.permission_mode.as_deref(), Some("default"));

        let respond = |p: &mut ClaudeProtocol, line: &str, answer: Value| {
            let request: Value = serde_json::from_str(line).unwrap();
            p.parse(
                &json!({
                    "type": "control_response",
                    "response": { "subtype": "success", "request_id": request["request_id"], "response": answer }
                })
                .to_string(),
            )
        };
        let line = p
            .set(&ChatSetting::Model("sonnet".into()))
            .unwrap()
            .remove(0);
        assert!(line.contains("set_model"));
        let line2 = p.set(&ChatSetting::Effort("low".into())).unwrap().remove(0);
        assert!(line2.contains("\"effortLevel\":\"low\""));
        respond(&mut p, &line, json!({ "model": "claude-sonnet-5-5" }));
        assert_eq!(
            respond(&mut p, &line2, Value::Null),
            [ChatEvent::Session {
                model: Some("claude-sonnet-5-5".into()),
                permission_mode: Some("default".into()),
                effort: Some("low".into()),
            }]
        );

        let line = p
            .set(&ChatSetting::PermissionMode("bypassPermissions".into()))
            .unwrap()
            .remove(0);
        let request: Value = serde_json::from_str(&line).unwrap();
        let refused = p.parse(
            &json!({
                "type": "control_response",
                "response": { "subtype": "error", "request_id": request["request_id"], "error": "not allowed" }
            })
            .to_string(),
        );
        assert!(
            matches!(&refused[..], [ChatEvent::Notice { text }] if text.contains("not allowed"))
        );
    }

    #[test]
    fn reads_the_earlier_conversation_from_the_transcript() {
        let mut p = ClaudeProtocol::new("s");
        let transcript = [
            r#"{"type":"session-owner","sessionId":"s"}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"image","source":{}},{"type":"text","text":"<attached-file name=\"notes.md\">\nx\n</attached-file>"},{"type":"text","text":"Fix it"}]},"isSidechain":false}"#,
            r#"{"type":"user","message":{"role":"user","content":"<system-reminder>hidden</system-reminder>"},"isMeta":true}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"thinking","thinking":""}]}}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/w/a"}}]}}"#,
            r#"{"type":"assistant","message":{"id":"x","content":[{"type":"text","text":"subagent"}]},"isSidechain":true}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"a"}]}}"#,
            r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"Done"}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
            "not json",
        ]
        .join("\n");
        let events = p.history(&transcript);
        assert_eq!(
            events,
            [
                ChatEvent::User {
                    id: None,
                    text: "Fix it".into(),
                    attachments: vec!["image".into(), "notes.md".into()],
                },
                ChatEvent::ToolCall {
                    id: "t1".into(),
                    name: "Read".into(),
                    input: json!({ "file_path": "/w/a" }),
                    parent: None,
                },
                ChatEvent::ToolResult {
                    id: "t1".into(),
                    output: "a".into(),
                    is_error: false,
                    parent: None,
                },
                ChatEvent::Text {
                    id: "m2:history1".into(),
                    text: "Done".into(),
                    parent: None,
                },
                ChatEvent::Notice {
                    text: "Request interrupted by user".into(),
                },
                ChatEvent::Notice {
                    text: "Continued from an earlier session".into(),
                },
            ]
        );
        assert!(p.history("").is_empty());
    }

    #[test]
    fn writes_messages_and_interrupts() {
        let mut p = ClaudeProtocol::new("s");
        let start: Value = serde_json::from_str(&p.start()[0]).unwrap();
        assert_eq!(start["request"]["subtype"], "initialize");
        let message: Value = serde_json::from_str(&p.message("m1", "hi", &[])[0]).unwrap();
        assert_eq!(message["type"], "user");
        assert_eq!(message["message"]["content"][0]["text"], "hi");
        let attachments = [
            Attachment::Image {
                name: "shot.png".into(),
                media_type: "image/png".into(),
                data: "AAAA".into(),
            },
            Attachment::Text {
                name: "notes.md".into(),
                text: "# Notes".into(),
            },
        ];
        let message: Value =
            serde_json::from_str(&p.message("m1", "look", &attachments)[0]).unwrap();
        let content = &message["message"]["content"];
        assert_eq!(content[0]["source"]["media_type"], "image/png");
        assert_eq!(
            content[1]["text"],
            "<attached-file name=\"notes.md\">\n# Notes\n</attached-file>"
        );
        assert_eq!(content[2]["text"], "look");
        assert_eq!(message["session_id"], "s");
        let interrupt: Value = serde_json::from_str(&p.interrupt()[0]).unwrap();
        assert_eq!(interrupt["request"]["subtype"], "interrupt");
        assert_ne!(interrupt["request_id"], start["request_id"]);
    }

    #[test]
    fn turns_remote_control_on_and_reports_its_link() {
        let mut p = ClaudeProtocol::new("s").with_remote_control(true);
        let start = p.start();
        let enable: Value = serde_json::from_str(&start[1]).unwrap();
        assert_eq!(enable["request"]["subtype"], "remote_control");
        assert_eq!(enable["request"]["enabled"], true);

        let bridge = |state: &str| {
            json!({ "type": "system", "subtype": "bridge_state", "state": state, "session_id": "s" })
                .to_string()
        };
        let remote = |p: &mut ClaudeProtocol| p.take_state().and_then(|s| s.agent?.remote_control);
        let expect = |state: &str, url: Option<&str>| {
            Some(RemoteControl {
                state: state.into(),
                url: url.map(Into::into),
            })
        };
        let url = "https://claude.ai/code/session_1";
        assert!(p.parse(&bridge("ready")).is_empty());
        assert_eq!(remote(&mut p), expect("ready", None));
        let answer = json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": enable["request_id"],
                "response": { "session_url": url, "bridge_session_id": "cse_1" }
            }
        });
        assert!(p.parse(&answer.to_string()).is_empty());
        assert_eq!(remote(&mut p), expect("ready", Some(url)));
        p.parse(&bridge("reconnecting"));
        assert_eq!(remote(&mut p), expect("reconnecting", Some(url)));

        let disable: Value =
            serde_json::from_str(&p.set(&ChatSetting::RemoteControl(false)).unwrap()[0]).unwrap();
        assert_eq!(disable["request"]["enabled"], false);
        let answer = json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": disable["request_id"] }
        });
        p.parse(&answer.to_string());
        assert_eq!(remote(&mut p), expect("off", None));
        p.parse(&bridge("disconnected"));
        assert!(p.take_state().is_none());
    }

    #[test]
    fn says_why_remote_control_did_not_start() {
        let mut p = ClaudeProtocol::new("s");
        let line = p.set(&ChatSetting::RemoteControl(true)).unwrap().remove(0);
        let state = |p: &mut ClaudeProtocol| {
            p.take_state()
                .and_then(|s| s.agent?.remote_control)
                .map(|r| r.state)
        };
        assert_eq!(state(&mut p).as_deref(), Some("connecting"));
        let request: Value = serde_json::from_str(&line).unwrap();
        let refused = json!({
            "type": "control_response",
            "response": {
                "subtype": "error",
                "request_id": request["request_id"],
                "error": "Remote Control is disabled by your organization"
            }
        });
        assert_eq!(
            p.parse(&refused.to_string()),
            [ChatEvent::Notice {
                text: "Couldn't turn on Remote Control: Remote Control is disabled by your organization"
                    .into()
            }]
        );
        assert_eq!(state(&mut p).as_deref(), Some("off"));
    }
}
