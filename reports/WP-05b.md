# Report — WP-05b: Functions Tier 0 — text, date, and time

## Plan (written before coding)

### 2026-09-04 text/date parsing follow-up (written before coding)

- Add the reviewed scientific-number, locale-date, omitted-date-part,
  date-with-time, overflowing-time, text-date extractor, identical separator,
  and all-weekend-mask cases to their owning corpora before implementation.
- Keep parsing locale-strict: recognize explicit year-first numeric dates, but
  do not reinterpret an invalid en-US month as day-first. Use the pass-stable
  workbook clock only when a text date omits its year.
- Share date/time text coercion with date extractors, normalize exponent syntax
  without weakening separator validation, reject identical `NUMBERVALUE`
  separators and seven-day weekend masks immediately, then update both reports
  and run the complete function suite, strict Clippy, and exact `just check`.

### 2026-09-04 `YEARFRAC` compatibility follow-up (written before coding)

- Correct the reversed-date corpus and the stale compatibility-table claim
  first, then add the reviewed basis-0 January-31-to-leap-day case and
  symmetry checks across all five bases.
- Keep `DAYS360`'s existing NASD end-of-month behavior unchanged; give
  `YEARFRAC` basis 0 its own Excel-compatible day-count helper and return the
  absolute fraction when the date arguments are reversed.
- Update this report and the authoritative repository-review checklist, run
  the complete function suite and strict Clippy, then run exact `just check`
  before opening the PR.

- Files/modules to create:
  - `crates/fn/src/text/` — Tier-0 text functions (~35) plus `REGEXTEST` / `REGEXEXTRACT` / `REGEXREPLACE` (`regex` crate with pattern/haystack/size/nest limits).
  - `crates/fn/src/datetime.rs` — Tier-0 date/time functions (~25), including replacing the WP-05F `NOW` probe with real `NOW`/`TODAY`.
  - `crates/fn/src/util.rs` — shared argument coercion, error walk, integer truncation, date-system access, 32,767-char cap (not a public API).
  - `crates/fn/src/lib.rs` — **append only**: `mod text; mod datetime;` and `register_text` / `register_datetime` from `register_all`. Do not remove ABS/SUM/IF/RAND/SEQUENCE probes.
  - `crates/fn/src/probes.rs` — remove **only** the `NOW` probe (replaced by datetime). Leave 05a/05c probes untouched.
  - `crates/fn/src/metadata.rs` / `corpus.rs` — `functions_json` and the corpus runner include all currently shipped specs; runner uses `register_all`; TSV allows optional `locale` and `date_system` columns so 3-column 05F/05a/05c files keep parsing.
  - `crates/fn/Cargo.toml` — add pre-approved `regex`.
  - `crates/fn/benches/text_date.rs` — TEXTSPLIT, regex, 100k-row LEN/YEAR scans.
  - `crates/fn/tests/{text_date,probes}.rs` — corpus, locale/date-system matrix, pass-stable clock, regex limits, fuzz smoke over every registered eager function.
  - `tests/corpus/functions/<NAME>.tsv` — ≥ 10 rows per function.
  - `docs/compat/known-differences.md` — **append only**.
  - `fuzz/fuzz_targets/fn_eager.rs` — iterate every eager spec, not only probes.
  - `scripts/lo-crosscheck.py` — `_xlfn.` prefix for post-2007 names so LibreOffice can evaluate them.
- Interfaces to expose (types, commands, schemas, CLI):
  - `omacell_fn::{register_text, register_datetime, TEXT_SPECS, DATETIME_SPECS, all_specs}`.
  - `register_all` = probes (minus NOW) + text + datetime.
  - Catalog via existing `functions_json()` (schema version 1 unchanged).
  - No commands, no CLI, no WP-01 type changes, no `WorkbookSettings` fields.
