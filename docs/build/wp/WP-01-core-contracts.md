# WP-01 — Core contracts: addressing, values, errors, styles, commands, changesets, events

| | |
|---|---|
| Phase | 0 — Foundations |
| Lane | A — Engine / core |
| Size | M (≈ 3–5) |
| Depends on | WP-00 |
| Unblocks | WP-02, WP-03, WP-05a, WP-05b, WP-05c, WP-06, WP-12 |
| Spec sections | §6.1, §6.2, §6.10 F-10.4, §8.6, §11.3, Appendix F |
| Where | `crates/core` (modules `addr`, `value`, `style`, `command`, `changeset`, `event`, `error`, `limits`, `locale`) |

## Goal

Freeze the types every other crate compiles against. After this package merges, changing a public signature here requires an RFC note in the PR and human approval (see PLAN.md, interface freeze).

## Deliverables

- `addr`: `SheetId`, `CellRef { sheet: Option<SheetId>, row: u32, col: u16, row_abs: bool, col_abs: bool }`, `RangeRef` (incl. whole-row/column and 3-D forms), A1 parse/print with `$`, R1C1 parse/print with relative offsets, column letters ↔ index up to `XFD`, `limits::{MAX_ROWS=1_048_576, MAX_COLS=16_384, MAX_FORMULA_LEN=8_192}`.
- `value`: `Value { Empty, Number(f64), Bool(bool), Text(StrId), Error(ErrorKind), Array(ArrayId) }` with `size_of::<Value>() <= 16`; `ErrorKind` for every Excel error (`#NULL!`, `#DIV/0!`, `#VALUE!`, `#REF!`, `#NAME?`, `#NUM!`, `#N/A`, `#GETTING_DATA`, `#SPILL!`, `#CALC!`, `#FIELD!`, `#CONNECT!`, `#BLOCKED!`, `#UNKNOWN!`) with exact display strings and `ERROR.TYPE` codes (verify against Excel documentation and record the table in tests); `Array2D` shape type.
- `style`: `Font`, `Fill` (solid/pattern/gradient-preserved), `Border` per side with Excel border styles, `Alignment`, `Protection`, `StyleId`, `NumFmtId`; all `serde`.
- `command`: `CommandId` (dotted string newtype), `CommandDescriptor { id, doc, arg_schema: schemars::RootSchema, mutating: bool }`, `Origin { User, Script, PalettePlan, InAppAgent, ExternalAgent, Ipc }`, `Outcome`, `UndoUnit` marker types.
- `changeset`: `Changeset { id, origin, status: Proposed|Applied|Reverted, forward: Vec<CommandCall>, inverse: Vec<CommandCall>, summary: ChangeSummary }`, `CommandCall { id, args: serde_json::Value }`.
- `event`: `Event` enum (`CellChanged`, `RecalcDone`, `FileSaved`, `ChangesetProposed`, `ThemeChanged`, …) with `serde` for the IPC wire format.
- `error`: `CoreError` (thiserror) with stable machine codes `{code, message, hint}` mirrored in the CLI later.
- `docs/contracts.md`: one page listing the frozen types with links to rustdoc.

## Interface sketch

```rust
// crates/core/src/value.rs — shape only; fields and docs are the deliverable
pub enum Value { Empty, Number(f64), Bool(bool), Text(StrId), Error(ErrorKind), Array(ArrayId) }

// crates/core/src/command.rs
pub struct CommandCall { pub id: CommandId, pub args: serde_json::Value }
pub struct Changeset { pub id: ChangesetId, pub origin: Origin, pub status: ChangesetStatus,
                       pub forward: Vec<CommandCall>, pub inverse: Vec<CommandCall>, pub summary: ChangeSummary }
```

## Implementation notes

- `col: u16` is deliberate (16,384 fits); `row: u32`.
- Text and arrays are handles into interners owned by the workbook (WP-02); the core crate defines the handle types only.
- No I/O, no toolkit, no async in this crate. Keep dependencies to `serde`, `serde_json`, `schemars`, `thiserror`, `smallvec`.

## Acceptance criteria

- [ ] Property tests: A1 ↔ (row, col) round-trips for every column to `XFD` and rows to the limit; out-of-range addresses are rejected with `#REF!`-class errors; R1C1 relative/absolute round-trips.
- [ ] Error display strings match Excel exactly; `ERROR.TYPE` table asserted.
- [ ] `size_of::<Value>() <= 16` asserted in a test.
- [ ] All public items documented; `cargo doc` has no warnings; serde round-trip for every public type.

## Tests

- `proptest` for addressing and serde; unit tests for error tables; a doc test per public type showing construction.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-01.md` before writing code.
4. Create branch `wp/01-core-contracts`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-01: Core contracts: addressing, values, errors, styles, commands, changesets, events`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
