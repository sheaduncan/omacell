# WP-17 — Editing and structure operations (data tools I)

| | |
|---|---|
| Phase | 4 — Surfaces II — GUI and data tools |
| Lane | A — Engine / core |
| Size | L (≈ 6–10) |
| Depends on | WP-04, WP-07a |
| Unblocks | WP-18, WP-19 |
| Spec sections | §6.5 F-5.5–F-5.7, §6.6 F-6.6–F-6.10, §13 |
| Where | `crates/core` (module `ops`), `crates/bus` (commands) |

## Goal

The mutating operations of daily spreadsheet work, each as a registered command with Excel semantics and exact undo.

## Deliverables

- Structure: sheet remove/reorder with exact restoration; insert/delete cells (shift right/down), rows, columns with reference rewriting and `#REF!` for removed ranges; hide/unhide; group/outline levels with collapse state; merge, merge-across, unmerge; row height and column width incl. auto-fit via a measurement callback. Freeze/split are WP-14 session commands.
- Fill: series detection (linear, growth, dates, weekdays, months, years, custom lists), fill down/right/up/left, fill options (copy/series/formats only), `Ctrl+Enter` fill-selection.
- Clipboard semantics: copy adjusts relative references; cut retargets; Paste Special (values, formulas, formats, number formats, column widths, transpose, skip blanks, add/subtract/multiply/divide, paste link); clear variants (contents, formats, comments, all).
- Comments and notes (legacy notes, threaded comments with replies and resolve), hyperlinks (external/internal), sheet and workbook protection with Excel-compatible legacy hash and allowed-actions list, locked/hidden cell flags.
- Text to columns (delimited/fixed width with per-column types via WP-08's plan), remove duplicates (selected columns, report count), consolidate by position (category consolidation later).

## Implementation notes

- Write the expected-formula corpus before implementing: for each structural op, a fixture sheet, the op, and the formulas Excel would produce afterwards.
- Every operation is a command with an inverse; `proptest` the op/undo identity.

## Acceptance criteria

- [ ] Structural-edit corpus passes (formulas after insert/delete/move match Excel behavior documented per case).
- [ ] Paste Special matrix tests; fill-series corpus; protection hash matches known vectors.
- [ ] Undo property for every command in this package; `.xlsx` round-trip (with WP-10) preserves comments, hyperlinks, protection, outline, merges.

## Tests

- Corpus tests; `proptest` op/undo; round-trip tests coordinated with WP-10 fixtures.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-17.md` before writing code.
4. Create branch `wp/17-editing-structure`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-17: Editing and structure operations (data tools I)`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