- Tests and corpora to write first:
  - One TSV per function under `tests/corpus/functions/`, ≥ 10 cited rows covering normal / empty / error / array-lift / Unicode / locale / boundary as applicable.
  - Dedicated 1900 leap-bug and 1904 boundary rows (`DATE`, `DATEVALUE`, `YEAR`/`MONTH`/`DAY`, `WEEKDAY`, `DATEDIF`, `EDATE`/`EOMONTH`).
  - `NOW`/`TODAY` rows against the injected corpus clock (`45000.5` / `45000`).
  - Locale rows for `TEXT`, `VALUE`, `NUMBERVALUE`, `DATEVALUE`, `TIMEVALUE` at `en-US`, `en-GB`, `de-DE`.
  - Astral-character `LEN`/`MID`/`LEFT` rows citing Excel UTF-16 vs Omacell scalar-value behaviour.
  - Unit tests: pass-stable clock, regex size/time rejection, fuzz smoke, locale/date-system matrix.
- Items the package says to "decide and document" and the decision taken:
  - **`LEN`/`MID`/`LEFT`/`RIGHT` and astral characters.** Excel for Windows (including Microsoft 365) counts UTF-16 code units, so a single scalar value in `U+10000..=U+10FFFF` (e.g. `😀` U+1F600) has `LEN` = 2 and `MID` can split a surrogate pair. Spec §6.4 requires Unicode-correct text. Omacell counts Unicode scalar values (`str::chars`): `LEN("😀")` = 1, and `MID`/`LEFT`/`RIGHT` never emit unpaired surrogates. No workbook compatibility flag (frozen `WorkbookSettings`). Recorded in corpus notes and `docs/compat/known-differences.md`. Combining marks are separate scalars (`LEN("é")` of `e` + U+0301 = 2), matching Excel’s code-unit count for BMP combining sequences.
  - **`CHAR`/`CODE` vs `UNICHAR`/`UNICODE`.** `UNICHAR`/`UNICODE` are Unicode scalar values. `CHAR`/`CODE` use Latin-1 code points `1..=255` (not Windows-1252), documented as a known difference for `128..=159`.
  - **`NOW`/`TODAY`.** Read `EvalCtx::clock()` / `today()` only. Never sample the wall clock inside a function. `TODAY` is `clock.trunc()` as defined by WP-05F.
  - **Date system.** Read `ctx.workbook().settings().date_system` (already on frozen settings). Do not add a flag.
  - **Locale.** `TEXT`/`VALUE`/`NUMBERVALUE`/`DATEVALUE`/`TIMEVALUE` use `EvalCtx::locale()` and WP-06 `numfmt` / `LocaleInfo` tables. `NUMBERVALUE` omitted separators fall back to that locale.
  - **Regex limits.** Pattern ≤ 256 chars, haystack ≤ 32,767 chars, `RegexBuilder` `size_limit`/`dfa_size_limit` 1 MiB, `nest_limit` 32. Compile or match failure → `#VALUE!`. The `regex` crate is linear-time; the “time limit” is the size bound, plus a unit test that an oversized pattern returns an error rather than hanging.
  - **Result length.** Concatenation / `REPT` / regex-replace results longer than 32,767 characters → `#VALUE!` (Excel cell cap).
  - **`TRIM`.** ASCII space `U+0020` only (Excel), not Unicode whitespace.
  - **Inapplicable corpus categories.** Scalar-only functions omit array-return rows; nullary `NOW`/`TODAY` omit empty-arg rows; documented in each function’s rustdoc / this report rather than changing `functions.schema.json`.
- Open questions at planning time:
  1. Excel `DATEDIF` `"MD"` / `"YD"` leap-year bugs — match Excel where a cited corpus row exists; otherwise record LO/Excel disagreement as a known difference.
  2. Excel `YEARFRAC` basis 1 (actual/actual) across year boundaries has multiple published algorithms. Implement the commonly cited Excel algorithm, corpus-cite it, triage LO mismatches.
  3. Live Excel confirmation of `CHAR(128)` (Windows-1252 euro vs Latin-1 C1). Plan: Latin-1, documented.
  4. `TEXTSPLIT` of empty text and `CONCAT` of a whole-column reference — confirm LO/Excel if the runner cannot host a full column; corpus uses small arrays.

