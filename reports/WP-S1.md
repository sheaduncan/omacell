# Report — WP-S1: Spike: build the engine or adopt IronCalc (ADR-002)

## Plan (written before coding)

- Files/modules to create:
  - `spikes/ironcalc/` — throwaway, workspace-excluded crate (not a workspace member; no types leak into `crates/`).
    - `Cargo.toml` depending on `ironcalc` / `ironcalc_base` from crates.io if they exist; git only as a fallback documented in this report.
    - `src/main.rs` — measurement harness: generate 100k-formula and (if feasible) 1M-formula workbooks in memory, edit one input cell, time incremental and full recalc; probe dynamic arrays / spill / `LET` / `LAMBDA`; probe `SUM(A:A)` graph behavior; optional `.xlsx` round-trip of a tiny L1/L2 fixture; print binary-size of the spike binary.
    - `README.md` — how to run the harness (`cargo run --release --manifest-path spikes/ironcalc/Cargo.toml`).
  - `docs/adr/0002-engine.md` — fill the proposed ADR with an explicit **build** or **adopt** decision and the rubric from WP-S1.
  - `reports/WP-S1.md` — this report.
  - If and only if evidence for **adopt** is strong: replacement package sketches WP-02A/03A/04A. Do **not** rewrite `PLAN.md` unless that evidence is strong (plan default D2 is **build**).
- Interfaces to expose (types, commands, schemas, CLI):
  - None in product crates. This package does not edit `crates/core` (that is WP-01).
  - The durable interface is the ADR decision plus measured numbers for WP-01 / Phase 1 agents.
- Tests and corpora to write first:
  - No workspace tests (WP-S1: "None beyond the measurements recorded in the ADR").
  - Spike harness is a `main` binary, not a `#[test]`, so it never hits the network from test code and is not run by `just check`.
  - Tiny in-memory workbooks only; no third-party `.xlsx` committed.
- Items the package says to "decide and document" and the decision taken:
  - **Build vs adopt (ADR-002).** Time-box Size S. If evidence is inconclusive, or IronCalc cannot be used (license, API, missing crate, registry), the decision is **build** `omacell-core` (plan default D2).
  - License vs `deny.toml`: MIT/Apache-2.0 is on the allowlist; GPL-family is not. Confirm the crate's declared license and transitive graph *in the spike only* (do not add IronCalc to the product workspace graph).
  - `.xlsx` L1/L2 coverage, dynamic arrays / spill / `LET` / `LAMBDA`, range-aware graph (`SUM(A:A)` one edge vs a million), fit to upcoming WP-01 contracts, upstreaming path, binary size, async-node hook feasibility (§8.3 / WP-04 `AsyncNodeProvider`).
  - Work-package cost of each path: **build** keeps WP-02/03/04 as written; **adopt** would require WP-02A/03A/04A adapters and would *not* be executed in this package.
- Open questions at planning time:
  - Is `ironcalc` 0.8.x on crates.io complete enough (xlsx I/O + eval) or is the engine split (`ironcalc_base` vs `ironcalc`)?
  - Does the public API expose the dependency graph so we can count edges for `SUM(A:A)`?
  - Can a 1M-formula workbook be built in-process within the Size S budget without exhausting RAM?
  - Does IronCalc have any hook for async / pending cell values, or would AI nodes force a fork?
  - Maintainer responsiveness: GitHub activity is public; we will not open an upstream issue in this spike unless we need a blocking answer.

## What was built

Throwaway crate `spikes/ironcalc` (own `[workspace]` so it is not a member of `crates/*` and does not inherit the product `profile.release` LTO). Depends on **`ironcalc = "0.8.3"` from crates.io** (`ironcalc_base` comes in transitively). No git dependency.

Harness (`spikes/ironcalc/src/main.rs`):

- Formula probes: `LET`, `LAMBDA`, named lambda, `SEQUENCE` spill, blocked `#SPILL!`, `UNIQUE`, `MAP`, `SUM(A:A)`.
- Tiny L1/L2 `.xlsx` save/load under `$TMPDIR/omacell-wp-s1/` (removed after).
- 100k- and 1M-formula in-memory workbooks (A = numbers, B = `=An*2`), edit A1, time `evaluate()`.
- Isolated `--numeric-memory-only` probe: one million plain numbers, RSS delta over an empty model.
- RSS via `/proc/self/status`, release binary size.

`docs/adr/0002-engine.md` is **Decided: build `omacell-core`**. `PLAN.md` is unchanged. WP-02A/03A/04A files were not added (evidence for adopt is not strong).

The primary checkout was claimed by WP-01 mid-spike; this branch lives in worktree `/tmp/omacell-wp-s1` so WP-01's tree is untouched. Spike sources are in the repo under `spikes/ironcalc/`.

## Interfaces exposed (for dependents)

None in product crates. Dependents should read:

| Item | Where |
|---|---|
| Decision | **Build `omacell-core`** — `docs/adr/0002-engine.md` |
| Plan D2 | Unchanged: WP-02, WP-03, WP-04 as written |
| Measurements | This report + the ADR |
| Spike (not a dep) | `cargo run --release --manifest-path spikes/ironcalc/Cargo.toml` |

WP-01 proceeds against `crates/core` with no IronCalc types.

