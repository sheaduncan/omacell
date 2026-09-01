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

`ERROR.TYPE` follows the Excel 365 extended table: classic errors and
`#GETTING_DATA` use 1–8, then `#SPILL!` 9, `#CONNECT!` 10, `#BLOCKED!` 11,
`#UNKNOWN!` 12, `#FIELD!` 13, and `#CALC!` 14.

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

No commands are registered here. WP-07a creates the first versioned command catalog and `docs/schemas/commands.schema.json`; those schemas freeze when WP-07a merges, not retroactively at G0.

## Command catalog — `docs/schemas/commands.schema.json` (WP-07a)

Envelope `{schema: 1, commands[]}` from `omacell_bus::commands_json`. Command ids and public argument schemas freeze when WP-07a merges. Internal restore handlers (`cell.restore`, `style.restore`, `sheet.remove`) are excluded from the catalog. Frozen WP-01 `CommandDescriptor` is unchanged.

The WP-19 UI integration adds public `edit.searchnext`, `edit.searchprev`, and
`edit.explainerror` commands. Each uses the shared optional `{count: u32}` UI
argument schema, mutates session presentation only, and is not changeset
eligible. Search results return `{count, sheet, row, col}` when a match is
selected; error explanation returns the existing optional
`ErrorExplanation` JSON representation.

The WP-16 file-lifecycle completion adds public `file.new`, `file.close`, and
`file.saveas` commands. New and close use the closed empty-object schema;
save-as requires one string `path`. All three are non-changeset-eligible
mutating commands. `file.new` replaces the workbook, detaches its path, and
emits `WorkbookOpened { path: None }`; `file.close` returns the frontend control
result `{close: true}`; `file.saveas` uses the existing save pipeline and makes
the destination the active path. The catalog envelope remains schema version 1
and existing command ids and schemas are unchanged.

The final pre-release command integration adds five public workflow commands:

- `name.manager` uses a closed empty-object schema and opens the session's
  defined-name panel. `name.paste` requires `{name: string}` and inserts the
  resolved workbook/sheet name into the current formula edit. Both mutate only
  UI session state and are not changeset-eligible.
- `name.createfrom` requires `{range: string, positions: (top|left|bottom|right)[]}`.
  It creates workbook-scoped, absolute range names from text labels, excludes
  every selected label edge from the referents, normalizes invalid label
  separators to `_`, rejects collisions atomically, and is changeset-eligible.
- `ai.assist` uses a closed empty-object schema and opens the formula workflow
  picker for `ai.formula.generate|explain|fix|refactor`. It mutates UI session
  state only and is not changeset-eligible.
- `chart.export` requires `path`, accepts optional `sheet` and chart `id`, and
  defaults `width`/`height` to 800×480. It is a non-changeset-eligible mutating
  composition command because it writes bounded SVG/PNG output atomically.

These are additive schemas; the catalog envelope remains version 1 and no
existing command schema or IPC envelope changes.

## IPC v1 — `docs/schemas/ipc/` (WP-07b)

Unix-socket JSON-lines envelopes freeze when WP-07b merges: request, reply, event/overflow, and discovery records. Mutating changeset-eligible commands default to `propose`; internal command ids are never addressable on the socket. Limits (`MAX_FRAME_BYTES` 1 MiB, `MAX_JSON_DEPTH` 32, `MAX_CONNECTIONS` 32) are part of the freeze.

## Changesets — `omacell_core::changeset`

| Type | Source |
|---|---|
| `ChangesetId`, `ChangesetStatus` | [`changeset.rs`](../crates/core/src/changeset.rs) |
| `CommandCall` | same |
| `ChangeSummary` | same |
| `Changeset` | same |

Proposed changesets carry no inverse commands. WP-07a computes inverses from trusted workbook state before moving a changeset to `Applied`; applied and reverted non-empty changesets must carry those inverses. Agent-supplied inverses are not trusted.

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
| `LocaleInfo`, `DateOrder` | same (WP-06 tables; unknown LCIDs fall back to `en-US`) |

