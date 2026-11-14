//! Knowledge about specific agent CLIs: where their user config lives, which parts are safe to
//! copy into an isolated environment, and which token they authenticate with. Blocking; call from
//! a worker.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;
use worki_isolation::{host_which, Backend};

use crate::model::JsonObject;

pub struct Kit {
    pub executable: &'static str,
    /// How agents sign in without the human's own login, if worki can set that up.
    pub sign_in: Option<SignIn>,
    /// Where the agent records that it trusts a folder, so worki can answer its question.
    pub trust: Option<Trust>,
}

pub struct Trust {
    /// Config file, relative to the home directory.
    pub file: &'static str,
    /// The file's new content with the folder trusted, or `None` when it already is.
    pub apply: fn(&str, &Path) -> Result<Option<String>>,
}

/// A token worki creates with `claude setup-token` (approved in the browser, bound to the
/// subscription) and passes to the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignIn {
    /// Env var the token is stored and passed as.
    pub secret: &'static str,
}

const KITS: &[Kit] = &[
    Kit {
        executable: "claude",
        sign_in: Some(SignIn {
            secret: "CLAUDE_CODE_OAUTH_TOKEN",
        }),
        trust: Some(Trust {
            file: ".claude.json",
            apply: trusted_claude_json,
        }),
    },
    Kit {
        executable: "codex",
        sign_in: None,
        trust: Some(Trust {
            file: ".codex/config.toml",
            apply: trusted_codex_config,
        }),
    },
];

pub fn kit(executable: &str) -> Option<&'static Kit> {
    KITS.iter().find(|k| k.executable == executable)
}

fn home() -> Result<PathBuf> {
    dirs::home_dir().context("no home directory")
}

/// Makes the human's installation of `executable` usable inside the environment. Node-based
/// installs (`#!/usr/bin/env node`) also need `node`, which is shared the same way.
pub fn share_install(backend: &dyn Backend, executable: &str) -> Result<String> {
    let host = host_which(executable)
        .with_context(|| format!("`{executable}` is not installed for you"))?;
    backend.share_host_executable(executable, Path::new(&host))?;
    if needs_node(Path::new(&host)) && backend.which("node").is_none() {
        let node =
            host_which("node").context("this agent needs `node`, which isn't installed for you")?;
        backend.share_host_executable("node", Path::new(&node))?;
    }
    Ok(host)
}

fn needs_node(path: &Path) -> bool {
    let Ok(resolved) = path.canonicalize() else {
        return false;
    };
    let Ok(mut file) = std::fs::File::open(resolved) else {
        return false;
    };
    let mut head = [0u8; 128];
    let n = std::io::Read::read(&mut file, &mut head).unwrap_or(0);
    let head = String::from_utf8_lossy(&head[..n]);
    head.starts_with("#!") && head.lines().next().is_some_and(|l| l.contains("node"))
}

/// Settings left out of the host's when importing them: they run scripts from the host's home
/// folder, or restrict the agent where the isolation already confines it.
const NOT_IMPORTED: &[&str] = &[
    "hooks",
    "statusLine",
    "sandbox",
    "permissions.deny",
    "permissions.ask",
];

/// The host's own Claude Code settings (`~/.claude/settings.json`), without [`NOT_IMPORTED`].
pub fn host_claude_settings() -> Result<JsonObject> {
    let path = home()?.join(".claude/settings.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(JsonObject::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut settings: JsonObject =
        serde_json::from_str(&text).context("~/.claude/settings.json is not a JSON object")?;
    for path in NOT_IMPORTED {
        remove_path(&mut settings, path);
    }
    Ok(settings)
}

/// Removes a dotted key path such as `permissions.deny`, if it's there.
fn remove_path(settings: &mut JsonObject, path: &str) {
    let (parents, key) = path.rsplit_once('.').unwrap_or(("", path));
    let mut object = Some(settings);
    for part in parents.split('.').filter(|p| !p.is_empty()) {
        object = object
            .and_then(|o| o.get_mut(part))
            .and_then(Value::as_object_mut);
    }
    if let Some(object) = object {
        object.remove(key);
    }
}

/// Marks `worktree` as trusted in the agent's own config inside an isolated environment, where
/// nobody can answer the agent's trust question in a chat and the isolation already limits what
/// it reaches.
pub fn trust_worktree(backend: &dyn Backend, executable: &str, worktree: &Path) -> Result<()> {
    let Some(trust) = kit(executable).and_then(|k| k.trust.as_ref()) else {
        return Ok(());
    };
    let file = Path::new(trust.file);
    let current = backend
        .read_home_file(file)?
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    match (trust.apply)(&current, worktree)? {
        Some(updated) => backend.write_home_file(file, updated.as_bytes()),
        None => Ok(()),
    }
}

/// `~/.claude.json` with `worktree`'s trust question answered, or `None` when it already is.
fn trusted_claude_json(current: &str, worktree: &Path) -> Result<Option<String>> {
    let mut json: Value = serde_json::from_str(current)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Default::default()));
    let key = worktree.to_string_lossy();
    if json["hasCompletedOnboarding"] == true
        && json["projects"][key.as_ref()]["hasTrustDialogAccepted"] == true
    {
        return Ok(None);
    }
    json["hasCompletedOnboarding"] = Value::Bool(true);
    json["projects"][key.as_ref()]["hasTrustDialogAccepted"] = Value::Bool(true);
    Ok(Some(serde_json::to_string_pretty(&json)?))
}

