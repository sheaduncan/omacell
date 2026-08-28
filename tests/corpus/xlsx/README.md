# Corpus — xlsx

Synthetic `.xlsx` fixtures for WP-09 (read) and WP-10 (round-trip). Generated
by `scripts/corpus-gen/xlsx/gen.py` (stdlib `zipfile`; no third-party files).

Each `.xlsx` has a JSON sidecar of L2 expectations. L1 cell values are also
cross-checked against `calamine` in `crates/io/tests/xlsx_corpus.rs`.

| File | What it covers |
|---|---|
| `l1_values.xlsx` | number, shared string, bool, error, inlineStr, date serial + numFmt |
| `l1_formulas.xlsx` | normal formula + shared formula shift |
| `l2_merges_freeze.xlsx` | merge + frozen pane + zoom |
| `l2_names.xlsx` | defined name |
| `l2_hyperlinks.xlsx` | external hyperlink |
| `l2_table.xlsx` | table part |
| `l2_comments.xlsx` | legacy comment / note |
| `l2_print.xlsx` | page setup (preserved extras) |
| `l2_hidden_sheet.xlsx` | hidden sheet |
| `omacell_part.xlsx` | `xl/omacell/*.json` custom part |

Regenerate: `python3 scripts/corpus-gen/xlsx/gen.py`
