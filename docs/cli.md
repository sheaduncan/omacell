# Omacell CLI

The `omacell` binary is a thin adapter over the command bus, file I/O, and configuration. Every capability of the engine is reachable from a shell before any UI exists.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success, or `--help` / `--version` |
| 1 | Operational error (I/O, config, command, IPC) |
| 2 | Usage error (unknown flag, missing argument) |
| 3 | Stub: the subcommand arrives in a later work package |

`--json` does not change the exit code. On failure it writes `{code, message, hint}` to stderr.

## Global flags

`--json` · `--dry-run` · `--set KEY=VALUE` (repeatable) · `--config FILE` · `--theme FILE` · `--from-workbook FILE` · `--quiet` · `--verbose`

`--config` replaces `~/.config/omacell/config.toml`. `--theme` wins over `OMACELL_THEME`. Repeated `--set` values are retained on reload with the rest of `LoadOptions`.

`--dry-run` is also forwarded to registry commands sent through `omacell ipc`; changeset apply/revert controls become local no-ops. It never creates the rotating file log. Usage failures under `--json` use the same `{code, message, hint}` error object as operational failures.

## Commands

See `omacell --help` and the per-command help snapshots. `omacell --tui [file]` launches the terminal UI (WP-15). Without a TTY it exits 1 with `tui.tty` rather than hanging. Bare `omacell [file...]` still opens the GUI (WP-16).

Stubs that exit 3:

- `omacell [file...]` (WP-16)
- `omacell run` (WP-20)
- `omacell audit` (WP-19)
- `omacell ai` (WP-22)
- `omacell agent` / `omacell mcp` (WP-21)

`omacell ipc theme.reload --all --quiet` is the Omarchy theme-set hook. It enumerates live owned instances and executes the registered `theme.reload` command. It does not add an IPC `ControlOp`.

`omacell convert input.csv output.xlsx --plan plan.json` consumes the shared WP-08 `ImportPlan` JSON (bounded to 1 MiB). `omacell config diff` emits sorted effective user/package differences and honors `--config`.

## Completions and man page

`cargo test -p omacell-cli --test dist` writes bash/zsh/fish completions and `omacell.1` to `target/dist/` (or `$CARGO_TARGET_DIR/dist`).
