# AGENTS.md — conventions for agents working in this repository

You are building **Omacell**, a spreadsheet for Omarchy Linux. The design spec is `docs/spec/omacell-design-spec.md`; the build plan and work packages are in `docs/build/`. You are implementing exactly one work package at a time. This file is binding.

## Read order for every task
1. This file.
2. Your work package: `docs/build/wp/WP-NN-*.md`.
3. Only the spec sections your package lists. Do not read the whole spec into context.
4. `reports/<dep>.md` for each package you depend on (their *Interfaces exposed* sections).
5. `docs/contracts.md` (frozen public types) and `docs/schemas/` when touching commands, config, IPC, MCP, or the workbook card.

## Toolchain and commands
- Rust stable (pinned in `rust-toolchain.toml`), edition 2024, one Cargo workspace.
- **Cargo target dir lives on `/home`, never on `/tmp`.** `/tmp` is a 16 GiB tmpfs; `rust-lld` mmaps the linker output and **SIGBUS**es when tmpfs is full (that is not a compiler bug). Before any `cargo` invocation export `CARGO_TARGET_DIR="${HOME}/.cache/omacell/target"` (the justfile and `.envrc` already do). Never clone, `cp -a`, or `git worktree add` this repo onto `/tmp`. Isolated work uses Grok `isolation: "worktree"` (under `~/.grok/worktrees`) or a worktree under `$HOME`. Leave existing `/tmp/omacell-pr*` trees alone unless the user asks to delete them. Fallback only if a build is already stuck on tmpfs: `CARGO_BUILD_JOBS=2` and `RUSTFLAGS='-C link-arg=--no-mmap-output-file'`. Details: `.grok/rules/cargo-target.md`.
- `just check` = `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo doc --no-deps`. It must pass before you open a PR.
- `just test-fast` for unit tests while iterating; `just bench` for criterion; `just fuzz <target>`; `just corpus-verify`; `just perf-baseline`.
- Cross-check scripts (`scripts/lo-crosscheck.py`, openpyxl loaders) require LibreOffice/Python; tests using them must skip cleanly when absent.

## Architecture rules (violations are PR rejections)
- Crate boundaries: `core` has no I/O, no toolkit, no async. `fn` depends on `core` only. `io` depends on `core`. `ui` (shared UI logic) has no `egui`/`ratatui`/`winit` types. `tui` and `gui` are thin renderers over `ui` + `bus`. `bus` is the only mutation path for anything outside `core`. `ai` and MCP are the only async code (`tokio`). `conf` owns configuration and Omarchy adapters.
- Every mutation is a registered command with a JSON schema. Models and agents mutate only through changesets. No exceptions, including "temporary" ones.
- Public types in `crates/core` from WP-01 are **frozen after Gate G0**. Command schemas freeze when WP-07a lands; the IPC envelope freezes when WP-07b lands; MCP and workbook-card wire formats freeze when their owning packages land. Changing a contract after its freeze point requires an `RFC` section in your report and human approval before merge.
- No feature may run on file open, and no feature may send data anywhere, without an explicit user or agent action.
- Never write under `/usr/share/omarchy` or `~/.config/omarchy` except from `omacell setup omarchy`, and never from the package installer. Use `omarchy-notification-send` when present, else the freedesktop D-Bus interface.

## Coding standards
- `#![forbid(unsafe_code)]` everywhere; justify any exception in the report.
- Library crates: `#![deny(missing_docs)]`, `thiserror` errors with stable `{code, message, hint}`, no `unwrap`/`expect` outside tests (use `?` and typed errors), no `panic!` on input data of any kind.
- Binaries: `anyhow` allowed at the top level only.
- Logging via `tracing`; no `println!` in libraries.
- Determinism: parallel code must yield bit-identical results regardless of thread count. Sort keys explicitly; never iterate a `HashMap` for output.
- Names: `snake_case` modules, dotted command ids (`range.sort`), config keys as in Appendix B of the spec. Product name lives in `crates/core/src/product.rs` (`PRODUCT_NAME`) and `packaging/name.env` only.
- Keep functions small; prefer data-driven tables (formats, function metadata, keymaps) over branches.

