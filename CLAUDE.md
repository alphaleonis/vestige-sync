# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

vestige-sync is a Rust wrapper binary around [vestige-mcp](https://github.com/samvallad33/vestige) that enables multi-machine memory synchronization via Syncthing (or any file-sync tool). It proxies MCP JSON-RPC stdio to a child `vestige-mcp` process while periodically exporting/importing memories through machine-specific JSON files in a shared sync directory.

## Build & Test Commands

```bash
cargo build              # debug build
cargo build --release    # release build
cargo test               # run all tests
cargo test <test_name>   # run a single test
```

## Architecture

**Stdio proxy** (`src/proxy.rs`): stdout is reserved exclusively for MCP JSON-RPC passthrough to the child `vestige-mcp` process. All wrapper logging must go to stderr only. The stdin relay runs as a background tokio task; the stdout relay runs in the main `select!` alongside the signal handler.

**Shell-out model**: Export and import use the `vestige` CLI binary (`vestige export` / `vestige import`) rather than linking vestige-core as a library. This keeps builds independent.

**Modules**:
- `src/cli.rs` — clap-derived `Args` struct
- `src/template.rs` — `{hostname}`, `{os}`, `{user}` placeholder expansion for `--filename`
- `src/proxy.rs` — child process spawning, stdio relay, signal handling, orchestrates export/import tasks
- `src/export.rs` — periodic `vestige export` → temp file → compare → atomic replace
- `src/import.rs` — filesystem watching (notify) or polling for other machines' files → `vestige import`

**Import modes**: Default uses `notify` crate with 2-second debounce. `--poll-interval` switches to mtime-based polling (disables notify entirely).

**Sync convergence**: Stable export order + content comparison + vestige's dedup-on-import ensures the sync loop settles after one round-trip between machines.

## Critical Constraints

- **Never write to stdout** from the wrapper itself — it breaks MCP protocol
- Export files use the pattern `<sync-dir>/<filename>.<format-ext>` (extension depends on `--format`, default `jsonl`); the wrapper must never import its own export file. Stale export files from a previous format are automatically cleaned up after each successful export.
- Temp file writes use a dotfile prefix (e.g., `.hostname.jsonl`) then compare-and-rename to avoid triggering unnecessary Syncthing syncs. Import filtering rejects dotfiles as defense-in-depth.
- **Import accepts all formats** (`json`, `jsonl`, `json.gz`, `jsonl.gz`) regardless of the configured export format, since other machines in the sync group may use different formats. The vestige CLI auto-detects format on import.
- **`--db-path`** is forwarded to both `vestige-mcp` and the `vestige` CLI (as `--data-dir`) — they must use the same database.
- **Concurrent database access**: Export and import CLI subprocesses run concurrently with the child `vestige-mcp` process, all sharing the same database. This is safe because vestige uses SQLite in WAL mode. Do not change this assumption without verifying the vestige fork's database backend.