/// Codex's `config.toml` with `worktree` as a trusted project, written the way Codex records it,
/// or `None` when it already is.
fn trusted_codex_config(current: &str, worktree: &Path) -> Result<Option<String>> {
    let header = format!(
        "[projects.{}]",
        toml::Value::String(worktree.to_string_lossy().into_owned())
    );
    if current.contains(&header) {
        return Ok(None);
    }
    Ok(Some(format!(
        "{current}\n{header}\ntrust_level = \"trusted\"\n"
    )))
}

/// Finds the token printed by `claude setup-token` in raw terminal output.
pub fn find_claude_token(output: &str) -> Option<String> {
    let text = strip_ansi(output);
    let start = text.find("sk-ant-oat")?;
    let token: String = text[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (token.len() > 20).then_some(token)
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' || (c == '\x1b' && chars.peek() == Some(&'\\')) {
                        chars.next_if_eq(&'\\');
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_scripts_and_restrictions_out_of_the_import() {
        let mut settings: JsonObject = serde_json::from_str(
            r#"{"model":"opus","hooks":{"Stop":[]},"statusLine":{"command":"x"},
            "sandbox":{"allowUnsandboxedCommands":false},
            "permissions":{"defaultMode":"auto","allow":["Bash(ls)"],"deny":["Read(.env)"]}}"#,
        )
        .unwrap();
        for path in NOT_IMPORTED {
            remove_path(&mut settings, path);
        }
        assert_eq!(
            Value::from(settings),
            serde_json::json!({
                "model": "opus",
                "permissions": {"defaultMode": "auto", "allow": ["Bash(ls)"]}
            })
        );
    }

    #[test]
    fn finds_token_between_escape_codes() {
        let out = "\x1b[32m✓ Token created\x1b[0m\r\n\x1b]8;;https://x\x07link\x1b]8;;\x07\r\nsk-ant-oat01-AbC_d-123456789012345678\x1b[0m\r\n";
        assert_eq!(
            find_claude_token(out).as_deref(),
            Some("sk-ant-oat01-AbC_d-123456789012345678")
        );
        assert_eq!(find_claude_token("no token here"), None);
    }

    #[test]
    fn trusts_the_worktree_in_each_agents_config() {
        let wt = Path::new("/Users/me/worki/worktrees/app/fix");
        let claude = trusted_claude_json(r#"{"projects":{"/other":{}},"theme":"dark"}"#, wt)
            .unwrap()
            .unwrap();
        let json: Value = serde_json::from_str(&claude).unwrap();
        assert_eq!(
            json["projects"][wt.to_str().unwrap()]["hasTrustDialogAccepted"],
            true
        );
        assert_eq!(json["theme"], "dark");
        assert_eq!(trusted_claude_json(&claude, wt).unwrap(), None);

        let codex = trusted_codex_config("model = \"o3\"\n", wt)
            .unwrap()
            .unwrap();
        let config: toml::Table = toml::from_str(&codex).unwrap();
        assert_eq!(config["model"].as_str(), Some("o3"));
        assert_eq!(
            config["projects"][wt.to_str().unwrap()]["trust_level"].as_str(),
            Some("trusted")
        );
        assert_eq!(trusted_codex_config(&codex, wt).unwrap(), None);
    }

    #[test]
    fn known_kits() {
        let sign_in = |exe| kit(exe).unwrap().sign_in;
        assert_eq!(sign_in("claude").unwrap().secret, "CLAUDE_CODE_OAUTH_TOKEN");
        assert!(sign_in("codex").is_none());
        assert!(kit("aider").is_none());
    }
}
