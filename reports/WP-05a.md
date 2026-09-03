# Report — WP-05a: Functions Tier 0 — math, statistics, logical, information, criteria aggregation

## Plan (written before coding)

- Files/modules to create:
  - `crates/fn/src/common.rs` — shared walk/coerce, Excel rounding, criteria matching, pairwise and descriptive-stat helpers. No registry of its own.
  - `crates/fn/src/math.rs` — math & trig specs + eager bodies; `register_math`. Replaces probe `ABS`, `SUM`, `RAND`.
  - `crates/fn/src/stat.rs` — descriptive statistics; `register_stat`. Compatibility aliases (`STDEV`, `VAR`, `MODE`, `RANK`, `PERCENTILE`, `QUARTILE`, `COVAR`, `FORECAST`, `PEARSON`) are `FunctionSpec.aliases`, not extra bodies.
  - `crates/fn/src/logical.rs` — `IF`/`IFS`/`SWITCH`/`IFERROR`/`IFNA` as `FnBody::Lazy`; `AND`/`OR`/`XOR`/`NOT`/`TRUE`/`FALSE` eager. `register_logical`. Replaces probe `IF`.
  - `crates/fn/src/info.rs` — `IS*` family, `TYPE`, `ERROR.TYPE`, `NA`, `N`, `CELL` subset; `ISOMITTED` catalog spec only (no `FnDef` registration). `register_info`.
  - `crates/fn/src/aggregate.rs` — `SUMIF(S)`, `COUNTIF(S)`, `AVERAGEIF(S)`, `MAXIFS`, `MINIFS`, `AGGREGATE`, `SUBTOTAL`. Hidden-row / nested-subtotal via `EvalCtx` sheet callbacks, not evaluator name special-cases. `register_aggregate`.
  - `crates/fn/src/probes.rs` — keep `NOW` and `SEQUENCE` only (WP-05b / WP-05c).
  - `crates/fn/src/{lib,metadata,corpus}.rs` — `register_all` concatenates probe + our `register_*`; `functions_json` iterates every spec including `ISOMITTED`; corpus runner uses `register_all`.
  - `crates/core/src/eval/mod.rs` — additive `EvalCtx` sheet callbacks (`is_row_hidden`, `formula_source`, stored-cell walk, style/sheet name). No function-name intercepts. Frozen WP-01 types unchanged.
  - `crates/fn/benches/aggregates.rs` — whole-column `SUM`, `SUMIFS`, `SUBTOTAL` baselines.
  - `crates/fn/tests/{corpus,integration}.rs` — TSV runner, lazy-branch / hidden-row / deterministic-random / whole-column / criteria-wildcard / fuzz-smoke.
  - `tests/corpus/functions/<NAME>.tsv` — ≥10 cited rows per function (including `ISOMITTED` integration corpus).
  - `docs/compat/known-differences.md` — append-only rows for triaged Excel/LibreOffice divergences.
  - `fuzz/fuzz_targets/fn_eager.rs` — smoke every eager spec, not just probes.
- Interfaces to expose (types, commands, schemas, CLI):
  - `omacell_fn::{register_math, register_stat, register_logical, register_info, register_aggregate, register_all, all_specs}`.
  - Existing `define_fn!` / `FunctionSpec` / `FnBody::{Eager,Lazy}` / `EvalCtx::{clock,locale,random_unit}` / `RuntimeArray::try_new` unchanged.
  - `EvalCtx` sheet callbacks listed above (core, additive).
  - Catalog still `functions_json()` + `docs/schemas/functions.schema.json` schema 1.
  - No CLI. No WP-01 type changes. No new runtime dependencies.
- Tests and corpora to write first:
  - Per-function TSV covering applicable: normal, empty/omitted, error-propagation / evaluation-order, array-lifting, reference-vs-literal coercion, boundaries. Inapplicable categories named in metadata `doc` rather than padded duplicates.
  - Lazy: `IF`/`IFS`/`SWITCH`/`IFERROR`/`IFNA` skip unselected error, volatile, and async branches.
  - `AND`/`OR` do **not** short-circuit (corpus: `AND(FALSE,1/0)` → `#DIV/0!`).
  - `RAND`/`RANDBETWEEN` from pass nonce + cell + function identity; 1 vs 8 threads bit-identical.
  - Hidden-row `SUBTOTAL` 101–111 / `AGGREGATE` options; nested `SUBTOTAL` ignored.
  - Whole-column `SUM`/`SUMIFS`/`SUBTOTAL` (formula off the column so A:A is not circular).
  - Criteria wildcards `* ? ~` and comparison prefixes.
  - Fuzz smoke over every eager registration (no panic).
