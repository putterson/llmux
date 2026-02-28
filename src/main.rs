mod agent;
mod cli;
mod config;
mod db;
mod error;
mod names;
mod session;
mod workspace;

use crate::cli::{Cli, Commands};
use crate::config::Config;
use crate::db::{Database, SessionStatus};
use crate::error::Result;
use clap::Parser;
use tabled::{Table, Tabled};
use tracing::info;

#[derive(Tabled)]
struct SessionRow {
    #[tabled(rename = "NAME")]
    name: String,
    #[tabled(rename = "AGENT")]
    agent: String,
    #[tabled(rename = "STATUS")]
    status: String,
    #[tabled(rename = "PID")]
    pid: String,
    #[tabled(rename = "DIR")]
    dir: String,
    #[tabled(rename = "STARTED")]
    started: String,
    #[tabled(rename = "ID")]
    id: String,
}

impl From<&db::Session> for SessionRow {
    fn from(s: &db::Session) -> Self {
        let dir = shorten_path(&s.work_dir);
        let started = s.started_at.format("%Y-%m-%d %H:%M").to_string();
        let pid = s
            .pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".to_string());
        let id = s.id.get(..8).unwrap_or(&s.id).to_string();

        SessionRow {
            name: s.name.clone(),
            agent: s.agent_type.clone(),
            status: format_status(s.status, s.exit_code),
            pid,
            dir,
            started,
            id,
        }
    }
}

fn format_status(status: SessionStatus, exit_code: Option<i32>) -> String {
    match status {
        SessionStatus::Running => "running".to_string(),
        SessionStatus::Stopped => match exit_code {
            Some(0) => "stopped (0)".to_string(),
            Some(code) => format!("stopped ({})", code),
            None => "stopped".to_string(),
        },
        SessionStatus::Crashed => "crashed".to_string(),
    }
}

fn shorten_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Some(rest) = path.strip_prefix(home.to_str().unwrap_or("")) {
            return format!("~{}", rest);
        }
    }
    path.to_string()
}

fn main() {
    let cli = Cli::parse();

    // Skip default tracing init for _serve (it sets up its own file-based tracing)
    if !matches!(cli.command, Commands::Serve(_)) {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive("llmux=info".parse().unwrap()),
            )
            .with_target(false)
            .with_writer(std::io::stderr)
            .init();
    }

    if let Err(e) = run(cli) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    // _serve handles its own config; avoid loading for it
    if let Commands::Serve(ref serve_args) = cli.command {
        return session::run_serve(serve_args);
    }

    let config = Config::load()?;

    match cli.command {
        Commands::Spawn {
            prompt,
            agent,
            name,
            dir,
            source,
            detach,
            agent_args,
        } => cmd_spawn(config, prompt, agent, name, dir, source, detach, agent_args),

        Commands::Ls { all, json } => cmd_list(all, json),

        Commands::Attach { name_or_id } => cmd_attach(name_or_id),

        Commands::History { limit, agent, json } => cmd_history(limit, agent, json),

        Commands::Resume {
            name_or_id,
            latest,
            agent: agent_override,
            detach,
        } => cmd_resume(config, name_or_id, latest, agent_override, detach),

        Commands::Kill {
            name_or_id,
            signal,
            all,
        } => cmd_kill(name_or_id, signal, all),

        Commands::Config { path, raw } => cmd_config(config, path, raw),

        Commands::Clean { dry_run, workspaces } => cmd_clean(config, dry_run, workspaces),

        Commands::Serve(serve_args) => session::run_serve(&serve_args),
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_spawn(
    config: Config,
    prompt: Option<String>,
    agent_name: Option<String>,
    name: Option<String>,
    dir: Option<String>,
    sources: Vec<String>,
    detach: bool,
    agent_args: Option<String>,
) -> Result<()> {
    let agent_def = agent::resolve_agent(agent_name.as_deref(), &config.agents)?;

    let extra_args: Vec<String> = agent_args
        .as_deref()
        .map(shell_words)
        .unwrap_or_default();

    let session = session::spawn(
        &agent_def,
        prompt.as_deref(),
        name.as_deref(),
        dir.as_deref(),
        &sources,
        detach,
        &extra_args,
        &config,
    )?;

    if !detach {
        // Small delay to let the session server start
        std::thread::sleep(std::time::Duration::from_millis(200));
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async {
            let socket_path = session.socket_path.as_ref().unwrap();
            session::attach::attach(std::path::Path::new(socket_path), &session.name).await
        })?;
    }

    Ok(())
}