## Testing rules
- Tests and corpora first, implementation second. Corpus rows cite the documented behavior they encode (comment or `note` column).
- Kinds: unit, `proptest` property tests, `insta` snapshots, corpus table tests, `criterion` benches with committed baselines, `cargo-fuzz` targets for every parser.
- No network in tests, ever. Provider tests replay recorded fixtures under `tests/fixtures/ai/`.
- Do not delete, `#[ignore]`, or loosen an existing test to make yours pass. If a test is wrong, fix it and say so in the report.
- Large or slow tests are `#[ignore]` and run by the nightly workflow; say so in the test name.
- Fixture licensing: Omarchy theme fixtures carry the upstream MIT notice; never commit third-party `.xlsx` files without permission (generate with `scripts/corpus-gen/`).

## Performance and memory budgets (from spec §12.1)
Cold start GUI < 300 ms, TUI < 100 ms · 100 MB CSV first paint < 1 s · 50 MB `.xlsx` open/save < 5 s · incremental recalc in a 100k-formula model < 50 ms · full recalc 1M formulas < 5 s (8 threads) · keystroke to paint < 16 ms · 60 fps scroll at any size · 1M×20 numeric ≤ 1.5 GB · theme reload < 100 ms · ≤ 64 B amortized per plain numeric cell. Packages that touch these record baselines with `just perf-baseline`.

## Security rules
- Parsers (formula, number format, zip, XML, CSV, `.omc`, IPC, MCP) enforce size/depth/ratio limits, disable external entities, and have fuzz targets.
- Secrets only via environment variable or `secret_cmd`; a leak test greps config and logs.
- Embedded scripts run sandboxed; trust is explicit and per file hash; nothing prompts on open.
- AI payloads are built only by `crates/ai::policy` (the single privacy choke point).

## Pre-approved dependencies
`serde`, `serde_json`, `schemars`, `thiserror`, `anyhow` (bins), `rayon`, `smallvec`, `indexmap`, `rustc-hash`, `memchr`, `regex` (with size limits), `chrono` (civil dates only), `unicode-segmentation`, `toml`, `notify`, `tracing`, `tracing-subscriber`, `clap`, `clap_complete`, `clap_mangen`, `zip`, `quick-xml`, `csv`, `encoding_rs`, `calamine` (dev only), `mlua` (`lua54`, `vendored`), `rmcp`, `tokio` (ai/mcp only), `reqwest` (`rustls-tls`, ai only), `async-trait`, `ratatui`, `crossterm`, `eframe`/`egui`/`egui_kittest`/`wgpu`, `accesskit` (via egui), `fontdb`, `resvg`/`tiny-skia`, `pdf-writer`, `ttf-parser`, `arrow`/`parquet`, `zbus`, `rustix`, `proptest`, `insta`, `assert_cmd`, `criterion`, `cargo-fuzz` (nightly job). Anything else: one justification line in your report and a green `cargo deny check`. License allowlist is in `deny.toml`; GPL-family crates are rejected.

## Git workflow
- Branch `wp/NN-slug` from `main`; PR title `WP-NN: <title>`; one package per PR.
- Atomic, conventional commits (`feat(core): …`, `test(fn): …`, `docs: …`); do not mix unrelated changes; never force-push shared branches; never merge your own PR.
- The PR description links the report and lists the acceptance boxes ticked.

## Reporting
Write `reports/WP-NN.md` from `docs/build/templates/wp-report.md`. Sections: Plan (before coding) · What was built · Interfaces exposed · Deviations with reasons · Measurements · Open questions · RFC (if contracts changed) · Checklist. Reports are read by the next agent; write for them.

## Definition of done
`just check` green on a clean clone · every acceptance criterion ticked with evidence · docs warning-free · baselines recorded where required · report complete · no new `TODO(` without a `WP-` reference · no new dependency without justification · nothing written outside the repo except documented locations (`$HOME/.cache/omacell/` for Cargo artifacts; small test fixtures via `std::env::temp_dir()`). Never a repo checkout or `target/` on `/tmp`.

## When unsure
Stop. Write the question and the options into *Open questions* in your report, pick nothing, and finish everything else in the package. Guessing on Excel semantics, contracts, or privacy behavior is worse than leaving a box unticked.
