# WP-24 — Pivot tables, Goal Seek, statistics panel

| | |
|---|---|
| Phase | 6 — Analysis and output |
| Lane | A — Engine / core |
| Size | XL (≈ 10–20) |
| Depends on | WP-18, WP-10 |
| Unblocks | WP-28 |
| Spec sections | §6.7, §13 |
| Where | `crates/core` (modules `pivot`, `whatif`, `stats`), `crates/io` (pivot parts), `crates/bus` |

## Goal

Summarize and model: pivots that Excel treats as live pivots, and Goal Seek on the real calc engine.

## Deliverables

- Pivot model: row/column/value/filter fields; SUM, COUNT, AVERAGE, MIN, MAX, COUNTA, DISTINCT COUNT, STDEV, VAR; show-values-as (% of total/row/column, running total, difference from); grouping by dates (days/months/quarters/years) and numeric bins; compact/outline/tabular layouts; subtotals/grand totals; refresh on demand or on open; output as a managed region on a sheet with its own styles.
- `.xlsx` pivot cache definition/records and pivot table parts: write (so Excel and LibreOffice see a live pivot) and read (rebuild the model from the definition; cache records used for display when the source is missing).
- Goal Seek: secant with bisection fallback over the calc engine, tolerance and max iterations, result command that sets the input; Data Tables and Scenario Manager deferred.
- Statistics summary API for any selection (descriptive stats, histogram bins) used by the UIs' statistics panel.

## Implementation notes

- Pivot output regions are read-only to the user; edits are refused with a hint, as in Excel.
- Keep the aggregation engine columnar and reusable by the AI card builder's column summaries.

## Acceptance criteria

- [ ] Pivot corpus: definitions → expected tables (including grouping and show-as) pass; refresh after source change correct.
- [ ] Exported pivots load as pivots in LibreOffice headless and `openpyxl` (structure check).
- [ ] Goal Seek convergence corpus passes within tolerances; non-convergence reported cleanly.

## Tests

- Corpus tests; external-loader checks; convergence tests.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-24.md` before writing code.
4. Create branch `wp/24-pivot-goalseek-stats`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-24: Pivot tables, Goal Seek, statistics panel`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
