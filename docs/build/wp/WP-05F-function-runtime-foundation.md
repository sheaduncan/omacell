# WP-05F — Function runtime, metadata, and conformance foundation

| | |
|---|---|
| Phase | 1 — Engine |
| Lane | A — Engine / core |
| Size | M (≈ 3–5) |
| Depends on | WP-04, WP-06 |
| Unblocks | WP-05a, WP-05b, WP-05c |
| Spec sections | §6.3 F-3.3–F-3.7, §6.4, §11.3, §11.5, §12.1, §14 |
| Where | `crates/core` (`eval`, `recalc`), `crates/fn` (`registry`, `metadata`, test support) |

## Goal

Freeze the runtime and metadata seams required by every Tier-0 function package before hundreds of functions depend on them. This package adds infrastructure and representative probes, not the Tier-0 library itself.

## Deliverables

- Function dispatch supports both:
  - eager implementations receiving evaluated `ArgVal`s; and
  - lazy/special-form implementations receiving unevaluated argument expressions and evaluating only the branches they select.
- A pass-stable, test-injectable calculation environment available through `EvalCtx`:
  - one clock sample per recalculation pass for `NOW`/`TODAY`;
  - locale information for `TEXT`, `VALUE`, `DATEVALUE`, and related functions;
  - one random nonce/seed per pass from which volatile random functions derive deterministic values by formula cell, function, and array index.
- Checked runtime-array construction and spill guards. Reject zero, oversized, overflowed, shape/payload-mismatched, or out-of-grid arrays with an Excel error before allocating or iterating unbounded dimensions. Keep formatting and error paths panic-safe for malformed public values.
- `crates/fn` depends on `core` and owns a data-driven definition macro plus richer `FunctionSpec` metadata: canonical name, aliases if any, tier/category, argument kinds, min/max arguments, eager/lazy strategy, volatility, array behavior, signature, and documentation.
- The richer `FunctionSpec` projects to the evaluator's runtime definition; runtime dispatch metadata has one owner and cannot drift from generated documentation.
- Deterministic `functions_json()` output for later CLI/autocomplete consumers, validated against `docs/schemas/functions.schema.json`. The library returns data; it does not write files at runtime.
- Shared function-corpus runner for `tests/corpus/functions/<NAME>.tsv`, fuzz-smoke harness over every registered eager function, `scripts/lo-crosscheck.py`, and `docs/compat/known-differences.md`.
- Representative probe functions/tests only: one eager scalar lift, one range aggregate, one lazy branch, one volatile clock, one deterministic random, and one bounded array producer. WP-05a/b/c replace probes with the real registrations.

## Required decisions

- Prefer an explicit eager/lazy implementation enum or equivalent typed dispatch over hard-coding `IF`-family names in the evaluator. `LET`, `LAMBDA`, and `ISOMITTED` remain evaluator language constructs.
- Do not add locale, clock, or random fields to frozen WP-01 `WorkbookSettings`. Put pass context on the WP-04 evaluator/recalc seam and preserve the existing convenience APIs where practical.
- The function macro and metadata live in `omacell-fn`; `omacell-core` must not depend on `omacell-fn`.
- A clock/random source is sampled once before parallel evaluation. Function results may not depend on Rayon scheduling or call order.
- Any change to a WP-01 public type requires an RFC and human approval. Additive changes to WP-04 runtime types must be documented in the report so WP-05a/b/c consume one final interface.

## Acceptance criteria

- [ ] Lazy probe: an unselected error/volatile/async branch is not evaluated; selected-branch errors still propagate in evaluation order.
- [ ] Clock probe: at least 1,000 independent formulas observe the identical pass timestamp; an injected clock makes the corpus deterministic.
- [ ] Random probe: 1-thread and 8-thread runs are bit-identical for the same injected nonce, different cells/array indices do not all collide, and a new pass changes volatile results.
- [ ] Array-limit tests reject oversized and malformed shapes promptly without panic or excessive allocation; valid arrays still spill and lift.
- [ ] `functions_json()` is sorted and stable and validates against a committed versioned schema owned by `crates/fn`.
- [ ] The shared corpus runner, fuzz smoke, LibreOffice cross-check script, and known-differences document are usable by all three WP-05 packages.
- [ ] Existing WP-04 determinism tests and the 100k incremental / 1M full-recalc performance gates remain green; any regression over 10% is explained and corrected before merge.

## Tests

- Focused evaluator/recalc integration tests for lazy dispatch, pass context, deterministic random values, and array limits.
- Snapshot/schema test for sorted `functions_json()`.
- Corpus-runner self-test with the representative probes.
- Fuzz smoke over arbitrary `ArgVal`/runtime-array payloads with strict size caps.
- Existing Criterion recalc benches before and after the runtime change.

## Out of scope

- The full Tier-0 function implementations (WP-05a/b/c).
- CLI commands and autocomplete UI (WP-13/WP-14); this package only exposes deterministic metadata.
- Provider-backed async AI functions (WP-23).

## Procedure

1. Read `AGENTS.md`, this file, and only the listed spec sections.
2. Read the *Interfaces exposed* sections of `reports/WP-04.md` and `reports/WP-06.md`.
3. Write the Plan section of `reports/WP-05F.md`, including the exact dispatch and pass-context types, before code. Stop for approval only if a frozen WP-01 contract would change.
4. Create branch `wp/05f-function-runtime-foundation`.
5. Write the acceptance tests first; implement the runtime, metadata, and harness until they pass.
6. Run `just check`, strict rustdoc, `cargo deny check`, the release determinism test, and both WP-04 performance gates.
7. Complete the report and open `WP-05F: Function runtime, metadata, and conformance foundation`. Do not merge.

## Done when

Every acceptance box is ticked with evidence, CI is green, the report fixes the interfaces consumed by WP-05a/b/c, and no Tier-0 implementation package must redesign the evaluator.
