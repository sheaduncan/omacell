# WP-05b — Functions Tier 0 — text, date, and time

| | |
|---|---|
| Phase | 1 — Engine |
| Lane | A — Engine / core |
| Size | L (≈ 6–10) |
| Depends on | WP-01, WP-04, WP-06 |
| Unblocks | WP-13 |
| Spec sections | §6.4 Tier 0, §6.2 F-2.1, Appendix D |
| Where | `crates/fn` (modules `text`, `datetime`) |

## Goal

Text functions that are Unicode-correct and date/time functions that honor both date systems.

## Deliverables

- Text (~35 + regex): `LEN`, `LEFT`, `RIGHT`, `MID`, `FIND`, `SEARCH`, `SUBSTITUTE`, `REPLACE`, `UPPER/LOWER/PROPER`, `TRIM`, `CLEAN`, `CONCAT`, `TEXTJOIN`, `TEXTSPLIT`, `TEXTBEFORE`, `TEXTAFTER`, `TEXT` (uses WP-06), `VALUE`, `NUMBERVALUE`, `FIXED`, `DOLLAR`, `REPT`, `CHAR`, `CODE`, `UNICHAR`, `UNICODE`, `EXACT`, `T`, `ARRAYTOTEXT`, `VALUETOTEXT`, `REGEXTEST`, `REGEXEXTRACT`, `REGEXREPLACE` (regex crate with size/time limits).
- Date/time (~25): `DATE`, `TIME`, `DATEVALUE`, `TIMEVALUE`, `YEAR/MONTH/DAY/HOUR/MINUTE/SECOND`, `WEEKDAY`, `WEEKNUM`, `ISOWEEKNUM`, `TODAY`, `NOW`, `EDATE`, `EOMONTH`, `DAYS`, `DAYS360`, `DATEDIF`, `YEARFRAC` (all bases), `NETWORKDAYS(.INTL)`, `WORKDAY(.INTL)`, `WEEKDAY` return types.
- Locale hooks for `TEXT`, `VALUE`, `DATEVALUE` via the WP-06 locale tables.

## Implementation notes

- Define functions with a macro that records name, arg count, arg kinds, volatility, and array-lifting behavior; the registry emits `functions.json` used by `omacell fn list --json` and by the autocomplete provider.
- Every function must return an `ErrorKind`, never panic, for any `Value` input; a fuzz smoke test feeds random values to every registered function.
- Criteria syntax for `*IF(S)`: comparison prefixes, wildcards `* ? ~`, numeric/text/date matching per Excel.
- Where Excel behavior is surprising (e.g. `ROUND` half-away-from-zero, `MOD` sign, `TEXT` format codes, `DATEDIF` units), write the corpus case first and cite the documented behavior in a comment. Cross-check with LibreOffice headless when available (`scripts/lo-crosscheck.py`); differences are triaged into `docs/compat/known-differences.md`.
- `LEN`/`MID`/`LEFT` operate on UTF-16 code units to match Excel? Decide, document, and test — the corpus must state the chosen semantics (recommendation: Unicode scalar values, with a compatibility flag).

## Acceptance criteria

- [ ] Every function in the list has ≥ 10 corpus rows in `tests/corpus/functions/<NAME>.tsv` covering normal, empty, error-propagation, array-lifting, and boundary cases; all pass.
- [ ] `functions.json` lists every function with signature and doc; no function panics under the fuzz smoke test.
- [ ] Cross-check script reports zero unexplained differences (explained ones documented).
- [ ] Date functions pass the 1900 leap-bug and 1904 boundary corpus.

## Tests

- Corpus table tests; fuzz smoke; regex timeout tests.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-05b.md` before writing code.
4. Create branch `wp/05b-functions-text-date`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-05b: Functions Tier 0 — text, date, and time`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
