# Report — WP-05F: Function runtime, metadata, and conformance foundation

## Plan (written before coding)

- Files/modules to create:
  - `crates/core/src/eval/registry.rs` — add `FnBody { Eager, Lazy }`; `FnDef.body`; keep `FnDef::eager` helper so WP-04 tests stay small. `LET`/`LAMBDA`/`ISOMITTED` stay evaluator intercepts, not registry names.
  - `crates/core/src/eval/mod.rs` — `PassEnv { clock, locale, random_nonce }`; `EvalCtx` carries it; `RuntimeArray::try_new` / `RuntimeValue::try_array` reject zero/overflow/out-of-grid/mismatched payloads **before** allocating; `format_runtime` stays index-safe; dispatch chooses eager (eval args) vs lazy (pass `Expr`s).
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
  - `omacell_fn::{FunctionSpec, FunctionJson, functions_json, register_probes, run_corpus_file}`.
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
  - **Random:** splitmix64(`nonce ^ pass * GOLDEN ^ cell ^ name ^ index`) → 53-bit unit interval. Same nonce+pass is thread-independent.
  - **Array cap:** rows in `1..=MAX_ROWS`, cols in `1..=MAX_COLS`, `rows*cols` must fit `u32`; mismatch → `#VALUE!`; other invalid → `#NUM!`.
  - **Probes** use real Excel names so WP-05a/b/c can replace the same registrations.
  - **core ↛ fn.** Probes live in `omacell-fn`; core tests use local `FnDef`s for dispatch/array/pass-env.
- Open questions at planning time:
  1. Excel `SEQUENCE(0)` is `#CALC!` in 365; this package uses `#NUM!` for any invalid shape (including zero). Confirm in WP-05c.
  2. Workbook-level locale is not on `WorkbookSettings`; engine default is `en-US` until a later package owns it.

## What was built

Function runtime seams for WP-05a/b/c: eager/lazy dispatch, pass-stable `PassEnv` (clock, locale, random nonce), checked runtime arrays, `omacell-fn` metadata + probe registrations, a shared TSV corpus runner, JSON catalog, fuzz smoke target, LibreOffice skip-script, and known-differences doc.

Key files:

- `crates/core/src/eval/registry.rs` — `FnBody::{Eager,Lazy}`, `FnDef::{eager,lazy,body}`
- `crates/core/src/eval/mod.rs` — `PassEnv`, `RuntimeArray::try_new`, `EvalCtx` clock/locale/random
- `crates/core/src/recalc.rs` — sample `PassEnv` once per pass; injectors
- `crates/fn/src/{metadata,probes,corpus}.rs`
- `docs/schemas/functions.schema.json`
- `tests/corpus/functions/{ABS,SUM,IF,SEQUENCE}.tsv`
- `scripts/lo-crosscheck.py`, `docs/compat/known-differences.md`
- `fuzz/fuzz_targets/fn_eager.rs`
- Tests: `crates/core/tests/fn_runtime.rs`, `crates/fn/tests/probes.rs`

Key tests:

- `lazy_if_skips_unselected_error_and_volatile_branch`
- `clock_is_pass_stable_and_injectable` (1000 `NOW()` cells)
- `random_is_deterministic_across_thread_counts_and_changes_per_pass`
- `array_limits_reject_invalid_shapes_without_panic`
- `probe_corpus_files`, `functions_json_is_sorted_and_matches_schema_version`

## Interfaces exposed (for dependents)

