# ADR-001 — GUI toolkit

| | |
|---|---|
| Status | **Proposed** |
| Date | 2026-08-27 |
| Spec | §11.2 |
| Spike | WP-S2 |
| Plan default (D3) | eframe/egui on wgpu, custom grid painter |

## Context

Omacell needs a native Wayland GUI that tiles under Hyprland, follows the
Omarchy theme, supports IME, and exposes the grid to Orca. The front-end is
isolated behind the command bus, so a later swap is contained.

## Options

1. **Qt Quick (QML) via `cxx-qt`**, custom scene-graph grid item — recommended
   by the spec for the spike: Omarchy 4's shell is Qt Quick; mature Wayland
   fractional scaling, IME, AT-SPI; two-language boundary and Qt weight.
2. **GTK4 (`gtk4-rs`)**, custom `GtkDrawingArea` — good a11y; theming via CSS
   is less direct than QML properties.
3. **Pure Rust (`eframe`/`egui` + `wgpu`)** — one language, one binary, fastest
   agent iteration; accessibility and IME are the weak points.

## Decision

Proposed (for agent execution until WP-S2 reports): **eframe/egui on wgpu**.
Revisit if WP-S2 fails IME or accessibility; then plan a Qt Quick front-end
as a separate lane.

## Spike exit criteria (WP-S2)

1M-row scroll at 60 fps on integrated graphics; theme hot-reload without
flicker; correct fractional scaling at 1.25×/1.5×; CJK IME into a cell; Orca
reading the active cell.
