# Report — WP-24a: Pivot fidelity and structural reference rewriting

## Plan (written before coding)

- Files/modules to create:
  - Core layout / calc-field / structural-rewrite extensions in `crates/core/src/pivot.rs`, `workbook.rs`, `ops.rs`
  - OOXML preserve + Distinct Count / calculated-field metadata in `crates/io/src/xlsx/pivot.rs` and `write.rs`
  - Tests in `crates/core/tests/pivot.rs` and `crates/io/tests/pivot_roundtrip.rs`
  - Corpus additions in `tests/corpus/pivot/`
- Interfaces to expose (types, commands, schemas, CLI):
  - Additive `PivotCalcField` and `PivotTable.calc_fields`
  - Serde-skipped OOXML identity on `PivotTable` (`ooxml_dirty`, original cache/table part names and `cacheId`)
  - `Workbook::set_pivot_ooxml_dirty`
  - No command schema changes
- Tests and corpora to write first:
  - Compact / outline / tabular golden tables
  - Calculated-field aggregation
  - Structural-edit matrix (before / inside / after, undo, refuse split)
  - Excel-shaped zip fixtures for opaque extension byte-preservation, Distinct Count x14, calculated fields
- Items the package says to "decide and document" and the decision taken:
  - Compact: two-space indent per depth plus a group-header row when an outer key changes; outline blanks repeated outer labels; tabular still repeats them
  - Calculated fields: arithmetic over named cache fields (`'Amount'*0.1`); text/missing → empty
  - Distinct Count: keep native aggregation; write/read `x14:dataField pivotShowAs="distinctCount"`
  - Preserve original cache/table XML and their relationships when `ooxml_dirty` is false; regenerate only dirty pivots
  - Insert at the source/output origin shifts the whole range (new row/col is before the header/origin); deleting the origin still errors `pivot.struct`
- Open questions at planning time:
  - None that block the package; slicers remain L3 worksheet-rel copies as in WP-10

## What was built

Closed the WP-24 fidelity deferrals.

- Compact nested row fields now emit hierarchical labels (`East`, then `  A`) instead of `East | A`. Outline blanks repeated outer labels; tabular still repeats them.
- Calculated fields are modeled (`PivotCalcField`), evaluated on refresh from the source, and written as `cacheField databaseField="0" formula="..."`.
- Distinct Count writes Excel 2013+ x14 metadata on both the cache definition and the data field, and reads `pivotShowAs="distinctCount"` back as `PivotAgg::DistinctCount`.
- Unchanged imported pivots re-emit original cache/table parts (and relationship-reachable extras) byte-for-byte. `pivot.refresh` and structural reference rewrites mark the pivot dirty so those parts regenerate.
- Row/column insert/delete and covering cell shifts rewrite pivot source and output ranges in the same undo transaction. Partial bands that would split a managed range still fail with `pivot.struct`.

Key tests: `crates/core/tests/pivot.rs` (corpus + structural + compact + calc field) and `crates/io/tests/pivot_roundtrip.rs` (preserve, dirty regenerate, Distinct Count, calculated field, LibreOffice structure check).

## Interfaces exposed (for dependents)

- `omacell_core::pivot::PivotCalcField { name, formula }`
- `PivotTable.calc_fields: Vec<PivotCalcField>`
- `PivotTable.ooxml_dirty` / `ooxml_cache_id` / `ooxml_cache_def` / `ooxml_table` (not serialized to `.omc`)
- `Workbook::set_pivot_ooxml_dirty`
- Structural-edit error code `pivot.struct`
- Excel-shaped fixtures built in `crates/io/tests/pivot_roundtrip.rs` (`excel_authored_pivot`)
- Corpus cases `compact_nested_rows`, `tabular_nested_rows`, `outline_nested_rows`, `calc_field_tax`

Command ids and schemas are unchanged.

## Deviations from the spec or the package (with reasons)

- Calculated-field formulas are arithmetic over field names only (no Excel functions). Unsupported formulas evaluate to empty rather than fail the refresh.
- Slicers are still unmodeled; they survive only as L3 parts related from the worksheet, which already happened for unchanged files.
- Inserting a row at the pivot source header shifts the whole source down rather than expanding it, so the header row identity is preserved.

## Measurements

- Core pivot suite: 15 passed (`cargo test -p omacell-core --test pivot`), including 13 corpus cases (9 from WP-24 plus compact/outline/tabular/calc-field).
- IO pivot suite: 11 passed (`cargo test -p omacell-io --test pivot_roundtrip`), including LibreOffice headless conversion of calculated-field and Distinct Count fixtures (present on this machine).
- No new Criterion gate; WP-24's 100k-row baseline is unchanged.

## Open questions / decisions needed

None for this package.

## RFC (only if a frozen contract changed)

Additive only; no existing command id, schema, event variant, or catalog envelope version changes.

1. Add public `PivotCalcField` and `PivotTable.calc_fields` (serde default empty).
2. Add serde-skipped OOXML identity fields on `PivotTable` and `Workbook::set_pivot_ooxml_dirty`. These are load/save hints, not `.omc` records.
3. Existing WP-24 command schemas are unchanged. Clients that deserialize `PivotTable` JSON must ignore unknown fields (already required).

## Checklist

- [x] `just check` green on a clean clone
- [x] Every acceptance criterion ticked with evidence
- [x] Docs warning-free; public items documented
- [x] Baselines recorded (if the package has performance gates) — none new
- [x] No new `TODO(` without a `WP-` reference; no new dependency without justification
- [x] Nothing written outside the repository except documented temp dirs

### Acceptance

- [x] An Excel-authored pivot with unsupported extensions survives open/save with its opaque parts and relationships unchanged. Evidence: `unchanged_pivot_preserves_opaque_extension_bytes`.
- [x] Calculated-field and Distinct Count fixtures reopen as live pivots in Excel/LibreOffice without semantic downgrade. Evidence: round-trip tests plus `libreoffice_opens_calc_and_distinct_fixtures_if_present` (LibreOffice present); generated Distinct Count writes `pivotShowAs="distinctCount"`.
- [x] Structural edits before, inside, and after pivot source/output ranges rewrite references and remain one undo unit. Evidence: `pivot_structural_edits_rewrite_source_and_output_as_one_undo` and `pivot_cell_shift_refuses_to_split_and_rewrites_full_bands`.
- [x] Compact, outline, and tabular golden tables match their documented layouts. Evidence: corpus cases `compact_nested_rows`, `outline_nested_rows`, `tabular_nested_rows` and `pivot_compact_layout_indents_nested_row_fields`.
