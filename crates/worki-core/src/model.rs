use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
pub use worki_isolation::{Isolation, ProjectIsolation};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub repo_path: PathBuf,
    pub worktree_root: PathBuf,
    pub default_base: String,
    pub isolation: ProjectIsolation,
    pub setup_script: Option<String>,
    /// Set when the repository lives on another machine; paths are then paths on that host.
    #[serde(default)]
    pub remote: Option<Remote>,
}

impl Project {
    /// Whether `path` is in the main checkout or under the worktree folder.
    pub fn contains(&self, path: &Path) -> bool {
        path.starts_with(&self.repo_path) || path.starts_with(&self.worktree_root)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    /// Host name or `~/.ssh/config` alias.
    pub host: String,
    /// Login name; unset uses the SSH config or the local user name.
    #[serde(default)]
    pub user: Option<String>,
    /// Unset uses the SSH config or 22.
    #[serde(default)]
    pub port: Option<u16>,
    /// Run sessions in tmux on the host so they outlive worki and the connection.
    pub keep_alive: bool,
    /// The saved host these connection settings come from; they follow its changes.
    #[serde(default)]
    pub host_id: Option<String>,
}

impl Remote {
    pub fn target(&self) -> worki_isolation::ssh::SshTarget {
        worki_isolation::ssh::SshTarget::new(self.host.clone(), self.user.clone(), self.port)
    }
}

/// Connection settings saved once and shared by remote projects.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshHost {
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
}

impl SshHost {
    pub fn target(&self) -> worki_isolation::ssh::SshTarget {
        worki_isolation::ssh::SshTarget::new(self.host.clone(), self.user.clone(), self.port)
    }

