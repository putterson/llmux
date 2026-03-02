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
    #[command(visible_alias = "s")]
    Spawn {
        /// Initial prompt to send to the agent
        prompt: Option<String>,

        /// Agent to use: a known name (claude, cursor) or any CLI command
        ///
        /// When omitted, auto-detects the first available agent in PATH
        /// (checks claude, cursor, then config-defined agents in order).
        ///
        /// Predefined agents have built-in prompt handling and resume support.
        /// Any other value is treated as a CLI command to run directly:
        ///   -a aider
        ///   -a "codex --model o4-mini"
        ///   -a /usr/local/bin/my-agent
        #[arg(short = 'a', long = "agent", verbatim_doc_comment)]
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
    #[command(visible_alias = "l", alias = "list")]
    Ls {
        /// Show all sessions (including stopped/crashed)
        #[arg(long = "all")]
        all: bool,

        /// Output as JSON
        #[arg(long = "json")]
        json: bool,
    },

    /// Attach to a running session
    #[command(visible_alias = "a")]
    Attach {
        /// Session name or ID (prefix match supported; omit to attach if only one running)
        name_or_id: Option<String>,
    },

    /// Show session history
    #[command(visible_alias = "H")]
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
    #[command(visible_alias = "r")]
    Resume {
        /// Session name or ID to resume
        name_or_id: Option<String>,

        /// Resume the latest session
        #[arg(long = "latest")]
        latest: bool,

        /// Override agent: a known name (claude, cursor) or any CLI command
        ///
        /// When omitted, uses the original session's agent type.
        #[arg(short = 'a', long = "agent", verbatim_doc_comment)]
        agent: Option<String>,

        /// Detach immediately after spawning
        #[arg(long = "detach")]
        detach: bool,
    },

    /// Kill a running session
    #[command(visible_alias = "k")]
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
    #[command(visible_alias = "c")]
    Clean {
        /// Show what would be cleaned without actually doing it
        #[arg(long = "dry-run")]
        dry_run: bool,

        /// Also clean up workspace directories
        #[arg(long = "workspaces")]
        workspaces: bool,
    },

    /// Spawn a session in an empty temporary directory
    #[command(visible_alias = "t")]
    Temp {
        /// Initial prompt to send to the agent
        prompt: Option<String>,

        /// Agent to use: a known name (claude, cursor) or any CLI command
        ///
        /// When omitted, auto-detects the first available agent in PATH.
        #[arg(short = 'a', long = "agent", verbatim_doc_comment)]
        agent: Option<String>,

        /// Session name (auto-generated if omitted)
        #[arg(short = 'n', long = "name")]
        name: Option<String>,

        /// Detach immediately after spawning
        #[arg(long = "detach")]
        detach: bool,

        /// Additional arguments to pass to the agent
        #[arg(long = "agent-args")]
        agent_args: Option<String>,
    },

    /// Debug terminal input — shows hex values for every keypress
    #[command(visible_alias = "di")]
    DebugInput,

    /// Internal: run a session server (not for direct use)
    #[command(hide = true)]
    #[command(name = "_serve")]
    Serve(crate::session::ServeArgs),
}
