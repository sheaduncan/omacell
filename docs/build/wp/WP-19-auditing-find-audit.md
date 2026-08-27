# WP-19 — Auditing, find/replace, Go To Special, and the deterministic `audit`

| | |
|---|---|
| Phase | 4 — Surfaces II — GUI and data tools |
| Lane | A — Engine / core |
| Size | M (≈ 3–5) |
| Depends on | WP-04, WP-17 |
| Unblocks | WP-21, WP-22 |
| Spec sections | §6.3 F-3.8, §6.5 F-5.8, §8.4 A-4.5 (deterministic part), §8.5 A-5.4 |
| Where | `crates/core` (modules `audit`, `find`), `crates/bus`, `crates/cli` (`audit`) |

## Goal

Make formulas explainable and workbooks checkable without a model — and give the AI layer structured findings to build on.

## Deliverables

- Precedents/dependents (direct and transitive) queries; Evaluate-Formula step trace (sub-expression sequence with intermediate values); show-formulas mode support; error explanations (`#NAME?` names the unknown token, `#REF!` names the removed range, `#SPILL!` names the blocker, `#DIV/0!` names the operand).
- Deterministic audit checks with stable ids and JSON output: inconsistent formula in a row/column pattern (R1C1 comparison), hard-coded constants inside formulas, ranges stopping short of the data region, unused defined names, circular reference sets, external links, volatile function counts, merged cells inside tables; each finding with location, severity, and (where safe) a fix command.
- Find/replace: values or formulas, whole cell, case, regex (bounded), scope sheet/workbook, replace preview with counts; Go To by address/name; Go To Special (blanks, constants by type, formulas by result type, errors, visible only, precedents/dependents, conditional formats, validation).
- Diagnostic bundle builder for WP-21 (`omacell agent diagnose`): error explanations, dependency neighborhood, recent undo history, with WP-22 redaction hook applied when present.

## Implementation notes

- Findings must be reproducible and ordered; the AI audit (WP-23) reads this JSON and only adds judgment.

## Acceptance criteria

- [ ] Seeded-defect corpus: 100% precision and recall on deterministic checks; explanations corpus passes.
- [ ] Regex safety: pathological patterns time out cleanly; find/replace preview counts equal applied counts.
- [ ] `omacell audit book.xlsx --json` output validates against `docs/schemas/audit.schema.json`.

## Tests

- Corpus tests; schema tests; regex timeout tests.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-19.md` before writing code.
4. Create branch `wp/19-auditing-find-audit`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-19: Auditing, find/replace, Go To Special, and the deterministic `audit``. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
