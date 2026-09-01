# Omacell CLI

The `omacell` binary is a thin adapter over the command bus, file I/O, and configuration. Every capability of the engine is reachable from a shell before any UI exists.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success, or `--help` / `--version` |
| 1 | Operational error (I/O, config, command, IPC) |
| 2 | Usage error (unknown flag, missing argument) |

`--json` does not change the exit code. On failure it writes `{code, message, hint}` to stderr.

## Global flags

`--json` · `--dry-run` · `--set KEY=VALUE` (repeatable) · `--config FILE` · `--theme FILE` · `--from-workbook FILE` · `--quiet` · `--verbose`

`--config` replaces `~/.config/omacell/config.toml`. `--theme` wins over `OMACELL_THEME`. Repeated `--set` values are retained on reload with the rest of `LoadOptions`.

`--dry-run` is also forwarded to registry commands sent through `omacell ipc`; changeset apply/revert controls become local no-ops. It never creates the rotating file log. Usage failures under `--json` use the same `{code, message, hint}` error object as operational failures.

## Commands

See `omacell --help` and the per-command help snapshots. `omacell --tui [file]` launches the terminal UI (WP-15). Without a TTY it exits 1 with `tui.tty` rather than hanging. Bare `omacell [file]` launches the GUI (WP-16). Without `WAYLAND_DISPLAY`/`DISPLAY` it exits 1 with `gui.display`.

`omacell mcp [--socket PATH] [--book FILE]` serves the MCP tool catalog over stdio or a Unix socket and also binds the WP-07b IPC socket so `omacell changeset list` can see proposals. `omacell agent "<prompt>"` hands off to `omarchy agent prompt` when a default agent is set; otherwise it prints the equivalent command and JSON `{hidden: true}`. `omacell agent diagnose [--pid] [--book]` builds the WP-19 diagnostic bundle. `omacell recalc --wait` is accepted for the skill done-checklist (async AI settle is WP-22).

`omacell run script.lua book.xlsx` runs Lua (WP-20). It explicitly loads trusted `init.lua`, then sorted `plugins/*/init.lua`, then the requested script. `--embedded` runs `xl/omacell/scripts/main.lua` only when the exact workbook bytes are trusted in `~/.local/state/omacell/trust.toml` (`omacell trust add|remove|list`); embedded Lua can invoke only a fixed, reviewed allowlist of workbook commands, so newly registered commands remain unavailable by default. `omacell run --python script.py [book.xlsx]` is an experimental stdio bridge using the versioned IPC JSON-lines request/reply envelopes and the configured `[ipc].max_frame_bytes` limit (16 MiB by default, configurable from 1–16 MiB). `--dry-run` is accepted only with `--embedded`; user Lua and Python have OS access and therefore cannot provide a no-write dry-run guarantee.

`omacell ipc theme.reload --all --quiet` is the Omarchy theme-set hook. It enumerates live owned instances and executes the registered `theme.reload` command. It does not add an IPC `ControlOp`.

TUI charts follow `[tui] graphics = "auto"` by default. Foot is detected as
sixel and Kitty/Ghostty use the Kitty protocol; unsupported terminals render
the same chart scene as ANSI-colored Unicode braille. Automatic mode uses the
Unicode fallback inside tmux or Herdr because passthrough is not guaranteed.
Set `graphics = "sixel"` or `"kitty"` only when passthrough is configured, or
`"off"` to always use Unicode. The setting and chart colors update on live
configuration/theme reload.

Without `--socket` or `--all`, `omacell ipc` targets the focused GUI/TUI
instance. If no frontend has published focus yet, it falls back to the newest
live owned instance. GUI focus comes from eframe/winit; the TUI enables and
consumes Crossterm focus-change events.

`omacell convert input.csv output.xlsx --plan plan.json` consumes the shared WP-08 `ImportPlan` JSON (bounded to 1 MiB). For JSON input, `--jq .items` selects an array with a dotted object path; it is a selector, not the full jq language. `omacell config diff` emits sorted effective user/package differences and honors `--config`.

Legacy Excel 97–2003 `.xls` input is read natively, without LibreOffice or an external converter. It is read-only: use `omacell convert old.xls new.xlsx` before editing in place, or save an opened workbook to a writable format.

## Completions and man page

`cargo test -p omacell-cli --test dist` writes bash/zsh/fish completions and `omacell.1` to `target/dist/` (or `$CARGO_TARGET_DIR/dist`).
