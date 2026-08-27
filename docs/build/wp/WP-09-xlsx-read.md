# WP-09 — .xlsx reader (L1–L2) with L3 part preservation

| | |
|---|---|
| Phase | 2 — File I/O |
| Lane | B — File I/O |
| Size | XL (≈ 10–20) |
| Depends on | WP-02, WP-03, WP-06 |
| Unblocks | WP-10 |
| Spec sections | §6.9 F-9.1, F-9.2, F-9.6, §12.3, §13, §14 |
| Where | `crates/io` (module `xlsx::read`, `xlsx::opc`) |

## Goal

Open real-world workbooks losslessly at L1 and L2 and preserve everything else byte-for-byte for re-emission.

## Deliverables

- OPC layer: zip with limits (entry count, uncompressed size, ratio), content types, relationships; XML via `quick-xml` with external entities disabled and depth limits.
- Workbook: sheets and order, visibility, defined names (scoped), `calcPr`, `date1904`, workbook views, external link references (preserved).
- Shared strings incl. rich runs and phonetic (preserved); inline strings.
- Styles: `numFmts`, fonts, fills (incl. gradient preserved), borders, `cellXfs`/`cellStyleXfs`, `dxfs`, named cell styles, theme colors and indexed colors resolved (theme part read for color tints).
- Worksheets: dimension, views (pane/freeze/split/selection/zoom/gridlines/showFormulas), cols (width, hidden, outline), rows (height, hidden, outline), cells (`n`, `s`, `b`, `e`, `str`, `inlineStr`, `d`), formulas (normal, shared, array/CSE, data table, dynamic-array `cm` metadata), merges, conditional formatting (incl. `x14` extensions), data validations (incl. `x14`), hyperlinks, autofilter, `tableParts`, page setup/margins/print options/header-footer, row/column breaks, sheet protection.
- Tables (`xl/tables/*.xml`), comments + VML notes, threaded comments and persons, sparklines (`x14`, preserved for WP-25), drawings/charts/images/VBA/customXml/unknown parts and their relationships preserved as `PreservedPart`s; Omacell custom part (`xl/omacell/*.json`) read into workbook custom payloads.
- Error model: recoverable warnings list (`FileWarnings`) surfaced to the UI/CLI.
- `scripts/corpus-gen/xlsx/` Python (openpyxl) generators for synthetic corpus files, checked in with their outputs.

## Implementation notes

- Cross-check values against `calamine` (dev-dependency) in tests; `calamine` is never used at runtime.
- Formulas are parsed with WP-03; unparsable formulas are kept as text with a warning, never dropped.

## Acceptance criteria

- [ ] Every file in `tests/corpus/xlsx/` opens; L1 values equal `calamine`'s for all cells; L2 structures match the fixture expectations (JSON sidecars).
- [ ] Limits tests: zip bomb, deep XML, entity expansion, path traversal entries are rejected with clean errors.
- [ ] Fuzz targets for zip and worksheet XML run 10 minutes without panic.
- [ ] Bench: a synthetic 50 MB workbook opens < 5 s on the CI reference.

## Tests

- Corpus tests with sidecar expectations; fuzz targets; criterion bench; warning-model tests.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-09.md` before writing code.
4. Create branch `wp/09-xlsx-read`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-09: .xlsx reader (L1–L2) with L3 part preservation`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
