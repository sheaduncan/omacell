# WP-08 — CSV/TSV import with preview, progressive load, and export

| | |
|---|---|
| Phase | 2 — File I/O |
| Lane | B — File I/O |
| Size | M (≈ 3–5) |
| Depends on | WP-02, WP-06 |
| Unblocks | WP-13, WP-27 |
| Spec sections | §6.9 F-9.4, §8.4 A-4.4 (hook only), §12.1 |
| Where | `crates/io` (module `csv`) |

## Goal

Import delimited text without ever silently converting a value, at speed, with a preview model the UIs and the AI import assistant can both drive.

## Deliverables

- Sniffer: delimiter, quote char, encoding (BOM detection; UTF-8/UTF-16/Latin-1 heuristics via `encoding_rs`), header-row guess, decimal and thousands separators, line endings.
- `ImportPlan`: per-column `ColumnType { Auto, Number, Text, Date(fmt), Boolean, KeepAsText }`, header flag, skip rows, locale; `preview(plan, n) -> PreviewRows` where each cell reports `(raw, would_become, changed: bool)`.
- Conservative inference: leading zeros, digit strings longer than 15, mixed alphanumerics (`SEPT1`, `MAR1`), ambiguous dates without locale certainty → text; only unambiguous dates convert.
- Progressive reader: chunked parse into the workbook with row-count events and cancellation; throughput target ≥ 100 MB/s parse on the CI reference.
- Exporter: delimiter, quoting policy, encoding, line endings, `--sheet`, `--range`, formulas-or-values.
- Clipboard helpers: parse pasted TSV/CSV/Markdown/HTML tables into a plan (used by WP-14).

## Implementation notes

- Expose `ImportPlan` as a serde type; the CLI (`convert --plan plan.json`), TUI/GUI preview, and WP-23's import assistant all read and write the same structure.

## Acceptance criteria

- [ ] Corpus `tests/corpus/csv/` (encodings, delimiters, quoted newlines, ragged rows, locales, BOMs) round-trips per its expectations.
- [ ] No-silent-conversion suite: gene names, ZIP codes, 16-digit ids, `007` stay text under `Auto`; the preview marks every changed cell.
- [ ] Ignored-by-default nightly test loads a synthetic 1 GB file progressively within memory budget; throughput bench recorded.

## Tests

- Corpus tests; `proptest` for quoting round-trips; criterion bench.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-08.md` before writing code.
4. Create branch `wp/08-csv`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-08: CSV/TSV import with preview, progressive load, and export`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
