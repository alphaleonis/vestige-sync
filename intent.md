# vestige-sync

## Background

[Vestige](https://github.com/samvallad33/vestige) is an MCP memory server that stores AI conversation memories in a local SQLite database with FSRS-6 spaced repetition metadata. It runs as a stdio MCP server, typically spawned by Claude Code.

The problem: vestige is single-machine. If you use multiple computers, your memories don't follow you. The database is a live SQLite file with WAL, which is unsafe to sync directly via tools like Syncthing.

## What we built in vestige (fork: alphaleonis/vestige, branch: decaf)

We added two features to vestige itself:

1. **`vestige import <file>`** — Merge-imports memories from a JSON export file with:
   - Embedding similarity-based deduplication (thresholds: >0.92 keep stronger, 0.75-0.92 merge, <0.75 insert)
   - Full FSRS metadata preservation (stability, difficulty, reps, retention strength, etc.)
   - Echo loop detection (importing your own export is a no-op)
   - Idempotent (running the same import twice produces no changes)

2. **Stable export order** — `vestige export` now sorts by `created_at ASC, id ASC`, producing byte-identical output when nothing has changed.

## What vestige-sync needs to do

A Rust wrapper binary that:

1. **Spawns `vestige-mcp`** as a child process and passes stdin/stdout through transparently (MCP JSON-RPC). The wrapper is what Claude Code launches instead of vestige-mcp directly.

2. **Periodically exports** local memories to a machine-specific JSON file in a sync directory:
   - Runs `vestige export <sync-dir>/<machine-name>.json.tmp`
   - Compares with existing `<machine-name>.json`
   - Only replaces if content differs (avoids unnecessary Syncthing syncs)
   - Default interval: 15 minutes, configurable

3. **Watches for other machines' export files** in the sync directory:
   - Detects when `<other-machine>.json` is modified (via filesystem watcher or polling)
   - Runs `vestige import <other-machine>.json` to merge new memories
   - Only imports files that aren't our own

4. **Signal forwarding** — propagates SIGTERM/SIGINT to the vestige-mcp child process for clean shutdown.

5. **Optional restore-on-start** — if `--restore-on-start` is set, imports all other machines' files on startup before starting vestige-mcp.

## CLI interface (draft)

```
vestige-sync [OPTIONS] [-- VESTIGE_ARGS...]

OPTIONS:
    --sync-dir <PATH>          Directory for export/import files (required)
    --machine-name <NAME>      This machine's identifier (default: hostname)
    --export-interval <SECS>   Export interval in seconds (default: 900)
    --restore-on-start         Import other machines' files before starting
    --vestige-bin <PATH>       Path to vestige-mcp binary (default: "vestige-mcp")
    --vestige-cli <PATH>       Path to vestige CLI binary (default: "vestige")
    -h, --help
    -V, --version

Anything after `--` is forwarded to vestige-mcp (e.g. --data-dir, --http-port).
```

## Example usage

```bash
# In Claude Code MCP config:
claude mcp add vestige -- vestige-sync --sync-dir ~/Sync/vestige -- --data-dir ~/.vestige

# This starts the wrapper, which:
# - Passes MCP stdio through to vestige-mcp --data-dir ~/.vestige
# - Every 15 min, exports to ~/Sync/vestige/<hostname>.json
# - Watches ~/Sync/vestige/ for other machines' files and imports them
# - Syncthing syncs ~/Sync/vestige/ between machines
```

## Sync flow between two machines

```
Machine A                          Syncthing                     Machine B
─────────                          ─────────                     ─────────
export → A.json ──────────────────→ sync ──────────────────────→ detect A.json changed
                                                                  import A.json (dedup)
                                                                  export → B.json
detect B.json changed ←────────────── sync ←────────────────────
import B.json (dedup)

Next cycle:
export → A.json (identical) ──────→ Syncthing sees no change ──→ nothing happens
```

The stable export order + temp-file-then-compare ensures the loop settles after one round-trip.

## Technical decisions

- **Language**: Rust (single binary, matches vestige's stack)
- **Dependencies**: `clap` (CLI), `tokio` (async runtime, signals, process, timers), `notify` (file watching, optional — could also poll)
- **Stderr only** for wrapper logging — stdout is reserved for MCP JSON-RPC passthrough
- **Shells out to `vestige` CLI** for export/import rather than linking vestige-core as a library dependency (keeps builds independent, simpler)
