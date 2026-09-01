# Report — WP-05F: Function runtime, metadata, and conformance foundation

## Plan (written before coding)

- Files/modules to create:
  - `crates/core/src/eval/registry.rs` — add `FnBody { Eager, Lazy }`; `FnDef.body`; keep `FnDef::eager` helper so WP-04 tests stay small. `LET`/`LAMBDA`/`ISOMITTED` stay evaluator intercepts, not registry names.
  - `crates/core/src/eval/mod.rs` — `PassEnv { clock, locale, random_nonce }`; `EvalCtx` carries it; `RuntimeArray::checked_len` / `try_new` / `validate` and `RuntimeValue::try_array` reject zero/overflow/out-of-grid/oversized/mismatched payloads before allocating or iterating; `format_runtime` stays bounded; dispatch chooses eager (eval args) vs lazy (pass `Expr`s).
  - `crates/core/src/recalc.rs` — sample `PassEnv` once per `run()` before parallel eval; `set_clock` / `set_random_nonce` / `set_locale` injectors; mix `pass` into the random stream so a new pass changes `RAND` with a locked nonce.
  - `crates/fn` — `FunctionSpec`, `define_fn!` macro, `to_fn_def()`, `register_probes()`, `functions_json()`.
  - Probe impls in `crates/fn/src/probes.rs`: `ABS` (eager scalar lift), `SUM` (range aggregate), `IF` (lazy branch), `NOW` (volatile clock), `RAND` (deterministic random), `SEQUENCE` (bounded array).
  - `docs/schemas/functions.schema.json` (schema version 1).
  - Shared corpus runner `crates/fn/src/corpus.rs` + `tests/corpus/functions/<NAME>.tsv`.
  - `scripts/lo-crosscheck.py`, `docs/compat/known-differences.md`.
  - Fuzz target `fuzz/fuzz_targets/fn_eager.rs` with strict ArgVal size caps.
  - Tests: `crates/core/tests/fn_runtime.rs`, `crates/fn/tests/metadata.rs`, `crates/fn/tests/probes.rs`.
- Interfaces to expose (types, commands, schemas, CLI):
  - `omacell_core::eval::{FnBody, PassEnv, RuntimeArray::try_new}`.
  - `EvalCtx::{pass_env, clock, today, locale, random_unit}`.
  - `RecalcEngine::{set_clock, set_random_nonce, set_locale, pass_env}`.
  - `omacell_fn::{define_fn!, FunctionSpec, FunctionJson, functions_json, register_probes, run_corpus_file}`.
  - No CLI. No WP-01 type changes.
- Tests and corpora to write first:
  - Lazy `IF`: unselected `1/0` / volatile / async not evaluated; selected errors propagate.
  - Clock: ≥1000 `NOW()` cells identical; injected clock is deterministic.
  - Random: 1 vs 8 threads bit-identical for locked nonce; different cells/indices differ; new pass changes values.
  - Array limits: 0, overflow, `MAX_ROWS+1`, payload mismatch → Excel error, no panic.
  - `functions_json()` sorted + schema.
  - Probe TSV self-test.
- Items the package says to "decide and document" and the decision taken:
  - **Dispatch:** `FnBody::Eager | Lazy` on `FnDef`. No `IF`-name special case in the evaluator.
  - **Pass context:** `PassEnv` on `EvalCtx` / sampled in `RecalcEngine::run`. Not on frozen `WorkbookSettings`.
  - **Clock:** Excel 1900 serial `f64` (date + time fraction). `NOW` returns it; `TODAY` is `clock.trunc()` (probe is `NOW` only).
  - **Random:** sequential splitmix64 domain mixing of nonce, pass, sheet, row, column, deterministic AST call path, array index, and function bytes → 53-bit unit interval. Components are not XOR-packed into overlapping bit ranges, and repeated `RAND()` calls in one formula remain distinct without a scheduling-sensitive counter. Same nonce+pass is thread-independent.
  - **Array cap:** rows in `1..=MAX_ROWS`, cols in `1..=MAX_COLS`, and at most 16,777,216 transient cells; mismatch → `#VALUE!`; other invalid → `#NUM!`.
  - **Probes** use real Excel names so WP-05a/b/c can replace the same registrations.
  - **core ↛ fn.** Probes live in `omacell-fn`; core tests use local `FnDef`s for dispatch/array/pass-env.
- Open questions at planning time:
  1. Excel `SEQUENCE(0)` is `#CALC!` in 365; this package uses `#NUM!` for any invalid shape (including zero). Confirm in WP-05c.
  2. Workbook-level locale is not on `WorkbookSettings`; engine default is `en-US` until a later package owns it.

## What was built

Function runtime seams for WP-05a/b/c: eager/lazy dispatch, pass-stable `PassEnv` (clock, locale, random nonce), checked and bounded runtime arrays, `omacell-fn` metadata + probe registrations, a strict shared TSV corpus runner, schema-validated JSON catalog, fuzz smoke target, real headless LibreOffice cross-check, and known-differences doc.

Key files:

- `crates/core/src/eval/registry.rs` — `FnBody::{Eager,Lazy}`, `FnDef::{eager,lazy,body}`
- `crates/core/src/eval/mod.rs` — `PassEnv`, checked/bounded `RuntimeArray`, collision-resistant deterministic random mixing, `EvalCtx` clock/locale/random
- `crates/core/src/recalc.rs` — sample `PassEnv` once per pass; injectors
- `crates/fn/src/{metadata,probes,corpus}.rs`
- `docs/schemas/functions.schema.json`
- `tests/corpus/functions/{ABS,SUM,IF,SEQUENCE}.tsv`
- `scripts/lo-crosscheck.py`, `docs/compat/known-differences.md`
- `fuzz/fuzz_targets/fn_eager.rs`
- Tests: `crates/core/tests/fn_runtime.rs`, `crates/fn/tests/probes.rs`

