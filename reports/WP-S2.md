# Report — WP-S2: Spike: GUI toolkit (ADR-001)

## Plan (written before coding)

- Files/modules to create:
  - `spikes/grid-egui/` — throwaway crate, **not** a workspace member (`Cargo.toml` workspace already `exclude`s `spikes/`).
    - `Cargo.toml` / `Cargo.lock` / `.gitignore` (`target/`) / `README.md`
    - `src/main.rs` — eframe entry, CLI (`--measure`), chrome, IME field, measurement loop
    - `src/grid.rs` — custom `Painter` virtualized 1,048,576 × 50 grid, frozen header row, pixel-snapped 1-px gridlines
    - `src/theme.rs` — two palettes; swap every color on `T`
    - `src/cache.rs` — shaping cache keyed by `(text, style, zoom)` wrapping egui galleys
    - `src/fonts.rs` — load fontconfig `monospace` via `fc-match` into egui `FontDefinitions`
  - `docs/adr/0001-gui-toolkit.md` — fill Decision, measured table, Qt Quick fallback + triggers (after measurements)
  - `spikes/README.md` — point at this spike
  - This report completed after the run
- Interfaces to expose (types, commands, schemas, CLI):
  - None into the product. WP-16 starts from contracts, not this code.
  - Spike-only: `cargo run --release -- --measure` prints JSON (frame times, theme-swap latency, startup, RSS, adapter) to stdout.
- Tests and corpora to write first:
  - None; package says measurements only. No network. Spike excluded from `just check` / CI.
- Items the package says to "decide and document" and the decision taken:
  - ADR-001 default stays **eframe/egui on wgpu** unless measurements or outright IME/a11y failure kill it.
  - If IME or AccessKit fail *outright*, stop and escalate in Open questions; do **not** try Qt Quick inside this spike.
  - Prefer a real GPU run on this Wayland/Omarchy box (Intel UHD 630 iGPU via `WGPU_POWER_PREF=low`; NVIDIA 1070 also present). Software rasterizer only as a fallback note for CI.
  - Manual checks to attempt: fractional scale 1.25× and 1.5× (1-px gridlines), CJK IME into a text field (fcitx5 is running), AccessKit tree for a focused cell (Orca is **not** installed — dump the tree / AT-SPI if possible, record honest miss), monospace via fontconfig (`fc-match monospace` → JetBrainsMono Nerd Font).
- Open questions at planning time:
  - Does winit/egui IME actually compose CJK on this Wayland session, or is that an outright fail (Qt trigger)?
  - AccessKit on Linux: is AT-SPI populated enough that Orca *could* read a focused cell, given Orca is not installed here?
  - Cold-start < 300 ms is a product GUI budget; a throwaway eframe window with wgpu init may miss it — record the number, do not treat miss as killing egui by itself.

## What was built

Throwaway eframe 0.36.1 / egui-on-wgpu app at `spikes/grid-egui` (not a workspace member). It paints a synthetic 1,048,576 × 50 grid with a custom `Painter`, virtualized rows and columns, a frozen header row (plus a frozen row-index column so addresses are visible), and galleys from a `(text, size, zoom, color)` shaping cache. `T` swaps every palette role. A `TextEdit` is the IME probe. The focused cell is an AccessKit `Label` (`cell A1 value A1`). `--measure` auto-scrolls, seeks near row 1,048,576, swaps the theme, prints JSON, and exits.

No product UI. `crates/gui` is untouched. WP-16 starts from contracts.

Run:

```
cargo run --release --manifest-path spikes/grid-egui/Cargo.toml -- --measure
```

## Interfaces exposed (for dependents)

None into the product. ADR-001 remains **proposed**, with egui as the plan default, until the CJK IME and Orca exit criteria are run. WP-16 should treat this crate as a sketch of painter + cache + pixel snap, not as a type source.

Spike CLI: `--measure`, `--frames N`.

## Deviations from the spec or the package (with reasons)

- Spec §11.2 recommended Qt Quick *for the spike*. Plan D3 and the WP say default egui unless measurements kill it. Measurements did not; Qt is the documented fallback, not the implementation.
- Frozen row-index column in addition to the required frozen header row — needed to verify virtualization; still throwaway.
- Tessellation feathering disabled so hairlines can be 1 device pixel (§11.4). WP-16 should keep this for the grid layer only if product chrome needs AA.
- CJK IME and Orca were not completed (environment). They are not recorded as failures, but they remain blocking exit criteria; see Open questions. Qt was **not** tried inside this spike.
- The spike/report merged, but ADR-001 intentionally remains proposed until the
  live CJK IME and Orca gates are completed.

## Measurements