/// Simple shell word splitting (handles quoting)
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

fn cmd_list(all: bool, json: bool) -> Result<()> {
    let db = Database::open()?;
    db.reap_dead_sessions()?;
    let sessions = db.list_sessions(all)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }

    if sessions.is_empty() {
        if all {
            eprintln!("No sessions found.");
        } else {
            eprintln!("No running sessions. Use --all to see all sessions.");
        }
        return Ok(());
    }

    let rows: Vec<SessionRow> = sessions.iter().map(SessionRow::from).collect();
    let table = Table::new(rows).to_string();
    println!("{}", table);

    Ok(())
}

fn cmd_attach(name_or_id: Option<String>) -> Result<()> {
    let db = Database::open()?;
    db.reap_dead_sessions()?;

    let session = match name_or_id {
        Some(query) => db.find_session(&query)?,
        None => db.get_sole_running_session()?.ok_or_else(|| {
            error::Error::Other(
                "multiple running sessions — specify a name or ID".to_string(),
            )
        })?,
    };

    if session.status != SessionStatus::Running {
        return Err(error::Error::SessionNotRunning(session.name));
    }

    let socket_path = session.socket_path.as_ref().ok_or_else(|| {
        error::Error::Socket(format!("session '{}' has no socket path", session.name))
    })?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        session::attach::attach(std::path::Path::new(socket_path), &session.name).await
    })
}

fn cmd_history(limit: usize, agent: Option<String>, json: bool) -> Result<()> {
    let db = Database::open()?;
    let sessions = db.history(limit, agent.as_deref())?;

    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }

    if sessions.is_empty() {
        eprintln!("No session history.");
        return Ok(());
    }

    let rows: Vec<SessionRow> = sessions.iter().map(SessionRow::from).collect();
    let table = Table::new(rows).to_string();
    println!("{}", table);

    Ok(())
}

fn cmd_resume(
    config: Config,
    name_or_id: Option<String>,
    latest: bool,
    agent_override: Option<String>,
    detach: bool,
) -> Result<()> {
    let db = Database::open()?;

    let old_session = if latest {
        db.history(1, None)?
            .into_iter()
            .next()
            .ok_or_else(|| error::Error::SessionNotFound("no sessions in history".to_string()))?
    } else {
        let query = name_or_id.ok_or_else(|| {
            error::Error::Other("specify a session name/ID or use --latest".to_string())
        })?;
        db.find_session(&query)?
    };

    let agent_type = agent_override
        .as_deref()
        .unwrap_or(&old_session.agent_type);
    let agent_def = agent::resolve_agent(Some(agent_type), &config.agents)?;

    let agent_session_id = old_session.agent_session_id.as_ref().ok_or_else(|| {
        error::Error::Other(format!(
            "session '{}' has no agent session ID — cannot resume",
            old_session.name
        ))
    })?;

    let resume_args = agent_def
        .build_resume_args(agent_session_id, &[])
        .ok_or_else(|| {
            error::Error::Other(format!(
                "agent '{}' does not support resume",
                agent_def.name
            ))
        })?;

    info!(
        old_session = %old_session.name,
        agent_session_id = %agent_session_id,
        "resuming session"
    );

    let session = session::spawn(
        &agent_def,
        None,
        None,
        Some(&old_session.work_dir),
        &old_session.source_dirs.unwrap_or_default(),
        detach,
        &resume_args,
        &config,
    )?;

    // Copy agent_session_id from old session
    let db = Database::open()?;
    db.update_session_agent_session_id(&session.id, agent_session_id)?;

    if !detach {
        std::thread::sleep(std::time::Duration::from_millis(200));
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async {
            let socket_path = session.socket_path.as_ref().unwrap();
            session::attach::attach(std::path::Path::new(socket_path), &session.name).await
        })?;
    }

    Ok(())
}

