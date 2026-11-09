use std::path::Path;

use serde::Deserialize;

use crate::{Host, Result};

pub const FILE_NAME: &str = ".worki.toml";

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    #[serde(default)]
    pub worktree: WorktreeConfig,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorktreeConfig {
    pub base: Option<String>,
    pub setup: Option<String>,
}

impl RepoConfig {
    pub async fn load(host: &Host, repo: &Path) -> Result<Option<Self>> {
        match host.read_to_string(&repo.join(FILE_NAME)).await? {
            Some(text) => Ok(Some(toml::from_str(&text)?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_worktree_section() {
        let cfg: RepoConfig = toml::from_str(
            r#"
            [worktree]
            base = "develop"
            setup = "pnpm install"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.worktree.base.as_deref(), Some("develop"));
        assert_eq!(cfg.worktree.setup.as_deref(), Some("pnpm install"));
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(toml::from_str::<RepoConfig>("[worktree]\nsetpu = 'x'").is_err());
    }
}