Host: Omarchy 4.0.1, Hyprland 0.56.2, Wayland, Intel UHD 630 (Vulkan iGPU), `WGPU_POWER_PREF=low`, 1920×1080@144, default scale 1.5. Command (after `cargo build --release` in `spikes/grid-egui`):

`WGPU_POWER_PREF=low cargo run --release -- --measure --frames 180`

```
adapter: Intel(R) UHD Graphics 630 (CFL GT2) backend=Vulkan type=IntegratedGpu
pixels_per_point: 1.5
startup_to_first_update_ms: 143.309
scroll_frames: 180
frame_ms_median: 6.943
frame_ms_p95: 7.039
frame_ms_max: 7.156
cpu_update_ms_median: 0.368
theme_swap_ms: 8.749
rss_mib: 347.9
shaping_cache_entries: 87
focused_cell after seek: A1048537
monospace: JetBrainsMono Nerd Font via fc-match
cjk_family: Noto Sans CJK SC
```

An earlier 240-frame run on the same adapter: startup to first update 196.9 ms, frame median 6.94 ms, theme swap 6.58 ms, RSS 317 MiB (before the CJK TTC). The startup measurement is taken at entry to the first eframe update, before tessellation, rendering, and presentation; it does not prove the GUI first-frame target.

`just check` on the workspace: pass (spike excluded). `cargo clippy --release -- -D warnings` in the spike: pass.

### Manual checks

| Check | Result |
|---|---|
| Fractional 1.5× (native Hyprland scale) | HUD `ppp=1.50`. Column pitch 132 px = 88 pt × 1.5. Hairlines 1–2 physical px. Screenshot `/tmp/wp-s2/grid-1.5-nofeather.png`. |
| Fractional 1.25× (`hyprctl eval 'hl.monitor({..., scale=1.25})'`, then restored 1.5) | HUD `ppp=1.25`. Vertical hairlines **1 physical px**, pitch 110 px = 88 pt × 1.25. Screenshot `/tmp/wp-s2/grid-1.25.png`. |
| CJK IME | **Could not run.** fcitx5 running, but `/usr/share/fcitx5/inputmethod` empty of CJK engines (`pacman -Q` has fcitx5/gtk/qt only). egui-winit has `on_ime` / `set_ime_allowed`. Not an outright fail. |
| AccessKit / Orca | Orca not installed. After `ScreenReaderEnabled=true`, pyatspi tree: `application: 'grid-egui'` → `label: 'cell A1 value A1'`. Flag restored to false. |
| fontconfig monospace | JetBrainsMono Nerd Font from `fc-match monospace`. |

## Open questions / decisions needed

1. **Human / WP-28 G4 — CJK IME on Wayland blocks ADR-001.** Re-test with a
   real engine (`fcitx5-chinese-addons`, mozc, or rime). If composition never
   reaches egui, that is Qt-swap trigger 1.
2. **Human / WP-28 G4 — Orca speech blocks ADR-001.** The AT-SPI node exists;
   run Orca and confirm the focused cell address and value are spoken. Failure is
   Qt-swap trigger 2.
3. **WP-28 G4:** verify the production painter's device-pixel snapping at 1.25×,
   1.5×, and 2×; a one-logical-pixel gridline must remain one physical pixel.
4. **Human / WP-28 fixed host:** measure presentation-aware product cold start;
   the 143–197 ms pre-render spike update is not evidence for the 300 ms gate.

## RFC (only if a frozen contract changed)

None. G0 has not happened; this package does not touch frozen contracts.

## Checklist

- [x] `just check` green on the workspace (spike excluded; fmt, clippy `-D warnings`, `cargo test --workspace`, `cargo doc --workspace --no-deps`)
- [ ] Every acceptance criterion ticked with evidence — **HUMAN / WP-28 G4:** blocked on CJK IME and Orca exit checks
  - [ ] **HUMAN / WP-28 G4:** ADR-001 remains proposed until both live checks pass and the measured decision is recorded
  - [x] Spike lives under `spikes/` and is excluded from the workspace (`Cargo.toml` `exclude = ["spikes"]`) and CI (`just check` does not build it)
- [x] Docs warning-free; public items documented (no product API)
- [x] Baselines recorded (if the package has performance gates) — measurements in this report and ADR-001; not `just perf-baseline` (no criterion benches in the workspace)
- [x] No new `TODO(` without a `WP-` reference; no new workspace dependency. Spike-only deps: `eframe`/`egui` 0.36.1, `anyhow` (all pre-approved).
- [x] Nothing written outside the repository except documented temp dirs (`/tmp/wp-s2` screenshots and RGB dumps; `/tmp/grid-egui-measure*.err`; AT-SPI `ScreenReaderEnabled` toggled then restored)
