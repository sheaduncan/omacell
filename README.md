# Omacell

A spreadsheet for [Omarchy](https://omarchy.org/) Linux.

Excel *semantics* — the grid, A1 references, the formula language, recalculation,
number formats, the error model, and `.xlsx` as the exchange format — with
Omarchy's contract: text configuration under `~/.config`, the active Omarchy
theme as the only theme, keyboard-first, a first-class terminal client, and
nothing that phones home.

One engine, three clients: a Wayland GUI, a TUI, and a JSON-speaking CLI that
doubles as an IPC surface for scripts and AI agents.

**Status:** pre-alpha. Gate G0 and the Phase-1 engine foundation through WP-04
and WP-06 are merged. The next independent packages are WP-05F (function
runtime foundation) and WP-07a (in-process command bus and changesets).

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
