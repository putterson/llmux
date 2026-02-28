use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("ambiguous session match: {0} matches multiple sessions")]
    AmbiguousSession(String),

    #[error("session not running: {0}")]
    SessionNotRunning(String),

    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error("PTY error: {0}")]
    Pty(String),

    #[error("socket error: {0}")]
    Socket(String),

    #[error("workspace error: {0}")]
    Workspace(String),

    #[error("path not found: {0}")]
    PathNotFound(PathBuf),

    #[error("nix error: {0}")]
    Nix(#[from] nix::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
