# WP-05c — Functions Tier 0 — lookup/reference, dynamic arrays, lambda helpers, financial, engineering basics

| | |
|---|---|
| Phase | 1 — Engine |
| Lane | A — Engine / core |
| Size | L (≈ 6–10) |
| Depends on | WP-01, WP-04 |
| Unblocks | WP-13 |
| Spec sections | §6.4 Tier 0, §6.3 F-3.3–F-3.4, Appendix D |
| Where | `crates/fn` (modules `lookup`, `array`, `lambda`, `financial`, `engineering`) |

## Goal

The functions that make dynamic-array Excel what it is, plus the financial core.

## Deliverables

- Lookup & reference (~35): `XLOOKUP`, `XMATCH`, `INDEX`, `MATCH`, `VLOOKUP`, `HLOOKUP`, `LOOKUP`, `CHOOSE`, `OFFSET`, `INDIRECT`, `ROW(S)`, `COLUMN(S)`, `ADDRESS`, `AREAS`, `TRANSPOSE`, `FILTER`, `SORT`, `SORTBY`, `UNIQUE`, `TAKE`, `DROP`, `CHOOSEROWS`, `CHOOSECOLS`, `VSTACK`, `HSTACK`, `TOCOL`, `TOROW`, `WRAPROWS`, `WRAPCOLS`, `EXPAND`, `HYPERLINK` (value part), `FORMULATEXT`.
- Lambda helpers (8): `MAP`, `REDUCE`, `SCAN`, `BYROW`, `BYCOL`, `MAKEARRAY`, `ISOMITTED` (if not in 05a), `LAMBDA`/`LET` evaluator integration tests.
- Financial (~20): `PMT`, `IPMT`, `PPMT`, `NPV`, `XNPV`, `IRR`, `XIRR`, `MIRR`, `FV`, `PV`, `RATE`, `NPER`, `SLN`, `DB`, `DDB`, `SYD`, `EFFECT`, `NOMINAL`, `CUMIPMT`, `CUMPRINC` — with the iterative solvers' tolerances documented.
- Engineering basics (12): `CONVERT` (full unit table), `DEC2BIN/OCT/HEX` and inverses, `BITAND/OR/XOR/LSHIFT/RSHIFT`, `DELTA`, `GESTEP`.

## Implementation notes

- Define functions with a macro that records name, arg count, arg kinds, volatility, and array-lifting behavior; the registry emits `functions.json` used by `omacell fn list --json` and by the autocomplete provider.
- Every function must return an `ErrorKind`, never panic, for any `Value` input; a fuzz smoke test feeds random values to every registered function.
- Criteria syntax for `*IF(S)`: comparison prefixes, wildcards `* ? ~`, numeric/text/date matching per Excel.
- Where Excel behavior is surprising (e.g. `ROUND` half-away-from-zero, `MOD` sign, `TEXT` format codes, `DATEDIF` units), write the corpus case first and cite the documented behavior in a comment. Cross-check with LibreOffice headless when available (`scripts/lo-crosscheck.py`); differences are triaged into `docs/compat/known-differences.md`.
- Binary search modes and wildcard modes of `XLOOKUP`/`XMATCH` need explicit corpus coverage; `VLOOKUP` approximate-match semantics on unsorted data must mirror Excel, not be 'fixed'.

## Acceptance criteria

- [ ] Every function in the list has ≥ 10 corpus rows in `tests/corpus/functions/<NAME>.tsv` covering normal, empty, error-propagation, array-lifting, and boundary cases; all pass.
- [ ] `functions.json` lists every function with signature and doc; no function panics under the fuzz smoke test.
- [ ] Cross-check script reports zero unexplained differences (explained ones documented).

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
