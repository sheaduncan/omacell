# WP-11 — .omc text workbook and change records

| | |
|---|---|
| Phase | 2 — File I/O |
| Lane | B — File I/O |
| Size | M (≈ 3–5) |
| Depends on | WP-02, WP-06, WP-07 |
| Unblocks | WP-13 |
| Spec sections | §6.9 F-9.3, §8.6 A-6.4, Appendix E |
| Where | `crates/io` (module `omc`); `docs/formats/omc.md` |

## Goal

A stable, diff-friendly text format for workbooks and for changesets.

## Deliverables

- `docs/formats/omc.md`: finalized grammar from Appendix E — header, records (`book`, `name`, `style`, `sheet`, `cell`, `cf`, `validation`, `comment`, `hyperlink`, `merge`, `table`, `aicache` reserved for WP-23, `change`), typed literals, escaping, ordering rules (row-major, stable style ids), lossy-conversion report format.
- Reader and writer; conversion report listing dropped `.xlsx` parts.
- Changeset export/import as `change` records (`change <origin> <cmd> <json>`), used by `omacell changeset export --omc` and MCP `changeset_propose` from files.

## Implementation notes

- Single-cell edits must produce single-line diffs; test it.

## Acceptance criteria

- [ ] `.xlsx` → `.omc` → `.xlsx` is L1/L2-equal for the corpus (minus reported losses).
- [ ] Diff stability test passes; fuzz target for the reader runs 10 minutes clean.
- [ ] Changeset round-trip through `.omc` reproduces `forward`/`inverse` exactly.

## Tests

- Round-trip tests; diff-stability test; fuzz target.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-11.md` before writing code.
4. Create branch `wp/11-omc-format`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-11: .omc text workbook and change records`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
