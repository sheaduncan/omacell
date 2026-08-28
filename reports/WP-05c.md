# Report — WP-05c: Functions Tier 0 — lookup/reference, dynamic arrays, lambda helpers, financial, engineering basics

## Plan (written before coding)

- Files/modules to create:
  - `crates/fn/src/args.rs` — private helpers: scalar/number/int extraction, first-error walk, `Reference`/`RuntimeValue` → checked array, wildcard match, 1-based indexing, shape checks via `RuntimeArray::checked_len` **before** allocation. Named `args` (not `common`/`util`) so parallel WP-05a/05b helpers do not collide.
  - `crates/fn/src/lookup.rs` — `XLOOKUP`, `XMATCH`, `INDEX`, `MATCH`, `VLOOKUP`, `HLOOKUP`, `LOOKUP`, `CHOOSE`, `OFFSET`, `INDIRECT`, `ROW`, `ROWS`, `COLUMN`, `COLUMNS`, `ADDRESS`, `AREAS`. `OFFSET`/`INDIRECT` are volatile and call `EvalCtx::record_dynamic_ref`. `ROWS`/`COLUMNS`/`INDEX`/`OFFSET` operate on `Reference` dimensions without materializing whole-column payloads. `register_lookup`.
  - `crates/fn/src/array.rs` — `TRANSPOSE`, `FILTER`, `SORT`, `SORTBY`, `UNIQUE`, `SEQUENCE` (replaces the WP-05F probe; full `rows, [columns], [start], [step]`), `RANDARRAY` (volatile; `EvalCtx::random_unit("RANDARRAY", index)`), `TAKE`, `DROP`, `CHOOSEROWS`, `CHOOSECOLS`, `VSTACK`, `HSTACK`, `TOCOL`, `TOROW`, `WRAPROWS`, `WRAPCOLS`, `EXPAND`. Every user-controlled output shape is rejected (`#NUM!`) via `RuntimeArray::checked_len` / `try_new` before `Vec` allocation. `register_array`.
  - `crates/fn/src/lambda.rs` — `MAP`, `REDUCE`, `SCAN`, `BYROW`, `BYCOL`, `MAKEARRAY` (eager; last arg is a `RuntimeValue::Lambda`, applied through `omacell_core::lambda::apply`, which already enforces `MAX_CALL_DEPTH`). Catalog-only `FunctionSpec`s for evaluator-owned `LET`, `LAMBDA`, `ISOMITTED` (lazy stubs, **not** registered on `FnRegistry`). `register_lambda` skips `is_language_fn` names.
  - `crates/fn/src/financial.rs` — `PMT`, `IPMT`, `PPMT`, `NPV`, `XNPV`, `IRR`, `XIRR`, `MIRR`, `FV`, `PV`, `RATE`, `NPER`, `SLN`, `DB`, `DDB`, `SYD`, `EFFECT`, `NOMINAL`, `CUMIPMT`, `CUMPRINC`. Newton solvers: `RATE`/`IRR` max 20 iterations, `XIRR` max 100; residual `|f| < 1e-8` or `#NUM!`. `register_financial`.
  - `crates/fn/src/engineering.rs` — `CONVERT` (full Excel unit table + SI prefixes; temperature via Kelvin), `DEC2BIN`/`DEC2OCT`/`DEC2HEX` and inverses `BIN2DEC`/`OCT2DEC`/`HEX2DEC`, `BITAND`/`BITOR`/`BITXOR`/`BITLSHIFT`/`BITRSHIFT`, `DELTA`, `GESTEP`. `register_engineering`.
  - `crates/fn/src/lib.rs` — **append only**: `mod args/lookup/array/lambda/financial/engineering` and `register_*` calls inside `register_all`. Do **not** delete `ABS`/`SUM`/`IF`/`NOW`/`RAND` probes.
  - `crates/fn/src/probes.rs` — remove **only** `SEQUENCE` (moved to `array.rs`). Keep `ABS`/`SUM`/`IF`/`NOW`/`RAND`.
  - `crates/fn/src/{metadata,corpus}.rs` — `functions_json` and the corpus runner use `all_specs()` / `register_all` so 05a/05b can append their specs later.
  - `crates/fn/tests/{probes,wp05c}.rs` — catalog/schema; ≥10-row TSV runner; eager fuzz-smoke; shape-limit tests; solver tests; RANDARRAY determinism.
  - `crates/fn/benches/lookup_array.rs` — 1M-row `XLOOKUP`/`XMATCH`/`FILTER`/`SORT`/`UNIQUE`, representative `MAP` and `RATE`/`IRR` solver baselines.
  - `tests/corpus/functions/<NAME>.tsv` — ≥10 cited rows per function (including `LET`/`LAMBDA`/`ISOMITTED` integration corpora). Array results observed via `INDEX`/`SUM`/`ROWS`/`COLUMNS` because `format_cell` reads the spill origin.
  - `docs/compat/known-differences.md` — **append only**.
  - `fuzz/fuzz_targets/fn_eager.rs` — smoke every eager registered spec (size-capped args).
  - `scripts/lo-crosscheck.py` — `_xlfn.` prefix for post-2007 names (`XLOOKUP`, `FILTER`, `MAP`, …).