Key tests:

- `lazy_if_skips_unselected_error_and_volatile_branch` (including an unselected async node)
- `clock_is_pass_stable_and_injectable` (1000 `NOW()` cells)
- `random_is_deterministic_across_thread_counts_and_changes_per_pass`, including the former row/column collision pair and repeated calls in one formula; distinct array-index streams are checked separately
- `array_limits_reject_invalid_shapes_without_panic`, `malformed_function_array_is_rejected_before_spill_iteration`
- `probe_corpus_files`, `functions_json_is_sorted_and_matches_schema_version`

## Interfaces exposed (for dependents)

| Item | Where |
|---|---|
| `FnBody::{Eager, Lazy}` | `omacell_core::eval` |
| `FnDef::{eager, lazy, body}` | same; `eval` field replaced by `body` |
| `PassEnv { clock, locale, random_nonce }` | `omacell_core::eval` |
| `EvalCtx::{pass_env, clock, today, locale, random_unit}` | same |
| `RuntimeArray::{checked_len, try_new, validate}`, `MAX_RUNTIME_ARRAY_CELLS`, `RuntimeValue::try_array` | same |
| `eval_formula_in(..., PassEnv)` | same; `eval_formula` uses `PassEnv::default()` |
| `RecalcEngine::{set_clock, set_random_nonce, set_locale}` | `omacell_core::recalc` |
| `define_fn!`, `FunctionSpec`, fallible `functions_json()`, `register_probes()`, `register_all()` | `omacell_fn` |
| Strict corpus runner | `omacell_fn::run_corpus_file` |
| Catalog schema | `docs/schemas/functions.schema.json` (`schema: 1`) |

**WP-05a/b/c:** add `FunctionSpec`s, register via `FnRegistry` / `register_all`. Use `FnBody::Lazy` for `IF`/`IFS`/`SWITCH`. Read clock/locale/random from `EvalCtx`. Build arrays with `RuntimeValue::try_array` / `RuntimeArray::try_new`. Append corpus TSVs and known-differences rows. Do not special-case function names in the evaluator.

**WP-13:** `functions_json()` is the catalog; do not write files at runtime.

Frozen WP-01 types unchanged. `WorkbookSettings` unchanged.

## Deviations from the spec or the package (with reasons)

- **Resolved 2026-08-31:** zero-sized `SEQUENCE` results return `#CALC!`; negative and oversized dimensions return `#NUM!` before allocation.
- **Transient array limit:** one evaluator array is capped at 16,777,216 cells to bound allocation and iteration even though the worksheet grid is larger. Range aggregates retain references and do not materialize ordinary full-column inputs.
- **Probe names** (`ABS`, `SUM`, `IF`, `NOW`, `RAND`, `SEQUENCE`) are the real Excel names so WP-05a/b/c can replace the same registrations.
- **WP-04 test `IF`/`NOW`/`SUM`** remain local eager stubs in `crates/core/tests/eval.rs` so existing eval corpora stay independent of `omacell-fn`.
- **`FnDef.eval` field renamed to `body`.** Additive WP-04 runtime change; helpers `FnDef::eager` / `FnDef::lazy` cover call sites.

## Measurements

Host: rustc 1.98.0, Linux.

- `just check` — pass
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — pass
- `cargo deny check` — pass
- `cargo test -p omacell-core --release --test recalc determinism_200k -- --ignored` — **ok, 1.67 s**
- `ASAN_OPTIONS=detect_leaks=0:detect_odr_violation=0 cargo +nightly fuzz run fn_eager -- -runs=10000` — pass (leak detection disabled because this sandbox blocks LeakSanitizer's ptrace requirement)
- Criterion: original full-sample typical 100k incremental **9.14 ms** (gate 50 ms) and star 100k **216 ms** on `--quick` (WP-04 report: 228 ms). Review reran paired `--quick` comparisons on the same host and separate build trees: typical 100k **9.65 ms** vs WP-04 **10.17 ms** (~5% faster); 1M full 8 threads **2.87 s** vs WP-04 **2.82 s** (~2% slower). Both deltas are within the 10% review threshold and the 50 ms / 5 s product gates.
- Probe corpus: ABS 3, SUM 2, IF 4, SEQUENCE 3 rows.
- `scripts/lo-crosscheck.py` — LibreOffice 26.2 evaluated all 12 rows: zero unexplained differences and two documented invalid-`SEQUENCE` differences.

## Open questions / decisions needed

1. Resolved 2026-08-31: zero-sized `SEQUENCE` results use `#CALC!`.
2. **Resolved:** locale remains application-level, not workbook state; formats
   stay locale-independent and `RecalcEngine::set_locale` supplies the runtime
   locale.

## RFC (only if a frozen contract changed)

None. WP-01 types unchanged.

## Checklist

- [x] `just check` green
- [x] Every acceptance criterion ticked with evidence
- [x] Docs warning-free; public items documented
- [x] WP-04 determinism + performance gates re-run (see Measurements)
- [x] No new `TODO(` without a `WP-` reference; no new dependency without justification (`omacell-fn` uses existing workspace serde/schemars)
- [x] Nothing written outside the repository except documented temp dirs
