use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "llmux", version, about = "Agent multiplexer — spawn, attach, and manage CLI coding agents")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Spawn a new agent session
    Spawn {
        /// Initial prompt to send to the agent
        prompt: Option<String>,

        /// Agent type (default: claude)
        #[arg(short = 'a', long = "agent")]
        agent: Option<String>,

        /// Session name (auto-generated if omitted)
        #[arg(short = 'n', long = "name")]
        name: Option<String>,

        /// Working directory (default: current directory)
        #[arg(short = 'd', long = "dir")]
        dir: Option<String>,

        /// Source directories for workspace mode (creates temp dir with symlinks)
        #[arg(short = 's', long = "source")]
        source: Vec<String>,

        /// Detach immediately after spawning
        #[arg(long = "detach")]
        detach: bool,

        /// Additional arguments to pass to the agent
        #[arg(long = "agent-args")]
        agent_args: Option<String>,
    },

    /// List running sessions
    #[command(alias = "list")]
    Ls {
        /// Show all sessions (including stopped/crashed)
        #[arg(long = "all")]
        all: bool,

        /// Output as JSON
        #[arg(long = "json")]
        json: bool,
    },

    /// Attach to a running session
    Attach {
        /// Session name or ID (prefix match supported; omit to attach if only one running)
        name_or_id: Option<String>,
    },

    /// Show session history
    History {
        /// Maximum number of entries to show
        #[arg(short = 'n', default_value = "20")]
        limit: usize,

        /// Filter by agent type
        #[arg(long = "agent")]
        agent: Option<String>,

        /// Output as JSON
        #[arg(long = "json")]
        json: bool,
    },

    /// Resume a previous session
    Resume {
        /// Session name or ID to resume
        name_or_id: Option<String>,

        /// Resume the latest session
        #[arg(long = "latest")]
        latest: bool,

        /// Override agent type
        #[arg(short = 'a', long = "agent")]
        agent: Option<String>,

        /// Detach immediately after spawning
        #[arg(long = "detach")]
        detach: bool,
    },

    /// Kill a running session
    Kill {
        /// Session name or ID
        name_or_id: Option<String>,

        /// Signal to send (default: SIGTERM)
        #[arg(long = "signal", default_value = "TERM")]
        signal: String,

        /// Kill all running sessions
        #[arg(long = "all")]
        all: bool,
    },

    /// Show configuration
    Config {
        /// Show config file paths
        #[arg(long = "path")]
        path: bool,

        /// Show raw TOML
        #[arg(long = "raw")]
        raw: bool,
    },

    /// Clean up stale sessions and workspaces
    Clean {
        /// Show what would be cleaned without actually doing it
        #[arg(long = "dry-run")]
        dry_run: bool,

        /// Also clean up workspace directories
        #[arg(long = "workspaces")]
        workspaces: bool,
    },

    /// Internal: run a session server (not for direct use)
    #[command(hide = true)]
    #[command(name = "_serve")]
    Serve(crate::session::ServeArgs),
}