## Deviations from the spec or the package (with reasons)

- Spike branch is `wp/s1-spike-engine` (package table also mentioned throwaway `spike/ironcalc`). Same content.
- IronCalc from crates.io 0.8.3, not a git tag. Docs still recommend git until 1.0; crates.io worked and satisfies `deny.toml` `unknown-git = "deny"` if we ever productized it.
- `SUM(A:A)` edge count is from source (`CellOrRange::Range`) plus issue #849, not a public counter (`support` is `pub(crate)`).
- Isolated 100k and the 100k pass that shared a process with 1M disagree by ~2×; both show incr ≈ full. The ADR quotes the isolated process as the primary 100k number.
- Concurrent WP-01/WP-S2 on other worktrees; this package does not commit their files and does not edit `crates/core`.
- The historical PR could not tick its own merge gate; ADR-002 and the report acceptance item were subsequently merged and are now checked below.

## Measurements

Host: Intel Core i7-8750H @ 2.20 GHz, 12 threads, Linux. `ironcalc` 0.8.3.
`cargo run --release --manifest-path spikes/ironcalc/Cargo.toml`.
No network from the harness after crates were fetched.

**Isolated 100k** (`--skip-1m`, fresh process):

| Step | Time | Notes |
|---|---|---|
| Build 100k inputs + 100k formulas | 224 ms | |
| Full `evaluate()` #1 | **97.2 ms** | B1=2, B100000=200000 |
| `evaluate()` after A1→100 | **95.3 ms** | B1=200; ratio 0.98 vs full |
| Full `evaluate()` #2 | 96.8 ms | |
| RSS | 74 MB | Mixed input/formula model; not used to infer plain numeric-cell cost |

**1M** (same binary, after 100k in-process):

| Step | Time | Notes |
|---|---|---|
| Build 1M + 1M | 3.72 s | |
| Full `evaluate()` #1 | **1.69 s** | B1=2, B1000000=2000000 |
| After A1→100 | **1.61 s** | ratio 0.95 |
| Full #2 | 1.51 s | |
| RSS | 761 MB | Mixed input/formula model |

**Numeric-only memory** (fresh process, `--numeric-memory-only`): one million
plain numbers increased RSS from 6,756 KiB to 498,488 KiB, a 491,732 KiB
delta or approximately **503.5 B/plain numeric cell**. This avoids mixing
formula ASTs and dependency state into the numeric-cell estimate.

§12.1: 100k incremental **< 50 ms — miss** (and not incremental). 1M full **< 5 s / 8 threads — meet on 1 thread** (1.69 s); no 8-thread API. 64 B/plain numeric cell — **miss** (~503.5 B/cell isolated RSS delta).

`SUM(A:A)` vs `SUM(A1:A100000)` at 100k numbers: 57 ms vs 50 ms (isolated).

`.xlsx` L1: A1=42, A2=84, formula `=A1*2`, `100$` format. L2 style bold + fill `#FF9011` round-tripped.

Formula probes: `LET`/`LAMBDA`/named lambda/`SEQUENCE` spill/`#SPILL!`/`UNIQUE`/`MAP` as in the ADR.

Binary under matching thin-LTO/codegen settings: spike 8.1 MiB vs `omacell`
CLI 428 KiB. The spike includes its harness and `anyhow`, so the observed
~7.6 MiB difference is not presented as a dependency-size floor.

`cargo deny --config deny.toml --manifest-path spikes/ironcalc/Cargo.toml check` — pass (spike only).

`just check` — run on this worktree (spike excluded); see Checklist.

## Open questions / decisions needed

1. **Resolved:** merge recorded ADR-002 as decided. No further engine ADR unless
   IronCalc later ships a public DAG + async hook + 64 B-class storage.
2. **Resolved:** do not add a test-only IronCalc oracle; LibreOffice, openpyxl,
   Excel-authored fixtures, and the live Excel checklist provide better coverage.
3. **Resolved planning questions:**
   - crates.io `ironcalc` 0.8.3 includes xlsx I/O; `ironcalc_base` is the engine.
   - Graph is not public; `CellOrRange` exists internally; eval is full-sheet (#849).
   - 1M workbook ran (761 MB RSS, 1.69 s eval).
   - No async hook.
   - Maintainers active (commits on 2026-08-27).

## RFC (only if a frozen contract changed)

None. G0 has not happened. This package does not touch frozen types.

## Checklist

- [x] `just check` green on this worktree (spike excluded from the workspace). See Measurements / commit notes if CI on GitHub has not run.
- [x] Acceptance: ADR filled with **build** and measured numbers (`docs/adr/0002-engine.md`). Spike lives under `spikes/` (workspace `exclude`). Merge recorded the human ADR decision.
- [x] Docs warning-free; no new public product items
- [x] Baselines recorded in the ADR/report (spike, not `just perf-baseline` — no criterion target in this package)
- [x] No new `TODO(` without a `WP-` reference; IronCalc is spike-only, not a product dependency
- [x] Nothing written outside the repository except `$TMPDIR/omacell-wp-s1/` (xlsx probe, deleted) and cargo/registry caches; this branch was finished in git worktree `/tmp/omacell-wp-s1` because the primary worktree was on `wp/01-core-contracts`
