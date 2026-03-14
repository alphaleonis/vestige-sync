use std::path::PathBuf;

use clap::Parser;

/// Vestige MCP wrapper that syncs memories between machines.
///
/// Proxies MCP JSON-RPC stdio to a child vestige-mcp process while
/// periodically exporting/importing memories through a shared sync directory.
#[derive(Parser, Debug)]
#[command(name = "vestige-sync", version)]
pub struct Args {
    /// Directory where export files are written and watched.
    #[arg(long)]
    pub sync_dir: PathBuf,

    /// Output file stem (template). The .json extension is appended automatically.
    ///
    /// Supports placeholders: {hostname}, {os}, {user}.
    #[arg(long, default_value = "{hostname}")]
    pub filename: String,

    /// Export interval in seconds.
    #[arg(long, default_value_t = 900)]
    pub export_interval: u64,

    /// Poll the sync directory for changes at this interval (seconds) instead of
    /// using filesystem notifications. Useful for network mounts or unreliable
    /// filesystem event sources.
    #[arg(long)]
    pub poll_interval: Option<u64>,

    /// Run a final export before shutting down, to capture any memories
    /// created during this session.
    #[arg(long)]
    pub export_on_exit: bool,

    /// Custom data directory for vestige. Forwarded to both vestige-mcp
    /// (--data-dir) and the vestige CLI (--data-dir).
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Import all other machines' export files before starting vestige-mcp.
    #[arg(long)]
    pub restore_on_start: bool,

    /// Path to the vestige-mcp binary (MCP server).
    #[arg(long, default_value = "vestige-mcp")]
    pub vestige_bin: PathBuf,

    /// Path to the vestige CLI binary (used for export/import).
    #[arg(long, default_value = "vestige")]
    pub vestige_cli: PathBuf,

    /// Arguments forwarded to vestige-mcp (pass after --).
    #[arg(last = true)]
    pub vestige_args: Vec<String>,
}
