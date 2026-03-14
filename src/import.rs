use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use notify_debouncer_mini::{new_debouncer, DebouncedEvent, DebouncedEventKind};
use tokio::process::Command;
use tokio::sync::{mpsc, watch};
use tokio::time;

use crate::cli::ExportFormat;

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

/// List files in sync_dir matching any supported export format, excluding
/// our own export file. Accepts all formats (json, jsonl, json.gz, jsonl.gz)
/// since other machines in the sync group may use different formats.
async fn list_import_candidates(
    sync_dir: &Path,
    own_export_file: &Path,
) -> Vec<PathBuf> {
    let mut entries = match tokio::fs::read_dir(sync_dir).await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("vestige-sync: failed to read sync dir: {e}");
            return Vec::new();
        }
    };

    let own_filename = own_export_file.file_name();
    let mut result = Vec::new();

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if ExportFormat::is_supported_path(&path)
            && path.file_name() != own_filename
            && tokio::fs::symlink_metadata(&path)
                .await
                .map_or(false, |m| m.is_file())
        {
            result.push(path);
        }
    }

    result
}

/// Import all other machines' export files (one-shot, for --restore-on-start).
pub async fn import_all(
    vestige_cli: &Path,
    sync_dir: &Path,
    own_export_file: &Path,
    data_dir: Option<&Path>,
) {
    let candidates = list_import_candidates(sync_dir, own_export_file).await;

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

/// Run a single poll iteration: scan for candidates and import any with new/changed mtimes.
async fn poll_once(
    vestige_cli: &Path,
    sync_dir: &Path,
    own_export_file: &Path,
    data_dir: Option<&Path>,
    known_mtimes: &mut HashMap<PathBuf, SystemTime>,
) {
    let candidates = list_import_candidates(sync_dir, own_export_file).await;

    // Remove stale entries for files that no longer exist
    known_mtimes.retain(|path, _| candidates.contains(path));

    for file in candidates {
        let mtime = match tokio::fs::metadata(&file).await.and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => continue,
        };

        let needs_import = match known_mtimes.get(&file) {
            Some(prev) => mtime > *prev,
            None => true, // first time seeing this file
        };

        if needs_import {
            match import_file(vestige_cli, &file, data_dir).await {
                Ok(()) => {
                    known_mtimes.insert(file, mtime);
                }
                Err(e) => {
                    eprintln!("vestige-sync: poll import failed: {e}");
                }
            }
        }
    }
}

/// Polling-based import loop. Scans sync_dir every `poll_secs` and imports
/// files whose mtime has changed since the last scan.
/// Stops gracefully when `shutdown` is signaled.
pub async fn import_poll_loop(
    vestige_cli: PathBuf,
    sync_dir: PathBuf,
    own_export_file: PathBuf,
    poll_secs: u64,
    data_dir: Option<PathBuf>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut known_mtimes: HashMap<PathBuf, SystemTime> = HashMap::new();
    let mut interval = time::interval(Duration::from_secs(poll_secs));

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = shutdown.changed() => break,
        }
        poll_once(
            &vestige_cli,
            &sync_dir,
            &own_export_file,
            data_dir.as_deref(),
            &mut known_mtimes,
        )
        .await;
    }
}

/// Notify-based import watcher. Uses filesystem notifications with debouncing
/// to detect changes to other machines' export files.
/// Stops gracefully when `shutdown` is signaled.
pub async fn import_watch_loop(
    vestige_cli: PathBuf,
    sync_dir: PathBuf,
    own_export_file: PathBuf,
    data_dir: Option<PathBuf>,
    mut shutdown: watch::Receiver<bool>,
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
            eprintln!("vestige-sync: WARNING: import watching is permanently disabled for this session");
            // Wait for shutdown signal so the task stays alive and responds
            // to cooperative shutdown from proxy.rs.
            let _ = shutdown.changed().await;
            return;
        }
    };

    if let Err(e) = debouncer
        .watcher()
        .watch(&sync_dir, notify_debouncer_mini::notify::RecursiveMode::NonRecursive)
    {
        eprintln!("vestige-sync: failed to watch sync dir: {e}");
        eprintln!("vestige-sync: WARNING: import watching is permanently disabled for this session");
        drop(debouncer); // Release OS watcher thread
        let _ = shutdown.changed().await;
        return;
    }

    eprintln!("vestige-sync: watching {} for changes", sync_dir.display());

    loop {
        let events = tokio::select! {
            event = rx.recv() => match event {
                Some(Ok(events)) => events,
                Some(Err(e)) => {
                    eprintln!("vestige-sync: watch error: {e}");
                    continue;
                }
                None => break, // channel closed
            },
            _ = shutdown.changed() => break,
        };

        // Collect unique regular files that were modified (no symlinks)
        let own_filename = own_export_file.file_name();
        let mut to_import: Vec<PathBuf> = events
            .into_iter()
            .filter(|e| e.kind == DebouncedEventKind::Any)
            .map(|e| e.path)
            .filter(|path| {
                ExportFormat::is_supported_path(path)
                    && path.file_name() != own_filename
                    && path.symlink_metadata().map_or(false, |m| m.is_file())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn failed_import_does_not_record_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let own_export = dir.path().join("self.jsonl");
        let other_file = dir.path().join("other.jsonl");
        std::fs::write(&other_file, "{}").unwrap();

        // Use a nonexistent binary so import_file will fail
        let fake_cli = PathBuf::from("/nonexistent/vestige-cli-does-not-exist");
        let mut known_mtimes = HashMap::new();

        poll_once(&fake_cli, dir.path(), &own_export, None, &mut known_mtimes).await;

        // After a failed import, the mtime should NOT be recorded,
        // so the file will be retried on the next poll
        assert!(
            known_mtimes.is_empty(),
            "known_mtimes should be empty after failed import, but contains: {known_mtimes:?}",
        );
    }

    #[tokio::test]
    async fn list_candidates_accepts_all_formats() {
        let dir = tempfile::tempdir().unwrap();
        let own = dir.path().join("self.jsonl");
        std::fs::write(dir.path().join("other.jsonl"), "{}").unwrap();
        std::fs::write(dir.path().join("other.json"), "{}").unwrap();
        std::fs::write(dir.path().join("other.json.gz"), "{}").unwrap();
        std::fs::write(dir.path().join("other.jsonl.gz"), "{}").unwrap();
        std::fs::write(dir.path().join(".hidden.jsonl"), "{}").unwrap(); // dotfile excluded
        std::fs::write(dir.path().join("other.txt"), "{}").unwrap(); // wrong ext excluded

        let mut candidates = list_import_candidates(dir.path(), &own).await;
        candidates.sort();

        let names: Vec<&str> = candidates
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["other.json", "other.json.gz", "other.jsonl", "other.jsonl.gz"]
        );
    }

    #[tokio::test]
    async fn list_candidates_excludes_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let own = dir.path().join("self.json");
        std::fs::write(&own, "{}").unwrap();
        std::fs::write(dir.path().join("other.json"), "{}").unwrap();

        let candidates = list_import_candidates(dir.path(), &own).await;

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].file_name().unwrap() == "other.json");
    }
}
