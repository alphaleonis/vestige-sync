# vestige-sync

A Rust wrapper around [vestige-mcp](https://github.com/samvallad33/vestige) that enables multi-machine memory synchronization via [Syncthing](https://syncthing.net/) (or any file-sync tool).

vestige-sync sits between Claude Code and vestige-mcp, transparently proxying MCP JSON-RPC stdio while periodically exporting and importing memories through machine-specific JSON files in a shared sync directory.

## Dependencies

This project requires a modified version of vestige with `import` and stable export order support:

**Fork:** [alphaleonis/vestige](https://github.com/alphaleonis/vestige) (branch: `decaf`)

The fork adds:
- `vestige import <file>` — merge-import with embedding similarity-based deduplication
- `vestige export --data-dir` — CLI support for custom data directory
- Stable export order (`created_at ASC, id ASC`) for byte-identical output when nothing changes

## Usage

```bash
# In Claude Code MCP config:
claude mcp add vestige -- vestige-sync --sync-dir ~/Sync/vestige --export-on-exit
```

This starts the wrapper, which:
- Passes MCP stdio through to `vestige-mcp`
- Periodically exports to `~/Sync/vestige/<hostname>.json`
- Watches for other machines' export files and imports them
- Runs a final export on exit to capture late-session memories

## CLI

```
vestige-sync [OPTIONS] --sync-dir <PATH> [-- VESTIGE_ARGS...]

Options:
    --sync-dir <PATH>            Sync directory for export/import files (required)
    --filename <TEMPLATE>        Output file stem [default: {hostname}]
    --export-interval <SECS>     Export interval [default: 900]
    --poll-interval <SECS>       Poll instead of filesystem watching
    --export-on-exit             Export on shutdown
    --restore-on-start           Import other machines' files on startup
    --data-dir <PATH>            Forwarded to vestige-mcp and vestige CLI
    --vestige-bin <PATH>         Path to vestige-mcp [default: vestige-mcp]
    --vestige-cli <PATH>         Path to vestige CLI [default: vestige]
```

### Filename template placeholders

| Placeholder | Example | Notes |
|---|---|---|
| `{hostname}` | `decaf-laptop` | Machine hostname |
| `{os}` | `linux` | OS family |
| `{platform}` | `wsl` | Like `{os}` but distinguishes WSL from native Linux |
| `{distro}` | `fedora` | Linux distro ID from `/etc/os-release` |
| `{user}` | `decaf` | Current username |

## Installation

Requires [Rust/Cargo](https://www.rust-lang.org/tools/install).

Install the vestige fork (provides both `vestige-mcp` and `vestige` CLI):

```bash
cargo install --git https://github.com/alphaleonis/vestige --branch decaf vestige-mcp
```

Install vestige-sync:

```bash
cargo install --git https://github.com/alphaleonis/vestige-sync
```
