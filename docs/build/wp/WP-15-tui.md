# WP-15 — Terminal UI (ratatui)

| | |
|---|---|
| Phase | 3 — Surfaces I — config, CLI, UI core, TUI |
| Lane | C — Surfaces (conf, UI core, TUI, GUI, charts, print) |
| Size | L (≈ 6–10) |
| Depends on | WP-14, WP-13 |
| Unblocks | WP-25, WP-28 |
| Spec sections | §5.4, §7.7, §10, Appendix A |
| Where | `crates/tui` |

## Goal

A complete keyboard-driven spreadsheet in a terminal — the first front-end real users touch, and the one that works over SSH.

## Deliverables

- Grid widget: virtualized rendering, headers, gridlines per `[tui] unicode_borders`, type-aware alignment, number formats, overflow into empty neighbors, frozen panes, selection and cursor styles, stale hatching, spill outlines, reference colorization while editing.
- Formula bar, status line (segments from `[layout] status_line`), sheet tabs, command palette, panels (format, sort/filter, find/replace, go-to, comments list, changeset review — the last two backed by WP-07a/WP-19 models as they land).
- Theming through ANSI palette indices (so Omarchy's terminal theme applies for free) with truecolor only for colors that came from a file; `[tui] truecolor = auto` detection.
- Mouse support where the terminal reports it; resize handling; `omacell --tui file`; IPC available inside the TUI process; optional sixel/kitty graphics hook (chart previews land with WP-25).
- Startup < 100 ms measured and recorded.

## Implementation notes

- Render only the visible window; a 1M-row sheet must feel the same as a 100-row sheet.
- Test through the real event loop with `TestBackend`; snapshot the frames.

## Acceptance criteria

- [ ] Snapshot tests across terminal sizes (80×24, 120×40, 200×60) and three fixture themes; keymap tests through the event loop for classic and modal.
- [ ] Frame budget: redraw < 16 ms on a 200×60 terminal with a 1M-row sheet (bench).
- [ ] Manual checklist in the report: Foot, Alacritty, Ghostty, Kitty, tmux, SSH.

## Tests

- `TestBackend` snapshots; event-loop tests; criterion redraw bench.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-15.md` before writing code.
4. Create branch `wp/15-tui`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-15: Terminal UI (ratatui)`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
