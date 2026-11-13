use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} is not a git repository")]
    NotARepo(PathBuf),
    #[error("project not found: {0}")]
    ProjectNotFound(String),
    #[error("saved SSH host not found: {0}")]
    SshHostNotFound(String),
    #[error("project already added: {0}")]
    ProjectExists(PathBuf),
    #[error("invalid branch name: {0}")]
    InvalidBranch(String),
    #[error("branch {0} already exists")]
    BranchExists(String),
    #[error("unknown base branch: {0}")]
    UnknownRef(String),
    #[error("path already exists: {0}")]
    PathExists(PathBuf),
    #[error("git {args}: {stderr}")]
    Git { args: String, stderr: String },
    #[error("{host}: {message}")]
    Remote { host: String, message: String },
    #[error("command failed with exit code {0:?}")]
    CommandFailed(Option<i32>),
    #[error("no install command for {0}; add one in the Agents settings")]
    NoInstaller(String),
    #[error("{0}")]
    Isolation(String),
    #[error("{0}")]
    GitHub(String),
    #[error("invalid .worki.toml: {0}")]
    Config(#[from] toml::de::Error),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn isolation(e: anyhow::Error) -> Self {
        Self::Isolation(format!("{e:#}"))
    }
}