## What was built

Tier-0 text (~35 + 3 regex) and date/time (~25) functions in `omacell-fn`, using WP-05F `define_fn!`, pass-stable `EvalCtx` clock/locale, WP-06 `numfmt`/`dates`, and checked `RuntimeValue::array`.

`NOW` moved out of the WP-05F probe set onto the real datetime implementation (`EvalCtx::clock()`). `TODAY` uses `EvalCtx::today()`. ABS/SUM/IF/RAND/SEQUENCE probes are unchanged.

The shared corpus runner now calls `register_all`, accepts optional `locale` / `date_system` columns, preserves significant spaces in the expected column (`CHAR(32)`), and formats spilled arrays as `{a,b;c,d}` so TEXTSPLIT and lift results are comparable.

Key files:

- `crates/fn/src/text/{mod,parse,split,regex_fns,format}.rs`
- `crates/fn/src/datetime.rs`, `crates/fn/src/util.rs`
- `crates/fn/src/{lib,probes,metadata,corpus}.rs`
- `tests/corpus/functions/<NAME>.tsv` (60 new files, ≥ 10 rows each)
- `crates/fn/tests/text_date.rs`, `crates/fn/benches/text_date.rs`
- `docs/compat/known-differences.md` (appended)
- `fuzz/fuzz_targets/fn_eager.rs`, `scripts/lo-crosscheck.py`

Key tests:

- `text_and_date_corpus_files` — 734 rows across 64 TSVs
- `locale_matrix_text_value_datevalue` — en-US / en-GB / de-DE
- `now_and_today_are_pass_stable` — 64 cells, injected clock
- `lotus_leap_date_parts` — serials 60 / 61
- `regex_oversized_pattern_is_value_error`
- `eager_functions_do_not_panic_on_random_args`

Review hardening enforces the 32,767-character limit while building `SUBSTITUTE`, joined text, regex replacements, and `ARRAYTOTEXT`, rather than after potentially unbounded allocation. Regex replacement expansion is capture-aware and capped incrementally. `TEXTSPLIT` and lifted `WORKDAY` validate output shapes before allocation; extreme `WORKDAY`, `REPLACE`, `FIXED`, and `DOLLAR` arguments fail closed without integer overflow. The fuzz target now honors each function's declared minimum arity.

The 2026-09-04 `YEARFRAC` follow-up separates basis 0 from `DAYS360`'s
February end-of-month adjustment and treats reversed dates symmetrically. The
two functions retain distinct corpus coverage for the January-31-to-leap-day
case: `YEARFRAC` counts 29/360 while `DAYS360` counts 30 days.

The 2026-09-04 parsing follow-up accepts scientific notation, explicit
year-first numeric dates, named dates with omitted parts, and date/time text.
Date extractors now share the locale-aware text-date parser, while time
extractors accept recognized text and preserve Excel's overflowing-hour
normalization. `NUMBERVALUE` rejects identical separators, en-US dates no
longer fall back to day-first order, and `WORKDAY.INTL` rejects an all-weekend
calendar before its bounded search. `NETWORKDAYS.INTL` deliberately retains
Excel's distinct result of zero for that same calendar.

## Interfaces exposed (for dependents)

| Item | Where |
|---|---|
| `TEXT_SPECS` (35) | `omacell_fn` |
| `DATETIME_SPECS` (25) | `omacell_fn` |
| `register_text` / `register_datetime` | `omacell_fn` |
| `all_specs()` / `register_all()` | probes (ABS, SUM, IF, RAND, SEQUENCE) + text + datetime |
| `functions_json()` | schema version 1, sorted by name, now includes text/date |
| Corpus TSV grammar | `formula`, `expected`, `note`, optional `locale`, optional `date_system` |

