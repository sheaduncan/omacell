# WP-S1 — Spike: build the engine or adopt IronCalc (ADR-002)

| | |
|---|---|
| Phase | 0 — Foundations |
| Lane | A — Engine / core |
| Size | S (≈ 1–2 sessions) |
| Depends on | WP-00 |
| Unblocks | — |
| Spec sections | §3.3, §11.2 ADR-002, §11.3, §12.1 |
| Where | throwaway branch `spike/ironcalc` |

## Goal

Decide, within a fixed budget, whether `omacell-core` is built from the §11.3 plan or layered on IronCalc. The rest of the plan assumes **build**; this spike exists to overturn that cheaply if the evidence is strong.

## Deliverables

- A throwaway branch that loads a generated 100k-formula and 1M-formula workbook through IronCalc, edits one input cell, and measures incremental and full recalc.
- `docs/adr/0002-engine.md` with the rubric filled in: license compatibility with `deny.toml`; `.xlsx` read/write coverage against fidelity levels L1/L2; dynamic arrays, spill, `LET`/`LAMBDA` support; range-aware dependency graph behavior (does `SUM(A:A)` create one edge or a million?); fit to the WP-01 contracts; upstreaming path and maintainer responsiveness; binary size; async-node hook feasibility (§8.3).
- A recommendation with the concrete cost of each path in work-package terms.

## Implementation notes

- Time-box: two agent sessions. If inconclusive, the decision is **build**.
- If the decision is **adopt**, write replacement packages WP-02A/03A/04A (adapters over IronCalc) before any Phase 1 work starts, and mark WP-02, WP-03, WP-04 superseded in `PLAN.md`.

## Acceptance criteria

- [ ] ADR merged with an explicit decision and measured numbers.
- [ ] Spike code is deleted or moved under `spikes/` (excluded from the workspace).

## Tests

- None beyond the measurements recorded in the ADR.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-S1.md` before writing code.
4. Create branch `wp/s1-spike-engine`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-S1: Spike: build the engine or adopt IronCalc (ADR-002)`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
