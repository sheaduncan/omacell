# WP-31 — Repository review remediation

| | |
|---|---|
| Phase | Correctness and reliability hotfix |
| Lane | D — Integration |
| Size | XL |
| Depends on | WP-02, WP-07a, WP-10, WP-12, WP-16, WP-17, WP-18, WP-22, WP-28 |
| Unblocks | Release candidate |
| Spec sections | §6.1, §6.5, §6.6, §6.9, §7.2, §8.2, §8.5, §8.7, §11.5, §12.2, §12.3, Appendix B |
| Where | `crates/core`, `crates/bus`, `crates/io`, `crates/ai`, `crates/cli`, retained frontends |

## Goal

Remediate all findings in `reports/code-review-2026-09-05.md`, with regression
coverage at the boundary where each defect was observable.

## Deliverables

- Keep accepted AI redaction marks attached to their cells through structural
  edits and prevent marked header values from entering workbook cards.
- Apply structural row/column transforms to defined names, tables, and other
  range-bearing workbook records; keep sort side records attached to cells.
- Make sheet rename/delete operate on formula-valued names and stable sheet
  identity, including owned-object cleanup and XLSX preservation sidecars.
- Enforce sheet/workbook protection and password checks across command-bus
  mutation paths.
- Implement configured autosave and recoverable snapshot discovery for retained
  frontends without changing a frozen command schema.
- Route CSV/TSV and OMC through the shared lock, backup, and atomic-save policy.
- Reject JSON object-table imports whose implied rectangle exceeds the shared
  cell budget.
- Prevent a newly discovered per-cell font from being selected until egui has
  activated its family definition, so first-frame workbook rendering cannot
  panic on fonts such as Aptos Narrow.
- Ensure the normal standalone GUI dependency graph enables native Vulkan and
  GLES backends instead of relying on dev-dependency feature unification.
- Launch Omarchy agent hand-offs without waiting for the agent terminal on a
  retained frontend's event thread.
- Make classic GUI text entry start an overwrite edit on the selected cell and
  honor the documented cell double-click edit gesture. Coalesce duplicate
  plain-text and IME commits without discarding intentional repeated input.

## Acceptance criteria

- [x] Marked values cannot appear in card headers or after insert/delete/move/sort.
- [x] Structural edits keep defined names, tables, and supported range metadata
      aligned; undo/redo is exact.
- [x] Sheet rename/delete preserves identity semantics and XLSX sidecars.
- [x] Protected mutations and incorrect unprotect passwords fail atomically.
- [x] Sorting moves notes, comments, and hyperlinks with their logical cells.
- [x] Dirty retained sessions create bounded autosave snapshots and surface
      recoverable snapshots on the next launch.
- [x] CSV/TSV and OMC honor cooperative locks and configured backup rotation.
- [x] Oversized implied JSON tables fail before workbook construction.
- [x] Newly discovered workbook fonts use a safe fallback during the frame in
      which their egui family definition is installed.
- [x] A standalone `omacell` binary has at least one native wgpu backend and
      survives an affected-workbook startup smoke test.
- [x] An Omarchy agent hand-off returns immediately while its terminal remains
      open, leaving the spreadsheet responsive.
- [x] Printable text starts one in-cell edit in the classic GUI, and
      double-clicking a cell edits its existing input.
- [x] A printable character delivered as both plain text and an IME commit is
      inserted once, while two distinct commit pairs remain two characters.
- [x] No frozen public core type, command schema, IPC envelope, MCP surface, or
      workbook-card schema changes.
- [x] `just check` is green.

## Tests

- AI privacy integration tests for marked headers and structural edits.
- Core/bus tests for names, tables, sheet identity, protection, sorting side
  records, and exact undo/redo.
- Generated XLSX rename round trips for preserved worksheet fragments.
- CLI file-lifecycle tests for autosave/recovery, locking, and backups.
- CLI hand-off regression with an agent process that deliberately stays open.
- GUI lifecycle regressions for direct printable entry, paired text/IME commit
  normalization, and cell double-click.
- JSON import limit tests using compact sparse objects.
- GUI font-cache regression for a named family discovered during an active
  frame.
- Full `just check`.

## Procedure

1. Write the Plan section of `reports/WP-31.md` before product code.
2. Add a failing regression test for each review finding.
3. Implement the smallest shared invariant or policy that closes the finding.
4. Run focused crate suites after each cluster and `just check` at completion.
5. Re-run the original standalone reproductions and complete the report.

## Done when

Every acceptance box has source or test evidence, the original reproductions no
longer fail, the report records any deviations, and the full repository check is
green.
