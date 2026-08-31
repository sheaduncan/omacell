# `.omc` text workbook (WP-11)

Line-oriented UTF-8, one record per line, tab-separated fields. Designed for
`git diff`, `grep`, and scripts. Native save remains `.xlsx` (ADR-003).
`.omc` carries `.xlsx` L1–L2 except binary parts (spec F-9.3, Appendix E).

## Header

The first non-comment, non-blank line MUST be:

```
omc 1
```

Unknown version numbers are an error (`omc.format`).

## Comments and blank lines

A line whose first non-whitespace character is `#` is a comment.
Blank lines are ignored. Trailing comments on a record line are not supported.

## Fields

Fields are separated by U+0009 TAB. A field is either:

- raw text containing no TAB, LF, CR, or `"`, or
- a double-quoted string with C-escapes: `\\`, `\"`, `\n`, `\t`, `\r`.

Writers quote a field when it is empty, begins with `#`, contains TAB/LF/CR/`"`,
or has leading/trailing space. Unknown escapes, quotes in raw fields, and bytes
after a closing quote are parse errors.

## Record types

| Kind | Shape | Notes |
|---|---|---|
| `book` | `book	k=v…` | `date_system=`, `calc=`, `active=`, plus lossless `settings=<json>` and `meta=<json>`. |
| `numfmt` | `numfmt	<id>	<code>` | Custom `numFmtId` (≥ 164) and format code. |
| `style` | `style	<id>	<json>` | Dense ids from 1. Id 0 is the engine default and is omitted. JSON is a `Style`. Compact `k=v` lists are also accepted on read. |
| `name` | `name	<n>	<referent>	k=v…` | Referent is A1, `=formula`, a typed literal, or `array=<json>`. Optional `scope`, `comment`, and `type`. |
| `sheet` | `sheet	<name>	k=v…` | `hidden`/`veryHidden`; lossless JSON `view`, `protection`, `tab_color`; sparse JSON `row_sizes`, `row_hidden`, `col_sizes`, `col_hidden`. Legacy view/column keys are accepted. |
| `cell` | `cell	Sheet!A1	<literal>	k=v…` | Row-major. Optional `s`, `type=text\|formula`, `rich`, `array`, and formula-cache `v`, `v_text`, `v_rich`, or `v_array`. |
| `merge` | `merge	Sheet!A1:B2` | |
| `comment` | `comment	Sheet!A1	author=…	<text>` | Legacy notes. |
| `threaded_comment` | `threaded_comment	Sheet!A1	<json>` | Full threaded comment and replies. |
| `hyperlink` | `hyperlink	Sheet!A1	<target>	display=?	tooltip=?` | |
| `table` | `table	<name>	<range>	k=v…` | Header/totals/banding/auto-expand flags and JSON `columns`. Legacy comma-separated `cols` is accepted. |
| `pivot` | `pivot	<json>` | Full native `PivotTable` model plus source/output sheet names (ids are remapped on read), including its stable pivot id, fields, grouping, filters, layout, and output bounds. |
| `extra` | `extra	<sheet>	<kind>	<json-string>` | Opaque CF/DV/print/sparkline/autofilter fragments (`kind` = `cf`/`dv`/`print`/`sparkline`/`autofilter`). |
| `custom` | `custom	<part>	<utf8>` | `Workbook::custom_parts` (e.g. `xl/omacell/meta.json`). Non-UTF-8 is dropped. |
| `cf` / `validation` | sketch forms | Accepted as `extra` of kind `cf` / `dv` (raw remainder). |
| `aicache` | `aicache	<json>` | AI-cell cache (`xl/omacell/aicache.json`). |
| `changeset` | `changeset	id=	status=	origin=	…` | Optional header; carries summary counts (`cells`, `rows`, `columns`, `sheets`, `styles`) and `text`. |
| `change` | `change	forward\|inverse	<cmd>	<json>` | Or sketch `change	<origin>	<cmd>	<json>` (proposed, forward only). |

## Typed literals

| Form | Meaning |
|---|---|
| `=…` | Formula source (leading `=` stored on the interned formula). |
| `TRUE` / `FALSE` | Booleans. |
| `#N/A`, `#DIV/0!`, `#NAME?`, … | `ErrorKind::from_display`. |
| JSON number / Rust `f64` parse | Number. |
| syntactically quoted field | Text, including `"TRUE"`, `"1"`, `"#N/A"`, `"=…"`, and `""`. |
| empty | Empty value (formula-only cells). |

Writers add `type=text` or `type=formula` where needed so quoting used for field
escaping never changes the value type. Literal/spill arrays use a depth-limited
JSON value tree in `array=` (or `v_array=` for a formula cache). Rich-text runs
are JSON in `rich=` / `v_rich=` and use UTF-8 byte offsets.

## Ordering (writers)

1. `omc 1`
2. `book`
3. `numfmt` records, id order
4. `style` records, id order
5. Each sheet in workbook order: `sheet`, then `cell` row-major, then `merge` / `comment` / `threaded_comment` / `hyperlink` / `table` / `pivot` / `extra`
6. `name` records (after sheets so `scope=` can resolve)
7. `custom`
8. `changeset` / `change`

Readers defer defined names until all sheets exist. A `sheet` must still appear
before cells and annotations that name it, unless it is the default `Sheet1`.

## Limits (fuzz / DoS)

| Cap | Value |
|---|---|
| File bytes | 32 MiB |
| Line bytes | 1 MiB |
| Records | 1_000_000 |

NUL bytes are rejected. The same caps apply to decoding and encoding, so the
writer never produces a document that the reader rejects for size. Array value
trees are capped at 16 levels. No external entities (plain text).

## Lossy conversion (`.xlsx` → `.omc`)

`ConversionReport.dropped` lists binary / L3 parts (VBA, media, drawings, theme,
vml, …) and non-UTF-8 custom parts that have no `.omc` record. Modeled L1/L2 and
UTF-8 `extra` fragments are kept. Re-saving `.omc` → `.xlsx` is L1/L2-equal for
those modeled pieces. Rich text nested inside an array or used as a defined-name
constant is rejected rather than silently flattened; ordinary rich-text cells
round-trip exactly.

## Changesets

A changeset document is a valid `.omc` whose body is `changeset` + `change`
lines. `changeset_to_omc` / `changeset_from_omc` round-trip the complete
`Changeset`, including summary counters and `forward` / `inverse` `CommandCall`
JSON. A standalone changeset import must contain at least one `change` record.
CLI `omacell changeset export --omc` is WP-13.