- Items the package says to "decide and document" and the decision taken:
  - **Dispatch:** reuse WP-05F `FnBody`. Lazy only for `IF`/`IFS`/`SWITCH`/`IFERROR`/`IFNA`. No new evaluator name special-cases. `ISOMITTED` stays WP-04 intercept; this package adds catalog + corpus only.
  - **Sheet callback:** `EvalCtx` methods that read workbook geometry / formula intern / occupied cells. Functions never import sheet types beyond what `EvalCtx` already exposes.
  - **Range vs literal:** SUM-family skips text/bools/empties in refs and arrays; literals coerce (Excel). Probe `SUM(1,TRUE)=2` stays valid.
  - **Whole-column:** aggregates walk occupied cells via the stored-cell callback; they do not materialize `A:A` into a runtime array. `COUNTBLANK` uses dimension − occupied (+ empty-string slots).
  - **Random:** `ctx.random_unit("RAND"|"RANDBETWEEN", index)` only; no thread-local RNG.
  - **Math set (~60):** explicit WP list plus the remaining Excel math/trig names needed to reach Appendix D (~60), including hyperbolic/reciprocal trig, legacy `CEILING`/`FLOOR`, `CEILING.PRECISE`/`FLOOR.PRECISE`/`ISO.CEILING`, `FACTDOUBLE`, `SQRTPI`. Not owned: `RANDARRAY`/`SEQUENCE` (05c), `NOW` (05b probe).
  - **Stat set (~45 unique + compat aliases):** WP list. `AVERAGEIF(S)` live in `aggregate`. Compatibility names are aliases.
  - **CELL subset:** `address`, `col`, `row`, `contents`, `type`, `format` only. Omitted reference uses the last changed cell retained by `RecalcEngine`; direct evaluation without a live session falls back to the formula cell.
  - **Criteria:** comparison prefixes, `* ? ~` wildcards, numeric/text/bool/blank matching per Excel; `*IFS` ranges must be the same height/width.
- Open questions at planning time:
  1. Excel desktop `CELL` without a reference tracks the last edited cell; live recalculation now does the same, while direct evaluation without a session falls back to the formula cell.
  2. Legacy `CEILING`/`FLOOR` sign rules differ across Excel versions; corpus cites 365-style and LibreOffice disagreements go to known-differences.
  3. `MODE.MULT` / `FREQUENCY` spill arrays; 1×1 collapse follows WP-05F `RuntimeValue::array`.

### 2026-09-03 empty/filter aggregate plan (written before coding)

- Add failing integration regressions first for `MIN`, `MAX`, `MINA`, and
  `MAXA` over an empty range; and for `SUBTOTAL` distinguishing a row excluded
  by AutoFilter from a separately, manually hidden row. Add corpus coverage for
  empty MAX/MIN and aggregate function numbers 4/5.
- Return zero when MIN/MAX-family aggregation has no numeric values, including
  `SUBTOTAL` and `AGGREGATE` function numbers 4/5, while preserving existing
  error propagation and coercion rules.
- Expose the filter-owned hidden-row distinction through `EvalCtx` so every
  `SUBTOTAL` function number excludes filtered rows, while only 101–111 exclude
  manually hidden rows. Leave `AGGREGATE`'s option-controlled hidden-row policy
  unchanged.
- Keep frozen types and existing method signatures unchanged, add no
  dependency, run the function/core suites and strict Clippy, then run exact
  `just check` and reconcile this report.

### 2026-09-03 SUMIF value-range plan (written before coding)

- Add a failing 2-D integration regression first where the written
  `sum_range`/`average_range` has a different shape from the criteria range.
  Assert both the initial result and an incremental edit to a formula cell that
  lies inside the effective resized range but outside the written range.
- Derive the effective value reference from its normalized top-left cell and
  the criteria reference's height and width, matching Microsoft's documented
  `SUMIF` and `AVERAGEIF` behavior. Reject an implied range outside worksheet
  bounds without materializing it.
- Record a differing effective reference through the existing resolved-range
  dependency channel and classify `SUMIF`/`AVERAGEIF` as functions with
  evaluation-resolved precedents. This keeps initial ordering and later dirty
  propagation correct even when the implicit cells contain formulas.
- Preserve same-shaped and omitted value-range behavior, keep public
  signatures/frozen contracts unchanged, add no dependency, run the complete
  function/core suites, strict Clippy, exact `just check`, and reconcile this
  report.

### 2026-09-03 array-valued IF plan (written before coding)

- Add a failing sheet integration regression for the audited
  `SUM(IF(A1:A3>1,A1:A3,0))` case, plus array-branch broadcasting and
  all-true/all-false cases proving an entirely unselected error branch remains
  unevaluated.
- Keep scalar `IF` on its existing lazy path. For an array logical test,
  validate and coerce each condition cell, evaluate only branch expressions
  selected by at least one condition cell, then broadcast the materialized
  branch values over the result shape with per-cell errors.
- Reuse the frozen WP-05F runtime-array limits and constructors, preserve
  existing omitted-branch behavior and public interfaces, add no dependency,
  run the complete function/core suites and strict Clippy, then run exact
  `just check` and reconcile this report.

### 2026-09-03 logical aggregate text plan (written before coding)

- Add failing corpus rows for `AND`, `OR`, and `XOR` over arrays containing
  text, plus a sheet integration matrix covering text and empty cells inside
  references, a remaining logical value, and a reference with no logical
  values.
- Use the existing value-origin signal in the shared logical fold: ignore text
  and empty values encountered through an array or reference, retain direct
  literal coercion/errors and error propagation, and return `#VALUE!` when no
  usable value remains.
