# Omacell Manual

Omacell is a keyboard-first spreadsheet for Omarchy Linux. It uses one engine
behind a Wayland GUI, a terminal UI, a command-line interface, IPC, and MCP.
The native exchange format is `.xlsx`; `.omc` preserves Omacell-specific
features. CSV, TSV, ODS, JSON, PDF, and legacy `.xls` are also supported.

## Start here

Run `omacell [FILE]` for the GUI or `omacell --tui [FILE]` for the terminal
client. Omacell intentionally opens at most one workbook per process. Launch a
second process for another workbook. The command palette exposes the same
registered command ids as menus, keymaps, Lua, and automation.

Use `omacell convert old.xls new.xlsx` for legacy Excel 97–2003 files. `.xls`
reading is native and does not require LibreOffice, but the format is read-only.

## Configuration and data

Package defaults live under `/usr/share/omacell/default`. User configuration is
read from `~/.config/omacell`; state, logs, and trust grants use the normal XDG
state/runtime paths. Omacell never modifies workbook files until an explicit
save and uses atomic replacement for writes.

`Ctrl+Shift+P` opens the command palette. `omacell keys check` reports conflicts
between the classic keymap and local Hyprland bindings. Consult the generated
[configuration](configuration.md) and [CLI](cli-reference.md) references for
the authoritative options.

## Formats and compatibility

The project targets Excel formula and workbook semantics, not pixel-identical
Microsoft Office rendering. Unsupported or lossy structures are reported as
warnings rather than silently discarded. Current differences are tracked in
`docs/compat/known-differences.md`. Parser limits are documented separately so
automation can treat an intentional rejection differently from a corrupt file.

## Help and diagnostics

Use `omacell --help`, `omacell COMMAND --help`, and `omacell audit FILE` first.
`omacell agent diagnose --book FILE` builds a bounded diagnostic bundle. Logs
are local and redact configured secrets; Omacell has no telemetry or update
checker.