WP-01 public fields of `LocaleId` / `LocaleSeparators` and the `EN_US` constants are unchanged.

## Dates — `omacell_core::dates` (WP-06)

| Type / fn | Source |
|---|---|
| `DateSystem` (`Excel1900`, `Excel1904`) | [`date_system.rs`](../crates/core/src/date_system.rs), re-exported by [`dates.rs`](../crates/core/src/dates.rs) |
| `CivilDate`, `TimeOfDay` | same |
| `serial_to_date` / `date_to_serial` / `weekday_sun0` | same |
| `MAX_SERIAL_1900`, `MAX_SERIAL_1904` | same |

Civil-to-serial conversion rejects impossible calendar dates and inconsistent `lotus_leap` flags.

## Number formats — `omacell_core::numfmt` (WP-06)

| Type / fn | Source |
|---|---|
| `format` / `format_with` / `parse` / `general` / `general_for_width` | [`numfmt.rs`](../crates/core/src/numfmt.rs) |
| `FormatValue`, `Formatted`, `FormatOptions` | same |
| `builtin_format` (`numFmtId` 0–49) | [`numfmt/builtin.rs`](../crates/core/src/numfmt/builtin.rs) |
| `ColorHint`, `LayoutHints`, `MAX_FORMAT_LEN` | [`numfmt/token.rs`](../crates/core/src/numfmt/token.rs) |

`format` does not take interned `Value::Text`; pass [`FormatValue::Text`]. Default date system is 1900.

## Analysis — `omacell_core::{pivot,whatif,stats}` (WP-24; approval pending)

WP-24 proposes the following additions to the frozen public core contract. The PR must not merge until a human approves this RFC in `reports/WP-24.md`.

| Type / fn | Source |
|---|---|
| `PivotId`, `PivotTable`, `PivotRegistry`, `PivotDataField`, `PivotCalcField`, `PivotColumns`, `PivotCell`, `PivotValue` | [`pivot.rs`](../crates/core/src/pivot.rs) |
| `PivotAgg`, `ShowAs`, `DateGroup`, `PivotGroup`, `PivotLayout`, `CacheValue` | same |
| `materialize`, `materialize_from_cache`, `cache_table`, `write_output` | same |
| `GoalSeekResult`, `goal_seek`, `validate_goal_seek`, `DEFAULT_MAX_ITER`, `DEFAULT_TOL` | [`whatif.rs`](../crates/core/src/whatif.rs) |
| `StatsSummary`, `HistBin`, `describe_range` | [`stats.rs`](../crates/core/src/stats.rs) |

`Workbook` adds `pivots`, `add_pivot`, `import_pivot`, `refresh_pivot`, `refresh_pivot_from_cache`, `remove_pivot`, `restore_pivot`, and `set_pivot_ooxml_dirty`; `WorkbookSnapshot` adds `pivots`. Pivot output cells are managed and reject ordinary edits with `pivot.readonly`. Percentage show-as values use fractional storage (`0.3` means 30%) and a percentage number format. `PivotTable` additive fields from WP-24a: `calc_fields`, plus serde-skipped OOXML identity (`ooxml_dirty`, `ooxml_cache_id`, `ooxml_cache_def`, `ooxml_table`) used only for `.xlsx` round-trip.

The WP-07a command catalog adds public ids `pivot.create`, `pivot.refresh`, `pivot.remove`, `whatif.goalseek`, and `stats.describe` with the typed schemas in [`analysis.rs`](../crates/bus/src/analysis.rs). `pivot.restore` is an internal inverse and is excluded from `commands_json`. The catalog envelope remains schema version 1; existing ids and schemas are unchanged.

## Lua scripting extensions — WP-20 (approval pending)

