# WP-05a — Functions Tier 0 — math, statistics, logical, information, criteria aggregation

| | |
|---|---|
| Phase | 1 — Engine |
| Lane | A — Engine / core |
| Size | L (≈ 6–10) |
| Depends on | WP-05F |
| Unblocks | WP-13 |
| Spec sections | §6.4 Tier 0, Appendix D, §14 |
| Where | `crates/fn` (modules `math`, `stat`, `logical`, `info`, `aggregate`, `registry`) |

## Goal

Implement the first third of Tier 0 on the frozen WP-05F runtime and metadata foundation.

## Deliverables

- Math & trig (~58): `SUM`, `PRODUCT`, `ABS`, `ROUND*`, `INT`, `TRUNC`, `MOD`, `QUOTIENT`, `POWER`, `SQRT`, `EXP`, `LN`, `LOG*`, trig/hyperbolic, `PI`, `RADIANS`, `DEGREES`, `CEILING.MATH`, `FLOOR.MATH`, `MROUND`, `GCD`, `LCM`, `FACT`, `COMBIN*`, `PERMUT*`, `SUMSQ`, `SUMPRODUCT`, `SUMX2MY2` family, `RAND`, `RANDBETWEEN`, `SIGN`, `EVEN`, `ODD`. WP-05c owns `RANDARRAY` and `SEQUENCE`.
- Statistical descriptive (~45): `AVERAGE*`, `MEDIAN`, `MODE*`, `MIN*`, `MAX*`, `LARGE`, `SMALL`, `COUNT*`, `STDEV*`, `VAR*`, `RANK*`, `PERCENTILE.INC/EXC`, `PERCENTRANK*`, `QUARTILE*`, `CORREL`, `PEARSON`, `COVARIANCE.*`, `SLOPE`, `INTERCEPT`, `RSQ`, `FORECAST.LINEAR`, `GEOMEAN`, `HARMEAN`, `TRIMMEAN`, `DEVSQ`, `AVEDEV`, `SKEW*`, `KURT`, `FREQUENCY`, `STANDARDIZE`.
- Logical (10): `IF`, `IFS`, `SWITCH`, `AND`, `OR`, `XOR`, `NOT`, `IFERROR`, `IFNA`, `TRUE`/`FALSE`.
- Information (15): `IS*` family, `TYPE`, `ERROR.TYPE`, `NA`, `N`, `CELL` (subset: address, col, row, contents, type, format). `ISOMITTED` remains a WP-04 language construct; this package adds its catalog metadata and integration corpus only.
- Criteria aggregation: `SUMIF(S)`, `COUNTIF(S)`, `AVERAGEIF(S)`, `MAXIFS`, `MINIFS`, `AGGREGATE`, `SUBTOTAL` (hidden-row semantics via a sheet callback).

## Implementation notes

- Use the WP-05F definition macro and metadata as-is. Do not fork the registry, corpus runner, eager/lazy dispatch, clock, random, locale, or array-limit policy.
- `IF`, `IFS`, `SWITCH`, `IFERROR`, and `IFNA` use WP-05F lazy dispatch and must prove that unselected error, volatile, and async branches are not evaluated. Other logical-function evaluation order follows cited corpus behavior rather than assumed short-circuiting.
- `RAND` and `RANDBETWEEN` derive values from the pass nonce plus formula coordinate/function identity; never use thread-local or call-order RNG state.
- Every function must return an `ErrorKind`, never panic, for any `Value` input; a fuzz smoke test feeds random values to every registered function.
- Criteria syntax for `*IF(S)`: comparison prefixes, wildcards `* ? ~`, numeric/text/date matching per Excel.
- Where Excel behavior is surprising (e.g. `ROUND` half-away-from-zero, `MOD` sign, `TEXT` format codes, `DATEDIF` units), write the corpus case first and cite the documented behavior in a comment. Cross-check with LibreOffice headless when available (`scripts/lo-crosscheck.py`); differences are triaged into `docs/compat/known-differences.md`.

## Acceptance criteria

- [ ] Every function in the list has ≥ 10 corpus rows in `tests/corpus/functions/<NAME>.tsv` covering all applicable categories among normal, empty/omitted, error-propagation/evaluation-order, array-lifting, reference-vs-literal coercion, and boundary cases; all pass. Inapplicable categories are identified in metadata rather than padded with duplicate rows.
- [ ] `functions.json` lists every function with signature and doc; no function panics under the fuzz smoke test.
- [ ] Cross-check script reports zero unexplained differences (explained ones documented).
- [ ] Lazy-branch, hidden-row aggregate, deterministic-random, whole-column aggregate, and criteria/wildcard integration tests pass.
- [ ] WP-04 100k incremental and 1M full-recalc gates remain within budget; function-specific baselines for whole-column `SUM`, `SUMIFS`, and `SUBTOTAL` are recorded and regressions over 10% fail review.

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