fn cmd_kill(name_or_id: Option<String>, signal: String, all: bool) -> Result<()> {
    let db = Database::open()?;
    db.reap_dead_sessions()?;

    if all {
        session::kill_all(&signal)?;
    } else {
        let query = name_or_id.ok_or_else(|| {
            error::Error::Other("specify a session name/ID or use --all".to_string())
        })?;
        session::kill_session(&query, &signal)?;
    }
    Ok(())
}

fn cmd_config(config: Config, path: bool, raw: bool) -> Result<()> {
    if path {
        if let Some(global) = config::global_config_path() {
            println!("Global: {}", global.display());
        }
        println!("Local:  {}", config::local_config_path().display());
        return Ok(());
    }

    if raw {
        println!("{}", config.to_toml()?);
        return Ok(());
    }

    println!("# Effective configuration");
    println!("{}", config.to_toml()?);
    Ok(())
}

fn cmd_clean(config: Config, dry_run: bool, workspaces: bool) -> Result<()> {
    let db = Database::open()?;

    // Reap dead sessions
    let reaped = db.reap_dead_sessions()?;
    if reaped > 0 {
        eprintln!("Marked {} dead session(s) as crashed", reaped);
    }

    if !dry_run {
        let deleted = db.delete_stopped_sessions()?;
        if deleted > 0 {
            eprintln!(
                "Cleaned {} stopped/crashed session(s) from database",
                deleted
            );
        }
    } else {
        let sessions = db.list_sessions(true)?;
        let stale: Vec<_> = sessions
            .iter()
            .filter(|s| s.status != SessionStatus::Running)
            .collect();
        if !stale.is_empty() {
            eprintln!("Would clean {} session(s) from database:", stale.len());
            for s in &stale {
                eprintln!("  {} ({}, {})", s.name, s.agent_type, s.status);
            }
        }
    }

    if workspaces {
        let running = db.list_sessions(false)?;
        let active_names: Vec<String> = running.iter().map(|s| s.name.clone()).collect();
        let cleaned =
            workspace::clean_workspaces(&config.workspaces.base_dir, &active_names, dry_run)?;
        if !cleaned.is_empty() {
            if dry_run {
                eprintln!("Would clean {} workspace(s):", cleaned.len());
            } else {
                eprintln!("Cleaned {} workspace(s):", cleaned.len());
            }
            for p in &cleaned {
                eprintln!("  {}", p.display());
            }
        }
    }

    // Clean stale socket files
    let uid = nix::unistd::getuid();
    let socket_dir = std::path::PathBuf::from(format!("/tmp/llmux-{}", uid));
    if socket_dir.exists() {
        let running = db.list_sessions(false)?;
        let active_sockets: std::collections::HashSet<String> = running
            .iter()
            .filter_map(|s| s.socket_path.clone())
            .collect();

        if let Ok(entries) = std::fs::read_dir(&socket_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let path_str = path.to_string_lossy().to_string();
                if path_str.ends_with(".sock") && !active_sockets.contains(&path_str) {
                    if dry_run {
                        eprintln!("Would remove stale socket: {}", path.display());
                    } else {
                        let _ = std::fs::remove_file(&path);
                        eprintln!("Removed stale socket: {}", path.display());
                    }
                }
            }
        }
    }

    if dry_run {
        eprintln!("(dry run — no changes made)");
    } else {
        eprintln!("Done.");
    }
    Ok(())
}
