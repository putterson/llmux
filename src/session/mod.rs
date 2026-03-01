pub mod attach;
pub(crate) mod chord;
pub mod pty;
pub mod socket;
pub mod watcher;

use crate::agent::AgentDef;
use crate::config::Config;
use crate::db::{Database, Session, SessionStatus};
use crate::error::{Error, Result};
use crate::names;
use crate::workspace;
use chrono::Utc;
use std::path::PathBuf;
use tracing::{error, info};

/// Spawn a new agent session.
/// Returns the session record. If `detach` is false, the caller should attach.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    agent: &AgentDef,
    prompt: Option<&str>,
    name: Option<&str>,
    work_dir: Option<&str>,
    sources: &[String],
    detach: bool,
    extra_agent_args: &[String],
    config: &Config,
) -> Result<Session> {
    let db = Database::open()?;

    // Generate or validate name
    let session_name = match name {
        Some(n) => {
            if db.name_exists(n)? {
                return Err(Error::Other(format!("session name '{}' already in use", n)));
            }
            n.to_string()
        }
        None => {
            let mut n = names::generate();
            while db.name_exists(&n)? {
                n = names::generate();
            }
            n
        }
    };

    let session_id = uuid::Uuid::new_v4().to_string();
    let agent_session_id = uuid::Uuid::new_v4().to_string();

    // Determine working directory
    let (effective_work_dir, is_workspace, source_dirs) = if !sources.is_empty() {
        let ws_dir = workspace::create_workspace(
            &config.workspaces.base_dir,
            &session_name,
            sources,
        )?;
        (ws_dir, true, Some(sources.to_vec()))
    } else {
        let dir = match work_dir {
            Some(d) => {
                let p = PathBuf::from(d);
                p.canonicalize().map_err(|_| Error::PathNotFound(p))?
            }
            None => std::env::current_dir()?,
        };
        (dir, false, None)
    };

    // Build socket path
    let uid = nix::unistd::getuid();
    let socket_dir = PathBuf::from(format!("/tmp/llmux-{}", uid));
    std::fs::create_dir_all(&socket_dir)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&socket_dir, std::fs::Permissions::from_mode(0o700))?;
    let socket_path = socket_dir.join(format!("{}.sock", session_id));

    let session = Session {
        id: session_id.clone(),
        name: session_name.clone(),
        agent_type: agent.name.clone(),
        agent_session_id: Some(agent_session_id.clone()),
        pid: None,
        socket_path: Some(socket_path.to_string_lossy().to_string()),
        work_dir: effective_work_dir.to_string_lossy().to_string(),
        source_dirs,
        is_workspace,
        status: SessionStatus::Running,
        exit_code: None,
        started_at: Utc::now(),
        stopped_at: None,
        initial_prompt: prompt.map(|s| s.to_string()),
        extra: None,
    };

    db.insert_session(&session)?;

    // Build agent command args
    let args = agent.build_spawn_args(prompt, Some(&agent_session_id), extra_agent_args);

    info!(
        session_id = %session_id,
        name = %session_name,
        agent = %agent.name,
        "spawning agent session"
    );

    // Instead of fork(), spawn a new llmux process with the hidden `_serve` subcommand.
    // This avoids the "fork in multithreaded process" SEGFAULT on macOS.
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("_serve");
    cmd.arg("--session-id").arg(&session_id);
    cmd.arg("--command").arg(&agent.command);
    cmd.arg("--work-dir").arg(&effective_work_dir);
    cmd.arg("--socket-path").arg(&socket_path);

    for arg in &args {
        cmd.arg("--agent-arg").arg(arg);
    }

    // Serialize alert patterns
    for pattern in &agent.alert_patterns {
        cmd.arg("--alert-pattern").arg(pattern);
    }

    // Pass config values
    cmd.arg("--replay-buffer-size")
        .arg(config.workspaces.replay_buffer_bytes.to_string());
    if config.alerts.enabled {
        cmd.arg("--alerts-enabled");
    }
    cmd.arg("--idle-timeout-secs")
        .arg(config.alerts.idle_timeout_secs.to_string());
    if config.alerts.terminal_bell {
        cmd.arg("--alerts-bell");
    }
    if config.alerts.desktop_notification {
        cmd.arg("--alerts-desktop");
    }
    if !config.alerts.custom_command.is_empty() {
        cmd.arg("--alerts-custom-command")
            .arg(&config.alerts.custom_command);
    }

    // Detach: redirect stdio
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    // Redirect stderr to a log file for diagnostics
    let stderr_log_path = socket_path.with_extension("log");
    let stderr_file = std::fs::File::create(&stderr_log_path)?;
    cmd.stderr(std::process::Stdio::from(stderr_file));

    // Remove env vars that prevent agent nesting
    cmd.env_remove("CLAUDECODE");
    cmd.env_remove("CLAUDE_CODE_ENTRYPOINT");

    let child = cmd.spawn()?;
    let child_pid = child.id();

    db.update_session_pid(&session_id, child_pid as i64)?;

    let mut session = session;
    session.pid = Some(child_pid as i64);

    if detach {
        eprintln!("Session '{}' spawned (detached)", session_name);
        eprintln!("  Attach with: llmux attach {}", session_name);
    }

    Ok(session)
}

