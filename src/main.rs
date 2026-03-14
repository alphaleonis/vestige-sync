mod cli;
mod export;
mod import;
mod proxy;
mod template;

use std::process::ExitCode;

use clap::Parser;

use cli::Args;
use template::expand_filename;

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = Args::parse();
    args.resolve_paths();

    // Expand filename template
    let filename = match expand_filename(&args.filename) {
        Ok(name) => name,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Validate sync directory exists
    if !args.sync_dir.is_dir() {
        eprintln!(
            "error: sync directory '{}' does not exist or is not a directory",
            args.sync_dir.display()
        );
        return ExitCode::FAILURE;
    }

    let export_file = args.sync_dir.join(format!("{filename}.json"));

    eprintln!("vestige-sync starting");
    eprintln!("  sync dir: {}", args.sync_dir.display());
    eprintln!("  export file: {}", export_file.display());
    eprintln!("  export interval: {}s", args.export_interval);
    if let Some(ref data_dir) = args.data_dir {
        eprintln!("  data dir: {}", data_dir.display());
    }
    if let Some(poll) = args.poll_interval {
        eprintln!("  poll interval: {poll}s (filesystem watching disabled)");
    } else {
        eprintln!("  watching: filesystem notifications");
    }
    if args.restore_on_start {
        eprintln!("  restore on start: enabled");
    }
    if !args.vestige_args.is_empty() {
        eprintln!("  vestige-mcp args: {:?}", args.vestige_args);
    }

    // Import other machines' files before starting vestige-mcp
    if args.restore_on_start {
        eprintln!("vestige-sync: restoring from other machines' exports");
        import::import_all(
            &args.vestige_cli,
            &args.sync_dir,
            &export_file,
            args.data_dir.as_deref(),
        )
        .await;
    }

    proxy::run(&args, &export_file).await
}
