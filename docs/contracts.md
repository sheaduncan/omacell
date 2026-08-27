# Frozen contracts (Gate G0)

Public types in `omacell-core` freeze after Gate G0. Changing a signature
here requires an RFC note in the PR and human approval.

Generate rustdoc with `cargo doc -p omacell-core --open`. Canonical paths
are listed below; each links to the source that rustdoc renders.

Product identity (`PRODUCT_NAME`) stays in [`crates/core/src/product.rs`](../crates/core/src/product.rs).

## Limits — `omacell_core::limits`

| Item | Source |
|---|---|
| `MAX_ROWS` (`1_048_576`) | [`limits.rs`](../crates/core/src/limits.rs) |
| `MAX_COLS` (`16_384`, column `XFD`) | same |
| `MAX_FORMULA_LEN` (`8_192`) | same |

Indices are 0-based. Valid rows are `0..MAX_ROWS`; valid columns `0..MAX_COLS`.

## Addressing — `omacell_core::addr`

| Type / fn | Source |
|---|---|
| `SheetId` | [`addr.rs`](../crates/core/src/addr.rs) |
| `SheetSpec` | same |
| `CellRef` | same |
| `RangeRef` | same |
| `ParsedRef`, `RefKind` | same |
| `col_to_letters` / `col_from_letters` | [`addr/letters.rs`](../crates/core/src/addr/letters.rs) |
| `parse_a1` / `parse_a1_cell` / `quote_sheet_name` | [`addr/a1.rs`](../crates/core/src/addr/a1.rs) |
| `parse_r1c1` / `parse_r1c1_cell` | [`addr/r1c1.rs`](../crates/core/src/addr/r1c1.rs) |

`CellRef.sheet` is a resolved id. Parsers put names in `SheetSpec`; WP-02 maps names to ids. The cell-only parsers reject sheet-qualified input so they cannot silently discard a name. R1C1 relative offsets resolve against a validated base cell into the same `CellRef` (grid index + abs flags). Invalid `CellRef` wire values are rejected; formatting a directly constructed out-of-grid value produces `#REF!` rather than panicking.

## Values — `omacell_core::value`

| Type | Source |
|---|---|
| `Value` (`Empty`, `Number(f64)`, `Bool`, `Text(StrId)`, `Error(ErrorKind)`, `Array(ArrayId)`) | [`value.rs`](../crates/core/src/value.rs) |
| `StrId`, `ArrayId` | same |
| `Array2D` (shape only) | same |

`size_of::<Value>() <= 16`. Text and array payloads are interned by WP-02. `Array2D` construction and deserialization require non-zero dimensions whose product fits in `u32`.

JSON of `Value::Number` is a JSON number. That is not bit-exact for every IEEE value; `NaN` / `Inf` are valid in-memory and not JSON-portable.

## Errors — `omacell_core::error`

| Type | Source |
|---|---|
| `ErrorKind` (Excel cell errors, exact display strings, `error_type()`) | [`error.rs`](../crates/core/src/error.rs) |
| `CoreError` `{code, message, hint}` | same |
| `codes::*` (`addr.ref`, `addr.parse`, `command.id`, `changeset.id`, `changeset.inverse`, `value.array_shape`) | same |

`ERROR.TYPE` codes 1–8 follow Microsoft’s documentation; newer errors return `None` (`#N/A`).

## Styles — `omacell_core::style`

| Type | Source |
|---|---|
| `StyleId`, `NumFmtId` | [`style.rs`](../crates/core/src/style.rs) |
| `Color`, `Font`, `Underline` | same |
| `Fill`, `PatternType`, `GradientFill`, `GradientKind`, `GradientStop` | same |
| `Border`, `BorderSide`, `BorderStyle` | same |
| `Alignment`, `HorizontalAlign`, `VerticalAlign` | same |
| `Protection`, `Style` | same |

Records are `Eq` + `Hash` (`f64` fields compare by `to_bits`) so WP-02 can intern by value.

## Commands — `omacell_core::command`

| Type | Source |
|---|---|
| `CommandId` | [`command.rs`](../crates/core/src/command.rs) |
| `CommandDescriptor` (`arg_schema: schemars::Schema`) | same |
| `Origin` | same |
| `Outcome` | same |
| `UndoUnit`, `UndoUnitId` | same |

No commands are registered here (WP-07).

## Changesets — `omacell_core::changeset`

| Type | Source |
|---|---|
| `ChangesetId`, `ChangesetStatus` | [`changeset.rs`](../crates/core/src/changeset.rs) |
| `CommandCall` | same |
| `ChangeSummary` | same |
| `Changeset` | same |

Proposed changesets carry no inverse commands. WP-07 computes inverses from trusted workbook state before moving a changeset to `Applied`; applied and reverted non-empty changesets must carry those inverses. Agent-supplied inverses are not trusted.

## Events — `omacell_core::event`

| Type | Source |
|---|---|
| `Event` (`#[non_exhaustive]`) | [`event.rs`](../crates/core/src/event.rs) |

Wire format: internally tagged JSON, `snake_case` variant names.

## Locale — `omacell_core::locale`

| Type | Source |
|---|---|
| `LocaleId` (`EN_US` = `0x0409`) | [`locale.rs`](../crates/core/src/locale.rs) |
| `LocaleSeparators` | same |

Separator tables for LCIDs other than `en-US` are WP-06.

## Corpora

- [`tests/corpus/addr/a1.tsv`](../tests/corpus/addr/a1.tsv)
- [`tests/corpus/addr/r1c1.tsv`](../tests/corpus/addr/r1c1.tsv)
- [`tests/corpus/errors/error_type.tsv`](../tests/corpus/errors/error_type.tsv)
