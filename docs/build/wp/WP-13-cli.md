# WP-13 — CLI: the `omacell` binary

| | |
|---|---|
| Phase | 3 — Surfaces I — config, CLI, UI core, TUI |
| Lane | D — Integration (bus, CLI, Lua, MCP, AI, release) |
| Size | L (≈ 6–10) |
| Depends on | WP-05a, WP-05b, WP-05c, WP-07b, WP-08, WP-10, WP-11, WP-12 |
| Unblocks | WP-15, WP-20, WP-21, WP-28 |
| Spec sections | §6.10 F-10.5, §7.9, §12.3 |
| Where | `crates/cli` |

## Goal

Every capability of the engine and I/O layer is reachable from a shell, with `--json` on every read and `--dry-run` on every write, before any UI exists.

## Deliverables

- `clap` command tree mirroring spec F-10.5: `convert`, `query`, `set`, `eval`, `recalc`, `run` (stub until WP-20), `fn list|doc`, `config check|edit|reset|show|diff`, `theme show|reload`, `keys check`, `setup omarchy`, `commands`, `ipc`, `changeset list|show|apply|revert|export`, `diff`, `audit` (deterministic checks arrive with WP-19; stub reports 'not yet'), `ai` / `agent` / `mcp` (stubs that explain they arrive in WP-21/22 — must exit non-zero with a hint), `--tui` / GUI dispatch behind features.
- Global flags: `--json`, `--dry-run`, `--set key=value`, `--config <file>`, `--theme <file>`, `--quiet`, `--verbose`; errors as `{code, message, hint}` on stderr in `--json` mode; documented exit codes in `docs/cli.md`.
- `omacell query` formats `json|csv|md`; range/sheet selection; formulas or values.
- Register the deferred `file.open`, `file.save`, and `file.export` command adapters against the real WP-08/10/11 I/O services, with schemas added through WP-07a's extension API. Do not place file business logic in `crates/bus` or duplicate it in the CLI parser.
- Completions (bash/zsh/fish via `clap_complete`), man page (`clap_mangen`), both generated at build into `target/dist/`.
- Logging via `tracing` to stderr and `~/.local/state/omacell/logs/` with rotation.

## Implementation notes

- The CLI is a thin adapter over the command bus and `io`; business logic here is a bug.
- Snapshot the help text: agents and docs depend on it not drifting silently.

### Binding handoff from WP-12

- Build one `Paths` and one `LoadOptions` at the composition root. Start with `LoadOptions::from_process()`, then set `config_file` from `--config`, `theme_override` from `--theme`, `cli_sets` from every `--set`, and `workbook` from `workbook_settings_overlay(book.settings())`. Pass that exact value to `load_with_options` or `ConfigStore::load_and_watch_with`; never reparse configuration in the CLI or discard these layers on reload.
- `config show --all --json` is `show_all_json`; single-key display/explanation is `LoadedConfig::{get_json,explain}`. Surface `LoadedConfig::migrations` as status/output. Default reset calls `reset_user_file`. A named `config reset [file]` must remain beneath `Paths::user_config`, reject non-normal path components, and use the same backup policy by adding a narrow helper to `conf` rather than duplicating filesystem logic in the binary.
- `theme show` reports `LoadedConfig::{theme,shell}`. Register `theme.reload` as a public, non-changeset-eligible mutating command at the binary composition layer, backed by the shared `ConfigStore`; return/emit the frozen `core::Event::ThemeChanged`. Do **not** add an IPC `ControlOp` or change the frozen WP-07b envelope.
- `omacell ipc theme.reload --all --quiet` must enumerate `ipc::list_live_instances`, connect to each owned instance, and execute the registered command. This exact path is called by the installed WP-12 hook. Bind `SIGUSR1` to the same reload operation in long-running TUI/GUI processes; use a safe signal adapter and justify it in the report if it adds a non-pre-approved dependency.
- `setup omarchy` owns the interactive menu confirmation, then calls `setup_omarchy`; `--show-hyprland` prints `HYPRLAND_SNIPPET`. Run `keys::check_hyprland` during setup and for `keys check`. Do not write or merge Omarchy files in CLI code.

## Acceptance criteria

- [ ] `assert_cmd` integration tests for every subcommand incl. `--json` outputs validated against schemas and `--dry-run` proving no file changes.
- [ ] Help snapshots (`insta`) for every command; man page and completions generated in CI.
- [ ] Exit codes documented and asserted; malformed input never panics (fuzz smoke on args).
- [ ] Configuration integration tests prove `--config`, `--theme`, workbook settings, env and repeated `--set` reach one retained `LoadOptions`; `theme.reload` works locally, over IPC `--all`, and through the SIGUSR1 adapter without changing the IPC schema.

## Tests

- Integration tests; snapshot tests; arg fuzz smoke.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-13.md` before writing code.
4. Create branch `wp/13-cli`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-13: CLI: the `omacell` binary`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
