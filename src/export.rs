use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::watch;
use tokio::time;

/// Run the export loop: periodically export memories to the sync directory.
///
/// The first export fires immediately, then repeats every `interval_secs`.
/// Stops gracefully when `shutdown` is signaled, allowing any in-flight
/// export subprocess to complete.
pub async fn export_loop(
    vestige_cli: PathBuf,
    export_file: PathBuf,
    interval_secs: u64,
    data_dir: Option<PathBuf>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut interval = time::interval(Duration::from_secs(interval_secs));

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = shutdown.changed() => break,
        }

        if let Err(e) = export_once(&vestige_cli, &export_file, data_dir.as_deref()).await {
            eprintln!("vestige-sync: export failed: {e}");
        }
    }
}

/// Run a single export cycle:
/// 1. `vestige export <tmp_file>`
/// 2. Compare tmp with existing file
/// 3. Rename if different, delete tmp if identical
pub async fn export_once(
    vestige_cli: &Path,
    export_file: &Path,
    data_dir: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp_file = export_file.with_extension("json.tmp");

    // Run vestige export
    let mut cmd = Command::new(vestige_cli);
    if let Some(dir) = data_dir {
        cmd.args(["--data-dir", &dir.to_string_lossy()]);
    }
    let output = cmd
        .args(["export", &tmp_file.to_string_lossy()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "vestige export exited with {}: {}",
            output.status,
            stderr.trim()
        )
        .into());
    }

    // Check if tmp file was actually created
    if !tmp_file.exists() {
        return Err("vestige export did not create output file".into());
    }

    // Restrict permissions — memory data may contain sensitive information
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&tmp_file, std::fs::Permissions::from_mode(0o600)).await?;
    }

    // Compare with existing export file. Byte-level comparison works because our
    // vestige fork (alphaleonis/vestige@decaf) guarantees stable export ordering
    // (created_at ASC, id ASC). Without stable ordering, identical data would
    // produce different bytes, causing unnecessary Syncthing syncs on every cycle.
    let existing = match tokio::fs::read(&export_file).await {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };

    if let Some(existing) = existing {
        let new = tokio::fs::read(&tmp_file).await?;
        if existing == new {
            // Identical — delete tmp, preserve original mtime
            tokio::fs::remove_file(&tmp_file).await?;
            eprintln!("vestige-sync: export unchanged, skipped");
            return Ok(());
        }
    }

    // Different (or first export) — atomic rename
    tokio::fs::rename(&tmp_file, &export_file).await?;
    eprintln!("vestige-sync: export updated {}", export_file.display());

    Ok(())
}
