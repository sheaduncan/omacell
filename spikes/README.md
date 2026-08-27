# Spikes

Excluded from the Cargo workspace. WP-S1 (engine) and WP-S2 (GUI toolkit)
live here as throwaway crates so they cannot leak types into `crates/`.

- [`grid-egui/`](grid-egui/) — WP-S2: eframe/egui on wgpu, 1,048,576 × 50 virtualized grid (ADR-001).
