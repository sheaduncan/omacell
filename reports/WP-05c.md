# Report — WP-05c: Functions Tier 0 — lookup/reference, dynamic arrays, lambda helpers, financial, engineering basics

## Plan (written before coding)

### 2026-09-04 lookup/lambda/array edge follow-up (written before coding)

- Add the reviewed non-ASCII exact lookup, signed `MATCH` mode, mismatched
  `MAP` shape, nested `BYROW`/`BYCOL` result, and 2-D `WRAPROWS`/`WRAPCOLS`
  cases to their owning corpora before implementation.
- Make type-strict text lookup comparison Unicode case-insensitive and
  normalize nonzero `MATCH` modes by sign without changing its documented
  approximate binary-search behavior.
- Match Excel's array contracts: reject non-vector wrap inputs, pad missing
  `MAP` coordinates with `#N/A` rather than broadcasting, and reject a
  multi-cell per-axis lambda result with `#CALC!`; keep every output shape
  checked before allocation.
- Update the authoritative review ledger, run the complete function suite and
  strict Clippy, then run exact `just check` before opening the PR.

### 2026-09-04 one-row array and sort-order follow-up (written before coding)

- Correct the existing `SORT`, `UNIQUE`, `TAKE`, and `DROP` corpus rows first
  so a one-row array retains row-oriented defaults; add explicit `by_col` or
  columns arguments where column-wise behavior is intended.
- Give `SORT`/`SORTBY` a deterministic value ordering in which ordinary values
  compare normally, errors compare equal after ordinary values, and true blank
  cells remain last; keep the sort stable for equal keys.
- Remove the one-row axis overrides, update the authoritative review checklist
  and compatibility evidence, then run the complete function suite, strict
  Clippy, and exact `just check` before opening the PR.

### 2026-09-04 approximate-lookup error follow-up (written before coding)

- Add the canonical last-nonblank `LOOKUP(2,1/(range<>""),range)` regression
  first, including leading, interior, and trailing error sentinels whose result
  index must remain aligned with the unfiltered return vector.
- Make the approximate search helper ignore error keys while preserving
  original indexes; retain binary search for error-free sorted vectors and
  avoid changing exact-match error propagation.
- Update the authoritative function checklist and this report, run all function
  tests and strict Clippy, then run exact `just check` before opening the PR.

### 2026-09-04 financial-semantics review follow-up (written before coding)

- Add regression corpus rows first for beginning-of-period `IPMT`/`PPMT`,
  cumulative beginning-of-period windows, the false-zero `RATE` identity, and
  scale-independent solver convergence.
- Correct the closed-form payment timing without introducing user-sized loops;
  make Newton convergence depend on successive rate estimates and a normalized
  residual instead of an absolute currency residual.
- Reconcile the remaining financial claims from the repository review in the
  same pass: reject zero rates for `EFFECT`/`NOMINAL`, apply documented positive
  rate/present-value checks to cumulative loan functions, and return the Excel
  division error for zero-life `SLN`.
- Preserve public function signatures and dependencies, update the WP report
  and authoritative review ledger, run the function/core suites and strict
  Clippy, then run exact `just check` before opening the PR.

### 2026-09-03 bit-shift boundary follow-up (written before coding)

- Update the `BITLSHIFT` and `BITRSHIFT` corpora first with cases at 49,
  positive and negative 53, and 54 bits, using values whose results do not
  independently exceed Excel's 48-bit value ceiling.
- Raise only the shared absolute shift-amount limit from 48 to 53; retain the
  existing `(2^48)-1` input/result checks and checked Rust shifts.
- Correct the compatibility note to distinguish the 53-bit shift-amount limit
  from the 48-bit value limit, preserve public interfaces, add no dependency,
  run the complete function/core suites and strict Clippy, then run exact
  `just check` and reconcile this report.

### 2026-09-03 CONVERT unit-table follow-up (written before coding)

- Add failing corpus rows first for Microsoft's case-sensitive `Pica`/`Picapt`
  factors, pound-force, every currently missing documented unit alias, all
  eight binary prefixes on information units, and binary-prefix rejection on
  unsupported units.