/// Entry point for the session server subprocess (called via `llmux _serve`).
/// This runs in a clean process — no inherited tokio runtime or thread pool.
pub fn run_serve(args: &ServeArgs) -> Result<()> {
    // Set up a log file for the session server
    let socket_path = PathBuf::from(&args.socket_path);
    let log_path = socket_path.with_extension("log");

    // Install panic hook
    let log_path_panic = log_path.clone();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&log_path_panic)
        {
            use std::io::Write;
            let _ = writeln!(f, "PANIC: {}", info);
            let bt = std::backtrace::Backtrace::force_capture();
            let _ = writeln!(f, "Backtrace:\n{}", bt);
        }
    }));

    // Initialize tracing to the log file
    let log_path_tracing = log_path.clone();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("llmux=info".parse().unwrap()),
        )
        .with_target(false)
        .with_writer(move || -> Box<dyn std::io::Write> {
            if let Ok(f) = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&log_path_tracing)
            {
                Box::new(f)
            } else {
                Box::new(std::io::stderr())
            }
        })
        .init();

    // Become a session leader (detach from controlling terminal)
    let _ = nix::unistd::setsid();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let result = run_session_server_async(args).await;
        if let Err(e) = result {
            error!(session_id = %args.session_id, error = %e, "session server error");
            if let Ok(db) = Database::open() {
                let _ = db.update_session_status(&args.session_id, SessionStatus::Crashed, None);
            }
        }
    });

    Ok(())
}

async fn run_session_server_async(args: &ServeArgs) -> Result<()> {
    let work_dir = PathBuf::from(&args.work_dir);
    let socket_path = PathBuf::from(&args.socket_path);

    // Create PTY and spawn agent
    let (pty_handle, mut exit_rx) =
        pty::spawn_agent(&args.command, &args.agent_args, &work_dir)?;

    // Start the socket server
    let socket_server = socket::SocketServer::new(
        &socket_path,
        args.session_id.clone(),
        pty_handle.clone(),
        args.replay_buffer_size,
    )
    .await?;

    // Build alert config
    let alert_config = crate::config::AlertConfig {
        enabled: args.alerts_enabled,
        idle_timeout_secs: args.idle_timeout_secs,
        terminal_bell: args.alerts_bell,
        desktop_notification: args.alerts_desktop,
        custom_command: args.alerts_custom_command.clone(),
    };

    // Start the input watcher
    let _watcher_handle = if args.alerts_enabled {
        let watcher = watcher::InputWatcher::new(
            args.session_id.clone(),
            args.alert_patterns.clone(),
            alert_config,
            socket_server.clone(),
        );
        Some(watcher.start(pty_handle.subscribe_output(), socket_server.subscribe_input()))
    } else {
        None
    };

    // Run the socket server
    let server_handle = tokio::spawn({
        let server = socket_server.clone();
        async move { server.run().await }
    });

    // Wait for agent to exit
    let exit_code = exit_rx.recv().await.unwrap_or(None);

    info!(session_id = %args.session_id, exit_code = ?exit_code, "agent exited");

    // Notify connected clients
    socket_server.broadcast_session_end(exit_code).await;

    // Update database
    let db = Database::open()?;
    db.update_session_status(&args.session_id, SessionStatus::Stopped, exit_code)?;

    // Clean up socket
    let _ = std::fs::remove_file(&socket_path);

    // Shut down socket server
    socket_server.shutdown().await;
    let _ = server_handle.await;

    Ok(())
}

