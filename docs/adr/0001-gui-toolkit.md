# ADR-001 — GUI toolkit

| | |
|---|---|
| Status | **Proposed — blocked on IME and Orca exit criteria** |
| Date | 2026-08-27 |
| Spec | §11.2 |
| Spike | WP-S2 (`spikes/grid-egui`) |
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

## Proposed decision

**eframe/egui on wgpu**, custom `Painter` grid (plan D3).

The 1M-row scroll, theme swap, fractional-scale, AccessKit-tree, and
fontconfig checks that could be run on this Omarchy 4 / Hyprland box support
the plan default. CJK IME and Orca speech were not executable here (no IM
engine, no Orca package), however, and the spec defines both as M0 exit
criteria. ADR-001 therefore remains proposed until both checks pass. A
failure invokes the Qt Quick triggers below rather than being deferred as
polish.

WP-16 starts from contracts, not from `spikes/grid-egui`.

## Measured table (WP-S2)

Host: Omarchy 4.0.1, Hyprland 0.56.2, Wayland. Intel UHD Graphics 630
(Coffee Lake GT2) via Vulkan, `WGPU_POWER_PREF=low`. Display 1920×1080@144 Hz.
Spike: eframe/egui 0.36.1, synthetic 1,048,576 × 50 grid, virtualized,
shaping cache, `feathering = false`. Command:
`cargo run --release -- --measure` from `spikes/grid-egui`.

| Criterion | Target | Result | Notes |
|---|---|---|---|
| Scroll frame time (1M-row sheet) | < 16 ms (60 fps) | **median 6.94 ms**, p95 7.04 ms, max 7.16 ms | Vsync-capped at 144 Hz. CPU `update` median 0.37 ms. |
| Theme swap (all colors, keypress) | < 100 ms, no flicker | **8.7 ms** | Cache cleared; next frame painted the new palette. |
| Startup to first update | Informational; GUI first frame target < 300 ms | **143 ms** (repeat 197 ms) | Measured at entry to the first eframe update, before tessellation/render/present; this does not prove the first-frame target. |
| Resident memory | 1M×20 numeric < 1.5 GB | **348 MiB RSS** | Synthetic grid; no cell storage. Includes wgpu + Noto CJK TTC. |
| Fractional scale 1.25× | 1-px gridlines | **pass** | `ppp=1.25`. Vertical lines 1 physical px, pitch 110 px = 88 pt × 1.25. |
| Fractional scale 1.5× | 1-px gridlines | **pass with AA caveat** | `ppp=1.50` native. Pitch 132 px exact; hairlines 1–2 physical px (stroke coverage). |
| CJK IME into a text field | compose on Wayland | **not run** | fcitx5 is running (`XMODIFIERS=@im=fcitx`) but no CJK engine (no pinyin/rime/mozc). egui-winit implements `WindowEvent::Ime` / `set_ime_allowed`. |
| AccessKit / Orca, focused cell | AT-SPI name has address + value | **tree pass; Orca not installed** | With `org.a11y.Status.ScreenReaderEnabled=true`, pyatspi sees `application: 'grid-egui'` and `label: 'cell A1 value A1'`. Orca (`extra/orca`) is not installed. |
| Monospace via fontconfig | `fc-match monospace` | **pass** | JetBrainsMono Nerd Font (`/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf`). CJK fallback: Noto Sans CJK SC from fontconfig. |

Software rasterizer was not required; this machine has a real iGPU. CI has
no GPU — WP-16 should keep `egui_kittest` + a relaxed software-raster budget.

## Fallback: Qt Quick via `cxx-qt`

Adopt egui once the outstanding exit criteria pass, unless one of these
triggers fires. A Qt Quick front-end is a
**separate lane** (new WP, not a drive-by in WP-16): QML + a custom
scene-graph grid item, still talking to `omacell-bus` only.

**Swap triggers**

1. **IME outright fail.** A CJK engine (fcitx5-chinese-addons / mozc / rime)
   is installed, the spike or WP-16 `TextEdit` is focused, and composition
   never reaches egui (`WindowEvent::Ime` dropped, or committed text never
   appears). Pre-edit tofu because a CJK *font* is missing is not a trigger
   (load Noto CJK).
2. **Accessibility outright fail.** Orca is running and cannot read the
   focused cell's address and value from the AccessKit/AT-SPI tree. WP-S2
   produced the node, but that alone does not satisfy the Orca exit criterion.
3. **Fractional scale unusable.** At 1.25×/1.5×/1.75×, gridlines stay
   blurry >2 physical pixels after device-pixel snapping and with
   tessellation feathering off — i.e. winit/`wp-fractional-scale-v1` is
   wrong, not our painter.
4. **Frame budget miss on iGPU.** Sustained scroll of a 1M-row sheet
   > 16 ms median on Framework-class integrated graphics after WP-16's
   virtualized painter (not this throwaway) is in place.

Missing packages on a spike box (no Orca, no pinyin) are not failures, but
they leave the corresponding exit criterion open and block this decision.

GTK4 was not spiked; do not fall back to it without a new ADR.
