use std::ffi::OsString;
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

/// All supported export format extensions. Used by `is_supported_path` for
/// import filtering and by `cleanup_stale_exports` for removing old-format files.
/// Must be updated when new `ExportFormat` variants are added.
pub const SUPPORTED_EXTENSIONS: &[&str] = &["json", "jsonl", "json.gz", "jsonl.gz"];

/// Export format, controlling the file extension and the `--format` flag
/// passed to the vestige CLI.
#[derive(Clone, Debug, clap::ValueEnum)]
pub enum ExportFormat {
    Json,
    Jsonl,
    #[value(name = "json.gz")]
    JsonGz,
    #[value(name = "jsonl.gz")]
    JsonlGz,
}

impl ExportFormat {
    /// File extension for this format (without leading dot).
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Jsonl => "jsonl",
            Self::JsonGz => "json.gz",
            Self::JsonlGz => "jsonl.gz",
        }
    }

    /// Value to pass to `vestige export --format`.
    /// Currently identical to `extension()` — kept separate in case the
    /// vestige CLI adopts different format identifiers in the future.
    pub fn vestige_flag(&self) -> &'static str {
        self.extension()
    }

    /// Check whether a path matches any supported export format.
    /// Used for import filtering, since other machines may use different formats.
    pub fn is_supported_path(path: &Path) -> bool {
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => return false,
        };
        if name.starts_with('.') {
            return false;
        }
        SUPPORTED_EXTENSIONS
            .iter()
            .any(|ext| name.ends_with(&format!(".{ext}")))
    }

    /// Build the export file path: `<sync_dir>/<filename>.<extension>`.
    pub fn export_path(&self, sync_dir: &Path, filename: &str) -> PathBuf {
        sync_dir.join(format!("{filename}.{}", self.extension()))
    }

    /// Build a temporary file path for export. Uses a dotfile prefix so the
    /// file is rejected by import filtering (`starts_with('.')`), while
    /// preserving the real extension for vestige's format inference.
    /// Syncthing may also ignore dotfiles if configured, but the import
    /// filter is the primary defense.
    pub fn tmp_path(export_file: &Path) -> PathBuf {
        let dir = export_file.parent().unwrap_or(Path::new("."));
        let name = export_file.file_name().unwrap_or_default();
        let mut tmp_name = OsString::from(".");
        tmp_name.push(name);
        dir.join(tmp_name)
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

    /// Output file stem (template). The extension is determined by --format.
    ///
    /// Supports placeholders: {hostname}, {os}, {platform}, {distro}, {user}.
    #[arg(long, default_value = "{hostname}")]
    pub filename: String,

    /// Export format. Controls the file extension and the format passed to
    /// the vestige CLI for export. Import auto-detects format.
    #[arg(long, default_value = "jsonl")]
    pub format: ExportFormat,

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_returns_correct_values() {
        assert_eq!(ExportFormat::Json.extension(), "json");
        assert_eq!(ExportFormat::Jsonl.extension(), "jsonl");
        assert_eq!(ExportFormat::JsonGz.extension(), "json.gz");
        assert_eq!(ExportFormat::JsonlGz.extension(), "jsonl.gz");
    }

    #[test]
    fn export_path_uses_extension() {
        let dir = Path::new("/sync");
        assert_eq!(
            ExportFormat::Json.export_path(dir, "host"),
            PathBuf::from("/sync/host.json")
        );
        assert_eq!(
            ExportFormat::JsonGz.export_path(dir, "host"),
            PathBuf::from("/sync/host.json.gz")
        );
    }

    #[test]
    fn tmp_path_prepends_dot() {
        assert_eq!(
            ExportFormat::tmp_path(Path::new("/sync/host.json")),
            PathBuf::from("/sync/.host.json")
        );
        assert_eq!(
            ExportFormat::tmp_path(Path::new("/sync/host.json.gz")),
            PathBuf::from("/sync/.host.json.gz")
        );
        // Bare filename (no parent directory component)
        assert_eq!(
            ExportFormat::tmp_path(Path::new("host.jsonl")),
            PathBuf::from(".host.jsonl")
        );
    }

    #[test]
    fn is_supported_path_covers_all_variants() {
        for ext in SUPPORTED_EXTENSIONS {
            let path = PathBuf::from(format!("host.{ext}"));
            assert!(
                ExportFormat::is_supported_path(&path),
                "is_supported_path should match .{ext}"
            );
        }
    }

    #[test]
    fn is_supported_path_rejects_unsupported() {
        assert!(!ExportFormat::is_supported_path(Path::new("host.txt")));
        assert!(!ExportFormat::is_supported_path(Path::new("host.csv")));
        assert!(!ExportFormat::is_supported_path(Path::new(".host.json"))); // dotfile
    }
}
