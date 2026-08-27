# grid-egui (WP-S2 throwaway)

Not a workspace member. Supplies evidence for proposed ADR-001: eframe/egui
on wgpu can paint a synthetic 1,048,576 × 50 grid. The ADR remains blocked
on its CJK IME and Orca exit checks.

```
cargo run --release --manifest-path spikes/grid-egui/Cargo.toml
cargo run --release --manifest-path spikes/grid-egui/Cargo.toml -- --measure
```

Keys: `T` theme swap · click a cell to focus · type in the IME field · `Q` / `Esc` quit.

`--measure` auto-scrolls, swaps the theme, prints JSON, and exits.
Prefer `WGPU_POWER_PREF=low` to pin the Intel iGPU on this dual-GPU laptop.
