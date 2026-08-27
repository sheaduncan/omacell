# WP-10 — .xlsx writer, round-trip diff tool, atomic save

| | |
|---|---|
| Phase | 2 — File I/O |
| Lane | B — File I/O |
| Size | L (≈ 6–10) |
| Depends on | WP-09 |
| Unblocks | WP-13, WP-23, WP-24, WP-25, WP-27 |
| Spec sections | §6.9 F-9.1, F-9.2, F-9.7, §8.3 A-3.7 (custom part only), §12.2 |
| Where | `crates/io` (module `xlsx::write`, `xlsx::diff`) |

## Goal

Save what WP-09 reads without loss at L1/L2, re-emit L3 parts untouched, and prove it with a semantic diff.

## Deliverables

- Writers for every part WP-09 reads: workbook, shared strings (rebuilt from the interner), styles (deduplicated tables, stable ordering), worksheets (cells with cached values and formula metadata, merges, CF, DV, hyperlinks, autofilter, views, page setup, breaks, protection), tables, comments (legacy + VML, threaded + persons), defined names, `calcChain` (optional), content types and relationships regenerated consistently with preserved parts.
- Preserved parts re-emitted byte-identical with their relationships; custom part `xl/omacell/` written from workbook custom payloads (WP-11/WP-23 fill it).
- Atomic save: write temp in the target directory, fsync, rename; keep-backups option; LibreOffice-compatible lock file `.~lock.<name>#` create/check/remove.
- `xlsx::diff`: model-level comparison producing a JSON report (cells, styles, names, tables, CF, DV, views, parts) exposed as `omacell diff a.xlsx b.xlsx --json`.

## Implementation notes

- Excel is strict about part ordering and content types; validate output with `openpyxl` load and, when present, LibreOffice headless conversion in tests.
- Never write `fullCalcOnLoad` unless calc chain is absent.

## Acceptance criteria

- [ ] Round-trip corpus: open → save → `diff` is empty at L1 and L2 for every corpus file; preserved parts byte-identical (L3).
- [ ] Saved files load in `calamine`, `openpyxl`, and LibreOffice headless (skip if absent) without errors.
- [ ] Crash-safety test: kill during save leaves the original intact.
- [ ] Bench: 50 MB save < 5 s on the CI reference.

## Tests

- Round-trip corpus tests; external-loader tests; fault-injection save test; bench.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-10.md` before writing code.
4. Create branch `wp/10-xlsx-write`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-10: .xlsx writer, round-trip diff tool, atomic save`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
