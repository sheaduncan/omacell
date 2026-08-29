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

Writers quote a field when it is empty or contains TAB/LF/CR/`"` / leading or
trailing space.

## Record types

| Kind | Shape | Notes |
|---|---|---|
| `book` | `book	k=v…` | `date_system=1900\|1904`, `calc=automatic\|manual\|autoNoTable`, `active=<sheet>` |
| `numfmt` | `numfmt	<id>	<code>` | Custom `numFmtId` (≥ 164) and format code. |
| `style` | `style	<id>	<json>` | Dense ids from 1. Id 0 is the engine default and is omitted. JSON is a `Style`. Compact `k=v` lists are also accepted on read. |
| `name` | `name	<n>	<referent>	scope=<sheet>?` | Referent is A1, `=formula`, or a typed literal. |
| `sheet` | `sheet	<name>	k=v…` | `hidden`, `veryHidden`, `freeze=R,C`, `zoom=`, `split=x,y`, `protect=1` |
| `cell` | `cell	Sheet!A1	<literal>	s=<styleId>?` | Row-major on write. Formulas start with `=`. |
| `merge` | `merge	Sheet!A1:B2` | |
| `comment` | `comment	Sheet!A1	author=…	<text>` | Legacy notes. |
| `hyperlink` | `hyperlink	Sheet!A1	<target>	display=?` | |
| `table` | `table	<json>` | Full `Table` record (sheet name resolved). |
| `extra` | `extra	<sheet>	<kind>	<json-string>` | Opaque CF/DV/print/sparkline/autofilter fragments (`kind` = `cf`/`dv`/`print`/`sparkline`/`autofilter`). |
| `custom` | `custom	<part>	<utf8>` | `Workbook::custom_parts` (e.g. `xl/omacell/meta.json`). Non-UTF-8 is dropped. |
| `cf` / `validation` | sketch forms | Accepted as `extra` of kind `cf` / `dv` (raw remainder). |
| `aicache` | reserved WP-23 | Skipped; listed on the conversion report. |
| `changeset` | `changeset	id=	status=	origin=` | Optional document header for change records. |
| `change` | `change	forward\|inverse	<cmd>	<json>` | Or sketch `change	<origin>	<cmd>	<json>` (proposed, forward only). |

## Typed literals

| Form | Meaning |
|---|---|
| `=…` | Formula source (leading `=` stored on the interned formula). |
| `TRUE` / `FALSE` | Booleans. |
| `#N/A`, `#DIV/0!`, `#NAME?`, … | `ErrorKind::from_display`. |
| JSON number / Rust `f64` parse | Number. |
| `"…"` | Text. |
| empty | Empty value (formula-only cells). |

## Ordering (writers)

1. `omc 1`
2. `book`
3. `numfmt` records, id order
4. `style` records, id order
5. Each sheet in workbook order: `sheet`, then `cell` row-major, then `merge` / `comment` / `hyperlink` / `table` / `extra`
6. `name` records (after sheets so `scope=` can resolve)
7. `custom`
8. `changeset` / `change`

Readers accept any order except that a `sheet` must appear before cells that name it, unless the sheet is the default `Sheet1`. Scoped names must follow their sheet.

## Limits (fuzz / DoS)

| Cap | Value |
|---|---|
| File bytes | 32 MiB |
| Line bytes | 1 MiB |
| Records | 1_000_000 |

NUL bytes are rejected. No external entities (plain text).

## Lossy conversion (`.xlsx` → `.omc`)

`ConversionReport.dropped` lists binary / L3 parts (VBA, media, drawings, theme,
vml, …) that have no `.omc` record. Modeled L1/L2 and `extra` fragments are
kept. Re-saving `.omc` → `.xlsx` is L1/L2-equal for those modeled pieces.

## Changesets

A changeset document is a valid `.omc` whose body is `changeset` + `change`
lines. `changeset_to_omc` / `changeset_from_omc` round-trip `forward` and
`inverse` `CommandCall` JSON exactly. CLI `omacell changeset export --omc` is
WP-13.
