# WP-S1 IronCalc spike

Throwaway crate. **Not** a Cargo workspace member (`Cargo.toml` `exclude = ["spikes"]`).
Do not depend on this from `crates/`.

```bash
# from the repository root
cargo run --release --manifest-path spikes/ironcalc/Cargo.toml
```

Writes a tiny `.xlsx` under `$TMPDIR/omacell-wp-s1/` (created and removed by the harness).
Does not hit the network after crates are fetched.
