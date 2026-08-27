# WP-05a — Functions Tier 0 — math, statistics, logical, information, criteria aggregation

| | |
|---|---|
| Phase | 1 — Engine |
| Lane | A — Engine / core |
| Size | L (≈ 6–10) |
| Depends on | WP-01, WP-04 |
| Unblocks | WP-13 |
| Spec sections | §6.4 Tier 0, Appendix D, §14 |
| Where | `crates/fn` (modules `math`, `stat`, `logical`, `info`, `aggregate`, `registry`) |

## Goal

Implement the registry and the first third of Tier 0 with a conformance corpus per function.

## Deliverables

- Registry crate scaffolding and definition macro (shared by WP-05b/05c).
- Math & trig (~60): `SUM`, `PRODUCT`, `ABS`, `ROUND*`, `INT`, `TRUNC`, `MOD`, `QUOTIENT`, `POWER`, `SQRT`, `EXP`, `LN`, `LOG*`, trig/hyperbolic, `PI`, `RADIANS`, `DEGREES`, `CEILING.MATH`, `FLOOR.MATH`, `MROUND`, `GCD`, `LCM`, `FACT`, `COMBIN*`, `PERMUT*`, `SUMSQ`, `SUMPRODUCT`, `SUMX2MY2` family, `RAND`, `RANDBETWEEN`, `RANDARRAY`, `SEQUENCE` (shared with 05c — implement here), `SIGN`, `EVEN`, `ODD`.
- Statistical descriptive (~45): `AVERAGE*`, `MEDIAN`, `MODE*`, `MIN*`, `MAX*`, `LARGE`, `SMALL`, `COUNT*`, `STDEV*`, `VAR*`, `RANK*`, `PERCENTILE.INC/EXC`, `PERCENTRANK*`, `QUARTILE*`, `CORREL`, `PEARSON`, `COVARIANCE.*`, `SLOPE`, `INTERCEPT`, `RSQ`, `FORECAST.LINEAR`, `GEOMEAN`, `HARMEAN`, `TRIMMEAN`, `DEVSQ`, `AVEDEV`, `SKEW*`, `KURT`, `FREQUENCY`, `STANDARDIZE`.
- Logical (10): `IF`, `IFS`, `SWITCH`, `AND`, `OR`, `XOR`, `NOT`, `IFERROR`, `IFNA`, `TRUE`/`FALSE`.
- Information (15): `IS*` family, `TYPE`, `ERROR.TYPE`, `NA`, `N`, `CELL` (subset: address, col, row, contents, type, format), `ISOMITTED`.
- Criteria aggregation: `SUMIF(S)`, `COUNTIF(S)`, `AVERAGEIF(S)`, `MAXIFS`, `MINIFS`, `AGGREGATE`, `SUBTOTAL` (hidden-row semantics via a sheet callback).

## Implementation notes

- Define functions with a macro that records name, arg count, arg kinds, volatility, and array-lifting behavior; the registry emits `functions.json` used by `omacell fn list --json` and by the autocomplete provider.
- Every function must return an `ErrorKind`, never panic, for any `Value` input; a fuzz smoke test feeds random values to every registered function.
- Criteria syntax for `*IF(S)`: comparison prefixes, wildcards `* ? ~`, numeric/text/date matching per Excel.
- Where Excel behavior is surprising (e.g. `ROUND` half-away-from-zero, `MOD` sign, `TEXT` format codes, `DATEDIF` units), write the corpus case first and cite the documented behavior in a comment. Cross-check with LibreOffice headless when available (`scripts/lo-crosscheck.py`); differences are triaged into `docs/compat/known-differences.md`.

## Acceptance criteria

- [ ] Every function in the list has ≥ 10 corpus rows in `tests/corpus/functions/<NAME>.tsv` covering normal, empty, error-propagation, array-lifting, and boundary cases; all pass.
- [ ] `functions.json` lists every function with signature and doc; no function panics under the fuzz smoke test.
- [ ] Cross-check script reports zero unexplained differences (explained ones documented).

## Tests

- Corpus table tests; fuzz smoke; LibreOffice cross-check (skips when absent).

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-05a.md` before writing code.
4. Create branch `wp/05a-functions-math-stat-logic`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-05a: Functions Tier 0 — math, statistics, logical, information, criteria aggregation`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
