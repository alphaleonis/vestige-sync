use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use notify_debouncer_mini::{new_debouncer, DebouncedEvent, DebouncedEventKind};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time;

type NotifyResult = Result<Vec<DebouncedEvent>, notify_debouncer_mini::notify::Error>;

/// Run `vestige import <file>`, logging the result to stderr.
async fn import_file(
    vestige_cli: &Path,
    file: &Path,
    data_dir: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new(vestige_cli);
    if let Some(dir) = data_dir {
        cmd.args(["--data-dir", &dir.to_string_lossy()]);
    }
    let output = cmd
        .args(["import", &file.to_string_lossy()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await?;

    if output.status.success() {
        eprintln!(
            "vestige-sync: imported {}",
            file.file_name().unwrap_or_default().to_string_lossy()
        );
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "vestige import exited with {}: {}",
            output.status,
            stderr.trim()
        )
        .into());
    }

    Ok(())
}

/// List `.json` files in sync_dir, excluding our own export file and `.tmp` files.
fn list_import_candidates(sync_dir: &Path, own_export_file: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(sync_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("vestige-sync: failed to read sync dir: {e}");
            return Vec::new();
        }
    };

    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "json")
                && path != own_export_file
                && !path.to_string_lossy().ends_with(".json.tmp")
        })
        .collect()
}

/// Import all other machines' export files (one-shot, for --restore-on-start).
pub async fn import_all(
    vestige_cli: &Path,
    sync_dir: &Path,
    own_export_file: &Path,
    data_dir: Option<&Path>,
) {
    let candidates = list_import_candidates(sync_dir, own_export_file);

    if candidates.is_empty() {
        eprintln!("vestige-sync: restore: no files to import");
        return;
    }

    for file in &candidates {
        eprintln!(
            "vestige-sync: restore: importing {}",
            file.file_name().unwrap_or_default().to_string_lossy()
        );
        if let Err(e) = import_file(vestige_cli, file, data_dir).await {
            eprintln!("vestige-sync: restore: import failed: {e}");
        }
    }
}

/// Polling-based import loop. Scans sync_dir every `poll_secs` and imports
/// files whose mtime has changed since the last scan.
pub async fn import_poll_loop(
    vestige_cli: PathBuf,
    sync_dir: PathBuf,
    own_export_file: PathBuf,
    poll_secs: u64,
    data_dir: Option<PathBuf>,
) {
    let mut known_mtimes: HashMap<PathBuf, SystemTime> = HashMap::new();
    let mut interval = time::interval(Duration::from_secs(poll_secs));

    loop {
        interval.tick().await;

        let candidates = list_import_candidates(&sync_dir, &own_export_file);

        for file in candidates {
            let mtime = match std::fs::metadata(&file).and_then(|m| m.modified()) {
                Ok(t) => t,
                Err(_) => continue,
            };

            let needs_import = match known_mtimes.get(&file) {
                Some(prev) => mtime > *prev,
                None => true, // first time seeing this file
            };

            if needs_import {
                known_mtimes.insert(file.clone(), mtime);
                if let Err(e) = import_file(&vestige_cli, &file, data_dir.as_deref()).await {
                    eprintln!("vestige-sync: poll import failed: {e}");
                }
            }
        }
    }
}

/// Notify-based import watcher. Uses filesystem notifications with debouncing
/// to detect changes to other machines' export files.
pub async fn import_watch_loop(
    vestige_cli: PathBuf,
    sync_dir: PathBuf,
    own_export_file: PathBuf,
    data_dir: Option<PathBuf>,
) {
    let (tx, mut rx) = mpsc::channel::<NotifyResult>(64);

    // Create a debounced watcher with 2-second debounce window
    let mut debouncer = match new_debouncer(Duration::from_secs(2), move |events| {
        // This closure runs on the notify thread — send events to our async task
        let _ = tx.blocking_send(events);
    }) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("vestige-sync: failed to create file watcher: {e}");
            eprintln!("vestige-sync: falling back to no import watching");
            // Park forever so the task doesn't exit
            std::future::pending::<()>().await;
            return;
        }
    };

    if let Err(e) = debouncer
        .watcher()
        .watch(&sync_dir, notify_debouncer_mini::notify::RecursiveMode::NonRecursive)
    {
        eprintln!("vestige-sync: failed to watch sync dir: {e}");
        eprintln!("vestige-sync: falling back to no import watching");
        std::future::pending::<()>().await;
        return;
    }

    eprintln!("vestige-sync: watching {} for changes", sync_dir.display());

    while let Some(events) = rx.recv().await {
        let events = match events {
            Ok(events) => events,
            Err(e) => {
                eprintln!("vestige-sync: watch error: {e}");
                continue;
            }
        };

        // Collect unique files that were modified
        let mut to_import: Vec<PathBuf> = events
            .into_iter()
            .filter(|e| e.kind == DebouncedEventKind::Any)
            .map(|e| e.path)
            .filter(|path| {
                path.extension().is_some_and(|ext| ext == "json")
                    && *path != own_export_file
                    && !path.to_string_lossy().ends_with(".json.tmp")
            })
            .collect();

        to_import.sort();
        to_import.dedup();

        for file in to_import {
            if let Err(e) = import_file(&vestige_cli, &file, data_dir.as_deref()).await {
                eprintln!("vestige-sync: watch import failed: {e}");
            }
        }
    }
}
