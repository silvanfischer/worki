//! Agent sessions without a terminal: worki talks to the agent over its JSON protocol and turns
//! what it says into [`ChatEvent`]s, the same for every agent. Each agent gets a
//! [`ChatProtocol`] that translates both ways.

mod claude;
mod host;

use std::collections::BTreeMap;

pub use claude::ClaudeProtocol;
pub(crate) use host::Chat;
pub use host::ChatSpec;
use serde::{Deserialize, Serialize};

use crate::{AgentState, ModelUsage};

/// What happened in a chat, in the order it happened. The UI rebuilds the conversation from
/// these; `parent` is set for what a subagent does on behalf of the tool call it names.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ChatEvent {
    /// The agent's current settings; sent at the start of each turn and when one changes.
    Session {
        model: Option<String>,
        permission_mode: Option<String>,
        effort: Option<String>,
    },
    /// What the user can switch the agent to.
    Options {
        models: Vec<ModelChoice>,
        permission_modes: Vec<String>,
    },
    /// The user's message, as worki sent it, with the names of what was attached. `id` is set
    /// for a message sent in this session, which the agent then reads (`UserRead`), possibly
    /// only after its current step.
    User {
        id: Option<String>,
        text: String,
        attachments: Vec<String>,
    },
    UserRead {
        id: String,
    },
    /// A piece of text the agent is writing; `id` names the text block it belongs to.
    TextDelta {
        id: String,
        text: String,
        parent: Option<String>,
    },
    /// A text block the agent finished; replaces the deltas with the same `id`.
    Text {
        id: String,
        text: String,
        parent: Option<String>,
    },
    ToolCall {
        id: String,
        name: String,
        input: serde_json::Value,
        parent: Option<String>,
    },
    ToolResult {
        id: String,
        output: String,
        is_error: bool,
        parent: Option<String>,
    },
    /// The agent waits for the user to allow a tool call or answer its questions.
    Permission {
        request_id: String,
        tool: String,
        input: serde_json::Value,
        /// Questions the agent asks, when the tool is a question to the user.
        questions: Vec<Question>,
    },
    /// The permission request was answered (here or by the agent giving up on it).
    PermissionResolved {
        request_id: String,
        allowed: bool,
    },
    /// Something the agent reports outside the conversation, such as an interruption.
    Notice {
        text: String,
    },
    /// What the main agent is doing between tool calls: `requesting`, `thinking`, `writing`,
    /// or `tool` while it writes the input of the tool `tool`.
    Status {
        status: String,
        tool: Option<String>,
    },
    /// The jobs the agent runs in the background, all of them each time the list changes.
    BackgroundTasks {
        tasks: Vec<BackgroundTask>,
    },
    TurnEnded {
        ok: bool,
        interrupted: bool,
        error: Option<String>,
    },
    /// The agent process ended; `error` holds the end of what it printed when it failed.
    Exited {
        code: u32,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTask {
    pub id: String,
    /// Such as `local_bash` or `local_agent`.
    pub kind: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelChoice {
    /// What to pass to switch to it, such as an alias like `opus`.
    pub value: String,
    /// The model it currently means.
    pub model: Option<String>,
    pub label: String,
    pub description: Option<String>,
    /// Empty when the model has no effort levels.
    pub effort_levels: Vec<String>,
}

/// Something the user attaches to a message. worki reads files itself and sends their content,
/// so attachments work even where the agent can't reach the file.
#[derive(Debug, Clone, PartialEq)]
pub enum Attachment {
    Text {
        name: String,
        text: String,
    },
    Image {
        name: String,
        media_type: String,
        /// Base64.
        data: String,
    },
}

/// Text files larger than this aren't attached.
const MAX_TEXT_ATTACHMENT: u64 = 512 * 1024;
/// The largest image Claude's API takes.
const MAX_IMAGE: u64 = 5 * 1024 * 1024;

impl Attachment {
    /// An image given as base64, such as one pasted from the clipboard.
    pub fn image(name: String, media_type: String, data: String) -> anyhow::Result<Self> {
        // Base64 takes four characters for every three bytes.
        if data.len() as u64 / 4 * 3 > MAX_IMAGE {
            anyhow::bail!("{name} is too large to attach (images can be up to 5 MB)");
        }
        Ok(Attachment::Image {
            name,
            media_type,
            data,
        })
    }

    pub fn name(&self) -> &str {
        match self {
            Attachment::Text { name, .. } | Attachment::Image { name, .. } => name,
        }
    }

    /// Reads a file as an image (by its extension) or as UTF-8 text.
    pub fn from_path(path: &std::path::Path) -> anyhow::Result<Self> {
        use base64::Engine;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let image = match ext.as_str() {
            "png" => Some("image/png"),
            "jpg" | "jpeg" => Some("image/jpeg"),
            "gif" => Some("image/gif"),
            "webp" => Some("image/webp"),
            _ => None,
        };
        if let Some(media_type) = image {
            if std::fs::metadata(path)?.len() > MAX_IMAGE {
                anyhow::bail!("{name} is too large to attach (images can be up to 5 MB)");
            }
            let bytes = std::fs::read(path)?;
            return Ok(Attachment::Image {
                name,
                media_type: media_type.into(),
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
            });
        }
        if std::fs::metadata(path)?.len() > MAX_TEXT_ATTACHMENT {
            anyhow::bail!("{name} is too large to attach (over 512 KB)");
        }
        let text = String::from_utf8(std::fs::read(path)?)
            .map_err(|_| anyhow::anyhow!("{name} isn't a text file or an image"))?;
        Ok(Attachment::Text { name, text })
    }
}

/// A setting the user changes while chatting.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum ChatSetting {
    Model(String),
    PermissionMode(String),
    Effort(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: String,
    pub description: Option<String>,
}

/// The user's answer to a [`ChatEvent::Permission`].
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PermissionAnswer {
    Allow,
    Deny {
        message: Option<String>,
    },
    /// Answers to the agent's questions: question text to the chosen labels (or free text).
    Answers {
        answers: BTreeMap<String, String>,
    },
}

/// What the agent reported about itself, applied to the session's info.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StateUpdate {
    pub agent: Option<AgentState>,
    pub usage: Option<Vec<ModelUsage>>,
}

/// Translates between worki and one agent's JSON protocol. Each method returns the lines to
/// write to the agent's stdin, without the trailing newline.
pub trait ChatProtocol: Send {
    /// Lines to send right after the agent started.
    fn start(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// `id` names the message for `UserRead`.
    fn message(&mut self, id: &str, text: &str, attachments: &[Attachment]) -> Vec<String>;

    /// `None` when `request_id` isn't an open request.
    fn answer(&mut self, request_id: &str, answer: &PermissionAnswer) -> Option<Vec<String>>;

    fn interrupt(&mut self) -> Vec<String>;

    /// Moves the commands the agent runs in the foreground to the background, so it goes on
    /// with queued messages; `None` when the agent can't.
    fn background(&mut self) -> Option<Vec<String>> {
        None
    }

    /// Stops a job the agent runs in the background; `None` when the agent can't.
    fn stop_task(&mut self, _task_id: &str) -> Option<Vec<String>> {
        None
    }

    /// `None` when the agent can't change this setting.
    fn set(&mut self, setting: &ChatSetting) -> Option<Vec<String>>;

    /// Reads one line the agent printed.
    fn parse(&mut self, line: &str) -> Vec<ChatEvent>;

    /// The earlier messages of a resumed conversation, from the transcript the agent saved.
    fn history(&mut self, _transcript: &str) -> Vec<ChatEvent> {
        Vec::new()
    }

    /// Lines to send in reply to what the agent printed, such as answers to requests worki
    /// doesn't handle.
    fn take_replies(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// The agent's model, usage and the like, as of the last line read; `None` when unchanged.
    fn take_state(&mut self) -> Option<StateUpdate> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_images_over_5_mb() {
        let image = |bytes: usize| {
            Attachment::image(
                "a.png".into(),
                "image/png".into(),
                "A".repeat(bytes / 3 * 4),
            )
        };
        assert!(image(5 * 1024 * 1024).is_ok());
        assert!(image(5 * 1024 * 1024 + 3).is_err());

        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.png");
        std::fs::write(&big, vec![0u8; 5 * 1024 * 1024 + 1]).unwrap();
        assert!(Attachment::from_path(&big)
            .unwrap_err()
            .to_string()
            .contains("5 MB"));
        let notes = dir.path().join("notes.md");
        std::fs::write(&notes, "# Notes").unwrap();
        assert_eq!(
            Attachment::from_path(&notes).unwrap(),
            Attachment::Text {
                name: "notes.md".into(),
                text: "# Notes".into()
            }
        );
        let binary = dir.path().join("data.bin");
        std::fs::write(&binary, [0xff, 0xfe, 0x00]).unwrap();
        assert!(Attachment::from_path(&binary).is_err());
    }
}