- Interfaces to expose (types, commands, schemas, CLI):
  - `omacell_fn::{register_all, all_specs, functions_json}` now includes WP-05c specs.
  - `omacell_fn::{LOOKUP_SPECS, ARRAY_SPECS, LAMBDA_SPECS, FINANCIAL_SPECS, ENGINEERING_SPECS}` (crate-visible slices).
  - No new WP-01 types, commands, or CLI. No RFC.
  - Solver constants (`RATE`/`IRR`: 20 iters, `XIRR`: 100, tol `1e-8`) documented in this report and known-differences.
- Tests and corpora to write first:
  - `tests/corpus/functions/{XLOOKUP,XMATCH,INDEX,MATCH,VLOOKUP,HLOOKUP,LOOKUP,CHOOSE,OFFSET,INDIRECT,ROW,ROWS,COLUMN,COLUMNS,ADDRESS,AREAS}.tsv`
  - `tests/corpus/functions/{TRANSPOSE,FILTER,SORT,SORTBY,UNIQUE,SEQUENCE,RANDARRAY,TAKE,DROP,CHOOSEROWS,CHOOSECOLS,VSTACK,HSTACK,TOCOL,TOROW,WRAPROWS,WRAPCOLS,EXPAND}.tsv`
  - `tests/corpus/functions/{MAP,REDUCE,SCAN,BYROW,BYCOL,MAKEARRAY,LET,LAMBDA,ISOMITTED}.tsv`
  - `tests/corpus/functions/{PMT,IPMT,PPMT,NPV,XNPV,IRR,XIRR,MIRR,FV,PV,RATE,NPER,SLN,DB,DDB,SYD,EFFECT,NOMINAL,CUMIPMT,CUMPRINC}.tsv`
  - `tests/corpus/functions/{CONVERT,DEC2BIN,DEC2OCT,DEC2HEX,BIN2DEC,OCT2DEC,HEX2DEC,BITAND,BITOR,BITXOR,BITLSHIFT,BITRSHIFT,DELTA,GESTEP}.tsv`
  - Each file: ≥10 rows covering applicable categories among normal, empty/omitted, error-propagation, array/reference, lookup modes (exact / approx / wildcard / binary), shape limits, and boundaries; `note` cites the behaviour.
  - Unit tests: `SEQUENCE`/`RANDARRAY`/`MAKEARRAY`/`VSTACK`/`HSTACK`/`WRAP*` reject `0`, `MAX_ROWS+1`, `MAX_COLS+1`, and overflowed products **before** allocation; lambda call-cap via existing `enter_call`; IRR/XIRR/RATE convergence and non-convergence `#NUM!`; RANDARRAY 1-vs-8-thread identity under a locked nonce.
- Items the package says to "decide and document" and the decision taken:
  - **§6.4 vs Appendix D engineering:** Appendix D lists Engineering as Tier 0 (12, `CONVERT`, bases). §6.4 prose puts `CONVERT` / `BIN2*` / `DEC2*` / `HEX2*` in Tier 1. Execution decision: keep the WP-05c explicit list as **Tier 0**. Do not add Bessel/complex/`ERF`. Recorded below.
  - **`HYPERLINK` / `FORMULATEXT`:** Appendix D Tier 1. Not implemented.
  - **`LET` / `LAMBDA` / `ISOMITTED`:** evaluator-owned (WP-04). Catalog metadata + integration corpus only; no duplicate `FnDef`.
  - **Invalid array shapes:** keep WP-05F `#NUM!` (not Excel 365 `#CALC!`) for zero / negative / out-of-grid / overflow, including `MAKEARRAY` and stacking/wrapping. Confirmed for WP-05c.
  - **`SEQUENCE` arity:** replace the 2-arg probe with Excel's `SEQUENCE(rows, [columns], [start], [step])`. Dimension args truncate toward zero (Excel), not round (probe).
  - **Approximate `VLOOKUP`/`HLOOKUP`/`MATCH`/`LOOKUP`:** binary search assuming sorted input; unsorted data is **not** “fixed” — Excel-compatible wrong answers are corpus-covered.
  - **`FILTER` with no keepers:** `#CALC!` when `if_empty` omitted (Excel).
  - **Solver policy:** Newton–Raphson; `RATE`/`IRR` 20 iterations default guess `0.1`; `XIRR` 100 iterations; success when `|f| < 1e-8`; else `#NUM!`.
  - **Engineering count:** WP text says 12; the named list is 14 (`CONVERT` + 6 bases + 5 bit ops + `DELTA` + `GESTEP`). Implement the named list. Cross conversions (`BIN2HEX`, …) stay unimplemented.