    pub fn apply(&self, remote: &mut Remote) {
        remote.host = self.host.clone();
        remote.user = self.user.clone();
        remote.port = self.port;
        remote.host_id = Some(self.id.clone());
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub worktree_root: Option<PathBuf>,
    pub default_base: Option<String>,
    pub isolation: Option<ProjectIsolation>,
    /// An empty string clears the script.
    pub setup_script: Option<String>,
    /// Only for remote projects: new connection settings.
    pub remote: Option<Remote>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: String,
    pub is_main: bool,
    pub locked: bool,
    pub prunable: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeStatus {
    pub changed_files: Vec<ChangedFile>,
    pub insertions: u32,
    pub deletions: u32,
    pub ahead: u32,
    pub behind: u32,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BranchChanges {
    pub base: String,
    pub files: Vec<ChangedFile>,
    pub insertions: u32,
    pub deletions: u32,
    /// Commits on the worktree's branch since it left `base`.
    pub commits: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    /// Two-letter porcelain v1 code, e.g. " M", "??", "A ".
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorktree {
    /// Names both the worktree folder (slugified) and the new branch.
    pub name: String,
    /// Existing branch or ref the new branch starts from.
    pub base: String,
    pub run_setup: bool,
    /// The new worktree's isolation; `Inherit` (the default) follows the project default.
    #[serde(default)]
    pub isolation: ProjectIsolation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveWorktree {
    pub path: PathBuf,
    pub force: bool,
    pub delete_branch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "stream", content = "line")]
pub enum LogLine {
    Info(String),
    Stdout(String),
    Stderr(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    /// Shell command line, run through the user's login shell.
    pub command: String,
    /// Shell code that installs the agent inside an isolated environment.
    #[serde(default)]
    pub install: Option<String>,
}

impl AgentProfile {
    pub fn executable(&self) -> Option<&str> {
        self.command.split_whitespace().next()
    }
}

/// Install commands for agents worki knows. They run as the isolated user, so they must not need
/// root and should install into that user's home (`~/.local/bin` is on its PATH).
fn known_installer(executable: &str) -> Option<&'static str> {
    match executable {
        "claude" => Some("curl -fsSL https://claude.ai/install.sh | bash"),
        "codex" => Some(r#"npm install -g --prefix "$HOME/.local" @openai/codex"#),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettings {
    pub profiles: Vec<AgentProfile>,
    pub default_profile: Option<String>,
    /// Per isolation scope (`Isolation::scope_key`), names of environment variables whose
    /// values live in the Keychain and are passed to agents running there. Earlier versions
    /// stored one global list; it is read into [`LEGACY_SCOPE`] and moved on load.
    #[serde(default, deserialize_with = "scoped_secrets")]
    pub secrets: BTreeMap<String, Vec<String>>,
    /// Per isolation scope, agent setup to repeat in every new environment (e.g. each new
    /// sandbox): which agents to install and whose config to copy, by executable.
    #[serde(default)]
    pub environment_setup: BTreeMap<String, EnvironmentSetup>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentSetup {
    #[serde(default)]
    pub install: Vec<String>,
    #[serde(default)]
    pub copy_config: Vec<String>,
}

pub const LEGACY_SCOPE: &str = "legacy";

fn scoped_secrets<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, Vec<String>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Scoped(BTreeMap<String, Vec<String>>),
        Global(Vec<String>),
    }
    Ok(match Stored::deserialize(d)? {
        Stored::Scoped(map) => map,
        Stored::Global(names) if names.is_empty() => BTreeMap::new(),
        Stored::Global(names) => BTreeMap::from([(LEGACY_SCOPE.to_owned(), names)]),
    })
}

impl Default for AgentSettings {
    fn default() -> Self {
        let profile = |id: &str, name: &str, command: &str| AgentProfile {
            id: id.into(),
            name: name.into(),
            command: command.into(),
            install: known_installer(command).map(Into::into),
        };
        Self {
            profiles: vec![
                profile("claude", "Claude Code", "claude"),
                profile("codex", "Codex", "codex"),
            ],
            default_profile: Some("claude".into()),
            secrets: BTreeMap::new(),
            environment_setup: BTreeMap::new(),
        }
    }
}

impl AgentSettings {
    pub fn secret_names(&self, scope: &str) -> &[String] {
        self.secrets
            .get(scope)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn add_secret(&mut self, scope: &str, name: &str) {
        let names = self.secrets.entry(scope.to_owned()).or_default();
        if !names.iter().any(|n| n == name) {
            names.push(name.to_owned());
        }
    }

    /// Remembers that new environments in `scope` should get `executable` installed / its config.
    pub fn remember_setup(&mut self, scope: &str, executable: &str, install: bool) {
        let setup = self.environment_setup.entry(scope.to_owned()).or_default();
        let list = if install {
            &mut setup.install
        } else {
            &mut setup.copy_config
        };
        if !list.iter().any(|e| e == executable) {
            list.push(executable.to_owned());
        }
    }

    pub fn remove_secret(&mut self, scope: &str, name: &str) {
        if let Some(names) = self.secrets.get_mut(scope) {
            names.retain(|n| n != name);
        }
    }

    /// Fills in install commands for known agents saved before installs existed.
    pub fn with_known_installers(mut self) -> Self {
        for p in &mut self.profiles {
            if p.install.is_none() {
                p.install = p.executable().and_then(known_installer).map(Into::into);
            }
        }
        self
    }

    pub fn profile(&self, id: &str) -> Option<&AgentProfile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    pub fn default_profile(&self) -> Option<&AgentProfile> {
        self.default_profile
            .as_deref()
            .and_then(|id| self.profile(id))
            .or(self.profiles.first())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IsolationSettings {
    /// Used by projects whose isolation is `inherit`.
    pub default: Isolation,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfills_installers_for_known_agents() {
        let old: AgentSettings = serde_json::from_str(
            r#"{"profiles":[{"id":"c","name":"C","command":"claude --model opus"},{"id":"x","name":"X","command":"aider"}],"defaultProfile":"c"}"#,
        )
        .unwrap();
        let s = old.with_known_installers();
        assert!(s.profiles[0]
            .install
            .as_deref()
            .unwrap()
            .contains("claude.ai/install.sh"));
        assert_eq!(s.profiles[1].install, None);
        assert_eq!(s.profiles[0].executable(), Some("claude"));
    }

    #[test]
    fn reads_old_global_secret_list() {
        let old: AgentSettings =
            serde_json::from_str(r#"{"profiles":[],"defaultProfile":null,"secrets":["GH_TOKEN"]}"#)
                .unwrap();
        assert_eq!(old.secret_names(LEGACY_SCOPE), ["GH_TOKEN"]);
        let new: AgentSettings = serde_json::from_str(
            r#"{"profiles":[],"defaultProfile":null,"secrets":{"none":["A"]}}"#,
        )
        .unwrap();
        assert_eq!(new.secret_names("none"), ["A"]);
        assert!(new.secret_names("macUser:_worki").is_empty());
    }
}
