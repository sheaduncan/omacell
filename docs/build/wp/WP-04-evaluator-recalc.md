# WP-04 — Evaluator and recalculation engine

| | |
|---|---|
| Phase | 1 — Engine |
| Lane | A — Engine / core |
| Size | XL (≈ 10–20) |
| Depends on | WP-02, WP-03 |
| Unblocks | WP-05F, WP-07a, WP-17, WP-19, WP-23 |
| Spec sections | §6.3 F-3.3–F-3.8, §8.3 A-3.3, §11.3, §11.5, §12.1 |
| Where | `crates/core` (modules `eval`, `graph`, `recalc`, `spill`, `lambda`, `coerce`) |

## Goal

Evaluate formulas with Excel semantics over a range-aware dependency graph with incremental, deterministic, parallel recalculation — and the hook that lets AI cells be asynchronous nodes later.

## Deliverables

- `coerce`: the F-3.5 rules (empty → 0/`""` by context, booleans in arithmetic, numeric text in arithmetic but not comparison, case-insensitive text comparison, error propagation order).
- `eval`: tree-walking evaluator over `Expr`; operators with array broadcasting; range/union/intersection values; implicit intersection for legacy formulas and `@`; `LET` scopes; `LAMBDA` values and calls (closures capture scope); named ranges; structured references; spill references (`A1#`).
- `spill`: spill regions, `#SPILL!` with the blocking cell, legacy CSE arrays preserved.
- `graph`: per-cell precedent edges plus per-sheet range buckets (interval index over 256×256 blocks) so whole-column references are O(1) edges; dirty propagation; cycle detection returning the circular set; volatile set; dynamic-reference nodes (`INDIRECT`/`OFFSET`) re-resolved each pass.
- `recalc`: modes (automatic, automatic-except-tables, manual), `recalc_incremental`, `recalc_full`, `recalc_rebuild`; topological evaluation in generations; `rayon` work-stealing inside a generation with results committed in deterministic order; iterative calculation (max iterations, max change); persisted calc chain for warm loads.
- Async nodes: `AsyncNodeProvider` trait — evaluation of a registered async function returns `Pending(cached)`/`Ready`/`Failed(hint)`; pending nodes and dependents are marked stale; a completion re-dirties them for a second wave. Ship a mock provider in tests.
- Function registry interface consumed by the evaluator (`FnRegistry::lookup(name) -> Option<&FnDef>`) so WP-05 can proceed in parallel against a stub.

## Interface sketch

```rust
pub trait AsyncNodeProvider: Send + Sync {
    fn evaluate(&self, key: ContentHash, req: AsyncRequest) -> AsyncState; // Pending{cached}, Ready(Value), Failed{hint}
}
```

## Implementation notes

- Determinism is a requirement, not a nicety: with 1 thread and with 8 threads every cell must be bit-identical, including volatile ordering within a pass.
- Circular references never panic and never hang: iterative off → `CircularRef` error set surfaced; iterative on → bounded loop.

## Acceptance criteria

- [ ] Eval corpus `tests/corpus/eval/*.omc`-style fixtures (expected values per cell) passes, including spill, `LET`/`LAMBDA`, implicit intersection, 3-D, structured refs, volatile behavior.
- [ ] Cycle corpus: detection sets match; iterative mode converges within limits.
- [ ] Determinism test: 1 vs 8 threads produce identical values on a generated 200k-formula workbook.
- [ ] Performance gates on the CI reference: incremental recalc after one edit in a generated 100k-formula chain < 50 ms; full recalc of 1M formulas < 5 s with 8 threads. Baselines recorded via `just perf-baseline`.
- [ ] Async mock: pending → second wave → dependents updated; stale flags correct.

## Tests

- Corpus tests; determinism test; criterion benches with committed baselines; `proptest` for coercion rules.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-04.md` before writing code.
4. Create branch `wp/04-evaluator-recalc`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-04: Evaluator and recalculation engine`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
