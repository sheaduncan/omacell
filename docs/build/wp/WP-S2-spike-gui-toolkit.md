# WP-S2 — Spike: GUI toolkit (ADR-001)

| | |
|---|---|
| Phase | 0 — Foundations |
| Lane | C — Surfaces (conf, UI core, TUI, GUI, charts, print) |
| Size | M (≈ 3–5) |
| Depends on | WP-00 |
| Unblocks | WP-16 |
| Spec sections | §7.1, §7.2, §7.3, §11.2 ADR-001, §11.4, §12.1, §12.4 |
| Where | throwaway `spikes/grid-egui` |

## Goal

Confirm the plan's default of a pure-Rust GUI (eframe/egui on wgpu) by measuring the five exit criteria from ADR-001 on a throwaway grid, and record what would trigger a switch to Qt Quick.

## Deliverables

- An eframe app that renders a synthetic 1,048,576-row × 50-column grid with a custom `Painter`, virtualized rows/columns, frozen header row, and text drawn from a shaping cache.
- Measurements: frame time while scrolling (software rasterizer in CI, real GPU on a dev machine), theme swap latency (swap all colors on a keypress), startup to first frame, memory.
- Manual checks recorded in the ADR: fractional scaling at 1.25× and 1.5× (crisp 1-px gridlines), CJK IME input into a text field on Wayland, AccessKit tree visible to Orca for a focused cell, monospace font loaded through fontconfig.
- `docs/adr/0001-gui-toolkit.md`: decision (default: egui), the measured table, the fallback (Qt Quick via cxx-qt) and the triggers that would justify it.

## Implementation notes

- Do not build product UI here; WP-16 starts from the contracts, not from this code.
- If egui fails IME or accessibility outright, stop and escalate to the human gate rather than trying a second toolkit inside the spike.

## Acceptance criteria

- [ ] ADR merged with measurements.
- [ ] Spike lives under `spikes/` and is excluded from the workspace and CI.

## Tests

- None; measurements only.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-S2.md` before writing code.
4. Create branch `wp/s2-spike-gui-toolkit`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-S2: Spike: GUI toolkit (ADR-001)`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