WP-20 proposes additive extensions to frozen calculation and command contracts. The PR must not merge until a human approves the RFC in [`reports/WP-20.md`](../reports/WP-20.md).

| Type / fn | Source |
|---|---|
| `DynamicFnBody`, `DynamicFn` | [`eval/registry.rs`](../crates/core/src/eval/registry.rs) |
| `FnRegistry::{register_dynamic,lookup_dynamic,iter_dynamic}` | same |
| `CommandObserver`, `Bus::observe_commands` | [`session.rs`](../crates/bus/src/session.rs) |

Dynamic function names are case-insensitive and additive beside built-ins; built-ins retain lookup precedence. Dynamic arguments are materialized before dispatch, volatility participates in graph invalidation, and `ArrayLift::All` uses the same lift machinery as built-ins. `FnRegistry::len` and `is_empty` include dynamic entries.

The WP-07a public command catalog adds `macro.record`, `macro.stop`, `macro.save`, and `script.source`. The first, second, and fourth use the closed empty-object schema; `macro.save` requires one string `path`. All four are non-changeset-eligible mutating commands; `script.source` is a deferred host action, and its mutating classification prevents model origins from causing full-profile user code execution. The catalog envelope remains schema version 1 and existing command schemas are unchanged.

## MCP tools and resources — WP-21

The MCP tool names, argument schemas, and resource URI templates freeze with this package. Changing them after merge requires an RFC.

| Item | Source |
|---|---|
| Catalog envelope `schema = 1` | [`docs/schemas/mcp.schema.json`](schemas/mcp.schema.json) |
| Tool table | [`crates/bus/src/mcp/catalog.rs`](../crates/bus/src/mcp/catalog.rs) |
| Resource URIs `omacell://{file}/card`, `omacell://{file}/{sheet}` | [`crates/bus/src/mcp/uri.rs`](../crates/bus/src/mcp/uri.rs) |
| Markdown reference | [`docs/mcp.md`](mcp.md) |

`<file>` is a percent-encoded path (one URI segment). Write tools default to `Origin::ExternalAgent` changeset proposals. `apply=true` is denied by mutation policy.

`Workbook::ref_error_count()` exposes the incrementally maintained `#REF!` count used to gate the WP-21 diagnose offer without scanning stored cells during paint.

## AI provider and card extensions — WP-22 (approval pending)

WP-22 proposes additive extensions to the frozen WP-12 configuration and WP-21
MCP composition surfaces. The PR must not merge until a human approves the RFC
in [`reports/WP-22.md`](../reports/WP-22.md).

| Item | Source |
|---|---|
| `Provider`, `ChatRequest`/`ChatResponse`, `ChatMessage` | [`provider.rs`](../crates/ai/src/provider.rs) |
| `AiProvider.timeout`, `AiProvider.headers` | [`schema.rs`](../crates/conf/src/schema.rs) |
| `patch_ai_setup`, `merge_overlays` | [`edit.rs`](../crates/conf/src/edit.rs), [`layer.rs`](../crates/conf/src/layer.rs) |
| `CardHook`, `McpCtx.card` | [`session.rs`](../crates/bus/src/mcp/session.rs) |

Serialized configuration remains backward-compatible through serde defaults.
MCP tool names, `CardArgs`, resource templates, and wire schemas are unchanged.
The provider request contract first freezes here with correlated assistant tool
calls/tool results and an explicit output-token ceiling.
The workbook-card schema in [`card.schema.json`](schemas/card.schema.json) is
first frozen by WP-22 with bounded pagination and token-budget metadata.

## Corpora

- [`tests/corpus/addr/a1.tsv`](../tests/corpus/addr/a1.tsv)
- [`tests/corpus/addr/r1c1.tsv`](../tests/corpus/addr/r1c1.tsv)
- [`tests/corpus/errors/error_type.tsv`](../tests/corpus/errors/error_type.tsv)
- [`tests/corpus/numfmt/`](../tests/corpus/numfmt/) (WP-06)
