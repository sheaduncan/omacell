# WP-16 — GUI foundation (eframe/egui on wgpu): window, grid renderer, chrome, theme hot reload

| | |
|---|---|
| Phase | 4 — Surfaces II — GUI and data tools |
| Lane | C — Surfaces (conf, UI core, TUI, GUI, charts, print) |
| Size | XL (≈ 10–20) |
| Depends on | WP-14, WP-12, WP-S2 |
| Unblocks | WP-25, WP-26, WP-28 |
| Spec sections | §7.1–§7.3, §10, §11.4, §12.1, §12.4 |
| Where | `crates/gui` |

## Goal

The native Wayland front-end: the same state machines as the TUI, rendered with a virtualized grid that stays at 60 fps on a million rows, themed live from Omarchy.

## Deliverables

- eframe app (wgpu backend) with app id `omacell`, no server-side decorations, title `<file> — Omacell` with `•` dirty prefix, multi-window (one workbook per window), fractional-scale-aware pixel snapping, no minimum size beyond one cell plus chrome, compact layout below `[layout] compact_below_width`.
- Theme: roles → egui `Visuals` and grid colors; fonts loaded from the fontconfig-resolved path with fallbacks; text size from conf; hot reload on watcher/IPC/`SIGUSR1` without losing edit state, < 100 ms.
- Grid renderer: virtualized viewport; layers fills → gridlines → borders (Excel precedence) → text → conditional-format overlays → selection/cursor → spill and reference outlines; shaping cache keyed by (string id, style id, zoom); Fenwick-backed hit testing; frozen panes and split views; zoom.
- Chrome: sheet tabs, formula bar (expandable), status line (clickable segments), command palette, panel framework (docked, resizable, `Esc` returns focus), keys overlay (`F1`), optional classic menu bar behind `[layout] menu_bar`.
- Mouse: select, drag-move (with `Ctrl` copy), fill handle, header resize and auto-fit, header click select, context menu, `Ctrl+scroll` zoom, horizontal scroll; IME via winit; AccessKit: focused cell announced with address, value, formula presence, error.
- Session restore (files, panels, selection, window→workspace hint); `omacell` opens files from args and from `xdg-open`.

## Implementation notes

- CI has no GPU: run `egui_kittest` snapshots with the software rasterizer and relax the frame budget there (< 33 ms); record real-hardware numbers in the report.
- Panels are windows-within-the-window; the only real popups are transient (autocomplete, tooltips, dropdowns).

## Acceptance criteria

- [ ] `egui_kittest` snapshots of the grid at 1×, 1.5×, 2× with three fixture themes; crisp 1-px gridlines asserted pixel-wise.
- [ ] Bench: scroll through a 1M-row sheet with frame time < 16 ms on a dev GPU (recorded), < 33 ms on CI software rendering (gated).
- [ ] Theme hot-reload test: switch fixture themes mid-edit; edit buffer intact; < 100 ms.
- [ ] Startup to empty grid < 300 ms on the CI reference (recorded); AccessKit tree test shows the focused cell node.

## Tests

- Snapshot tests; benches; watcher/IPC reload tests; a11y tree tests.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-16.md` before writing code.
4. Create branch `wp/16-gui-foundation`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-16: GUI foundation (eframe/egui on wgpu): window, grid renderer, chrome, theme hot reload`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
