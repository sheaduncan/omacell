# WP-00 — Repository bootstrap, conventions, and CI

| | |
|---|---|
| Phase | 0 — Foundations |
| Lane | A — Engine / core |
| Size | M (≈ 3–5) |
| Depends on | — |
| Unblocks | WP-S1, WP-S2, WP-01 |
| Spec sections | §11.6, §12.2, §12.3, §14 |
| Where | workspace root, all crate skeletons |

## Goal

Create the workspace and the guardrails every later package relies on. Nothing product-specific is built here; everything later assumes this exists.

## Deliverables

- Cargo workspace with crates `core`, `fn`, `io`, `lua`, `ai`, `conf`, `bus`, `ui`, `tui`, `gui`, `cli` under `crates/` (package names `omacell-<name>`), each compiling with a `lib.rs` or `main.rs` stub, `#![forbid(unsafe_code)]` (exceptions must be justified in the report), and `#![deny(missing_docs)]` on library crates.
- `rust-toolchain.toml` (pinned stable, edition 2024), `rustfmt.toml`, `clippy.toml`, `.cargo/config.toml` with workspace lints, `deny.toml` (cargo-deny: license allowlist MIT/Apache-2.0/BSD-2/BSD-3/ISC/Zlib/MPL-2.0/Unicode-3.0/CC0; GPL family denied; advisories on).
- `justfile` recipes: `check` (fmt --check, clippy -D warnings, test, doc), `test`, `test-fast` (unit only), `bench`, `fuzz <target>`, `lint`, `fmt`, `corpus-verify`, `perf-baseline`.
- CI (`.github/workflows/ci.yml`): fmt, clippy, tests, docs, cargo-deny, criterion smoke (`--test` mode); cache; the job image installs `libreoffice-calc` and `python3-openpyxl` so cross-check tests can run (they must skip cleanly when absent). A `nightly.yml` runs fuzz targets for 10 minutes each and the ignored large-file tests.
- Repo files: `AGENTS.md` (from this bundle), `CLAUDE.md` containing `@AGENTS.md`, `docs/spec/omacell-design-spec.md`, `docs/build/` (this bundle), `docs/adr/0001…0006` seeded from spec §11.2 with status *proposed*/*decided*, `LICENSE` (MIT placeholder — human confirms), `README.md`, `CHANGELOG.md`, `reports/README.md`.
- Directories with README stubs: `tests/corpus/{formulas,eval,functions,numfmt,csv,xlsx,omc,themes,evals}`, `tests/fixtures/`, `default/{config.toml,keys/classic.toml,keys/modal.toml,themed/omacell.toml.tpl,ai/prompts/,agents/skills/omacell/SKILL.md}` (placeholders, each with a `TODO(WP-xx)` marker), `packaging/{PKGBUILD,omacell.desktop,omacell.xml,icons/}` stubs, `spikes/` (excluded from workspace).
- PR template with the Definition-of-Done checklist from `AGENTS.md`; branch protection notes in `docs/build/PLAN.md` execution protocol.

## Implementation notes

- Pre-approved dependency list lives in `AGENTS.md`; add nothing else without a justification line in the report and a passing `cargo deny`.
- No network access in tests, ever. Fixtures are files in the repo.

## Acceptance criteria

- [ ] `just check` passes on a clean clone; CI is green on `main`.
- [ ] `cargo test --workspace` runs (zero tests is fine); `cargo deny check` passes.
- [ ] Tree matches spec §11.6 plus `crates/ui` and `spikes/`; every placeholder file names the package that will fill it.

## Tests

- A smoke test in `crates/cli` asserting the binary prints its version.
- A repo-lint test that fails if any `TODO(` marker lacks a `WP-` reference.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-00.md` before writing code.
4. Create branch `wp/00-bootstrap`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-00: Repository bootstrap, conventions, and CI`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