- Introduce a private canonical-alias table reused by direct and SI-prefixed
  lookup, correct linear/square Pica factors, add cubic Pica and cubic nautical
  mile units, and recognize binary prefixes only for bit and byte.
- Preserve existing temperature-offset handling and the current decimal-prefix
  behavior, public interfaces, and dependencies; update compatibility notes,
  run the complete function/core suites and strict Clippy, then run exact
  `just check` and reconcile this report.

- Files/modules to create:
  - `crates/fn/src/args.rs` — private helpers: scalar/number/int extraction, first-error walk, `Reference`/`RuntimeValue` → checked array, wildcard match, 1-based indexing, shape checks via `RuntimeArray::checked_len` **before** allocation. Named `args` (not `common`/`util`) so parallel WP-05a/05b helpers do not collide.
  - `crates/fn/src/lookup.rs` — `XLOOKUP`, `XMATCH`, `INDEX`, `MATCH`, `VLOOKUP`, `HLOOKUP`, `LOOKUP`, `CHOOSE`, `OFFSET`, `INDIRECT`, `ROW`, `ROWS`, `COLUMN`, `COLUMNS`, `ADDRESS`, `AREAS`. `OFFSET`/`INDIRECT` are volatile and call `EvalCtx::record_dynamic_ref`. `ROWS`/`COLUMNS`/`INDEX`/`OFFSET` operate on `Reference` dimensions without materializing whole-column payloads. `register_lookup`.
  - `crates/fn/src/array.rs` — `TRANSPOSE`, `FILTER`, `SORT`, `SORTBY`, `UNIQUE`, `SEQUENCE` (replaces the WP-05F probe; full `rows, [columns], [start], [step]`), `RANDARRAY` (volatile; `EvalCtx::random_unit("RANDARRAY", index)`), `TAKE`, `DROP`, `CHOOSEROWS`, `CHOOSECOLS`, `VSTACK`, `HSTACK`, `TOCOL`, `TOROW`, `WRAPROWS`, `WRAPCOLS`, `EXPAND`. Every user-controlled output shape is rejected (`#NUM!`) via `RuntimeArray::checked_len` / `try_new` before `Vec` allocation. `register_array`.
  - `crates/fn/src/lambda.rs` — `MAP`, `REDUCE`, `SCAN`, `BYROW`, `BYCOL`, `MAKEARRAY` (eager; last arg is a `RuntimeValue::Lambda`, applied through `omacell_core::lambda::apply`, which already enforces `MAX_CALL_DEPTH`). Catalog-only `FunctionSpec`s for evaluator-owned `LET`, `LAMBDA`, `ISOMITTED` (lazy stubs, **not** registered on `FnRegistry`). `register_lambda` skips `is_language_fn` names.
  - `crates/fn/src/financial.rs` — `PMT`, `IPMT`, `PPMT`, `NPV`, `XNPV`, `IRR`, `XIRR`, `MIRR`, `FV`, `PV`, `RATE`, `NPER`, `SLN`, `DB`, `DDB`, `SYD`, `EFFECT`, `NOMINAL`, `CUMIPMT`, `CUMPRINC`. Newton solvers: `RATE`/`IRR` max 20 iterations, `XIRR` max 100; normalized residual `<= 1e-8` or successive-rate delta `<= 1e-7`, otherwise `#NUM!`. `register_financial`.
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
  - Solver constants (`RATE`/`IRR`: 20 iters, `XIRR`: 100, normalized residual `1e-8`, rate delta `1e-7`) documented in this report and known-differences.
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
  - **`HYPERLINK` / `FORMULATEXT`:** explicit post-1.0 Appendix D Tier 1 scope.
  - **`LET` / `LAMBDA` / `ISOMITTED`:** evaluator-owned (WP-04). Catalog metadata + integration corpus only; no duplicate `FnDef`.
  - **Invalid array shapes:** keep WP-05F `#NUM!` (not Excel 365 `#CALC!`) for zero / negative / out-of-grid / overflow, including `MAKEARRAY` and stacking/wrapping. Confirmed for WP-05c.
  - **`SEQUENCE` arity:** replace the 2-arg probe with Excel's `SEQUENCE(rows, [columns], [start], [step])`. Dimension args truncate toward zero (Excel), not round (probe).
  - **Approximate `VLOOKUP`/`HLOOKUP`/`MATCH`/`LOOKUP`:** binary search assuming sorted input; unsorted data is **not** “fixed” — Excel-compatible wrong answers are corpus-covered.
  - **`FILTER` with no keepers:** `#CALC!` when `if_empty` omitted (Excel).
  - **Solver policy:** Newton–Raphson; `RATE`/`IRR` 20 iterations default guess `0.1`; `XIRR` 100 iterations; success at normalized residual `<= 1e-8` or successive-rate delta `<= 1e-7`; else `#NUM!`.
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
- `tests/corpus/functions/<NAME>.tsv` — 934 cited rows
- `docs/compat/known-differences.md` — append-only
- `scripts/lo-crosscheck.py` — `_xlfn.` prefix + CSV error/percent mapping
- `fuzz/fuzz_targets/fn_eager.rs` — every eager registered spec

