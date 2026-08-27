# WP-18 — Sort, AutoFilter, tables, data validation, conditional formatting (data tools II)

| | |
|---|---|
| Phase | 4 — Surfaces II — GUI and data tools |
| Lane | A — Engine / core |
| Size | XL (≈ 10–20) |
| Depends on | WP-17, WP-06 |
| Unblocks | WP-24 |
| Spec sections | §6.6 F-6.1–F-6.5, §13 |
| Where | `crates/core` (modules `sort`, `filter`, `tables`, `validation`, `condfmt`), `crates/bus` (commands) |

## Goal

The data features people open Excel for, with file-faithful storage so they survive a trip through Excel.

## Deliverables

- Sort: multi-key on values, cell color, font color, icon; ascending/descending; custom lists; case sensitivity; header detection; left-to-right; Excel type ordering (numbers, text, logicals, errors, blanks last); hidden rows untouched; formula handling on moved cells documented and tested; sort within tables and filtered ranges.
- AutoFilter model: per-column criteria (value lists with search, text/number/date operators, top-N, above/below average, date periods, color), hidden-row flags, saved filter state (`.xlsx` `autoFilter` + `filterColumn`), clear-all, `Ctrl+Shift+L` command.
- Tables: create/resize/convert-to-range, banded styles referencing theme roles by default, totals row with function chooser, calculated columns auto-fill, auto-expand on adjacent entry, structured-reference updates on rename; table style records for `.xlsx`.
- Data validation: whole number, decimal, list (inline/range; dropdown source resolution), date, time, text length, custom formula; input message; error styles; `validate(cell)` API; circle-invalid-data query.
- Conditional formatting engine: cell-value, formula, text, date, blanks/errors, duplicates/uniques, top/bottom N or %, above/below average, 2/3-color scales, data bars (solid/gradient, negative axis), icon sets; priority and stop-if-true; `dxf` styles; evaluation cache invalidated by recalc; theme-derived defaults for new scales/bars.
- Slicers deferred (Tier 1); `.xlsx` storage coordinated with WP-10 for all of the above.

## Implementation notes

- Excel's sort of formulas: cells move as units and relative references are adjusted as in a move — document the exact rule you implement with tests, and note any divergence in `docs/compat/known-differences.md`.

## Acceptance criteria

- [ ] Sort corpus (stability, mixed types, custom lists, colors) passes; filter corpus passes; DV corpus passes; CF corpus with expected resolved styles per cell passes.
- [ ] All features round-trip through `.xlsx` (WP-10 fixtures extended) and reopen in LibreOffice headless without loss of definitions.
- [ ] Performance: CF evaluation for 100k cells with 20 rules < 100 ms after a single-cell edit.

## Tests

- Corpus tests; round-trip tests; criterion bench for CF evaluation.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-18.md` before writing code.
4. Create branch `wp/18-sort-filter-tables-validation-cf`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-18: Sort, AutoFilter, tables, data validation, conditional formatting (data tools II)`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
