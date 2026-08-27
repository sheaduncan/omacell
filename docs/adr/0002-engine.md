# ADR-002 — Engine: build or adopt

| | |
|---|---|
| Status | **Decided** (WP-S1) |
| Date | 2026-08-27 |
| Spec | §11.2, §11.3, §12.1 |
| Spike | WP-S1 (`spikes/ironcalc`, IronCalc 0.8.3 from crates.io) |
| Plan default (D2) | Build `omacell-core` per spec §11.3 |

## Context

The formula engine, dependency graph, dynamic arrays, `LET`/`LAMBDA`, and
async AI nodes are the product. IronCalc is a Rust, open-source,
Excel-compatible candidate.

## Options

1. **Build `omacell-core`** — full control over dynamic arrays, async AI
   nodes, and the 64-byte numeric-cell budget.
2. **Adopt IronCalc** — if license, LAMBDA/dynamic-array coverage, and
   1M-formula graph performance hold; contribute rather than fork.

## Decision

**Build `omacell-core`** per spec §11.3. Do not adopt IronCalc as
`omacell-core`. Do not rewrite `PLAN.md`. Phase 1 stays WP-02 / WP-03 /
WP-04 as written.

IronCalc 0.8.3 is a strong *reference* (functions, `LET`/`LAMBDA`, spill,
L1 `.xlsx`) and a possible later oracle, but it misses the incremental
graph, the 64-byte cell budget, WP-01 types, L3 part preservation, and
the §8.3 async-node hook. Adopting it would not delete the hard parts of
WP-04; it would add an adapter tax on a pre-1.0 API.

## Rubric (WP-S1)

### License vs `deny.toml`

IronCalc (`ironcalc` + `ironcalc_base` 0.8.3) is **MIT OR Apache-2.0**.
`cargo deny --config deny.toml --manifest-path spikes/ironcalc/Cargo.toml check`
on the spike graph: **advisories / bans / licenses / sources ok**.
Allowlist unused: ISC, MPL-2.0. Duplicate `syn` 2.x/3.x (warn only).

GPL-family crates are absent. License is **not** a reason to reject.

If adopted into the *product* workspace, `rand` (via `ironcalc_base` on
native) is not on the AGENTS.md pre-approved list and would need a
justification line. Spike-only: no change to the product graph.

### `.xlsx` L1 / L2 coverage

| Level | Spec | IronCalc 0.8.3 |
|---|---|---|
| L1 | values, formulas, number formats round-trip | **Yes** in the spike: `42`, `=A1*2` → `84`, `100$` format preserved across `save_to_xlsx` / `load_from_xlsx`. |
| L2 | styles, merged, names, tables, validation, CF, comments, hyperlinks, freeze/split, print, pivot, charts | **Partial.** Cell fill + bold round-tripped. Import loads styles, tables, CF, defined names, theme (source). Comments: no public API, upstream #295 (v2.0). Charts / pivots: roadmap v2.0. |
| L3 | unknown parts preserved byte-for-byte | **No.** Import rebuilds a `Workbook`/`Model`; there is no part-preserve API. D7 (own OOXML layer) remains. |

### Dynamic arrays / spill / `LET` / `LAMBDA`

All exercised in `spikes/ironcalc` and produced Excel-like results:

| Probe | Result |
|---|---|
| `=LET(x,2,x+40)` | `42` |
| `=LAMBDA(x,x+1)(41)` | `42` |
| `=LET(inc,LAMBDA(n,n+1),inc(41))` | `42` |
| defined name `IncOne` = `LAMBDA(x,x+1)`; `=IncOne(41)` | `42` |
| `=SEQUENCE(3)` | spills `1`,`2`,`3` |
| blocked `SEQUENCE` | `#SPILL!` |
| `=UNIQUE({1;1;2})` | `1` (spill) |
| `=MAP(A1:A3,LAMBDA(x,x*2))` | `20` (spill) |

Function enum includes `Let`, `Lambda`, `Map`/`Reduce`/`Scan`/`Byrow`/`Bycol`/`Makearray`,
`Sequence`, `Unique`, `Sort`, `Filter`, `Xlookup`. Coverage here is a
reason to *learn from* IronCalc, not sufficient to adopt it.

### Range-aware graph (`SUM(A:A)`)

Source: `support: HashMap<CellReferenceIndex, Vec<CellOrRange>>` with
`CellOrRange::{Cell, Range}`. A whole-column reference **can** be stored
as one `Range` edge. The map is `pub(crate)`; edges cannot be counted
from the public API.