/// Arguments for the `_serve` hidden subcommand
#[derive(Debug, Clone, clap::Parser)]
pub struct ServeArgs {
    #[arg(long)]
    pub session_id: String,
    #[arg(long)]
    pub command: String,
    #[arg(long)]
    pub work_dir: String,
    #[arg(long)]
    pub socket_path: String,
    #[arg(long = "agent-arg", allow_hyphen_values = true)]
    pub agent_args: Vec<String>,
    #[arg(long = "alert-pattern", allow_hyphen_values = true)]
    pub alert_patterns: Vec<String>,
    #[arg(long, default_value = "65536")]
    pub replay_buffer_size: usize,
    #[arg(long, default_value = "false")]
    pub alerts_enabled: bool,
    #[arg(long, default_value = "5")]
    pub idle_timeout_secs: u64,
    #[arg(long, default_value = "false")]
    pub alerts_bell: bool,
    #[arg(long, default_value = "false")]
    pub alerts_desktop: bool,
    #[arg(long, default_value = "")]
    pub alerts_custom_command: String,
}

/// Kill a session by name or ID
pub fn kill_session(query: &str, signal_name: &str) -> Result<()> {
    let db = Database::open()?;
    let session = db.find_session(query)?;

    if session.status != SessionStatus::Running {
        return Err(Error::SessionNotRunning(session.name.clone()));
    }

    let pid = session
        .pid
        .ok_or_else(|| Error::Other(format!("session '{}' has no PID", session.name)))?;

    let signal = parse_signal(signal_name)?;
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), signal)?;

    info!(name = %session.name, pid = pid, signal = %signal_name, "killed session");
    eprintln!(
        "Sent {} to session '{}' (PID {})",
        signal_name, session.name, pid
    );

    Ok(())
}

/// Kill all running sessions
pub fn kill_all(signal_name: &str) -> Result<usize> {
    let db = Database::open()?;
    let running = db.list_sessions(false)?;
    let signal = parse_signal(signal_name)?;
    let mut killed = 0;

    for session in &running {
        if let Some(pid) = session.pid {
            if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), signal).is_ok() {
                killed += 1;
            }
        }
    }

    eprintln!("Sent {} to {} session(s)", signal_name, killed);
    Ok(killed)
}

fn parse_signal(name: &str) -> Result<nix::sys::signal::Signal> {
    use nix::sys::signal::Signal;
    match name.to_uppercase().trim_start_matches("SIG") {
        "TERM" => Ok(Signal::SIGTERM),
        "KILL" => Ok(Signal::SIGKILL),
        "INT" => Ok(Signal::SIGINT),
        "HUP" => Ok(Signal::SIGHUP),
        "QUIT" => Ok(Signal::SIGQUIT),
        "USR1" => Ok(Signal::SIGUSR1),
        "USR2" => Ok(Signal::SIGUSR2),
        other => Err(Error::Other(format!("unknown signal: {}", other))),
    }
}
