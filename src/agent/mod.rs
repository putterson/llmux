pub mod claude;
pub mod cursor;

use crate::config::AgentConfig;
use std::collections::HashMap;

/// Definition of an agent that can be spawned
#[derive(Debug, Clone)]
pub struct AgentDef {
    pub name: String,
    pub command: String,
    pub default_args: Vec<String>,
    pub prompt_flag: Option<String>,
    pub resume_flag: Option<String>,
    #[allow(dead_code)]
    pub continue_flag: Option<String>,
    pub session_id_flag: Option<String>,
    pub session_id_strategy: SessionIdStrategy,
    pub alert_patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionIdStrategy {
    /// Agent accepts a session ID flag at spawn time (e.g. Claude --session-id)
    Flag,
    /// Session ID must be parsed from agent output or tracked manually
    Manual,
    /// No session ID concept
    None,
}

impl AgentDef {
    /// Build the command arguments for spawning this agent
    pub fn build_spawn_args(
        &self,
        prompt: Option<&str>,
        session_id: Option<&str>,
        extra_args: &[String],
    ) -> Vec<String> {
        let mut args = self.default_args.clone();

        // Add session ID if strategy supports it
        if self.session_id_strategy == SessionIdStrategy::Flag {
            if let (Some(flag), Some(sid)) = (&self.session_id_flag, session_id) {
                args.push(flag.clone());
                args.push(sid.to_string());
            }
        }

        // Add extra user-provided args
        args.extend(extra_args.iter().cloned());

        // Add prompt if provided
        if let Some(p) = prompt {
            if let Some(ref flag) = self.prompt_flag {
                args.push(flag.clone());
            }
            args.push(p.to_string());
        }

        args
    }

    /// Build the command arguments for resuming a session
    pub fn build_resume_args(
        &self,
        agent_session_id: &str,
        extra_args: &[String],
    ) -> Option<Vec<String>> {
        let flag = self.resume_flag.as_ref()?;
        let mut args = self.default_args.clone();
        args.push(flag.clone());
        args.push(agent_session_id.to_string());
        args.extend(extra_args.iter().cloned());
        Some(args)
    }
}

/// Get the built-in agent registry, optionally merged with config overrides
pub fn builtin_agents(config_agents: &HashMap<String, AgentConfig>) -> HashMap<String, AgentDef> {
    let mut agents = HashMap::new();

    // Built-in: Claude Code
    agents.insert("claude".to_string(), claude::agent_def());

    // Built-in: Cursor CLI
    agents.insert("cursor".to_string(), cursor::agent_def());

    // Apply config overrides and add custom agents
    for (name, config) in config_agents {
        if let Some(existing) = agents.get_mut(name) {
            // Override existing agent
            if let Some(ref cmd) = config.command {
                existing.command = cmd.clone();
            }
            if let Some(ref args) = config.default_args {
                existing.default_args = args.clone();
            }
            if let Some(ref flag) = config.resume_flag {
                existing.resume_flag = Some(flag.clone());
            }
            if let Some(ref patterns) = config.alert_patterns {
                existing.alert_patterns = patterns.patterns.clone();
            }
        } else {
            // Custom agent from config
            agents.insert(
                name.clone(),
                AgentDef {
                    name: name.clone(),
                    command: config
                        .command
                        .clone()
                        .unwrap_or_else(|| name.clone()),
                    default_args: config.default_args.clone().unwrap_or_default(),
                    prompt_flag: None,
                    resume_flag: config.resume_flag.clone(),
                    continue_flag: config.continue_flag.clone(),
                    session_id_flag: None,
                    session_id_strategy: match config
                        .session_id_strategy
                        .as_deref()
                    {
                        Some("flag") => SessionIdStrategy::Flag,
                        Some("manual") => SessionIdStrategy::Manual,
                        _ => SessionIdStrategy::None,
                    },
                    alert_patterns: config
                        .alert_patterns
                        .as_ref()
                        .map(|p| p.patterns.clone())
                        .unwrap_or_default(),
                },
            );
        }
    }

    agents
}

/// Preferred order when auto-detecting which agent to use.
const DEFAULT_AGENT_PRIORITY: &[&str] = &["claude", "cursor"];

/// Resolve an agent by name, or auto-detect the first available one.
///
/// When `name` is `None`, checks each known agent in priority order
/// (claude, cursor, then config-defined agents) and returns the first
/// whose command exists in PATH.
///
/// When `name` is `Some` but doesn't match a known agent, it's treated
/// as an arbitrary CLI command (e.g. `--agent aider` or `--agent "my-tool --flag"`).
pub fn resolve_agent(
    name: Option<&str>,
    config_agents: &HashMap<String, AgentConfig>,
) -> crate::error::Result<AgentDef> {
    let agents = builtin_agents(config_agents);

    if let Some(name) = name {
        if let Some(def) = agents.get(name) {
            return Ok(def.clone());
        }

        // Treat as an arbitrary CLI command
        let mut parts = shell_words(name);
        if parts.is_empty() {
            return Err(crate::error::Error::AgentNotFound(name.to_string()));
        }

        let command = parts.remove(0);
        let display_name = command
            .rsplit('/')
            .next()
            .unwrap_or(&command)
            .to_string();

        return Ok(AgentDef {
            name: display_name,
            command,
            default_args: parts,
            prompt_flag: None,
            resume_flag: None,
            continue_flag: None,
            session_id_flag: None,
            session_id_strategy: SessionIdStrategy::None,
            alert_patterns: vec![],
        });
    }

    // No agent specified — auto-detect first available in priority order
    for &key in DEFAULT_AGENT_PRIORITY {
        if let Some(def) = agents.get(key) {
            if command_exists(&def.command) {
                return Ok(def.clone());
            }
        }
    }

    // Check config-defined agents not already in the priority list
    for (key, def) in &agents {
        if !DEFAULT_AGENT_PRIORITY.contains(&key.as_str()) && command_exists(&def.command) {
            return Ok(def.clone());
        }
    }

    Err(crate::error::Error::Other(
        "no agent found in PATH; install claude or cursor, or specify --agent".to_string(),
    ))
}

fn command_exists(cmd: &str) -> bool {
    let path = std::path::Path::new(cmd);
    if path.is_absolute() {
        return path.exists();
    }
    if let Some(dirs) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&dirs) {
            if dir.join(cmd).is_file() {
                return true;
            }
        }
    }
    false
}

fn shell_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escape_next = false;

    for c in s.chars() {
        if escape_next {
            current.push(c);
            escape_next = false;
            continue;
        }
        match c {
            '\\' if !in_single_quote => escape_next = true,
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}