- Cite Microsoft's documented aggregate-logical rule, preserve public
  interfaces, add no dependency, run the complete function/core suites and
  strict Clippy, then run exact `just check` and reconcile this report.

### 2026-09-03 SUMPRODUCT shape/coercion plan (written before coding)

- Change the existing Boolean-array corpus expectation first and add a failing
  equal-length/different-shape case, citing Microsoft's `SUMPRODUCT` remarks.
- Preserve each materialized argument's row/column dimensions instead of
  flattening before validation, require exact shape equality, and multiply
  finite numbers only. Treat Boolean, text, and empty entries as zero while
  preserving cell-error and non-finite-number propagation.
- Reuse WP-05F runtime-array validation, preserve public interfaces, add no
  dependency, run the complete function/core suites and strict Clippy, then
  run exact `just check` and reconcile this report.

### 2026-09-03 PERCENTRANK significance plan (written before coding)

- Add failing corpus rows first from Microsoft's `PERCENTRANK.INC` and
  `PERCENTRANK.EXC` examples that distinguish decimal truncation from rounding,
  plus the documented `#NUM!` result for significance below one.
- Validate the truncated significance before converting it to an integer, then
  truncate the computed nonnegative rank to that many decimal places using the
  existing decimal helper rather than binary floating-point scaling.
- Preserve public interfaces, add no dependency, run the complete function/core
  suites and strict Clippy, then run exact `just check` and reconcile this
  report.

### 2026-09-03 explicit omitted numeric arguments plan (written before coding)

- Change the existing `AVERAGE`, `PRODUCT`, and `COUNT` trailing-omission
  corpus expectations first, add internal-omission and representative
  statistical aggregate cases, and add a sheet regression proving referenced
  empty cells remain ignored.
- Treat an explicitly omitted `ArgVal` as a literal numeric zero only in the
  shared numeric-list collectors and `COUNT`. Keep the generic value walker,
  reference/array coercion, optional positional defaults, lazy functions, and
  `ISOMITTED` behavior unchanged.
- Preserve public interfaces and dependencies; cite the Excel omission and
  empty-reference behavior, run the complete function/core suites and strict
  Clippy, then run exact `just check` and reconcile this report.

### 2026-09-03 exclusive-percentile endpoint plan (written before coding)

- Change the existing `PERCENTILE.EXC` lower-rank boundary corpus expectation
  first; add its exact upper endpoint, adjacent out-of-range controls, and
  `QUARTILE.EXC` cases whose first and third quartiles land exactly on ranks
  one and n.
- Admit `k = 1/(n+1)` and `k = n/(n+1)` in the shared exclusive-percentile
  helper so interpolation returns the first and last values. Retain `#NUM!`
  for non-finite k, samples smaller than two, and ranks outside those bounds.
- Preserve inclusive-percentile behavior, public interfaces, and dependencies;
  cite Microsoft's exclusive-percentile rule, run the complete function/core
  suites and strict Clippy, then run exact `just check` and reconcile this
  report.

### 2026-09-03 legacy CEILING zero-significance plan (written before coding)

- Change the existing `CEILING(4,0)` corpus expectation first and add a
  negative-number regression proving zero significance returns zero for either
  sign of the number.
- Return zero before sign validation or division in the legacy two-argument
  `CEILING` implementation. Keep `FLOOR`, the `.MATH`/`.PRECISE` variants,
  nonzero significance rules, public interfaces, and dependencies unchanged.
- Run the complete function/core suites and strict Clippy, then run exact
  `just check` and reconcile this report with test-first evidence.

### 2026-09-02 criteria-type follow-up plan (written before coding)

- Add a sheet-range integration regression first that distinguishes true
  blanks, formula empty text, numbers, numeric text, booleans, and ordinary
  text across every `*IF(S)` aggregate.
- Give criteria matching its own Excel type policy instead of falling through
  to general formula comparison: numeric criteria accept numeric text,
  wildcards inspect text only, and blank range cells do not become zero or
  FALSE. Preserve Excel's special rule that a reference to a truly empty
  criteria cell is treated as numeric zero.
- Re-run the WP-05a corpora/integration suite, the complete function and core
  suites, and the exact repository gate. Preserve frozen interfaces and add no
  dependency.

### 2026-09-02 decimal-rounding follow-up plan (written before coding)

