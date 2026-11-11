//! Knowledge about specific agent CLIs: where their user config lives, which parts are safe to
//! copy into an isolated environment, and which token they authenticate with. Blocking; call from
//! a worker.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;
use worki_isolation::{host_which, Backend};

pub struct Kit {
    pub executable: &'static str,
    /// Config to copy, relative to the home directory. Never credentials, history or caches.
    pub config: &'static [&'static str],
    /// Copied text files whose absolute home paths are rewritten to the environment's home.
    pub rewrite_paths: &'static [&'static str],
    /// JSON settings file and the keys dropped from it because they run the human's own scripts.
    pub strip_keys: Option<(&'static str, &'static [&'static str])>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignIn {
    /// Env var the token or key is stored and passed as.
    pub secret: &'static str,
    pub kind: SignInKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SignInKind {
    /// `claude setup-token`, approved in the browser, bound to the subscription.
    SetupToken,
    /// An API key the user pastes.
    ApiKey,
}

pub const CODEX_API_KEY: &str = "OPENAI_API_KEY";

const KITS: &[Kit] = &[
    Kit {
        executable: "claude",
        config: &[
            ".claude/settings.json",
            ".claude/CLAUDE.md",
            ".claude/agents",
            ".claude/commands",
            ".claude/skills",
            ".claude/output-styles",
            ".claude/keybindings.json",
        ],
        rewrite_paths: &[".claude/settings.json"],
        strip_keys: Some((".claude/settings.json", &["hooks", "statusLine"])),
        sign_in: Some(SignIn {
            secret: "CLAUDE_CODE_OAUTH_TOKEN",
            kind: SignInKind::SetupToken,
        }),
        trust: Some(Trust {
            file: ".claude.json",
            apply: trusted_claude_json,
        }),
    },
    Kit {
        executable: "codex",
        config: &[".codex/config.toml", ".codex/AGENTS.md", ".codex/prompts"],
        rewrite_paths: &[".codex/config.toml"],
        strip_keys: None,
        sign_in: Some(SignIn {
            secret: CODEX_API_KEY,
            kind: SignInKind::ApiKey,
        }),
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

pub fn host_config_paths(kit: &Kit) -> Vec<PathBuf> {
    let Ok(home) = home() else { return Vec::new() };
    kit.config
        .iter()
        .map(PathBuf::from)
        .filter(|p| home.join(p).exists())
        .collect()
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

#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopiedConfig {
    pub copied: Vec<String>,
    /// Settings left out, e.g. `hooks` from `.claude/settings.json`.
    pub skipped: Vec<String>,
}

/// Copies the agent's user config into the environment.
pub fn copy_config(backend: &dyn Backend, kit: &Kit) -> Result<CopiedConfig> {
    let home = home()?;
    let target_home = backend
        .home()
        .context("could not determine the environment's home")?;
    let found = host_config_paths(kit);
    let read_only = backend.read_only_home_paths(&found);
    let paths: Vec<PathBuf> = found
        .into_iter()
        .filter(|p| !read_only.contains(p))
        .collect();
    backend.copy_home_files(&home, &paths)?;

    let mut skipped: Vec<String> = read_only
        .iter()
        .map(|p| format!("{} (the environment provides it, read-only)", p.display()))
        .collect();
    for rel in kit
        .rewrite_paths
        .iter()
        .map(Path::new)
        .filter(|p| paths.iter().any(|c| c == p))
    {
        let mut text = std::fs::read_to_string(home.join(rel))?;
        if let Some((file, keys)) = kit.strip_keys.filter(|(f, _)| Path::new(f) == rel) {
            let (stripped, removed) = strip_json_keys(&text, keys)?;
            skipped.extend(
                removed
                    .into_iter()
                    .map(|k| format!("{k} from {file} (runs scripts from your home folder)")),
            );
            text = stripped;
        }
        backend.write_home_file(rel, rewrite_home(&text, &home, &target_home).as_bytes())?;
    }

    let mut copied: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    if kit.executable == "claude" {
        merge_claude_json(backend, &home, &target_home)?;
        copied.push(".claude.json (MCP servers, account info, onboarding)".into());
    }
    Ok(CopiedConfig { copied, skipped })
}

fn strip_json_keys(text: &str, keys: &[&str]) -> Result<(String, Vec<String>)> {
    let mut value: Value = serde_json::from_str(text).context("settings file is not valid JSON")?;
    let Some(obj) = value.as_object_mut() else {
        return Ok((text.to_owned(), Vec::new()));
    };
    let removed: Vec<String> = keys
        .iter()
        .filter(|k| obj.remove(**k).is_some())
        .map(|k| k.to_string())
        .collect();
    if removed.is_empty() {
        return Ok((text.to_owned(), removed));
    }
    Ok((serde_json::to_string_pretty(&value)?, removed))
}

fn rewrite_home(text: &str, from: &Path, to: &Path) -> String {
    text.replace(
        &format!("{}/", from.display()),
        &format!("{}/", to.display()),
    )
}

/// `~/.claude.json` mixes config with history and account state, so only MCP servers and the
/// account description (`oauthAccount`: email, organization, plan; no tokens, which Claude Code
/// shows to tell subscription use from API use) are taken over, and onboarding is marked done.
fn merge_claude_json(backend: &dyn Backend, home: &Path, target_home: &Path) -> Result<()> {
    let rel = Path::new(".claude.json");
    let mut target: Value = backend
        .read_home_file(rel)?
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Default::default()));
    target["hasCompletedOnboarding"] = Value::Bool(true);

    if let Ok(text) = std::fs::read_to_string(home.join(rel)) {
        let mine: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if let Some(servers) = mine.get("mcpServers") {
            let servers = rewrite_home(&servers.to_string(), home, target_home);
            target["mcpServers"] = serde_json::from_str(&servers)?;
        }
        if let Some(account) = mine.get("oauthAccount") {
            target["oauthAccount"] = account.clone();
        }
    }
    backend.write_home_file(rel, serde_json::to_string_pretty(&target)?.as_bytes())
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
    let project = &mut json["projects"][worktree.to_string_lossy().as_ref()];
    if project["hasTrustDialogAccepted"] == true {
        return Ok(None);
    }
    project["hasTrustDialogAccepted"] = Value::Bool(true);
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
    fn strips_hooks_and_status_line() {
        let text = r#"{"model":"opus","hooks":{"Stop":[]},"statusLine":{"command":"x"}}"#;
        let (out, removed) = strip_json_keys(text, &["hooks", "statusLine"]).unwrap();
        assert_eq!(removed, ["hooks", "statusLine"]);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, serde_json::json!({"model": "opus"}));
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
    fn rewrites_home_prefix_only() {
        let text = r#"{"command": "/Users/me/.claude/hooks/x.sh", "other": "/Users/meow/y"}"#;
        let out = rewrite_home(text, Path::new("/Users/me"), Path::new("/Users/_worki"));
        assert_eq!(
            out,
            r#"{"command": "/Users/_worki/.claude/hooks/x.sh", "other": "/Users/meow/y"}"#
        );
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
        let sign_in = |exe| kit(exe).unwrap().sign_in.unwrap();
        assert_eq!(sign_in("claude").secret, "CLAUDE_CODE_OAUTH_TOKEN");
        assert_eq!(sign_in("claude").kind, SignInKind::SetupToken);
        assert_eq!(sign_in("codex").secret, "OPENAI_API_KEY");
        assert_eq!(sign_in("codex").kind, SignInKind::ApiKey);
        assert!(kit("aider").is_none());
        assert!(kit("claude")
            .unwrap()
            .config
            .iter()
            .all(|p| !p.contains("credentials")));
    }
}
