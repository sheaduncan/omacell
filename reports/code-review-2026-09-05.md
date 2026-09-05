# Omacell repository review — 5 September 2026

## Scope

This review covers commit `ada858856fa922f1e064feb198e2601090cdbbcf` on
`docs/readme-pitch`. It concentrates on the paths most likely to corrupt a
workbook, disclose marked data, or defeat a user-facing safety feature:

- structural edits, sheet lifecycle, sorting, recalculation, and undo;
- AI card construction and accepted redaction marks;
- file open/save, XLSX preservation, JSON import/export, locking, and backups;
- command-bus mutation policy, protection, and retained frontend lifecycle; and
- the documented configuration surface versus its runtime consumers.

The findings below were reproduced through public command-bus or I/O APIs in a
standalone probe outside the repository. Source-only concerns are identified as
such. No product source was changed as part of this review.

Severity means:

- **P0** — release blocker with a privacy or destructive-data-loss outcome.
- **P1** — high-impact correctness, fidelity, or recovery failure.
- **P2** — reliability or scalability defect with a narrower trigger.

## Findings

### OMR-01 — P0 — Accepted AI redaction marks can disclose the marked value

**Locations:**
[`crates/ai/src/policy.rs:159`](../crates/ai/src/policy.rs#L159),
[`crates/ai/src/policy.rs:259`](../crates/ai/src/policy.rs#L259),
[`crates/ai/src/card.rs:289`](../crates/ai/src/card.rs#L289), and
[`crates/core/src/ops.rs:115`](../crates/core/src/ops.rs#L115).

An accepted `ai.redact` mark does not reliably keep its value out of a card.
There are two independently observable cases:

1. The column summary derives `columns[].name` from the first used row.
   `redact_marked_columns` replaces `samples` and removes `min`/`max`, but never
   redacts `name`. If the marked cell is that first-row value, it remains in the
   provider payload.
2. Marks are stored as A1 strings in `xl/omacell/ai.json`. Structural edits move
   cells but do not rewrite those strings. In the probe, `A2` contained
   `PRIVATE_REVIEW_PAYROLL` and was marked `Sheet1!A2`. A full card did not
   contain the value before inserting a row at row 2; after the insert moved the
   value to `A3`, the mark stayed on `A2` and the card contained the value.

This violates the explicit guarantee that accepted redactions apply before a
card leaves the process. It matters even with AI disabled by default: once a
user enables a provider and marks sensitive data, the mark is presented as a
privacy boundary.

**Recommendation:** Make every card field retain its source coordinate through
policy filtering, including inferred headers. Redact the header/name field when
its source cell is covered. Include redaction ranges in the same structural
reference transformation as names, tables, and validations. If a transformation
cannot preserve a mark unambiguously, fail closed by lowering the request to
`schema`. Add leak tests for first-used-row values and every move/insert/delete/
sort operation before all provider hooks.

### OMR-02 — P1 — Structural edits leave defined names and tables on stale ranges

**Locations:**
[`crates/core/src/ops.rs:115`](../crates/core/src/ops.rs#L115),
[`crates/core/src/ops.rs:595`](../crates/core/src/ops.rs#L595), and
[`crates/core/src/ops.rs:928`](../crates/core/src/ops.rs#L928).

Whole-row and whole-column insertion/deletion update cell storage, a limited set
of sheet side maps, pivots, and cell formulas. They do not update defined-name
referents or table bounds.

Two command-level reproductions show user-visible wrong answers:

- `Amount` referred to `A2`, `A2` held `42`, and `B1` was `=Amount`. Inserting a
  row at row 2 moved `42` to `A3`; `Amount` remained `A2`, and `B1` became empty.
- `Sales` covered `A1:A3` with values 10 and 20 under its header, and a structured
  formula returned 30. Inserting above row 1 left the table at `A1:A3`, and the
  formula returned 10 because the final data row fell outside the stale table.

The implementation shape suggests the same audit is needed for conditional
formatting, validation, filters, protected ranges, print areas/titles, charts,
sparklines, and other range-bearing records; those are not all claimed as
independently reproduced here.

**Recommendation:** Introduce one validated structural transform, expressed in
terms of stable `SheetId` plus row/column operations, and apply it transactionally
to every range-bearing workbook object. Define behavior for partial intersections
instead of silently retaining an old range. Property-test insert/delete followed
by undo/redo across each object type, including cross-sheet references.

### OMR-03 — P1 — Deleting a sheet leaves dangling objects and allows references to retarget

**Locations:**
[`crates/core/src/workbook.rs:1657`](../crates/core/src/workbook.rs#L1657) and
[`crates/core/src/workbook.rs:1747`](../crates/core/src/workbook.rs#L1747).

`remove_sheet` guards pivots, unlinks the sheet, and records undo. It does not
remove or reject tables and sheet-scoped names owned by that sheet, nor does it
permanently invalidate textual formulas that referenced the deleted sheet.

The probe created a `Data` sheet, a table on it, and `=Data!A2` on `Sheet1`.
After deleting `Data`, the formula correctly evaluated to `#REF!`, but the table
registry still contained the table. Creating a different sheet named `Data` and
putting `99` in `A2` made the old formula silently evaluate to 99. Excel-style
sheet deletion should leave a permanent `#REF!`; a later sheet that happens to
reuse the display name is a different identity.

**Recommendation:** Before unlinking, either reject deletion while owned objects
exist or remove them in the same undoable transaction. Rewrite references to the
deleted stable identity to `#REF!`. Preserve enough structured information in
the inverse to restore references and owned objects only when undo restores the
original `SheetId`.

### OMR-04 — P1 — Renaming a sheet breaks formula-valued defined names

**Location:**
[`crates/core/src/workbook.rs:1830`](../crates/core/src/workbook.rs#L1830).

The rename transaction applies `RewriteOp::SheetRename` only to formulas stored
in cells. A `NameReferent::Formula` remains raw text.

The probe defined `Amount` as `=Sheet1!$A$1`, put 42 in `A1`, and evaluated
`=Amount` successfully. Renaming the sheet to `Renamed` left the name definition
as `=Sheet1!$A$1`; its consumer then evaluated to `#REF!`.

**Recommendation:** Run the sheet-identity rewrite over every formula-bearing
object, including defined names and formula-backed validation/conditional rules.
Prefer resolved sheet identity internally so a display-name change does not
require searching unrelated strings.

### OMR-05 — P1 — Renaming an XLSX sheet drops preserved worksheet data

**Locations:**
[`crates/io/src/xlsx/read.rs:50`](../crates/io/src/xlsx/read.rs#L50),
[`crates/io/src/xlsx/read.rs:201`](../crates/io/src/xlsx/read.rs#L201), and
[`crates/io/src/xlsx/write.rs:323`](../crates/io/src/xlsx/write.rs#L323).

`WorksheetExtras` is keyed by the sheet's display name at load time. The writer
looks it up using the current display name. A rename therefore disconnects the
sheet from its preserved print, conditional-formatting, validation, autofilter,
and sparkline fragments.

The probe attached a valid, unmodeled even-page header to `Sheet1`. Open/save
without a rename preserved `REVIEW_EVEN_HEADER`; renaming `Sheet1` to `Renamed`
and saving omitted it. Similar loss is possible for any feature carried only or
partly by `WorksheetExtras`.

**Recommendation:** Key worksheet sidecars by stable `SheetId` or original OPC
part identity. Add rename and case-only-rename round trips to the L2/L3 corpus,
then compare preserved parts as well as the modeled workbook.

### OMR-06 — P1 — Sheet and workbook protection are metadata-only no-ops

**Locations:**
[`crates/bus/src/commands.rs:246`](../crates/bus/src/commands.rs#L246) and
[`crates/bus/src/edit.rs:1874`](../crates/bus/src/edit.rs#L1874).

Protection state and legacy hashes are read, written, and tested, but mutation
commands do not consult them. The probe enabled password protection on the
default locked cell and then changed its value from 42 to 99 successfully. It
also disabled protection with the wrong password. With workbook structure
protection enabled, `sheet.rename` still succeeded.

Legacy worksheet protection is not cryptographic security, but it is still a
functional editing constraint. The current behavior tells users a sheet is
protected while allowing the operations the feature is meant to prevent.

**Recommendation:** Put protection authorization in a central pre-mutation
guard, not in individual UI renderers. The guard should evaluate affected cells,
protected ranges, allowed actions, workbook structure flags, and password
verification. Exercise every mutating command through user, Lua, IPC, and
changeset paths so no alternate entry point bypasses the rule.

### OMR-07 — P1 — Sorting cells detaches hyperlinks and annotations from their records

**Location:**
[`crates/core/src/sort.rs:190`](../crates/core/src/sort.rs#L190).

The sort permutation contains only `CellSlot` values. Notes, threaded comments,
and hyperlinks live in separate coordinate maps and are never permuted.

The probe placed value 2 and a hyperlink in `A1`, value 1 in `A2`, and sorted
ascending. Values became 1, 2, while the hyperlink stayed at `A1`; it was now
attached to the wrong record. The same mechanism applies to notes and comments.

**Recommendation:** Build a row/column permutation once and apply it to the
complete logical cell record, including annotations and links. Add tests for
stable sorts, hidden rows, left-to-right sorts, undo/redo, and partial ranges.

### OMR-08 — P1 — Autosave and crash recovery are configured but not implemented

**Locations:**
[`default/config.toml:55`](../default/config.toml#L55),
[`crates/conf/src/schema.rs:222`](../crates/conf/src/schema.rs#L222), and
[`docs/spec/omacell-design-spec.md:250`](../docs/spec/omacell-design-spec.md#L250).

`files.autosave_interval` defaults to 60 seconds and is included in the shipped
configuration/schema/documentation. Repository-wide source search found no
runtime read of the field and no autosave or recovery coordinator. A user can
work with the default setting believing recovery snapshots exist when none are
created.

**Recommendation:** Treat this as a release gate because forced termination is
normal under systemd-oomd and because the product carries large mutable files.
Write dirty snapshots atomically to the documented state directory, retain the
live undo history, detect recoverable snapshots before normal open, and test
crashes at every fsync/rename boundary. Until that exists, set the shipped value
to 0 and label the setting unavailable rather than silently accepting it.

### OMR-09 — P2 — CSV and OMC bypass the shared lock and backup policy

**Locations:**
[`crates/cli/src/files.rs:1132`](../crates/cli/src/files.rs#L1132) and
[`crates/cli/src/files.rs:1218`](../crates/cli/src/files.rs#L1218).

XLSX, ODS, JSON, HTML, and Markdown route writes through the common atomic writer
with `keep_backups` and cooperative lock handling. CSV/TSV and OMC instead use a
private temp-write helper that does neither.

With foreign `.~lock.*#` files in place, `file.saveas` rejected XLSX with
`xlsx.lock` but successfully replaced both CSV and OMC. The source also shows
that the configured `files.keep_backups` count is discarded for these formats.

**Recommendation:** Route every writable workbook format through one format-
agnostic atomic-save policy. Keep serialization format-specific, but share lock
inspection, backup rotation, cancellation, permissions, fsync, rename, and
directory sync.

### OMR-10 — P2 — JSON import accepts compact inputs that cannot be exported

**Locations:**
[`crates/io/src/json.rs:45`](../crates/io/src/json.rs#L45),
[`crates/io/src/json.rs:58`](../crates/io/src/json.rs#L58), and
[`crates/io/src/json.rs:214`](../crates/io/src/json.rs#L214).

Import bounds the source byte count, row count, and number of distinct flattened
keys, but not the rectangular table implied by `rows × union(keys)`. Export does
apply a one-million-cell rectangle limit.

A 39,781-byte array containing 2,000 objects with one distinct key each imported
successfully in about 0.35 seconds and produced a 2,001 × 2,000 used range. An
immediate JSON export failed because it would visit 4,002,000 cells. Larger
adversarial key distributions also amplify the import's nested row/key loop.

**Recommendation:** Compute the union-key count first and enforce a shared
import/export table-cell budget before constructing the workbook. If sparse JSON
is a supported goal, preserve row objects sparsely and stream the export instead
of materializing the union rectangle. Use a JSON-specific limit error rather
than the current `xlsx.limit` classification.

## Post-review field finding

### OMR-11 — P1 — Handing a workbook to the Assistant blocks the GUI

**Locations:**
[`crates/conf/src/agent.rs`](../crates/conf/src/agent.rs) and
[`crates/gui/src/app.rs`](../crates/gui/src/app.rs).

The GUI dispatched `omarchy agent prompt` on its event thread, and the shared
hand-off adapter used `Command::status`. On this workstation, Omarchy's launched
terminal remained a child of Omacell for the lifetime of the Grok session. The
live GUI process was confirmed sleeping in the kernel `do_wait` path with the
Assistant terminal beneath it, so it could not process Wayland events and the
desktop offered to wait or terminate the spreadsheet.

**Recommendation:** Spawn the hand-off with detached standard streams, return
as soon as process creation succeeds, and reap the long-lived child away from
the UI thread. Exercise the boundary with a fake agent that deliberately stays
open and assert that `omacell agent` returns promptly.

### OMR-12 — P1 — Selecting a cell and typing cannot start an edit

**Locations:**
[`crates/gui/src/app.rs`](../crates/gui/src/app.rs) and
[`crates/gui/tests/lifecycle.rs`](../crates/gui/tests/lifecycle.rs).

The native input path forwarded printable key events only after an in-cell edit
was already active, and processed committed text only for that same state. In
classic mode, an ordinary selected cell therefore discarded both halves of the
toolkit's key/text pair. The GUI also computed the cell double-click state but
used it only for row/column auto-fit; double-clicking a cell did not run
`edit.cell`. Users could edit only if they discovered the `F2` shortcut or
focused the formula bar.

**Recommendation:** When the classic grid owns input, let committed printable
text begin one empty overwrite edit and then insert the text exactly once. Map
a plain cell double-click to `edit.cell` after updating the selection. Preserve
modal normal-mode commands, formula-bar ownership, and modified shortcuts.

### OMR-13 — P1 — Paired text and IME commits duplicate cell input

**Locations:**
[`crates/gui/src/input.rs`](../crates/gui/src/input.rs) and
[`crates/gui/tests/lifecycle.rs`](../crates/gui/tests/lifecycle.rs).

The native input adapter accepted both `Event::Text` and `ImeEvent::Commit` as
independent committed text. When the desktop input path delivered the same `/`
through both sources, the in-cell editor appended both and displayed `//`.
This was reproduced at the GUI boundary with the platform event sequence.

**Recommendation:** Coalesce an identical adjacent cross-source commit pair,
regardless of which source arrives first. Consume only that pair so two actual
keystrokes still produce two characters, and retain same-source repeats.

## Cross-cutting improvements

The highest-value design change is a single mutation layer for workbook
identity and coordinate transformations. It should own:

- stable sheet identity across rename/delete/undo;
- transforms for every range-bearing object, including privacy marks;
- permutations of complete logical cell records during sort/move; and
- one preflight that validates protection and partial-overlap rules before any
  state changes.

The present tests are strong within individual modules, but these defects live
between modules. Add an invariant suite that builds a workbook containing all
supported side records, applies each structural/sheet operation, saves and
reopens XLSX and OMC, then checks values, formulas, object identities, redaction,
and undo/redo. This would cover several findings with one durable regression
framework.

Configuration also needs a consumption audit. At minimum,
`files.autosave_interval` is a shipped no-op; source search suggests other parsed
fields such as `files.follow_external_links`, `files.xlsx.preserve_unknown_parts`,
and `session.workspace_binding` may also lack runtime consumers. Each public key
should be implemented, rejected with a clear diagnostic, or marked unavailable
in generated documentation.

## Verification

- A standalone command-bus/I/O probe reproduced every concrete before/after
  result cited above.
- Core, function, bus, and I/O library tests passed: 123 tests.
- AI, configuration, Lua, and shared-UI integration suites passed, with the
  documented local-model evaluation remaining ignored.
- The 22 Unix-socket IPC tests passed when run outside the syscall-restricted
  sandbox.
- A fresh-build run of the number-format integration suite passed all 17 tests.
- Clippy passed for `omacell-core`, `omacell-fn`, `omacell-bus`, and
  `omacell-io` with all targets and warnings denied.
- The full repository `just check` passed: formatting, workspace-wide Clippy
  with warnings denied, all non-ignored workspace tests and doctests, and
  documentation generation completed successfully.

No live Excel oracle, LibreOffice round trip, screen-reader session, CJK IME, or
GPU/terminal matrix was available for this review. Those limitations do not
affect the deterministic reproductions above.