Key tests: `crates/fn/tests/wp05c.rs`, `crates/fn/tests/probes.rs`.

Review hardening replaces quadratic `UNIQUE`/mode-style scans with first-seen hash indexes, preserves deterministic output order, validates output shapes before allocation, and handles extreme integer arguments without panicking. `DB`, `DDB`, `CUMIPMT`, and `CUMPRINC` now use constant-time closed forms instead of user-sized loops; `EFFECT` uses stable `ln_1p`/`exp_m1` compounding. `XNPV` accepts dates in any order provided none precedes the first date, matching Excel.

The 2026-09-03 bit-shift follow-up accepts absolute shift amounts through 53
for `BITLSHIFT` and `BITRSHIFT`, while retaining the independent 48-bit
input/result ceiling and returning `#NUM!` above 53.

The 2026-09-03 `CONVERT` follow-up completes the aliases in Microsoft's
documented unit table, distinguishes case-sensitive 1/72-inch `Pica`/`Picapt`
from 1/6-inch `pica`, adds pound-force and the missing cubic units, and supports
the eight binary prefixes on bit/byte units only.

The 2026-09-04 financial follow-up aligns beginning-of-period interest and
principal with the post-payment balance, including partial cumulative windows.
It removes `RATE`'s false zero shortcut and normalizes solver residuals by the
cash-flow scale, so multiplying every cash flow by a currency scale does not
change convergence. `EFFECT`, `NOMINAL`, cumulative loan functions, and
zero-life `SLN` now apply their documented Excel error boundaries.

The 2026-09-04 approximate-lookup follow-up gives classic `LOOKUP` an
error-skipping binary-search view that retains each comparable key's original
index. The canonical `LOOKUP(2,1/(range<>""),range)` idiom therefore reaches
the last nonblank result without an intermediate `#DIV/0!` abort. Other
approximate lookup functions retain their existing error behavior.

The 2026-09-04 array follow-up keeps `SORT` and `UNIQUE` row-oriented unless
`by_col` is explicitly true, including for one-row inputs. `TAKE` and `DROP`
now always interpret their first size argument as rows and their optional
second size argument as columns. Sort keys use a stable total ordering instead
of treating an error as equal to every value: ordinary values sort normally,
errors form an equal group after them, and blank cells remain last.

The 2026-09-04 lookup/lambda/array follow-up extends exact text lookup's
case-insensitive comparison beyond ASCII and applies classic `MATCH`'s
positive/negative mode semantics to every signed mode value. `WRAPROWS` and
`WRAPCOLS` reject matrices instead of silently flattening them. `MAP` sizes its
result to the largest input and supplies `#N/A` for coordinates missing from a
smaller input, while `BYROW` and `BYCOL` reject multi-cell lambda results with
`#CALC!` instead of constructing unsupported nested arrays.

