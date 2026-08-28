# Report — WP-07a: In-process command bus, changesets, and events

## Plan (written before coding)

- Files/modules to create:
  - `docs/schemas/commands.schema.json` — catalog envelope schema version 1 (frozen at merge).
  - `crates/bus/src/lib.rs` — crate root, re-exports.
  - `crates/bus/src/error.rs` — bus `{code, message, hint}` codes on `CoreError`.
  - `crates/bus/src/registry.rs` — `CommandRegistry`, `CommandSpec`, `Exposure`, `CommandKind`, typed `register<A, F>`.
  - `crates/bus/src/handler.rs` — `CommandContext`, `Effect`.
  - `crates/bus/src/policy.rs` — `MutationPolicy` keyed by trusted `Origin`.
  - `crates/bus/src/event.rs` — bounded in-process `EventBus`.
  - `crates/bus/src/changeset.rs` — `ChangesetStore` (`propose`/`apply`/`revert`/`list`/`get`); trusted inverses held privately while `Proposed`.
  - `crates/bus/src/session.rs` — `Bus`: execute, dry-run, batch boundary recalc, event emission.
  - `crates/bus/src/catalog.rs` — `SCHEMA = 1`, `commands_json()`.
  - `crates/bus/src/args.rs` — typed arg structs (`deny_unknown_fields` + schemars).
  - `crates/bus/src/logical.rs` — intern-handle-free restore payloads.
  - `crates/bus/src/resolve.rs` — A1/sheet resolution and range iteration with an area cap.
  - `crates/bus/src/commands/*.rs` — core + internal restore handlers.
  - `crates/bus/tests/{catalog,commands,policy,changeset,events,recalc,proptest_bus}.rs`.
  - Narrowly additive `Workbook` / `UndoLog` methods only (no evaluator/recalc module edits).
- Interfaces to expose (types, commands, schemas, CLI):
  - **Registry/handler/effect/policy types** (see below). Schema version **1**.
  - Public commands (ids freeze with this package): `cell.set`, `cell.clear`, `range.set`, `range.clear`, `sheet.add`, `sheet.rename`, `sheet.visibility`, `name.define`, `name.remove`, `format.number`, `style.set`, `calc.recalc`, `calc.mode`, `undo`, `redo`.
  - Internal restore (excluded from `commands_json()`, illegal as proposal forwards): `cell.restore`, `style.restore`, `sheet.remove`.
  - No CLI (WP-13). No Unix sockets (WP-07b).
  - Frozen WP-01 types are **not** changed (`CommandDescriptor` stays `{id, doc, arg_schema, mutating}`). Extra catalog fields live in bus JSON, not on the frozen struct.
- Tests and corpora to write first:
  - Catalog snapshot: `commands_json()` sorted, validates against `commands.schema.json`, round-trip of every public arg type, unknown-field rejection.
  - Per-command: normal, invalid, boundary, no-op, inverse, event-order, automatic-recalc, manual-recalc.
  - Policy matrix: `InAppAgent` / `ExternalAgent` / `PalettePlan` cannot direct-mutate or submit internal restore; queries remain callable (none in this set; mutating denial is the proof).
  - Changeset: propose does not touch live state; apply then revert restores logical state; apply and revert are each one undo unit; failed multi-command batch is atomic and emits no success/change events.
  - Dry-run leaves workbook, recalc, undo/redo, changeset store, and events unchanged.
  - Event backpressure: stalled subscriber cannot block a command or grow memory.
  - `proptest` inverse / apply-revert / failed-batch.
- Items the package says to "decide and document" and the decision taken:
  - **Schema version:** `1`. Envelope `{schema, commands[]}` matching the functions catalog shape.
  - **Registry:** `BTreeMap<CommandId, RegisteredCommand>` for sorted lookup. `register<A, F>` is the extension API; later WPs register without editing bus command modules.
  - **Handler:** `fn(&mut CommandContext, A) -> Result<Effect, CoreError>`. Context borrows workbook + engine + origin. Handlers do not recalc (except `calc.recalc`) and do not emit; they return an `Effect`.
  - **Effect:** `{inverse, events, summary, dirty, result, auto_recalc}`. Summary is built from the effect, never by scanning the workbook.
  - **Policy:** direct mutate allowed for `User`, `Script`, `Ipc`. `InAppAgent`, `ExternalAgent`, `PalettePlan` may propose only. Internal restore and `undo`/`redo`/`calc.recalc` are not changeset-eligible forwards. Apply/revert allowed for `User`/`Script`/`Ipc`.
  - **Preflight:** clone workbook (COW interners/blocks), run handlers on the scratch copy, then `transact_try` on live. Recalc once at the outer boundary (`notify_edit` + `recalc_incremental`; no-op in `Manual` until `calc.recalc`).
  - **Inverses:** computed on scratch from logical state. Agent-supplied inverses rejected. Public `Changeset` keeps `inverse` empty while `Proposed` (frozen validate); store holds trusted inverses until `Applied`.
  - **cell.set input:** formula-bar text. Leading `=` is formula source (stored with `=`). Else `TRUE`/`FALSE`, finite number, else text. Empty input = clear contents, style kept.
  - **Range area cap:** reject ranges whose cell count exceeds `MAX_ROWS` (one full column).
  - **Default keys (classic, Appendix A):** `Delete` → clear; `Ctrl+Enter` → `range.set`; `Ctrl+F3` → `name.define`; `Ctrl+Shift+~` → `format.number`; `Ctrl+1` → `style.set`; `F9` → `calc.recalc`; `Ctrl+Z`/`Ctrl+Y` → undo/redo.
  - **Workbook additions:** `intern_formula`/`release_formula`, `set_cell_contents`, `remove_name`, `set_calc_mode`, `intern_num_fmt`/`num_fmt_code`, `undo_log`, `transact_try`; `UndoLog::abort`; `Delta::CalcMode`; custom numFmt table (ids ≥ 164) so `format.number` can persist codes. No eval/graph/recalc/coerce/spill/lambda edits.
- Open questions at planning time:
  1. Should `Ipc` stay a trusted direct-mutate origin in-process (WP-07b wrapping mutations as propose), or share the model-origin policy? Plan: trusted in-process; WP-07b enforces propose-by-default on the wire.
  2. Excel `+1` / `'text` formula-bar rules are not in this package; only `=`, numbers, bools, and text.
  3. `sheet.remove` as a public command is WP-17; this package registers it internal-only as the inverse of `sheet.add`.

## What was built

(Short prose + a file list. Link to the key tests.)

## Interfaces exposed (for dependents)

(Public types, command ids and schemas, CLI subcommands, fixtures other packages can reuse.)

## Deviations from the spec or the package (with reasons)

## Measurements

(Bench numbers, memory, corpus counts, eval pass rates — with the command that produced them.)

## Open questions / decisions needed

## RFC (only if a frozen contract changed)

None planned. Frozen WP-01 types and `docs/contracts.md` public type signatures are unchanged.

## Checklist

- [ ] `just check` green on a clean clone
- [ ] Every acceptance criterion ticked with evidence
- [ ] Docs warning-free; public items documented
- [ ] Baselines recorded (if the package has performance gates)
- [ ] No new `TODO(` without a `WP-` reference; no new dependency without justification
- [ ] Nothing written outside the repository except documented temp dirs