- Open questions at planning time:
  1. LibreOffice coverage of `XLOOKUP`/`FILTER`/`MAP`/`MAKEARRAY`/`RANDARRAY` via `_xlfn.` may still be incomplete; unexplained mismatches will be appended to known-differences rather than weakening corpora.
  2. `ISOMITTED` catalog also appears in WP-05a's plan (`info.rs`). This package owns it under `lambda` per kickoff; 05a should skip a duplicate spec on merge.
  3. 1M-row criterion benches may be too slow for default `just check` — they live in `crates/fn/benches` (not unit tests). Gates are recorded in Measurements.

## What was built

WP-05c Tier 0 lookup/reference, dynamic arrays, lambda helpers, financial core, and engineering basics on the WP-05F runtime. The WP-05F `SEQUENCE` probe is replaced by the full 4-argument function. `ABS`/`SUM`/`IF`/`NOW`/`RAND` probes are unchanged. `LET`/`LAMBDA`/`ISOMITTED` are catalog + integration corpus only.

Key files:

- `crates/fn/src/args.rs` — argument/array helpers; `RuntimeArray::checked_len` before allocation
- `crates/fn/src/lookup.rs` — 16 lookup/reference functions
- `crates/fn/src/array.rs` — 18 array functions including `SEQUENCE`/`RANDARRAY`
- `crates/fn/src/lambda.rs` — `MAP`/`REDUCE`/`SCAN`/`BYROW`/`BYCOL`/`MAKEARRAY` + catalog-only language constructs
- `crates/fn/src/financial.rs` — 20 financial functions + Newton solvers
- `crates/fn/src/engineering.rs` — 14 engineering functions (`CONVERT` unit table, bases, bits, `DELTA`, `GESTEP`)
- `crates/fn/src/{lib,metadata,corpus,probes}.rs` — `all_specs` / `register_all`; SEQUENCE probe removed
- `crates/fn/tests/wp05c.rs` — corpus (≥10 rows × 77 names), shape limits, solvers, RANDARRAY determinism, eager smoke
- `crates/fn/benches/lookup_array.rs` — 1M-row `XLOOKUP`/`XMATCH`/`FILTER`/`SORT`/`UNIQUE`, `MAP`, `IRR`/`RATE`
- `tests/corpus/functions/<NAME>.tsv` — 861 cited rows
- `docs/compat/known-differences.md` — append-only
- `scripts/lo-crosscheck.py` — `_xlfn.` prefix + CSV error/percent mapping
- `fuzz/fuzz_targets/fn_eager.rs` — every eager registered spec

Key tests: `crates/fn/tests/wp05c.rs`, `crates/fn/tests/probes.rs`.

Review hardening replaces quadratic `UNIQUE`/mode-style scans with first-seen hash indexes, preserves deterministic output order, validates output shapes before allocation, and handles extreme integer arguments without panicking. `DB`, `DDB`, `CUMIPMT`, and `CUMPRINC` now use constant-time closed forms instead of user-sized loops; `EFFECT` uses stable `ln_1p`/`exp_m1` compounding. `XNPV` accepts dates in any order provided none precedes the first date, matching Excel.

## Interfaces exposed (for dependents)

| Item | Where |
|---|---|
| `register_all` now includes lookup/array/lambda/financial/engineering | `omacell_fn` |
| `all_specs()` | catalog = probes + WP-05c (includes `LET`/`LAMBDA`/`ISOMITTED`) |
| `LOOKUP_SPECS`, `ARRAY_SPECS`, `LAMBDA_SPECS`, `FINANCIAL_SPECS`, `ENGINEERING_SPECS` | `omacell_fn` |
| `functions_json()` | includes every spec, sorted by name |
| Solver constants | `RATE_IRR_MAX_ITERS=20`, `XIRR_MAX_ITERS=100`, `SOLVER_TOL=1e-8`, `DEFAULT_GUESS=0.1` in `financial.rs` |
| Corpora | `tests/corpus/functions/<NAME>.tsv` |

