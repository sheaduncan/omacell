# WP-27 — Additional formats: ODS, JSON, Parquet/Arrow, native `.xls`, HTML/Markdown tables

| | |
|---|---|
| Phase | 6 — Analysis and output |
| Lane | B — File I/O |
| Size | L (≈ 6–10) |
| Depends on | WP-08, WP-10 |
| Unblocks | WP-28 |
| Spec sections | §6.9 F-9.5, F-9.7, §13 |
| Where | `crates/io` (modules `ods`, `json`, `parquet`, `bridge`, `html`) |

## Goal

Round out interoperability so Omacell is never the reason a file cannot be opened.

## Deliverables

- ODS read (content, styles, number formats, merges, names) and basic write; JSON import/export (array-of-objects ↔ table; documented flattening rules; `--jq`-style path option); Parquet/Arrow read via the `arrow`/`parquet` crates with type mapping; HTML and Markdown table import (clipboard and files).
- `.xls` read in-process with a bounded BIFF parser; no external office suite or subprocess required. The deprecated `[integrations] libreoffice_fallback` key remains accepted and is ignored.
- LibreOffice lock-file convention honored on open/save across all formats (with WP-10).

## Implementation notes

- Parquet is read-only in this package; write is a later enhancement.

## Acceptance criteria

- [ ] Per-format corpora pass, including committed `.xls` fixtures on hosts without LibreOffice; lock-file interplay tested with a simulated Calc lock.

## Tests

- Corpus tests; native `.xls` parser tests; lock-file tests.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-27.md` before writing code.
4. Create branch `wp/27-other-formats`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-27: Additional formats: ODS, JSON, Parquet/Arrow, native `.xls`, HTML/Markdown tables`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
