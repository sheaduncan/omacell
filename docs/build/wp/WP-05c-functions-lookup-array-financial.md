# WP-05c — Functions Tier 0 — lookup/reference, dynamic arrays, lambda helpers, financial, engineering basics

| | |
|---|---|
| Phase | 1 — Engine |
| Lane | A — Engine / core |
| Size | L (≈ 6–10) |
| Depends on | WP-05F |
| Unblocks | WP-13 |
| Spec sections | §6.4 Tier 0, §6.3 F-3.3–F-3.4, Appendix D |
| Where | `crates/fn` (modules `lookup`, `array`, `lambda`, `financial`, `engineering`) |

## Goal

The functions that make dynamic-array Excel what it is, plus the financial core.

## Deliverables

- Lookup, reference, and arrays: `XLOOKUP`, `XMATCH`, `INDEX`, `MATCH`, `VLOOKUP`, `HLOOKUP`, `LOOKUP`, `CHOOSE`, `OFFSET`, `INDIRECT`, `ROW(S)`, `COLUMN(S)`, `ADDRESS`, `AREAS`, `TRANSPOSE`, `FILTER`, `SORT`, `SORTBY`, `UNIQUE`, `SEQUENCE`, `RANDARRAY`, `TAKE`, `DROP`, `CHOOSEROWS`, `CHOOSECOLS`, `VSTACK`, `HSTACK`, `TOCOL`, `TOROW`, `WRAPROWS`, `WRAPCOLS`, `EXPAND`. `HYPERLINK` and `FORMULATEXT` follow Appendix D and are deferred to Tier 1 rather than silently expanding Tier 0.
- Lambda helpers: implement `MAP`, `REDUCE`, `SCAN`, `BYROW`, `BYCOL`, and `MAKEARRAY`; add integration/metadata coverage for the existing evaluator-owned `LET`, `LAMBDA`, and `ISOMITTED` constructs rather than registering duplicate implementations.
- Financial (~20): `PMT`, `IPMT`, `PPMT`, `NPV`, `XNPV`, `IRR`, `XIRR`, `MIRR`, `FV`, `PV`, `RATE`, `NPER`, `SLN`, `DB`, `DDB`, `SYD`, `EFFECT`, `NOMINAL`, `CUMIPMT`, `CUMPRINC` — with the iterative solvers' tolerances documented.
- Engineering basics (12): `CONVERT` (full unit table), `DEC2BIN/OCT/HEX` and inverses, `BITAND/OR/XOR/LSHIFT/RSHIFT`, `DELTA`, `GESTEP`.

## Implementation notes

- Use the WP-05F definition macro, checked arrays, deterministic random context, and corpus runner as-is. Every user-controlled output shape is validated before allocation.
- Every function must return an `ErrorKind`, never panic, for any `Value` input; a fuzz smoke test feeds random values to every registered function.
- Criteria syntax for `*IF(S)`: comparison prefixes, wildcards `* ? ~`, numeric/text/date matching per Excel.
- Where Excel behavior is surprising (e.g. `ROUND` half-away-from-zero, `MOD` sign, `TEXT` format codes, `DATEDIF` units), write the corpus case first and cite the documented behavior in a comment. Cross-check with LibreOffice headless when available (`scripts/lo-crosscheck.py`); differences are triaged into `docs/compat/known-differences.md`.
- Binary search modes and wildcard modes of `XLOOKUP`/`XMATCH` need explicit corpus coverage; `VLOOKUP` approximate-match semantics on unsorted data must mirror Excel, not be 'fixed'.
- Execution decision: the engineering basics listed here remain Tier 0 according to Appendix D even though the prose summary in §6.4 groups some under Tier 1. Record this known spec inconsistency in the report; do not expand beyond the explicit list.

## Acceptance criteria

- [ ] Every function in the list has ≥ 10 corpus rows in `tests/corpus/functions/<NAME>.tsv` covering all applicable categories among normal, empty/omitted, error-propagation, array/reference behavior, lookup modes, shape limits, and boundaries; all pass. Inapplicable categories are identified in metadata.
- [ ] `functions.json` lists every function with signature and doc; no function panics under the fuzz smoke test.
- [ ] Cross-check script reports zero unexplained differences (explained ones documented).
- [ ] `SEQUENCE`, `RANDARRAY`, `MAKEARRAY`, and stacking/wrapping functions reject out-of-grid or overflowed shapes before allocating; lambda-helper recursion/call caps are enforced.
- [ ] WP-04 recalc gates remain within budget; 1M-row `XLOOKUP`/`XMATCH`, `FILTER`, `SORT`, `UNIQUE`, and representative lambda/financial solver baselines are committed and regressions over 10% fail review.

## Tests

- Corpus table tests; fuzz smoke; solver convergence tests for `IRR`/`XIRR`/`RATE`.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-05c.md` before writing code.
4. Create branch `wp/05c-functions-lookup-array-financial`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-05c: Functions Tier 0 — lookup/reference, dynamic arrays, lambda helpers, financial, engineering basics`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
