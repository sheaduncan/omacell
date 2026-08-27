# WP-25 — Charts and sparklines: model, vector renderer, `.xlsx` DrawingML core types

| | |
|---|---|
| Phase | 6 — Analysis and output |
| Lane | C — Surfaces (conf, UI core, TUI, GUI, charts, print) |
| Size | XL (≈ 10–20) |
| Depends on | WP-16, WP-10, WP-15 |
| Unblocks | WP-26, WP-28 |
| Spec sections | §6.8, §7.1 (palette roles), §13 |
| Where | `crates/core` (module `chart`), `crates/io` (drawingml), `crates/gui`, `crates/tui` |

## Goal

Charts that follow the theme, render identically on screen and in exports, and survive a trip through Excel for the core types.

## Deliverables

- Chart model: line, column/bar (clustered/stacked/100%), area, pie/donut, scatter, bubble, combo with secondary axis, histogram; series/axes/titles/legend/data labels/gridlines; trendlines (linear, exponential, moving average); in-cell sparklines (line, column, win/loss); ranges update on recalc.
- Vector renderer: chart → 2-D scene → SVG/PNG (`resvg`/`tiny-skia`) and an egui painter adapter; theme palette defaults from Appendix C; per-chart overrides.
- `.xlsx`: read/write drawing anchors and chart parts (DrawingML `c:` namespace) for the core types; unsupported types preserved as opaque parts and shown as placeholders with title and source ranges; sparklines via `x14` extension.
- GUI embedding (selection, move/resize, chart builder panel), TUI approximations (sparkline/bar glyphs), `omacell export --svg|--png` for a chart or range.

## Implementation notes

- Screen and export must use the same scene; snapshot both from the same fixture.

## Acceptance criteria

- [ ] Golden SVGs for every chart type across three fixture themes; egui and SVG output match structurally.
- [ ] Chart round-trip corpus for core types: open → save → LibreOffice headless renders without error; unsupported types survive byte-identical.
- [ ] Sparklines round-trip; charts update after recalc in tests.

## Tests

- Golden-file tests; round-trip tests; renderer parity tests.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-25.md` before writing code.
4. Create branch `wp/25-charts-sparklines`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-25: Charts and sparklines: model, vector renderer, `.xlsx` DrawingML core types`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