- Add corpus regressions for binary-representation boundaries in `ROUND`,
  `ROUNDUP`, `ROUNDDOWN`, and `TRUNC`, including positive and negative inputs.
  The expected direction follows Microsoft's documentation for
  [`ROUND`](https://support.microsoft.com/en-us/excel/functions/round-function),
  [`ROUNDUP`](https://support.microsoft.com/en-us/excel/roundup-function),
  [`ROUNDDOWN`](https://support.microsoft.com/en-us/excel/functions/rounddown-function),
  and [`TRUNC`](https://support.microsoft.com/en-US/Excel/trunc-function),
  while input normalization follows Excel's documented
  [15-significant-digit precision](https://learn.microsoft.com/en-us/troubleshoot/microsoft-365-apps/excel/floating-point-arithmetic-inaccurate-result).
- Replace binary multiply-and-floor decisions with an integer operation on a
  normalized 15-digit decimal coefficient. Keep the existing public helpers,
  argument coercion, extreme-digit behavior, and frozen runtime interfaces.
- Re-run all WP-05a corpora, function/core suites, strict documentation, and
  the exact repository gate. Add no dependency.

## What was built

WP-05a fills the first third of Tier 0 on the frozen WP-05F runtime. Probe `ABS`/`SUM`/`RAND`/`IF` are replaced; `NOW`/`SEQUENCE` stay for WP-05b/05c. `ISOMITTED` is catalog + corpus only.

Key files:

- `crates/fn/src/{common,math,stat,logical,info,aggregate}.rs`
- `crates/fn/src/{lib,metadata,corpus,probes}.rs` — `register_all` / `all_specs` / `functions_json`
- `crates/core/src/eval/mod.rs` — `EvalCtx::{for_each_cell_at,for_each_stored_cell,reference_cell_count,is_row_hidden,formula_source,cell_num_fmt,sheet_name}`
- `crates/core/src/workbook.rs` — `Workbook::set_row_hidden`
- `crates/fn/tests/{corpus,integration,probes}.rs`
- `crates/fn/benches/aggregates.rs`
- `tests/corpus/functions/<NAME>.tsv` (155 files)
- `docs/compat/known-differences.md` (append-only)
- `scripts/lo-crosscheck.py` — `_xlfn.` modern names, numeric compare, LO CSV error tokens
- `fuzz/fuzz_targets/fn_eager.rs`

Key tests: `crates/fn/tests/corpus.rs`, `integration.rs` (`lazy_if_family_*`, `and_or_do_not_short_circuit`, `hidden_row_subtotal_*`, `nested_subtotal_is_ignored`, `random_is_deterministic_across_thread_counts`, `whole_column_sum_sumifs_subtotal`, `criteria_wildcards_on_sheet_ranges`, `if_family_requires_range_references`, `fuzz_smoke_eager_functions_do_not_panic`).

Review hardening corrects the Excel `AGGREGATE` option table, including hidden-row/nested-aggregate behavior and `COUNTA`/error handling. `GCD`/`LCM` now reject negative and ≥2^53 inputs/results. Mode calculations use a deterministic first-seen hash index instead of quadratic scans; `FREQUENCY` uses binary-search bins and validates its spill shape before allocation. Criteria wildcards are non-recursive and treat `?` as one Unicode scalar, eliminating adversarial recursion/stack growth.

The 2026-09-02 criteria-type follow-up gives `*IF(S)` aggregates a
criteria-specific comparison policy. Numeric criteria accept numeric text but
not blanks or booleans, wildcards inspect text values only, Boolean criteria
remain Boolean, and literal blank criteria match both true blanks and formula
empty text. A reference to a truly empty criteria cell retains Excel's special
numeric-zero behavior. One sheet-range integration matrix covers these cases
across `SUMIF(S)`, `COUNTIF(S)`, `AVERAGEIF(S)`, `MAXIFS`, and `MINIFS`.

The 2026-09-02 decimal-rounding follow-up normalizes finite inputs to Excel's
15-significant-digit decimal coefficient before deciding which requested
digits to discard. The rounding direction is then an exact integer decision,
so binary products just below or above an integer no longer change results for
`ROUND`, `ROUNDUP`, `ROUNDDOWN`, or `TRUNC`.

The 2026-09-03 empty/filter aggregate follow-up makes `MIN`, `MAX`, `MINA`,
`MAXA`, and aggregate function numbers 4/5 return zero when their inputs contain
no numeric values. `SUBTOTAL` now distinguishes rows excluded by AutoFilter from
rows hidden manually: every function number excludes the former, while only
101–111 exclude the latter. `AGGREGATE` retains its existing option-controlled
hidden-row behavior.

The 2026-09-03 SUMIF value-range follow-up resizes an explicit `sum_range` or
`average_range` from its normalized top-left cell to the criteria range's height
and width. Direct references contribute that exact effective range to the static
dependency graph; references resolved through names or expressions use the
existing evaluation-resolved dependency channel. This orders formula cells in
the implicit extent, dirties dependents after later edits, and avoids false
cycles from cells in the written but excluded tail.

The SUMIF value-range follow-up adds
`sumif_value_ranges_resize_from_top_left_and_track_implicit_cells` and
`sumif_resized_range_does_not_create_a_false_cycle_from_the_written_tail` to
`crates/fn/tests/integration.rs`.

The 2026-09-03 array-valued `IF` follow-up keeps the scalar lazy path unchanged
and adds cell-wise selection when the logical test is an array. Only branches
selected by at least one condition cell are evaluated; scalar and array branch
results then use the evaluator's existing singleton-dimension broadcasting
rules. Condition errors stay local to their output cells, and transient output
shapes retain the WP-05F validation and allocation cap.

The follow-up adds
`array_if_selects_and_broadcasts_cells_without_evaluating_unused_branches` to
`crates/fn/tests/integration.rs`.

The 2026-09-03 logical aggregate text follow-up uses the existing value-origin
signal in `AND`, `OR`, and `XOR`: text and empty values encountered inside an
array or reference are ignored, while a direct text argument still errors.
Errors inside aggregates still propagate, and an aggregate with no usable
values still returns `#VALUE!`.

The follow-up adds one cited corpus row to each logical aggregate and
`logical_aggregates_ignore_text_and_empty_cells_inside_references` to
`crates/fn/tests/integration.rs`.

The 2026-09-03 `SUMPRODUCT` follow-up retains each materialized argument's
row/column shape until all arguments have been validated as exact matches.
Only finite numeric entries participate in multiplication; Boolean, text, and
empty entries contribute zero, while cell errors and non-finite numbers retain
their existing error behavior.

The 2026-09-03 `PERCENTRANK` follow-up truncates inclusive and exclusive ranks
to the requested decimal significance and returns `#NUM!` when the truncated
significance is below one, matching Microsoft's examples and documented error
behavior.

The 2026-09-03 explicit-omission follow-up treats an empty numeric argument
slot as a direct zero for the shared numeric-list aggregates and `COUNT`.
Empty cells reached through arrays or references remain ignored, and generic
argument walking, positional defaults, lazy functions, and `ISOMITTED` are
unchanged.

The 2026-09-03 exclusive-percentile endpoint follow-up admits exact rank one
and rank n in the shared `PERCENTILE.EXC` interpolation helper. Those ranks
return the first and last sample values, including the corresponding
`QUARTILE.EXC` endpoints; ranks outside that interval still return `#NUM!`.

## Interfaces exposed (for dependents)

| Item | Where |
|---|---|
| `register_math` / `register_stat` / `register_logical` / `register_info` / `register_aggregate` / `register_all` | `omacell_fn` |
| `all_specs()`, `functions_json()` (includes `ISOMITTED`) | same |
| `EvalCtx` sheet callbacks listed above | `omacell_core::eval` |
| `EvalCtx::is_row_filtered` | same; distinguishes AutoFilter-owned hidden rows from manual row hiding for `SUBTOTAL` |
| `Workbook::set_row_hidden` | `omacell_core::workbook` |
| Catalog schema | unchanged, `docs/schemas/functions.schema.json` (`schema: 1`) |

**WP-05b/c:** do not reformat unrelated modules; keep `NOW`/`SEQUENCE` probes until you replace them; append known-differences rows only.

Frozen WP-01 types unchanged.

## Deviations from the spec or the package (with reasons)

- **`PEARSON`** is a full catalog spec sharing `CORREL`'s body (not only an alias), so `functions.json` documents it independently.
- **`CELL` direct-evaluation fallback.** A live `RecalcEngine` uses the last changed cell; formula-corpus and other direct evaluation without a session use the formula cell.
- **`PERMUTATIONA(0,0)`** returns `1` (empty product); Excel `#NUM!`.
- **`*IF` range arguments are references, not arrays.** Array constants now
  return `#VALUE!`; sheet-range integration tests retain the criteria, wildcard,
  and multi-range behavior that the original one-cell corpora exercised.
- **Spilled arrays** in `format_cell` show the origin scalar; corpus expected values follow that (1×1 `MODE.MULT` collapses via `RuntimeValue::array`).
- **LibreOffice headless** disagrees on importer names, CSV error tokens, `TYPE(TRUE)`, and some array-logical aggregates; tagged `known difference` in corpus notes and summarised in `docs/compat/known-differences.md`.

## Measurements

Host: rustc 1.98.0, Linux.

- `just check` — pass
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — pass
- 2026-09-02 criteria-type follow-up: `cargo test -p omacell-fn` and
  `cargo test -p omacell-core` pass; the exact `just check` gate passes with
  `CARGO_BUILD_JOBS=2` to avoid local parallel-linker contention.
- 2026-09-02 decimal-rounding follow-up: 11 new boundary corpus rows pass;
  `cargo test -p omacell-fn`, `cargo test -p omacell-core`, and strict
  all-target `omacell-fn` Clippy pass. The exact `just check` gate and
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` pass with
  `CARGO_BUILD_JOBS=2` for this machine's linker constraint.
- 2026-09-03 empty/filter aggregate test-first evidence:
  - Six new corpus rows initially returned `#NUM!` for empty MIN/MAX forms;
    they now return zero. An eight-form integration matrix covers `MIN`, `MAX`,
    `MINA`, `MAXA`, `SUBTOTAL`, and `AGGREGATE` over an empty range.
  - The AutoFilter regression initially returned **60** from
    `SUBTOTAL(9,A2:A4)`, including a filtered-out 10. It now returns **50**,
    while `SUBTOTAL(109,A2:A4)` also excludes a separately manually hidden 20
    and returns **30**.
  - The complete function and core suites pass (17 function integration tests,
    73 core unit tests, every integration suite, and 103 core doctests); strict
    all-target Clippy for both crates is warning-free.
  - Exact `just check` passes formatting, workspace all-target strict Clippy,
    workspace tests and doctests, repository policy checks, and warning-free
    rustdoc.
  - Criterion `--quick`: whole-column `SUBTOTAL` is **9.65 ms** versus the
    recorded **8.84 ms** baseline (**9.2%** slower, within the 10% review
    threshold). `SUM` is **5.08 ms** and `SUMIFS` is **62.96 ms**, both faster
    than their recorded baselines.
  - Semantics follow Microsoft's [`MAX`](https://support.microsoft.com/en-us/office/max-function-e0012414-9ac8-4b34-9a47-73e662c08098),
    [`MIN`](https://support.microsoft.com/en-us/office/min-function-61635d12-920f-4ce2-a70f-96f202dcc152),
    and [`SUBTOTAL`](https://support.microsoft.com/en-us/office/subtotal-function-7b027003-f060-4ade-9040-e478765b9939)
    documentation.
- 2026-09-03 SUMIF value-range test-first evidence:
  - The 2-D mismatch initially summed the written `D1:D4` tail and returned
    **1800** instead of the effective `D1:E2` result **70**. It now returns 70
    for `SUMIF` and 35 for `AVERAGEIF`.
  - Editing a formula precedent in the implicit `E2` cell updates those values
    to 80 and 40 through incremental recalc. A second regression initially
    reported a false circular reference when the SUMIF formula occupied `D3`,
    which is in the written tail but outside the effective range; it now returns
    70 with an empty circular set.
  - The complete function and core suites pass (19 function integration tests,
    73 core unit tests, every integration suite, and 103 core doctests); strict
    all-target Clippy for both crates is warning-free.
  - Exact `just check` passes formatting, workspace all-target strict Clippy,
    workspace tests and doctests, repository policy checks, and warning-free
    rustdoc.
  - Semantics follow Microsoft's [`SUMIF`](https://support.microsoft.com/en-us/office/sumif-function-169b8c99-c05c-4483-a712-1697a653039b)
    and [`AVERAGEIF`](https://support.microsoft.com/en-us/office/averageif-function-faec8e2e-0dec-4308-af69-f5576d8ac642)
    documentation.
- 2026-09-03 array-valued `IF` test-first evidence:
  - The audited `SUM(IF(A1:A3>1,A1:A3,0))` regression initially returned
    `#VALUE!`; it now returns **5**.
  - The same regression verifies array-branch/scalar-branch broadcasting,
    horizontal spill values `1, 10, 3`, and a condition error localized as
    `1, #N/A, 2`.
  - All-false and all-true condition arrays return **21** while leaving their
    respective `1/0` branch entirely unevaluated.
  - The complete function and core suites pass (20 function integration tests,
    73 core unit tests, every integration suite, and 103 core doctests); strict
    all-target Clippy for both crates is warning-free.
  - Exact `just check` passes formatting, workspace all-target strict Clippy,
    workspace tests and doctests, repository policy checks, and warning-free
    rustdoc.
  - Behavior follows Microsoft's documented [`IF`](https://support.microsoft.com/en-us/office/if-function-69aed7c9-4e8a-4755-a9bc-aa8bbff73be2)
    value-selection contract and Omacell's WP-05F array broadcasting policy.
- 2026-09-03 logical aggregate text test-first evidence:
  - A sheet range containing `TRUE`, text, and an empty cell initially made
    `AND` return `#VALUE!`; it now returns `TRUE`. The corresponding `OR` and
    `XOR` cases return `FALSE` and `TRUE`.
  - A text/empty-only reference and a direct text argument both still return
    `#VALUE!`.
  - Three new array corpus rows pass, as do the complete function and core
    suites (21 function integration tests, 73 core unit tests, every
    integration suite, and 103 core doctests); strict all-target Clippy for
    both crates is warning-free.
  - Exact `just check` passes formatting, workspace all-target strict Clippy,
    workspace tests and doctests, repository policy checks, and warning-free
    rustdoc.
  - Semantics follow Microsoft's [`AND`](https://support.microsoft.com/en-us/excel/functions/and-function),
    [`OR`](https://support.microsoft.com/en-us/excel/functions/or-function),
    and [`XOR`](https://support.microsoft.com/en-us/excel/functions/xor-function)
    documentation.
- 2026-09-03 `SUMPRODUCT` test-first evidence:
  - `SUMPRODUCT({1,2},{3,TRUE})` initially returned **5** by coercing `TRUE` to
    one; it now returns **3** because the Boolean entry contributes zero.
  - `SUMPRODUCT({1,2},{3;4})` initially returned **11** after losing argument
    shape during flattening; it now returns `#VALUE!` for the 1×2/2×1 mismatch.
  - The updated corpus and complete function/core suites pass (21 function
    integration tests, 73 core unit tests, every integration suite, and 103
    core doctests); strict all-target Clippy for both crates is warning-free.
  - Exact `just check` passes formatting, workspace all-target strict Clippy,
    workspace tests and doctests, repository policy checks, and warning-free
    rustdoc.
  - Semantics follow Microsoft's [`SUMPRODUCT`](https://support.microsoft.com/en-us/office/sumproduct-function-16753e75-9f68-4874-94ac-4d2145a2fd2e)
    documentation.
- 2026-09-03 `PERCENTRANK` test-first evidence:
  - Microsoft's inclusive example for a 6/9 rank initially returned **0.667**;
    it now truncates to **0.666** at the default three-digit significance.
  - Microsoft's exclusive `5.43` example at significance one initially
    returned **0.4**; it now truncates to **0.3**.
  - Significance zero initially returned **0.3** after being clamped to one; it
    now returns the documented `#NUM!` error.
  - The updated corpus and complete function/core suites pass (21 function
    integration tests, 73 core unit tests, every integration suite, and 103
    core doctests); strict all-target Clippy for both crates is warning-free.
  - Exact `just check` passes formatting, workspace all-target strict Clippy,
    workspace tests and doctests, repository policy checks, and warning-free
    rustdoc.
  - Semantics follow Microsoft's [`PERCENTRANK.INC`](https://support.microsoft.com/en-us/excel/functions/percentrank-inc-function)
    and [`PERCENTRANK.EXC`](https://support.microsoft.com/en-us/excel/functions/percentrank-exc-function)
    documentation.
- 2026-09-03 explicit omitted numeric argument test-first evidence:
  - The nine changed corpus cases initially failed: `AVERAGE(1,2,)` returned
    **1.5** instead of **1**, and `PRODUCT`, `COUNT`, `AVERAGEA`, `MIN`, `MAX`,
    `MEDIAN`, and `STDEV.P` all dropped the explicit empty slot. All nine now
    pass, including an internal omitted slot in `AVERAGE(1,,3)`.
  - The sheet regression initially returned **2** for
    `AVERAGE(A1:A2,2,)`; it now returns **1**, while `AVERAGE(A1:A2)` over the
    same empty reference remains `#DIV/0!`.
  - The complete function/core suites pass (22 function integration tests, 73
    core unit tests, every integration suite, and 103 core doctests); strict
    all-target Clippy for both crates is warning-free.
  - Exact `just check` passes formatting, workspace all-target strict Clippy,
    workspace tests and doctests, repository policy checks, and warning-free
    rustdoc.
  - The optional LibreOffice cross-check skipped because no LibreOffice
    converter is installed; no runtime or test dependency was introduced.
  - Omitted-slot behavior follows the verified Excel audit case and the
    [Microsoft Q&A trailing-comma example](https://learn.microsoft.com/en-us/answers/questions/4892755/impact-of-extra-comma-in-formulas);
    referenced empty cells follow Microsoft's [`AVERAGE`](https://support.microsoft.com/en-us/excel/functions/average-function)
    documentation.
- 2026-09-03 exclusive-percentile endpoint test-first evidence:
  - Four cases initially returned `#NUM!`: lower and upper
    `PERCENTILE.EXC` endpoints for a four-value sample, plus `QUARTILE.EXC`
    quartiles one and three for a three-value sample. They now return the
    sample endpoints **1** and **4**, or **1** and **3**, respectively.
  - Values immediately below `1/(n+1)` and above `n/(n+1)` remain `#NUM!`;
    existing invalid-k, undersized-sample, error-propagation, and interpolation
    cases remain green.
  - The complete function/core suites pass (22 function integration tests, 73
    core unit tests, every integration suite, and 103 core doctests); strict
    all-target Clippy for both crates is warning-free.
  - Exact `just check` passes formatting, workspace all-target strict Clippy,
    workspace tests and doctests, repository policy checks, and warning-free
    rustdoc.
  - The optional LibreOffice cross-check skipped because no LibreOffice
    converter is installed; no runtime or test dependency was introduced.
  - Semantics follow Microsoft's [`PERCENTILE.EXC`](https://support.microsoft.com/en-us/excel/functions/percentile-exc-function)
    and [`QUARTILE.EXC`](https://support.microsoft.com/en-us/excel/functions/quartile-exc-function)
    documentation.
- 2026-09-03 legacy `CEILING` zero-significance test-first evidence:
  - The positive and negative cases `CEILING(4,0)` and `CEILING(-4,0)` both
    initially returned `#DIV/0!`; they now return numeric zero.
  - Existing zero-number, exact-multiple, coercion, error-propagation, and
    Excel 2010+ asymmetric-sign cases remain green. `FLOOR` and the
    `.MATH`/`.PRECISE` implementations are unchanged.
  - The complete function/core suites pass (22 function integration tests, 73
    core unit tests, every integration suite, and 103 core doctests); strict
    all-target Clippy for both crates is warning-free.
  - Exact `just check` passes formatting, workspace all-target strict Clippy,
    workspace tests and doctests, repository policy checks, and warning-free
    rustdoc.
  - The optional LibreOffice cross-check skipped because no LibreOffice
    converter is installed; no runtime or test dependency was introduced.
  - The zero result follows the independently verified Excel audit case;
    retained nonzero behavior follows Microsoft's [`CEILING`](https://support.microsoft.com/en-us/excel/functions/ceiling-function)
    documentation.
- `cargo deny check` — pass (advisories/bans/licenses/sources ok)
- `cargo +nightly fuzz run fn_eager -- -runs=10000` — pass, 10,000 executions with no crashes; the target honors every eager function's declared minimum arity.
- `cargo test -p omacell-core --release --test recalc determinism_200k -- --ignored` — **ok, 2.00 s**
- Catalog: **156** specs in `all_specs()` / `functions.json` (67 math + 48 statistical + 11 logical + 17 information + 10 aggregate + 2 probes `NOW`/`SEQUENCE` + `ISOMITTED` catalog). Compatibility aliases: `MODE`, `STDEV`, `STDEVP`, `VAR`, `VARP`, `RANK`, `PERCENTILE`, `PERCENTRANK`, `QUARTILE`, `COVAR`, `FORECAST` (11 extra registry names).
- Corpus: **155** TSV files, **1579** data rows; `crates/fn/tests/corpus.rs` all pass. Owned functions each have ≥10 rows (`NOW` has no TSV — not owned).
- `scripts/lo-crosscheck.py` — LibreOffice 26.x via `soffice`: **1549 evaluated, 166 known difference(s), 0 unexplained**.
- Focused review cross-check (`AGGREGATE`, `COUNTIF`, `GCD`, `LCM`): **45 evaluated, 5 known, 0 unexplained**.
- Criterion `--quick --save-baseline wp05a` (`crates/fn/benches/aggregates.rs`, 10k occupied rows, whole column):
  - `whole_column_sum` **8.89 ms**
  - `whole_column_sumifs` **81.5 ms**
  - `whole_column_subtotal` **8.84 ms**
- WP-04 100k incremental / 1M full gates were not re-timed this pass beyond the 200k determinism test; typical 100k incremental remains the WP-05F baseline (~9–10 ms, gate 50 ms).

## Open questions / decisions needed

1. **Resolved 2026-09-01:** `CELL` without a reference uses the last changed cell
   retained by the live `RecalcEngine`; direct evaluation without a session
   deliberately falls back to the formula cell.
2. **Resolved 2026-08-31:** Excel 2010+ asymmetric sign handling
   (`CEILING(-2.5,2)=-2`, `FLOOR(-2.5,2)=-4`; a positive number with negative
   significance is `#NUM!`).
3. **Resolved in the P1 fidelity follow-up:** `SUMIF(S)`, `COUNTIF(S)`,
   `AVERAGEIF(S)`, `MAXIFS`, and `MINIFS` require reference-valued range
   arguments and reject array constants with `#VALUE!`.
4. **Resolved 2026-09-03:** AutoFilter-excluded rows and manually hidden rows
   have distinct `SUBTOTAL` semantics. All function numbers exclude filtered
   rows; only 101–111 additionally exclude manual hiding.
5. **Resolved 2026-09-03:** Explicit SUMIF/AVERAGEIF value ranges follow
   Excel's top-left resizing rule. Their effective range, rather than the
   written shape, is used for values, ordering, circular checks, and dirty
   propagation.
6. **Resolved 2026-09-03:** An array logical test makes `IF` select branch
   values cell by cell, with scalar/singleton broadcasting and per-cell errors.
   A branch that no condition cell selects remains unevaluated.
7. **Resolved 2026-09-03:** `AND`, `OR`, and `XOR` ignore text and empty cells
   inside arrays/references, but retain direct-text errors, aggregate error
   propagation, and `#VALUE!` when no usable value remains.
8. **Resolved 2026-09-03:** `SUMPRODUCT` requires identical row/column shapes,
   not only equal cell counts, and treats Boolean/text/empty entries as zero.
9. **Resolved 2026-09-03:** `PERCENTRANK.INC` and `PERCENTRANK.EXC` truncate,
   rather than round, to the requested significance and reject significance
   below one with `#NUM!`.
10. **Resolved 2026-09-03:** explicitly omitted numeric aggregate arguments
    contribute zero, while empty cells reached through arrays or references
    remain ignored.
11. **Resolved 2026-09-03:** exclusive-percentile ranks exactly one and n are
    valid endpoints; only ranks below one or above n are rejected.
12. **Resolved 2026-09-03:** legacy `CEILING` returns numeric zero when its
    significance is zero, for positive and negative numbers alike.

## RFC (only if a frozen contract changed)

None. WP-01 types unchanged.

The empty/filter aggregate follow-up adds one method to the post-WP-01
`EvalCtx` function-runtime interface and changes no frozen contract.

The SUMIF value-range follow-up changes no public signature or frozen contract.

The array-valued `IF` follow-up changes no public signature or frozen contract.

The logical aggregate text follow-up changes no public signature or frozen
contract.

The `SUMPRODUCT` follow-up changes no public signature or frozen contract.

The `PERCENTRANK` follow-up changes no public signature or frozen contract.

The explicit omitted numeric argument follow-up changes no public signature or
frozen contract.

The legacy `CEILING` zero-significance follow-up changes no public signature or
frozen contract.

The exclusive-percentile endpoint follow-up changes no public signature or
frozen contract.

## Checklist

- [x] `just check` green
- [x] Every acceptance criterion ticked with evidence (corpus ≥10 rows; catalog; fuzz smoke; LO 0 unexplained; lazy/hidden/random/whole-column/criteria tests; 200k determinism + aggregate baselines)
- [x] Docs warning-free; public items documented
- [x] Baselines recorded (`fn_aggregates` Criterion `--quick --save-baseline wp05a`)
- [x] No new `TODO(` without a `WP-` reference; no new runtime dependency (`criterion` already a workspace dev-dep)
- [x] Nothing written outside the repository except documented temp dirs
