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
  - **CELL subset:** `address`, `col`, `row`, `contents`, `type`, `format` only. Omitted reference = the formula cell (Excel’s “last changed cell” is not modeled).
  - **Criteria:** comparison prefixes, `* ? ~` wildcards, numeric/text/bool/blank matching per Excel; `*IFS` ranges must be the same height/width.
- Open questions at planning time:
  1. Excel desktop `CELL` without a reference tracks the last edited cell; we use the formula cell. Confirm for later UI packages.
  2. Legacy `CEILING`/`FLOOR` sign rules differ across Excel versions; corpus cites 365-style and LibreOffice disagreements go to known-differences.
  3. `MODE.MULT` / `FREQUENCY` spill arrays; 1×1 collapse follows WP-05F `RuntimeValue::array`.

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

Key tests: `crates/fn/tests/corpus.rs`, `integration.rs` (`lazy_if_family_*`, `and_or_do_not_short_circuit`, `hidden_row_subtotal_*`, `nested_subtotal_is_ignored`, `random_is_deterministic_across_thread_counts`, `whole_column_sum_sumifs_subtotal`, `criteria_wildcards_on_sheet_ranges`, `fuzz_smoke_eager_functions_do_not_panic`).

## Interfaces exposed (for dependents)

| Item | Where |
|---|---|
| `register_math` / `register_stat` / `register_logical` / `register_info` / `register_aggregate` / `register_all` | `omacell_fn` |
| `all_specs()`, `functions_json()` (includes `ISOMITTED`) | same |
| `EvalCtx` sheet callbacks listed above | `omacell_core::eval` |
| `Workbook::set_row_hidden` | `omacell_core::workbook` |
| Catalog schema | unchanged, `docs/schemas/functions.schema.json` (`schema: 1`) |

**WP-05b/c:** do not reformat unrelated modules; keep `NOW`/`SEQUENCE` probes until you replace them; append known-differences rows only.

Frozen WP-01 types unchanged.

## Deviations from the spec or the package (with reasons)

- **`PEARSON`** is a full catalog spec sharing `CORREL`'s body (not only an alias), so `functions.json` documents it independently.
- **`CELL` omitted reference** uses the formula cell, not Excel desktop's last-edited cell.
- **`PERMUTATIONA(0,0)`** returns `1` (empty product); Excel `#NUM!`.
- **`*IF` array constants** are walked like ranges so one-cell corpora can cover criteria; Excel often `#VALUE!`.
- **Spilled arrays** in `format_cell` show the origin scalar; corpus expected values follow that (1×1 `MODE.MULT` collapses via `RuntimeValue::array`).
- **LibreOffice headless** disagrees on importer names, CSV error tokens, `TYPE(TRUE)`, and some array-logical aggregates; tagged `known difference` in corpus notes (166 rows) and summarised in `docs/compat/known-differences.md`.

## Measurements

Host: rustc 1.98.0, Linux.

- `just check` — pass
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — pass
- `cargo deny check` — pass (advisories/bans/licenses/sources ok)
- `cargo test -p omacell-core --release --test recalc determinism_200k -- --ignored` — **ok, 2.00 s**
- Catalog: **156** specs in `all_specs()` / `functions.json` (67 math + 48 statistical + 11 logical + 17 information + 10 aggregate + 2 probes `NOW`/`SEQUENCE` + `ISOMITTED` catalog). Compatibility aliases: `MODE`, `STDEV`, `STDEVP`, `VAR`, `VARP`, `RANK`, `PERCENTILE`, `PERCENTRANK`, `QUARTILE`, `COVAR`, `FORECAST` (11 extra registry names).
- Corpus: **155** TSV files, **1549** data rows; `crates/fn/tests/corpus.rs` all pass. Owned functions each have ≥10 rows (`NOW` has no TSV — not owned).
- `scripts/lo-crosscheck.py` — LibreOffice 26.x via `soffice`: **1549 evaluated, 166 known difference(s), 0 unexplained**.
- Criterion `--quick --save-baseline wp05a` (`crates/fn/benches/aggregates.rs`, 10k occupied rows, whole column):
  - `whole_column_sum` **8.89 ms**
  - `whole_column_sumifs` **81.5 ms**
  - `whole_column_subtotal` **8.84 ms**
- WP-04 100k incremental / 1M full gates were not re-timed this pass beyond the 200k determinism test; typical 100k incremental remains the WP-05F baseline (~9–10 ms, gate 50 ms).

## Open questions / decisions needed

1. Excel desktop `CELL` without a reference tracks the last edited cell; we use the formula cell. Confirm when UI editing exists.
2. Legacy `CEILING`/`FLOOR` opposite-sign behaviour: we return `#NUM!` (older Excel); Excel 365 / LibreOffice may return a signed ceiling. Corpus + known-differences.
3. Whether `*IF` should reject array constants (`#VALUE!`) to match Excel strictly, or keep array walking for dynamic-array compatibility.

## RFC (only if a frozen contract changed)

None. WP-01 types unchanged.

## Checklist

- [x] `just check` green
- [x] Every acceptance criterion ticked with evidence (corpus ≥10 rows; catalog; fuzz smoke; LO 0 unexplained; lazy/hidden/random/whole-column/criteria tests; 200k determinism + aggregate baselines)
- [x] Docs warning-free; public items documented
- [x] Baselines recorded (`fn_aggregates` Criterion `--quick --save-baseline wp05a`)
- [x] No new `TODO(` without a `WP-` reference; no new runtime dependency (`criterion` already a workspace dev-dep)
- [x] Nothing written outside the repository except documented temp dirs