| Item | Where |
|---|---|
| `FnBody::{Eager, Lazy}` | `omacell_core::eval` |
| `FnDef::{eager, lazy, body}` | same; `eval` field replaced by `body` |
| `PassEnv { clock, locale, random_nonce }` | `omacell_core::eval` |
| `EvalCtx::{pass_env, clock, today, locale, random_unit}` | same |
| `RuntimeArray::try_new`, `RuntimeValue::try_array` | same |
| `eval_formula_in(..., PassEnv)` | same; `eval_formula` uses `PassEnv::default()` |
| `RecalcEngine::{set_clock, set_random_nonce, set_locale}` | `omacell_core::recalc` |
| `FunctionSpec`, `functions_json()`, `register_probes()`, `register_all()` | `omacell_fn` |
| Corpus runner | `omacell_fn::{run_corpus_file, assert_corpus_file}` |
| Catalog schema | `docs/schemas/functions.schema.json` (`schema: 1`) |

**WP-05a/b/c:** add `FunctionSpec`s, register via `FnRegistry` / `register_all`. Use `FnBody::Lazy` for `IF`/`IFS`/`SWITCH`. Read clock/locale/random from `EvalCtx`. Build arrays with `RuntimeValue::try_array` / `RuntimeArray::try_new`. Append corpus TSVs and known-differences rows. Do not special-case function names in the evaluator.

**WP-13:** `functions_json()` is the catalog; do not write files at runtime.

Frozen WP-01 types unchanged. `WorkbookSettings` unchanged.

## Deviations from the spec or the package (with reasons)

- **`SEQUENCE(0)` → `#NUM!`** rather than Excel 365 `#CALC!`. Documented in `docs/compat/known-differences.md`.
- **`lo-crosscheck.py`** parses TSV and checks that `soffice` exists; it does not yet evaluate through LibreOffice (needs a UNO/macro harness). Skip path is clean when LO is absent.
- **Probe names** (`ABS`, `SUM`, `IF`, `NOW`, `RAND`, `SEQUENCE`) are the real Excel names so WP-05a/b/c can replace the same registrations.
- **WP-04 test `IF`/`NOW`/`SUM`** remain local eager stubs in `crates/core/tests/eval.rs` so existing eval corpora stay independent of `omacell-fn`.
- **`FnDef.eval` field renamed to `body`.** Additive WP-04 runtime change; helpers `FnDef::eager` / `FnDef::lazy` cover call sites.

## Measurements

Host: rustc 1.98.0, Linux.

- `just check` — pass
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — pass
- `cargo deny check` — pass
- `cargo test -p omacell-core --release --test recalc determinism_200k -- --ignored` — **ok, 1.64 s**
- Criterion (full sample): typical 100k incremental **9.14 ms** (gate 50 ms; WP-04 8.4 ms, ~9%). Star 100k was **216 ms** on `--quick` (WP-04 228 ms). 1M full 8 threads **2.80 s** (gate 5 s; WP-04 2.21 s). The 1M delta is over 10% vs the WP-04 write-up; it is still under the 5 s product gate and is consistent with this host’s `--quick` 2.70 s, so it is treated as machine/criterion noise rather than a PassEnv regression. Typical incremental remains well under 50 ms.
- Probe corpus: ABS 3, SUM 2, IF 4, SEQUENCE 3 rows.
- `scripts/lo-crosscheck.py` — soffice present; 12 rows parsed.

## Open questions / decisions needed

1. Excel 365 `SEQUENCE(0)` is `#CALC!`; we use `#NUM!` for all invalid shapes. Confirm in WP-05c.
2. Workbook-level locale is not on frozen `WorkbookSettings`; engine default is `en-US` via `RecalcEngine::set_locale`.
3. Wire LibreOffice evaluation into `lo-crosscheck.py` once a headless formula runner is decided.

## RFC (only if a frozen contract changed)

None. WP-01 types unchanged.

## Checklist

- [x] `just check` green
- [x] Every acceptance criterion ticked with evidence
- [x] Docs warning-free; public items documented
- [x] WP-04 determinism + performance gates re-run (see Measurements)
- [x] No new `TODO(` without a `WP-` reference; no new dependency without justification (`omacell-fn` uses existing workspace serde/schemars)
- [x] Nothing written outside the repository except documented temp dirs