## Interfaces exposed (for dependents)

| Item | Where |
|---|---|
| `register_all` now includes lookup/array/lambda/financial/engineering | `omacell_fn` |
| `all_specs()` | catalog = probes + WP-05c (includes `LET`/`LAMBDA`/`ISOMITTED`) |
| `LOOKUP_SPECS`, `ARRAY_SPECS`, `LAMBDA_SPECS`, `FINANCIAL_SPECS`, `ENGINEERING_SPECS` | `omacell_fn` |
| `functions_json()` | includes every spec, sorted by name |
| Solver constants | `RATE_IRR_MAX_ITERS=20`, `XIRR_MAX_ITERS=100`, `SOLVER_TOL=1e-8`, `SOLVER_RATE_TOL=1e-7`, `DEFAULT_GUESS=0.1` in `financial.rs` |
| Corpora | `tests/corpus/functions/<NAME>.tsv` |

No commands, CLI, or WP-01 type changes. `LET`/`LAMBDA`/`ISOMITTED` are **not** `FnRegistry` entries.

**WP-05a/05b:** append your `register_*` next to the existing `register_all` calls; extend `all_specs()`. Do not re-register `SEQUENCE` or `LET`/`LAMBDA`/`ISOMITTED`. Do not delete probes this package left in place.

**WP-13:** `functions_json()` is the catalog.

## Deviations from the spec or the package (with reasons)

- **§6.4 vs Appendix D engineering:** implemented as **Tier 0** per Appendix D and the WP list. §6.4 prose grouping `CONVERT`/bases under Tier 1 is recorded as a spec inconsistency. Bessel/`COMPLEX`/`ERF` not added.
- **Engineering count 14 vs “12”:** the named WP list is 14 functions; Appendix D’s 12 is approximate.
- **`HYPERLINK` / `FORMULATEXT`:** explicit post-1.0 Appendix D Tier 1 scope.
- **Invalid shapes `#NUM!`:** confirmed (not Excel 365 `#CALC!`) for `SEQUENCE`/`RANDARRAY`/`MAKEARRAY`/stacking/wrapping.
- **`SEQUENCE` dimensions:** truncate toward zero (Excel), not the WP-05F probe’s `round`.
- **LibreOffice CSV:** many `_xlfn.*` helpers evaluate to `#NAME?`; error tokens are `Err:NNN`; some numeric cases disagree. Documented; `lo-crosscheck.py` maps error codes/percents and classifies missing modern names as known. Semantic LO disagreements keep Excel-matching Omacell results.

## Measurements

Host: rustc 1.98.0, Linux.

- `just check` — pass
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — pass
- `cargo deny check` — pass (advisories/bans/licenses/sources ok)
- Corpus: **77** function files, **943** rows, all ≥10, all pass (`cargo test -p omacell-fn --test wp05c`; 11 tests after review hardening)
- Catalog: **82** specs (`all_specs`); **79** registered (`LET`/`LAMBDA`/`ISOMITTED` catalog-only); **5** remaining probes (`ABS`/`SUM`/`IF`/`NOW`/`RAND`)
- Implemented WP-05c functions: **74** (16 lookup + 18 array + 6 lambda helpers + 20 financial + 14 engineering) + **3** metadata-only language constructs
- `scripts/lo-crosscheck.py` on the original 77-file set: 861 evaluated, **0 unexplained**, 186 known (LibreOffice CSV/`_xlfn` gaps). Review re-check of updated `XNPV.tsv`: 12 evaluated, 10 known, **0 unexplained**.
- Eager smoke: `eager_functions_do_not_panic_on_garbage_args` — pass
- Criterion `crates/fn/benches/lookup_array.rs`: 1M-row `XLOOKUP`/`XMATCH`/`FILTER`/`SORT`/`UNIQUE`, 10k `MAP`, `IRR`/`RATE`. Not part of `just check`. Run `cargo bench -p omacell-fn --bench lookup_array`. Review: regressions over 10% vs this harness fail review.
- Review measurement: optimized `unique_1m --quick` **469.34 ms** midpoint on the review host.
- 2026-09-04 financial test-first evidence:
  - Fourteen review assertions reproduced across `IPMT`, `PPMT`, `RATE`, `SLN`,
    `EFFECT`, `NOMINAL`, `CUMIPMT`, and `CUMPRINC` (the first batch reported 12
    mismatches, followed by the two partial-window cases); all updated WP-05c
    corpora and the complete `omacell-fn` suite now pass.
  - `RATE(3,-500,1000,-500)` returns `0.3836729` rather than the incorrect
    zero shortcut; scaling the 12-period cash flows by one billion preserves
    the `0.02922854` result.
  - Billion-period cumulative calculations remain constant-time with a small
    positive rate and return finite results.
  - Strict all-target Clippy for `omacell-fn` is warning-free.
