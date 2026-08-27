# WP-26 — Printing and PDF export

| | |
|---|---|
| Phase | 6 — Analysis and output |
| Lane | C — Surfaces (conf, UI core, TUI, GUI, charts, print) |
| Size | L (≈ 6–10) |
| Depends on | WP-16, WP-25 |
| Unblocks | WP-28 |
| Spec sections | §6.11, §12.1 |
| Where | `crates/io` (module `pdf`), `crates/core` (module `print`), `crates/gui`, `crates/cli` |

## Goal

Page setup and pagination as Excel defines them, output through Omacell's own renderer so print equals screen.

## Deliverables

- Page setup model per sheet (orientation, paper, margins, scaling/fit-to, print area, print titles, headers/footers with fields, manual and automatic page breaks, gridlines/headings, black-and-white); `.xlsx` mapping (with WP-09/10).
- Pagination engine producing page boxes from the grid geometry; print-preview mode data for the GUI.
- PDF writer (`pdf-writer` or equivalent) with font embedding via `ttf-parser`, vector charts, hyperlinks; `omacell export --pdf`; CUPS printing via `lp` with a device chooser.

## Implementation notes

- Fit-to-pages scaling must match Excel's rounding; write the corpus from documented examples.

## Acceptance criteria

- [ ] Pagination corpus passes (page counts and breaks); golden PDFs compared by extracted text and page geometry.
- [ ] Exported PDFs open in `pdftotext`/`mutool` without warnings; fonts embedded.

## Tests

- Corpus tests; golden PDF text/geometry tests.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-26.md` before writing code.
4. Create branch `wp/26-print-pdf`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-26: Printing and PDF export`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
