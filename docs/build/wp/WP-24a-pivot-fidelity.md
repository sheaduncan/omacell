# WP-24a — Pivot fidelity and structural reference rewriting

| | |
|---|---|
| Phase | 6 — Analysis and output |
| Lane | A/B — Engine and file I/O |
| Size | M (≈ 3–5) |
| Depends on | WP-24 |
| Unblocks | WP-28 fidelity sign-off |
| Spec sections | §6.7 F-7.1, §6.9 F-9.2/F-9.3, §13 |
| Where | `crates/core`, `crates/io` |

## Goal

Close the pivot fidelity work intentionally deferred from WP-24 without weakening its safe behavior.

## Deliverables

- Preserve unsupported pivot cache/table XML, relationships, and extensions byte-for-byte when a modeled pivot is unchanged; regenerate only pivots whose modeled definition changed.
- Model or faithfully preserve calculated fields and Excel Distinct Count metadata; add Excel-authored fixtures covering both.
- Rewrite pivot source and output references for row/column insertion, deletion, and cell shifts. Remove the conservative structural-edit refusal only for operations proven safe by tests.
- Improve compact layout from joined ` | ` labels to hierarchical indentation while retaining outline and tabular behavior.

## Acceptance criteria

- [ ] An Excel-authored pivot with unsupported extensions survives open/save with its opaque parts and relationships unchanged.
- [ ] Calculated-field and Distinct Count fixtures reopen as live pivots in Excel/LibreOffice without semantic downgrade.
- [ ] Structural edits before, inside, and after pivot source/output ranges rewrite references and remain one undo unit.
- [ ] Compact, outline, and tabular golden tables match their documented layouts.

## Tests

- OOXML byte-preservation fixtures plus LibreOffice/openpyxl structure checks.
- Core structural-edit and undo/redo matrix over source and output sheets.
- Pivot layout corpus additions.