`evaluate()` still walks every formula cell. Upstream
[ironcalc/IronCalc#849](https://github.com/ironcalc/IronCalc/issues/849)
(open, v1.0): *“when a cell is changed the whole sheet gets invalidated
and every cell gets re-evaluated.”* Roadmap still lists “Update main
evaluation algorithm with a support graph.”

Measured (100k numeric cells in A): `SUM(A:A)` 57–106 ms vs
`SUM(A1:A100000)` 50–94 ms — same order; they iterate the used range,
not a million empty cells. That is not the §11.3 incremental dirty walk.

### Fit to upcoming WP-01 contracts

| WP-01 / §11.3 | IronCalc 0.8.3 |
|---|---|
| `Value` 16-byte tagged union with `Error`, `Array` handle | `CellValue { None, String(String), Number, Boolean }` — owned strings, no error/array variants (`#SPILL!` appeared as `String`) |
| `CellRef { row: u32, col: u16 }` | `sheet: u32, row: i32, column: i32` |
| 256×256 block storage, ≤64 B/numeric cell | `HashMap<row, HashMap<col, Cell>>` |
| `Changeset` + command bus | `UserModel` undo/diffs; not origin/status/forward/inverse |
| `core` has no I/O | `ironcalc` is xlsx I/O; `ironcalc_base` depends on `csv` |
| Frozen after G0 | 0.8 is pre-1.0 (“expect things to change”) |

An adopt path is an adapter over a foreign model, not a drop-in for WP-01.

### Upstreaming path and maintainer responsiveness

- Dual MIT/Apache-2.0; contributing guide: contact before large work;
  PRs merged by `nhatcher`.
- Activity: multiple commits on 2026-08-27 (this spike’s date); Discord
  listed. NLnet / NGI0 funding. ~4.1k GitHub stars.
- 1.0 still open (arrays/CF mostly in; **support-graph eval is not**).
  2.0: charts, pivots, comments.
- Willingness to upstream is real. That does not land a DAG, async nodes,
  or L3 preserve on Omacell’s schedule. A product dependency on 0.8 is a
  fork-shaped risk.

### Binary size

| Binary | Size (release, stripped debuginfo) |
|---|---|
| `omacell` CLI (WP-00, no engine) | 429 KiB |
| `omacell-spike-ironcalc` (engine + xlsx) | 9.2 MiB |

Adopting IronCalc is a **~9 MiB** floor for any binary that links it
(GUI/TUI would add more). Not a veto; not free.

### Async-node hook feasibility (§8.3 / WP-04)

No public `Pending` / `Ready` / `Failed` cell state. `Model::evaluate()`
is synchronous, single-threaded, two-phase (spill then the rest).
`UserModel` re-evaluates on every user action. There is no
`AsyncNodeProvider`. AI cells would require a fork of the eval loop
(the work WP-04 already schedules). `core` also forbids async.

### Measured numbers

Host: Intel Core i7-8750H @ 2.20 GHz, 12 threads, Linux. Command:
`cargo run --release --manifest-path spikes/ironcalc/Cargo.toml`.
Workbook: column A numbers `1..=N`, column B `=An*2` (independent
formulas). Edit A1 from `1` to `100`; B1 becomes `200`. Engine is
**one thread**.

| Workbook | Build | Full eval | Eval after one edit | Full eval #2 | RSS after |
|---|---|---|---|---|---|
| 100k formulas (isolated process) | 224 ms | **97 ms** | **95 ms** (ratio 0.98) | 97 ms | 74 MB |
| 1M formulas | 3.72 s | **1.69 s** | **1.61 s** (ratio 0.95) | 1.51 s | 761 MB |

§12.1 gates:

- Incremental recalc after one edit in a 100k-formula model **< 50 ms**:
  **miss** (95 ms, and it is a full pass).
- Full recalc 1M formulas **< 5 s on 8 threads**: **meet on 1 thread**
  (1.69 s). No rayon; no 8-thread path.
- ≤ 64 B/numeric cell: **miss**. 1M formula + 1M input ≈ 2M cells at
  761 MB ≈ **380 B/cell**.

A second 100k pass in the same process as the 1M run was slower
(full 215 ms / incr 173 ms) — allocator/cache noise; both passes show
incr ≈ full.

Tiny `.xlsx` round-trip used `$TMPDIR/omacell-wp-s1/` (created and
removed by the harness).

## Recommendation and work-package cost

### Path A — **build** (chosen)

Keep Phase 1 as written:

| Package | Size | Notes |
|---|---|---|
| WP-02 Workbook model | L | Block storage, 64 B budget, snapshot reads |
| WP-03 Formula parser | L | Shared AST, spans, rewrite, fuzz |
| WP-04 Evaluator / graph | XL | Range-aware DAG, incremental, rayon, `AsyncNodeProvider` |
| WP-05a/b/c Functions | M+M+L | Can use IronCalc’s list as a coverage checklist |
| WP-09 / WP-10 `.xlsx` | XL+L | Own OOXML; L3 preserve (D7) |

Cost: the plan as estimated. Benefit: contracts, graph, async AI, and
memory budget are ours. IronCalc remains a throwaway oracle under
`spikes/` (or a later test-only crate, still not a workspace runtime dep).

### Path B — **adopt** (rejected)

Would require replacement packages **before** Phase 1, and would **not**
remove WP-04’s hard problems:

| Package | What it would be | Still missing |
|---|---|---|
| **WP-02A** | Adapter: IronCalc `Model` behind a `Workbook` façade | 256×256 blocks, 64 B/cell, snapshot reads for UI-during-recalc, interned `Value` |
| **WP-03A** | Either live with IronCalc’s parser (no editor spans / error-tolerant partial AST / our printer contract) or dual-parse | WP-03 deliverables for the formula bar |
| **WP-04A** | Wrap `evaluate()`; add a DAG, dirty set, rayon generations, `AsyncNodeProvider` | Essentially WP-04 **plus** fighting their eval order (issue #849 is still open) |
| WP-09A/10A | IronCalc xlsx I/O | L3 preserve, unknown parts, D7 |

Plus: pre-1.0 API churn; `core`/I/O layering fight; ~9 MiB; `rand` not
pre-approved; every WP-01 type is a translation layer frozen at G0.

Net: **more** work than build, worse fit, residual upstream risk.
Not strong enough evidence to overturn D2. `PLAN.md` is not rewritten.
WP-02A/03A/04A are **not** added to `docs/build/wp/`.

## Consequences

- Agents execute WP-01 then WP-02/03/04 against `crates/core` as specified.
- Do not add `ironcalc` / `ironcalc_base` to workspace members or
  `crates/*` dependencies.
- Spike remains under `spikes/ironcalc` (workspace `exclude`).
- Revisit only if IronCalc ships a public incremental DAG, a 64-byte-class
  storage layout, an async eval hook, and L3-preserving xlsx — and even
  then WP-01 types would still need an adapter RFC.
