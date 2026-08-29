# WP-20 — Lua scripting, sandbox and trust, macro recorder

| | |
|---|---|
| Phase | 5 — Scripting, agents, AI |
| Lane | D — Integration (bus, CLI, Lua, MCP, AI, release) |
| Size | L (≈ 6–10) |
| Depends on | WP-07a, WP-12, WP-13 |
| Unblocks | WP-23 |
| Spec sections | §6.10 F-10.1–F-10.3, §8.8 (Lua hooks), §12.3 |
| Where | `crates/lua`; `docs/lua-api.md` |

## Goal

The Omarchy-dialect escape hatch: Lua for configuration, automation, custom functions, and recorded macros — sandboxed for anything that arrived inside a file.

## Deliverables

- `mlua` (Lua 5.4, vendored) runtime; API objects `omacell.book/sheet/range/cell` (values, formulas, styles, iteration), `omacell.cmd(id, args)` over the command bus, events (`on_open`, `on_change`, `on_before_save`, `on_recalc`, `on_theme_change`), `omacell.keymap.set(mode, keys, cmd_or_fn)`, `omacell.fn(name, spec, fn)` registering typed, array-aware custom functions into the registry (namespaced, visible to autocomplete and `fn list`), `omacell.ui.prompt/status/notify`, `omacell.ai.*` hooks reserved (implemented in WP-23).
- Load order: `~/.config/omacell/init.lua`, then `plugins/*/init.lua`; `:source` reload; errors reported to the status line with file:line.
- Sandbox profiles: `user` (full standard library) for config-dir scripts; `embedded` (no `io`/`os`/`require`/`package`/`debug`, instruction-count and memory limits, no network) for scripts stored in workbooks; trust store `~/.local/state/omacell/trust.toml` keyed by file hash with `omacell trust add|remove|list`; never prompted on open.
- Embedded script storage in the workbook custom part (via WP-10/WP-11); `omacell run script.lua book.xlsx`.
- Macro recorder: command stream → readable Lua that replays through `omacell.cmd`; start/stop/save commands.
- Python bridge stub: `omacell run --python script.py` launches a subprocess speaking the IPC JSON protocol over stdio (minimal; documented as experimental).

## Implementation notes

- Custom functions participate in the dependency graph exactly like built-ins; their volatility is declared in `spec`.
- Embedded scripts never run without explicit trust; the test for this is a release blocker.

### Binding handoff from WP-12

- Consume `[scripting]` and related policy from the retained `LoadedConfig`; never parse `config.toml` in `lua`. Canonicalize trusted directories before comparison and treat missing/non-directory entries as untrusted.
- WP-12 watches the user config tree, but a filesystem notification for `init.lua` or a plugin is **not** permission to execute code. `:source` and explicit CLI commands may reload user scripts; embedded scripts still require the trust decision and never run merely because a workbook opened or a file changed.
- When configuration reload changes scripting policy, apply the stricter policy immediately to future invocations. Do not silently broaden an already-running embedded environment; rebuilding it requires an explicit source/run action.

## Acceptance criteria

- [ ] API tests for every documented function; `docs/lua-api.md` generated from the registration table and checked for drift.
- [ ] Sandbox tests: `io`/`os`/`require` blocked in `embedded`; infinite loop terminated; memory cap enforced; a trusted file runs, an untrusted one does not, and the untrusted one leaves the model untouched.
- [ ] Recorder test: recorded session replays to an identical model.

## Tests

- API tests; sandbox escape tests; recorder equivalence test.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-20.md` before writing code.
4. Create branch `wp/20-lua-scripting`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-20: Lua scripting, sandbox and trust, macro recorder`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
