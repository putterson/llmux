use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub alerts: AlertConfig,

    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,

    #[serde(default)]
    pub workspaces: WorkspaceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,

    #[serde(default = "default_true")]
    pub terminal_bell: bool,

    #[serde(default = "default_true")]
    pub desktop_notification: bool,

    #[serde(default)]
    pub custom_command: String,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_timeout_secs: 5,
            terminal_bell: true,
            desktop_notification: true,
            custom_command: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    pub command: Option<String>,
    pub default_args: Option<Vec<String>>,
    pub resume_flag: Option<String>,
    pub continue_flag: Option<String>,
    pub session_id_strategy: Option<String>,
    pub alert_patterns: Option<AlertPatterns>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlertPatterns {
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default = "default_true")]
    pub cleanup_on_exit: bool,

    #[serde(default = "default_workspace_dir")]
    pub base_dir: PathBuf,

    #[serde(default = "default_replay_buffer")]
    pub replay_buffer_bytes: usize,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            cleanup_on_exit: true,
            base_dir: default_workspace_dir(),
            replay_buffer_bytes: 65536,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_idle_timeout() -> u64 {
    5
}

fn default_workspace_dir() -> PathBuf {
    std::env::temp_dir().join("llmux-workspaces")
}

fn default_replay_buffer() -> usize {
    65536 // 64KB
}

impl Config {
    /// Load config by merging global config (~/.config/llmux/config.toml)
    /// with local config (.llmux.toml in current directory).
    pub fn load() -> Result<Self> {
        let mut config = Config::default();

        // Load global config
        if let Some(global_path) = global_config_path() {
            if global_path.exists() {
                let global = Self::load_from_file(&global_path)?;
                config.merge(global);
            }
        }

        // Load local config (overrides global)
        let local_path = PathBuf::from(".llmux.toml");
        if local_path.exists() {
            let local = Self::load_from_file(&local_path)?;
            config.merge(local);
        }

        Ok(config)
    }

    fn load_from_file(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("failed to read {}: {}", path.display(), e)))?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    fn merge(&mut self, other: Config) {
        // Merge alerts (other takes precedence for non-default values)
        self.alerts = other.alerts;

        // Merge agents
        for (name, agent_config) in other.agents {
            self.agents.insert(name, agent_config);
        }

        // Merge workspace config
        self.workspaces = other.workspaces;
    }

    /// Return the raw TOML string of the effective config
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))
    }
}

/// Return the global config path
pub fn global_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("llmux").join("config.toml"))
}

/// Return the local config path
pub fn local_config_path() -> PathBuf {
    PathBuf::from(".llmux.toml")
}
