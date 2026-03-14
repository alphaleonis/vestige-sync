use std::path::Path;
use std::process::ExitCode;

use tokio::io;
use tokio::process::Command;
use tokio::signal;

use crate::cli::Args;
use crate::export;
use crate::import;

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        let mut sigterm =
            signal::unix::signal(signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        signal::ctrl_c().await.ok();
    }
}

/// Spawn vestige-mcp as a child process, relay stdio, and handle signals.
pub async fn run(args: &Args, export_file: &Path) -> ExitCode {
    // Build vestige-mcp arguments, prepending --data-dir if specified
    let mut mcp_args: Vec<String> = Vec::new();
    if let Some(ref data_dir) = args.data_dir {
        mcp_args.push("--data-dir".to_string());
        mcp_args.push(data_dir.to_string_lossy().into_owned());
    }
    mcp_args.extend(args.vestige_args.iter().cloned());

    let mut child = match Command::new(&args.vestige_bin)
        .args(&mcp_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            eprintln!(
                "error: failed to spawn '{}': {e}",
                args.vestige_bin.display()
            );
            return ExitCode::FAILURE;
        }
    };

    let mut child_stdin = child.stdin.take().expect("child stdin was piped");
    let mut child_stdout = child.stdout.take().expect("child stdout was piped");

    let mut proc_stdin = io::stdin();
    let mut proc_stdout = io::stdout();

    // Spawn stdin relay as a background task.
    // When our stdin hits EOF, this completes and drops child_stdin,
    // signaling EOF to the child process.
    let stdin_task = tokio::spawn(async move {
        let result = io::copy(&mut proc_stdin, &mut child_stdin).await;
        // child_stdin is dropped here → child sees EOF
        result
    });

    // Spawn the export loop as a background task.
    let export_task = tokio::spawn(export::export_loop(
        args.vestige_cli.clone(),
        export_file.to_path_buf(),
        args.export_interval,
        args.data_dir.clone(),
    ));

    // Spawn the import watcher as a background task.
    let import_task = if let Some(poll_secs) = args.poll_interval {
        tokio::spawn(import::import_poll_loop(
            args.vestige_cli.clone(),
            args.sync_dir.clone(),
            export_file.to_path_buf(),
            poll_secs,
            args.data_dir.clone(),
        ))
    } else {
        tokio::spawn(import::import_watch_loop(
            args.vestige_cli.clone(),
            args.sync_dir.clone(),
            export_file.to_path_buf(),
            args.data_dir.clone(),
        ))
    };

    let shutdown_reason;

    tokio::select! {
        // Relay: child stdout → process stdout.
        // Completes when child closes its stdout (i.e. child exits).
        result = io::copy(&mut child_stdout, &mut proc_stdout) => {
            shutdown_reason = "child exited";
            if let Err(e) = result {
                eprintln!("vestige-sync: stdout relay error: {e}");
            }
        }

        // Signal: SIGINT or SIGTERM
        _ = wait_for_signal() => {
            shutdown_reason = "signal";
            eprintln!("vestige-sync: received signal, shutting down child");
            let _ = child.kill().await;
        }
    }

    // Cancel background tasks (no longer needed)
    stdin_task.abort();
    export_task.abort();
    import_task.abort();

    eprintln!("vestige-sync: shutdown reason: {shutdown_reason}");

    // Final export to capture any memories created during this session
    if args.export_on_exit {
        eprintln!("vestige-sync: running final export");
        if let Err(e) = export::export_once(&args.vestige_cli, export_file, args.data_dir.as_deref()).await {
            eprintln!("vestige-sync: final export failed: {e}");
        }
    }

    // Wait for child to exit and propagate its exit code
    match child.wait().await {
        Ok(status) => {
            eprintln!("vestige-sync: child exited with {status}");
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(sig) = status.signal() {
                    return ExitCode::from(128 + sig as u8);
                }
            }
            ExitCode::from(status.code().unwrap_or(1) as u8)
        }
        Err(e) => {
            eprintln!("vestige-sync: error waiting for child: {e}");
            ExitCode::FAILURE
        }
    }
}
