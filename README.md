# Omacell

A spreadsheet for [Omarchy](https://omarchy.org/) Linux.

Excel *semantics* — the grid, A1 references, the formula language, recalculation,
number formats, the error model, and `.xlsx` as the exchange format — with
Omarchy's contract: text configuration under `~/.config`, the active Omarchy
theme as the only theme, keyboard-first, a first-class terminal client, and
nothing that phones home.

One engine, three clients: a Wayland GUI, a TUI, and a JSON-speaking CLI that
doubles as an IPC surface for scripts and AI agents.

**Status:** pre-alpha. Merged: WP-00–WP-21, WP-15a, WP-05F, WP-24–WP-26, WP-29,
WP-S1, WP-S2 (reports in [`reports/`](reports/)). Remaining: WP-22 (AI
providers), WP-23 (in-app AI), WP-24a (pivot fidelity), WP-27 (other formats),
WP-28 (release), WP-30 (GitHub settings). See
[`docs/build/PLAN.md`](docs/build/PLAN.md) for the live dispatch point.

## Build

Requires Rust 1.98 (see `rust-toolchain.toml`) and [just](https://github.com/casey/just).

```bash
just check          # fmt, clippy, tests, docs
just test-fast      # unit tests while iterating
cargo run -p omacell-cli -- --version
```

## Layout

```
crates/     core fn io lua ai conf bus ui tui gui cli
default/    shipped config, keymaps, theme template, skill
docs/spec   design specification
docs/build  work packages and the agent build plan
docs/adr    architecture decision records
reports/    per-package agent reports
packaging/  PKGBUILD, desktop entry, mime, icons
spikes/     throwaway crates (excluded from the workspace)
tests/      corpora and fixtures
```

See [`AGENTS.md`](AGENTS.md) for repository conventions, [`docs/spec/omacell-design-spec.md`](docs/spec/omacell-design-spec.md)
for the product, and [`docs/build/PLAN.md`](docs/build/PLAN.md) for the build order.

## License

MIT. Placeholder — confirm before the first public tag.