No commands, schemas, or CLI. Frozen WP-01 types and `WorkbookSettings` unchanged.

**WP-05a/05c:** keep ABS/SUM/IF/RAND/SEQUENCE probes until you replace them. Use `register_all`. Append-only on `known-differences.md`.

**WP-13:** `functions_json()` is the catalog.

## Deviations from the spec or the package (with reasons)

- **Astral `LEN`/`MID`/`LEFT`/`RIGHT`:** Unicode scalar values, not Excel UTF-16 code units. Documented; no settings flag.
- **Resolved 2026-08-31:** `CHAR`/`CODE` use Windows-1252 for bytes 1–255, including C1 controls for the five undefined slots.
- **`WORKDAY`/`WORKDAY.INTL` array lift:** `ArrayBehavior::None` so the holidays argument is a set; start/days are lifted locally (Excel lifts the first two args only).
- **Corpus runner formats spills as `{…}`.** `format_cell` of a spill origin is the top-left scalar (WP-04); the runner reconstructs the spill so TEXTSPLIT/lift rows can assert shape.
- **LibreOffice:** the original run recorded 269 documented mismatches (date
  CSV formatting, no array lift, no REGEX*, and serial 0 epoch). Its reversed
  `YEARFRAC` result actually agrees with Excel; the former signed-result entry
  was a stale oracle and is now marked as a historical correction.

## Measurements

Host: rustc 1.98 / Linux.

- Function catalog: **35 text + 25 date/time + 5 remaining probes = 65** names in `functions_json()`.
- Corpus: **64** TSVs, **734** rows. `cargo test -p omacell-fn --test probes text_and_date_corpus_files` pass. Each WP-05b function has ≥ 10 rows. Review-specific text/date tests: 8 pass.
- `just check` — pass (fmt, clippy `-D warnings`, `cargo test --workspace`, `cargo doc --workspace --no-deps`).
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — pass.
- `cargo deny check` — pass (unused-allowlist warnings only). New dep: `regex` (pre-approved).
- `cargo +nightly fuzz run fn_eager -- -runs=10000` — pass, 10,000 executions with no crashes; the target honors every eager function's declared minimum arity.
- LibreOffice 26.x `scripts/lo-crosscheck.py`: **734 evaluated, 269 known, 0 unexplained**.
- Criterion `--quick --save-baseline wp05b` (`cargo bench -p omacell-fn --bench text_date`):
  - `textsplit_line` **104.7 µs**
  - `regex_1k` **39.7 ms**
  - `len_scan_100k` **297.7 ms**
  - `year_scan_100k` **299.6 ms**
