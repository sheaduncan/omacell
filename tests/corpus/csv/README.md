# Corpus — csv

Delimited-text fixtures for WP-08 (spec F-9.4). Each data row cites the
behavior it encodes in the `note` column.

No third-party files. Encoded (BOM / UTF-16 / Latin-1) variants are built
in `crates/io/tests` from the UTF-8 fixtures below.

## Auto conversion — `auto.tsv`

Columns: `raw`, `locale`, `column_type`, `kind`, `would_become`, `changed`, `note`.

`column_type` is `auto`, `number`, `text`, `boolean`, `keep_as_text`, or
`date:<format>`. `kind` is `text`, `number`, `bool`, `date`, or `empty`.
`changed` is `true` when the stored type is not text/empty.

Conservative Auto: leading zeros, digit strings longer than 15, mixed
alphanumerics (`SEPT1`, `MAR1`), two-digit years, and dates that are
invalid in the locale order stay text.

## Sniff — `sniff.tsv`

Columns: `file`, `delimiter`, `quote`, `encoding`, `bom`, `header`,
`decimal`, `thousands`, `eol`, `note`.

`file` is a fixture in this directory. `thousands` may be empty (none).
`eol` is `lf`, `crlf`, or `cr`. `bom` is `true`/`false`.

## Fixtures

| File | What it covers |
|---|---|
| `simple.csv` | comma, two numeric columns, no header |
| `simple.tsv` | tab-separated |
| `semicolon.csv` | EU semicolon delimiter |
| `pipe.csv` | pipe delimiter |
| `header.csv` | header row + ZIP / leading-zero traps |
| `quoted_newline.csv` | RFC 4180 quoted embedded newline |
| `ragged.csv` | short and long rows |
| `de_numbers.csv` | semicolon + `1.234,56` |
| `decimal_comma.csv` | semicolon + ungrouped decimal commas |
| `single_quote.csv` | comma delimiter + single-quoted fields |
| `crlf.csv` | CRLF line endings |
| `cr.csv` | CR line endings |
| `quotes.csv` | doubled quotes inside a field |