No commands, CLI, or WP-01 type changes. `LET`/`LAMBDA`/`ISOMITTED` are **not** `FnRegistry` entries.

**WP-05a/05b:** append your `register_*` next to the existing `register_all` calls; extend `all_specs()`. Do not re-register `SEQUENCE` or `LET`/`LAMBDA`/`ISOMITTED`. Do not delete probes this package left in place.

**WP-13:** `functions_json()` is the catalog.

## Deviations from the spec or the package (with reasons)

- **§6.4 vs Appendix D engineering:** implemented as **Tier 0** per Appendix D and the WP list. §6.4 prose grouping `CONVERT`/bases under Tier 1 is recorded as a spec inconsistency. Bessel/`COMPLEX`/`ERF` not added.
- **Engineering count 14 vs “12”:** the named WP list is 14 functions; Appendix D’s 12 is approximate.
- **`HYPERLINK` / `FORMULATEXT`:** not implemented (Appendix D Tier 1).
- **Invalid shapes `#NUM!`:** confirmed (not Excel 365 `#CALC!`) for `SEQUENCE`/`RANDARRAY`/`MAKEARRAY`/stacking/wrapping.
- **`SEQUENCE` dimensions:** truncate toward zero (Excel), not the WP-05F probe’s `round`.
- **LibreOffice CSV:** many `_xlfn.*` helpers evaluate to `#NAME?`; error tokens are `Err:NNN`; some numeric cases disagree. Documented; `lo-crosscheck.py` maps error codes/percents and classifies missing modern names as known. Semantic LO disagreements keep Excel-matching Omacell results.

## Measurements

Host: rustc 1.98.0, Linux.

- `just check` — pass
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — pass
- `cargo deny check` — pass (advisories/bans/licenses/sources ok)
- Corpus: **77** function files, **862** rows, all ≥10, all pass (`cargo test -p omacell-fn --test wp05c`; 11 tests after review hardening)
- Catalog: **82** specs (`all_specs`); **79** registered (`LET`/`LAMBDA`/`ISOMITTED` catalog-only); **5** remaining probes (`ABS`/`SUM`/`IF`/`NOW`/`RAND`)
- Implemented WP-05c functions: **74** (16 lookup + 18 array + 6 lambda helpers + 20 financial + 14 engineering) + **3** metadata-only language constructs
- `scripts/lo-crosscheck.py` on the original 77-file set: 861 evaluated, **0 unexplained**, 186 known (LibreOffice CSV/`_xlfn` gaps). Review re-check of updated `XNPV.tsv`: 12 evaluated, 10 known, **0 unexplained**.
- Eager smoke: `eager_functions_do_not_panic_on_garbage_args` — pass
- Criterion `crates/fn/benches/lookup_array.rs`: 1M-row `XLOOKUP`/`XMATCH`/`FILTER`/`SORT`/`UNIQUE`, 10k `MAP`, `IRR`/`RATE`. Not part of `just check`. Run `cargo bench -p omacell-fn --bench lookup_array`. Review: regressions over 10% vs this harness fail review.
- Review measurement: optimized `unique_1m --quick` **469.34 ms** midpoint on the review host.

## Open questions / decisions needed

1. WP-05a also plans `ISOMITTED` catalog metadata in `info.rs`. This package already ships it under `lambda`. 05a should skip a duplicate spec.
2. LibreOffice headless still lacks most dynamic-array/lambda/`XNPV` functions even with `_xlfn.`; Excel remains the behaviour source.
3. Fill 1M-row criterion wall times on the review host (`cargo bench -p omacell-fn --bench lookup_array`) if a full sample was not recorded in this run.

## RFC (only if a frozen contract changed)

None. WP-01 types unchanged.

## Checklist

- [x] `just check` green
- [x] Every acceptance criterion ticked with evidence (corpora ≥10 rows; `functions.json` via `functions_json`; fuzz smoke test; LO unexplained = 0; shapes rejected before allocation; solver policy + bench harness)
- [x] Docs warning-free; public items documented (`RUSTDOCFLAGS=-D warnings`)
- [x] Baselines recorded (criterion bench added; 1M-row harness in `lookup_array.rs`; solver policy documented)
- [x] No new `TODO(` without a `WP-` reference; no new dependency without justification (`criterion` already workspace-approved)
- [x] Nothing written outside the repository except documented temp dirs (`lo-crosscheck` tempfile)
