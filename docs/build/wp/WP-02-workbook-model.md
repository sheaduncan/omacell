# WP-02 — Workbook model and storage

| | |
|---|---|
| Phase | 1 — Engine |
| Lane | A — Engine / core |
| Size | L (≈ 6–10) |
| Depends on | WP-01 |
| Unblocks | WP-04, WP-07a, WP-08, WP-09, WP-11 |
| Spec sections | §6.1, §6.2, §11.3, §12.1 |
| Where | `crates/core` (modules `workbook`, `sheet`, `storage`, `intern`, `geometry`, `names`, `tables`, `undo`) |

## Goal

Implement the in-memory model with the storage layout and budgets from §11.3, plus the undo transaction log everything mutating will use.

## Deliverables

- `Workbook`: ordered sheets, defined names (workbook/sheet scope; range, constant, or formula text), tables registry, settings (date system, calc mode, iteration, precision-as-displayed), metadata, custom-part payloads (opaque bytes by key, for WP-10).
- `Sheet`: name rules (≤31 chars, forbidden `[]:*?/\`), visibility, tab color, view state (zoom, freeze, split, scroll, selection, gridlines, show-formulas), merged ranges, comments/notes/hyperlinks stores, protection state.
- `storage`: 256×256 blocks in a hash map keyed by block coordinate; per block a dense slot array plus occupancy bitmap; `CellSlot { value, formula: Option<FormulaId>, style: StyleId, flags }`; APIs: get/set/clear, row-major iteration, region iteration, `used_range()`, `dimension()`, block-level shift for insert/delete of rows and columns (formula rewriting arrives in WP-03/WP-17 through hooks).
- `intern`: workbook-scoped string interner shaped like a shared-string table (stable ids, refcounts, rich-text runs kept alongside), style interner (dedup by value, refcounted).
- `geometry`: Fenwick trees over row heights and column widths with hidden flags; `pixel_to_index`, `index_to_pixel`, batch updates.
- `undo`: `Transaction` grouping, inverse deltas for cell/style/structure ops, memory budget with oldest-first eviction, `undo()`/`redo()` returning affected ranges for redraw.

## Implementation notes

- Numeric cell without style must cost ≤ 64 bytes amortized — measure with a counting allocator in tests.
- Keep `Workbook` single-writer; provide a cheap read snapshot (`Arc` of immutable block pages or a copy-on-write scheme) so rendering can read during recalc (§11.5).

## Acceptance criteria

- [ ] Storage behaves identically to a `HashMap<(u32,u16), Cell>` oracle under `proptest` sequences of set/clear/shift.
- [ ] Memory test: 1,000,000 × 20 numeric cells ≤ 64 B/cell amortized.
- [ ] Geometry mapping correct with hidden and custom-sized rows; O(log n) verified by a bench that scales.
- [ ] Undo/redo property: any transaction followed by undo restores the exact model; redo restores the transaction result.
- [ ] Sheet naming and visibility rules enforced with clear errors.

## Tests

- `proptest` oracle tests; allocator-counting memory test; criterion bench for geometry and iteration.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-02.md` before writing code.
4. Create branch `wp/02-workbook-model`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-02: Workbook model and storage`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
