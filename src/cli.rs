use std::path::{Path, PathBuf};

use clap::Parser;

/// Expand a leading `~` or `~/` to the user's home directory.
fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.as_os_str().to_string_lossy();
    if s == "~" || s.starts_with("~/") {
        match dirs::home_dir() {
            Some(home) => {
                if s == "~" {
                    home
                } else {
                    home.join(&s[2..])
                }
            }
            None => {
                eprintln!("warning: could not determine home directory, '~' not expanded");
                path.to_path_buf()
            }
        }
    } else {
        path.to_path_buf()
    }
}

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
    /// Supports placeholders: {hostname}, {os}, {platform}, {distro}, {user}.
    #[arg(long, default_value = "{hostname}")]
    pub filename: String,

    /// Export interval in seconds.
    #[arg(long, default_value_t = 900, value_parser = clap::value_parser!(u64).range(1..))]
    pub export_interval: u64,

    /// Poll the sync directory for changes at this interval (seconds) instead of
    /// using filesystem notifications. Useful for network mounts or unreliable
    /// filesystem event sources.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub poll_interval: Option<u64>,

    /// Run a final export before shutting down, to capture any memories
    /// created during this session.
    #[arg(long)]
    pub export_on_exit: bool,

    /// Path to the vestige database file. Forwarded to both vestige-mcp
    /// and the vestige CLI as --data-dir.
    #[arg(long)]
    pub db_path: Option<PathBuf>,

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

impl Args {
    /// Expand `~` in all path arguments to the user's home directory.
    pub fn resolve_paths(&mut self) {
        self.sync_dir = expand_tilde(&self.sync_dir);
        self.db_path = self.db_path.as_deref().map(expand_tilde);
        self.vestige_bin = expand_tilde(&self.vestige_bin);
        self.vestige_cli = expand_tilde(&self.vestige_cli);
    }
}
