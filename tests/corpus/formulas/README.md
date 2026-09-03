# Corpus — formulas

Table-driven fixtures for WP-03 (lexer / parser / printer / rewrite).

| File | Columns | Rule |
|---|---|---|
| `valid.tsv` | `input`, `print`, `mode`, `note` | `print(parse(input))` equals `print`. `mode` is `a1` (default) or `r1c1` (base A1). |
| `invalid.tsv` | `input`, `offset`, `code`, `note` | Parse fails; `offset` is a UTF-8 byte index; `code` is `formula.parse` / `formula.len` / `formula.depth`. |
| `rewrite.tsv` | `op`, `src`, `arg1`, `arg2`, `arg3`, `expected`, `note` | Rewrite `src` and print. Ops: `copy` (dcol, drow), `move` (src range, dest cell), `insert_rows` / `delete_rows` (at 1-based, count), `insert_cols` / `delete_cols` (at letters, count), `sheet_rename` / `table_rename` (old, new). |

Each `note` cites the Excel / spec behaviour the row encodes (F-3.1, F-3.2, F-1.4, F-3.6). Canonical print always includes `=`, uses compact spacing, and upper-cases function names, booleans, and A1 letters.

Do not put TAB characters inside formula text (the files are TSV).
