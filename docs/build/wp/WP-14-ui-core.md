# WP-14 — Shared UI core: modes, keymaps, selection, editing, palette, viewport, clipboard, session

| | |
|---|---|
| Phase | 3 — Surfaces I — config, CLI, UI core, TUI |
| Lane | C — Surfaces (conf, UI core, TUI, GUI, charts, print) |
| Size | L (≈ 6–10) |
| Depends on | WP-07, WP-12 |
| Unblocks | WP-15, WP-16, WP-23 |
| Spec sections | §6.5, §9.1 (keys), §10.1, §10.2, Appendix A |
| Where | `crates/ui` |

## Goal

All interaction logic lives once, toolkit-free, so the TUI and GUI are thin renderers over the same state machines.

## Deliverables

- Mode machine: classic vs modal (Normal/Insert/Visual/Command); keymap loader for `keys.toml` (chords, `<leader>`, counts, per-mode tables, user overrides); key-event normalization shared by crossterm and winit.
- Selection model: single, rectangular, multi-area, row/column, current region, extend mode, selection statistics provider.
- Editing state machine: in-cell and formula-bar editing (same state), point mode inserting references on navigation clicks/keys, reference colorization spans from the WP-03 editor parse, `F4` anchor cycling, autocomplete provider interface (functions/names/table columns/column values), localized-entry conversion to canonical formulas.
- Command palette model: fuzzy search over the registry, recents, inline argument prompts from schemas, `?` prefix routed to an `AiPlanProvider` trait (implemented in WP-23; absent → hint).
- Status line segment model; panel model (docked side, one visible, focus rules); viewport model (virtualized rows/cols, frozen panes, split, zoom, pixel↔index through WP-02 geometry); fill-handle logic (series detection and fill options); find/replace/go-to models; clipboard encode/decode (TSV, CSV, HTML table, Markdown table, internal format); undo-history model; session state persistence (`~/.local/state/omacell/session.toml`).

## Implementation notes

- No `egui`, `ratatui`, or `winit` types may appear in this crate — enforce with a dependency lint in CI.
- Every keymap entry must resolve to a registered command id; the conformance test is the contract with WP-07.

## Acceptance criteria

- [ ] Keymap conformance: every default binding in classic and modal resolves to a registered command; duplicate chords within a mode are rejected.
- [ ] State-machine tests for editing/point mode/`F4`; selection `proptest`s; viewport tests with frozen panes and hidden rows; clipboard round-trips; palette fuzzy ranking snapshot.

## Tests

- Unit and property tests; snapshot tests for palette ranking.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-14.md` before writing code.
4. Create branch `wp/14-ui-core`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-14: Shared UI core: modes, keymaps, selection, editing, palette, viewport, clipboard, session`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