- Post-review Criterion `--quick`: `textsplit_line` **102.6 µs**, `regex_1k` **41.5 ms**, `len_scan_100k` **318.3 ms**, `year_scan_100k` **300.3 ms**. The safety hardening did not materially change the committed baseline profile.
- 2026-09-04 `YEARFRAC` test-first evidence: the corrected reverse-date and
  leap-day matrix produced six corpus mismatches before implementation. All
  five bases are now symmetric, the reviewed basis-0 edge returns 29/360, the
  paired `DAYS360` edge remains 30 days, and the complete `omacell-fn` suite
  plus strict all-target Clippy pass. Microsoft documents basis 0 as US NASD
  30/360 and flags its last-day-of-February behavior in the
  [`YEARFRAC` reference](https://support.microsoft.com/en-us/office/yearfrac-function-3844141e-c76d-4143-82b6-208454ddc6a8).
- 2026-09-04 parsing test-first evidence: 14 focused corpus rows cover the
  reviewed boundaries; 13 exposed old behavior before implementation (the
  locale-invalid en-US date was already rejected), and all now pass. The
  complete `omacell-fn` suite and strict all-target Clippy pass. Microsoft
  documents that [`DATEVALUE`](https://support.microsoft.com/en-us/office/datevalue-function-df8b07d4-7761-4a93-bc33-b7471bbff252)
  uses the current year for an omitted year and ignores time, gives an explicit
  year-first example, and documents that
  [`TIMEVALUE`](https://support.microsoft.com/en-us/office/timevalue-function-0b615c12-33d8-4431-bf3d-f3eb6d186645)
  ignores date information. The
  [`VALUE` reference](https://support.microsoft.com/en-US/Excel/functions/value-function)
  permits recognized number, date, or time formats. Microsoft also documents
  the intentional split between
  [`WORKDAY.INTL`](https://support.microsoft.com/en-us/excel/functions/workday-intl-function),
  where `1111111` is invalid, and
  [`NETWORKDAYS.INTL`](https://support.microsoft.com/en-us/excel/functions/networkdays-intl-function),
  where it returns zero.
- WP-04 recalc benches use an empty `FnRegistry` and were not re-run; this package does not change the `=1+1` formula path.

## Open questions / decisions needed

1. **Resolved 2026-08-31:** `CHAR(128)` is `€`; `CHAR`/`CODE` use Windows-1252.
2. **Resolved 2026-08-31:** retain Excel's documented `DATEDIF` month-end/leap
   quirks; pathological corpus rows cover them.
3. **Resolved 2026-08-31:** `YEARFRAC` basis 1 divides actual days by the
   applicable year length or the average length of all covered years.
4. **Resolved:** spilled cells show their values and the formula bar shows the
   anchor formula; only legacy CSE formulas use `{=…}` notation.
5. **Resolved 2026-09-04:** `YEARFRAC` returns an absolute fraction for
   reversed dates and uses its own basis-0 month-end rules rather than
   `DAYS360`'s February adjustment.
6. **Resolved 2026-09-04:** text dates with an omitted year use the
   pass-stable workbook clock, not a wall-clock read inside the function.

## RFC (only if a frozen contract changed)

None. WP-01 types and `WorkbookSettings` are unchanged.

The `YEARFRAC` compatibility follow-up changes no public signature or frozen
contract.

The parsing compatibility follow-up changes no public signature or frozen
contract.

## Checklist

- [x] `just check` green on a clean clone (local workspace; see Measurements)
- [x] Every acceptance criterion ticked with evidence (see below)
- [x] Docs warning-free (`RUSTDOCFLAGS="-D warnings"`); public items documented
- [x] Baselines recorded (`text_date` criterion numbers above)
- [x] No new `TODO(` without a `WP-` reference; `regex` is pre-approved and `cargo deny` is green
- [x] Nothing written outside the repository except documented temp dirs (`/tmp` used only for LO output and a branch-switch backup)

### Acceptance (WP-05b)

- [x] Every listed function has ≥ 10 corpus rows in `tests/corpus/functions/<NAME>.tsv` covering applicable categories; all pass — 60 new TSVs, 734 rows total, `text_and_date_corpus_files`
- [x] `functions.json` lists every function with signature and doc; no function panics under the fuzz smoke test — `functions_json_is_sorted_and_matches_schema_version`, `eager_functions_do_not_panic_on_random_args`, `fuzz/fuzz_targets/fn_eager.rs` iterates `all_specs()`
- [x] Cross-check script reports zero unexplained differences — 269 known, 0 unexplained
- [x] Date functions pass the 1900 leap-bug and 1904 boundary corpus — `DATE`/`YEAR`/`MONTH`/`DAY`/`EDATE` rows and `lotus_leap_date_parts` / `date_system_1904_date_and_year`
- [x] `NOW`/`TODAY` identical across a pass with the injected clock; locale matrix covers en-US, en-GB, de-DE — `now_and_today_are_pass_stable`, `locale_matrix_text_value_datevalue`, NOW/TODAY TSVs at clock `45000.5`
- [x] TEXTSPLIT, regex, and 100k-row text/date scans have committed baselines (report numbers; criterion baseline `wp05b`)
