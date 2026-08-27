# WP-12 — Configuration layering, Omarchy theme/font resolution, and `setup omarchy`

| | |
|---|---|
| Phase | 3 — Surfaces I — config, CLI, UI core, TUI |
| Lane | C — Surfaces (conf, UI core, TUI, GUI, charts, print) |
| Size | L (≈ 6–10) |
| Depends on | WP-01 |
| Unblocks | WP-13, WP-14, WP-16, WP-20, WP-21, WP-22 |
| Spec sections | §7.1–§7.6, §7.8, §9, Appendix B, Appendix C |
| Where | `crates/conf` |

## Goal

Everything the spec calls tunable becomes a typed, layered, explainable, hot-reloadable configuration — and the app takes its colors and fonts from the active Omarchy theme.

## Deliverables

- Schema types for Appendix B (serde + `schemars` → `docs/schemas/config.schema.json`); complete, commented `default/config.toml`; `default/keys/classic.toml` and `keys/modal.toml` from Appendix A (full maps, not the excerpts).
- Layering: package defaults → active theme → user files → workbook settings → `OMACELL_*` env → CLI `--set`; `explain(key)` provenance; validation with file/line errors; `schema` versioning with migrations that back up before rewriting; `reset` with timestamped backups under `~/.local/state/omacell/backups/`.
- Live reload via `notify` (debounced; last-good-config semantics; errors reported as events, never crashes).
- Theme resolution: `~/.local/state/omarchy/current/theme` → `~/.config/omarchy/current/theme` → none; `colors.toml` parser accepting canonical and legacy keys; role mapping implemented in code **and** as `default/themed/omacell.toml.tpl` (Appendix C) with a test asserting the two agree for every fixture theme; `mix` blending; light mode; contrast enforcement along the neutral ramp (WCAG AA for text roles, 1.5:1 for structure) with debug logging; `shell.toml` token reading (`[font]`, `[spacing]`, corner style); desktop-portal `color-scheme` fallback (via `zbus`) for non-Omarchy hosts.
- Fonts: resolve the fontconfig `monospace` alias (via `fontdb` or `fc-match`) and the text size (shell `[font]` scale → GTK `text-scaling-factor` → 11 pt); substitution table for file fonts (Calibri/Aptos → Carlito, Arial → Liberation Sans, Times New Roman → Liberation Serif, Cambria → Caladea).
- Watchers: theme swap on `current/`, `theme-set.d`/`font-set.d` hook integration, `SIGUSR1` reload.
- `setup omarchy` logic: install the template to `~/.config/omarchy/themed/`, the hook to `~/.config/omarchy/hooks/theme-set.d/omacell`, skill symlinks (WP-21 supplies the file; create the links here), optional `omarchy-menu.jsonc` rows only after explicit confirmation, `keys check` parsing `~/.config/hypr/bindings.lua` for conflicting chords, `--show-hyprland` snippet. Never writes under `/usr/share/omarchy`.

## Implementation notes

- Fixtures: snapshot every built-in Omarchy theme's `colors.toml` into `tests/fixtures/omarchy-themes/` with the upstream MIT notice, plus three community themes; refresh via `scripts/fetch-omarchy-themes.sh` (run by humans, not CI).
- Provenance must survive live reload; `omacell config show --all --json` output is the ground truth for WP-13 tests.

## Acceptance criteria

- [ ] Layering precedence tests for every layer pair; `explain` names the right layer.
- [ ] Template-vs-code equality across all fixture themes; contrast tests; light-mode mixing direction tests.
- [ ] Live reload test with an invalid intermediate write keeps the last good config and emits an error event.
- [ ] `setup omarchy` in a temporary `$HOME` creates exactly the expected files and touches nothing else; menu rows only when confirmed.

## Tests

- Fixture-driven theme tests; layering tests; watcher tests with temp dirs; snapshot of generated `config.schema.json`.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-12.md` before writing code.
4. Create branch `wp/12-config-theme-omarchy`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-12: Configuration layering, Omarchy theme/font resolution, and `setup omarchy``. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