- 2026-09-04 approximate-lookup test-first evidence:
  - Explicit mixed `#DIV/0!`/`#N/A` keys and the computed last-nonblank idiom
    both failed before the change and pass afterward; an all-error vector still
    returns `#N/A`.
  - The complete `omacell-fn` suite passes, including every function corpus,
    integration test, eager-function panic smoke, and doctest.
- 2026-09-04 array test-first evidence:
  - Correcting the existing one-row expectations and adding stable error-order
    cases produced 12 corpus mismatches before implementation; all now pass.
  - The complete `omacell-fn` suite and strict all-target Clippy pass, including
    every function corpus and eager-function panic smoke.
  - Default row orientation follows Microsoft's [`SORT`](https://support.microsoft.com/en-us/excel/sort-function)
    documentation; equal error grouping and final blank placement follow its
    documented [sort order](https://support.microsoft.com/en-us/excel/sort-data-in-a-workbook-in-the-browser).
- 2026-09-04 lookup/lambda/array test-first evidence:
  - Nine corpus cases reproduced all remaining review claims before the
    implementation: non-ASCII exact lookup, positive and negative noncanonical
    `MATCH` modes, both 2-D wrapping paths, both nested per-axis lambda paths,
    and two mismatched-`MAP` padding paths. All nine pass afterward.
  - The complete `omacell-fn` suite passes, including every function corpus,
    integration test, eager-function panic smoke, and doctest; strict
    all-target Clippy and exact `just check` pass.
  - Scalar-return enforcement follows Microsoft's [`BYROW`](https://support.microsoft.com/en-us/excel/functions/byrow-function)
    contract, and vector rejection follows its documented [`WRAPROWS`](https://support.microsoft.com/en-us/excel/wraprows-function)
    error behavior. The signed `MATCH` and mismatched `MAP` cases retain the
    review's live-compatibility-oracle results as executable corpus evidence.
- 2026-09-03 bit-shift test-first evidence:
  - Six cases at 49 and ±53 bits initially returned `#NUM!`; they now return
    zero when the input/result remains within the 48-bit value ceiling.
  - Both 54-bit cases return `#NUM!`, as do existing shifts whose result exceeds
    `(2^48)-1`.
  - The updated corpus and complete function/core suites pass (21 function
    integration tests, 73 core unit tests, every integration suite, and 103
    core doctests); strict all-target Clippy for both crates is warning-free.
  - Exact `just check` passes formatting, workspace all-target strict Clippy,
    workspace tests and doctests, repository policy checks, and warning-free
    rustdoc.
  - Semantics follow Microsoft's [`BITLSHIFT`](https://support.microsoft.com/en-us/excel/functions/bitlshift-function)
    and [`BITRSHIFT`](https://support.microsoft.com/en-us/office/bitrshift-function-274d6996-f42c-4743-abdb-4ff95351222c)
    documentation.
- 2026-09-03 `CONVERT` test-first evidence:
  - The added 66-row matrix initially had 63 mismatches: missing aliases/units
    and binary prefixes returned `#N/A`, uppercase `Pica` converted as 1/6 inch
    and returned **12** inches for 72 Pica, and square Pica returned **144**
    square inches for 5,184 Pica². All 66 rows now pass.
  - The matrix covers every alias currently listed by Microsoft across mass,
    distance, time, pressure, force, energy, power, temperature, volume, area,
    and speed; all eight binary prefixes are checked as adjacent 1024× ratios.
  - The exact internal pound-force factor is **4.4482216152605 N**; General
    formatting displays **4.448221615** in the corpus.
  - The complete function/core suites pass (21 function integration tests, 73
    core unit tests, every integration suite, and 103 core doctests); strict
    all-target Clippy for both crates is warning-free.
  - Exact `just check` passes formatting, workspace all-target strict Clippy,
    workspace tests and doctests, repository policy checks, and warning-free
    rustdoc.
  - `scripts/lo-crosscheck.py --help` skipped because a LibreOffice converter
    is unavailable; no runtime/test dependency on LibreOffice was introduced.
  - Unit names and prefixes follow Microsoft's [`CONVERT`](https://support.microsoft.com/en-us/excel/functions/convert-function)
    documentation; the pound-force constant follows the exact factor in
    [NIST SP 811](https://physics.nist.gov/cuu/pdf/sp811.pdf).

## Open questions / decisions needed

1. **Resolved:** `ISOMITTED` metadata remains under `lambda`; no duplicate spec
   was added.
2. **Human / WP-28 oracle:** LibreOffice lacks most dynamic-array/lambda/`XNPV`
   functions; record the remaining rows against live Excel 365.
3. **Human / WP-28 fixed host:** record the full 1M-row lookup-array Criterion
   sample and committed baseline.
4. **Resolved 2026-09-03:** bit-operation values/results remain limited to 48
   bits, while `BITLSHIFT`/`BITRSHIFT` shift magnitudes are allowed through 53.
5. **Resolved 2026-09-03:** `CONVERT` recognizes Microsoft's complete current
   alias table and binary prefixes, including case-sensitive Pica semantics.
6. **Resolved 2026-09-04:** beginning-of-period loan schedules, financial
   argument errors, and rate-solver convergence now match the documented Excel
   behavior. The live-Excel oracle remains the separately owned WP-28 gate.
7. **Resolved 2026-09-04:** classic approximate `LOOKUP` skips error sentinels
   while preserving the return vector's original indexes.
8. **Resolved 2026-09-04:** one-row dynamic arrays retain row-oriented defaults,
   `TAKE`/`DROP` use their documented row and column positions, and sort errors
   no longer violate comparator transitivity.
9. **Resolved 2026-09-04:** exact lookup handles non-ASCII case, all signed
   `MATCH` modes follow their sign, wrap functions reject matrices, `MAP` pads
   mismatched shapes, and per-axis lambdas cannot return nested arrays.

## RFC (only if a frozen contract changed)

None. WP-01 types unchanged.

The bit-shift boundary follow-up changes no public signature or frozen
contract.

The `CONVERT` unit-table follow-up changes no public signature or frozen
contract.

The financial-semantics follow-up adds one documented solver constant and
changes no function signature, command schema, or frozen WP-01 type.

The approximate-lookup follow-up changes no public signature or frozen
contract.

The array-orientation and sort-order follow-up changes no public signature or
frozen contract.

The lookup/lambda/array edge follow-up changes no public signature or frozen
contract.

## Checklist

- [x] `just check` green
- [x] Every acceptance criterion ticked with evidence (corpora ≥10 rows; `functions.json` via `functions_json`; fuzz smoke test; LO unexplained = 0; shapes rejected before allocation; solver policy + bench harness)
- [x] Docs warning-free; public items documented (`RUSTDOCFLAGS=-D warnings`)
- [x] Baselines recorded (criterion bench added; 1M-row harness in `lookup_array.rs`; solver policy documented)
- [x] No new `TODO(` without a `WP-` reference; no new dependency without justification (`criterion` already workspace-approved)
- [x] Nothing written outside the repository except documented temp dirs (`lo-crosscheck` tempfile)
