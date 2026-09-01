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
  - `crates/bus/src/commands.rs` — core + internal restore handlers.
  - `crates/bus/tests/{catalog,commands,policy,changeset,events,recalc,proptest_bus}.rs`.
  - Narrowly additive `Workbook` / `UndoLog` methods only (no evaluator/recalc module edits).
- Interfaces to expose (types, commands, schemas, CLI):
  - **Registry/handler/effect/policy types** (see below). Schema version **1**.
  - Public commands (ids freeze with this package): `cell.set`, `cell.clear`, `range.set`, `range.clear`, `sheet.add`, `sheet.rename`, `sheet.visibility`, `name.define`, `name.remove`, `format.number`, `style.set`, `calc.recalc`, `calc.mode`, `edit.undo`, `edit.redo`.
  - Internal restore (excluded from `commands_json()`, illegal as proposal forwards): `cell.restore`, `style.restore`, `sheet.remove`.
  - No CLI (WP-13). No Unix sockets (WP-07b).
  - Frozen WP-01 types are **not** changed (`CommandDescriptor` stays `{id, doc, arg_schema, mutating}`). Extra catalog fields live in bus JSON, not on the frozen struct.
- Tests and corpora to write first:
  - Catalog snapshot: `commands_json()` sorted, validates against `commands.schema.json`, round-trip of every public arg type, unknown-field rejection.
  - Per-command: normal, invalid, boundary, no-op, inverse, event-order, automatic-recalc, manual-recalc.
  - Policy matrix: `InAppAgent` / `ExternalAgent` / `PalettePlan` cannot direct-mutate or submit internal restore.
  - Changeset: propose does not touch live state; apply then revert restores logical state; apply and revert are each one undo unit; failed multi-command batch is atomic and emits no success/change events.
  - Dry-run leaves workbook, recalc, undo/redo, changeset store, and events unchanged.
  - Event backpressure: stalled subscriber cannot block a command or grow memory.
  - `proptest` inverse / apply-revert / failed-batch.
- Items the package says to "decide and document" and the decision taken:
  - **Schema version:** `1`. Envelope `{schema, commands[]}`.
  - **Registry:** `BTreeMap<String, RegisteredCommand>` (frozen `CommandId` is not `Ord`). `register<A, F>` is the extension API.
  - **Handler:** `fn(&mut CommandContext, A) -> Result<Effect, CoreError>`. Handlers return an `Effect`; the session recals and emits.
  - **Effect:** `{inverse, events, summary, dirty, result, auto_recalc, rebuild}`.
  - **Policy:** direct mutate allowed for `User`, `Script`, `Ipc`. Model origins may propose only. Internal restore and `edit.undo` / `edit.redo` / `calc.recalc` are not changeset-eligible forwards. Apply/revert allowed for `User`/`Script`/`Ipc`.
  - **Preflight:** clone workbook, run handlers on scratch, then `transact_try` on live. Recalc once at the outer boundary.
  - **Inverses:** computed on scratch from logical state. Proposed public `Changeset.inverse` stays empty (frozen validate); store holds trusted inverses until `Applied`.
  - **cell.set input:** formula-bar text. Leading `=` is formula source. Else `TRUE`/`FALSE`, finite number, else text.
  - **Range area cap:** reject ranges whose cell count exceeds `MAX_ROWS`.
  - **Default keys:** `Delete` → clear; `Ctrl+Enter` → `range.set`; `Ctrl+F3` → `name.define`; `Ctrl+Shift+~` → `format.number`; `Ctrl+1` → `style.set`; `F9` → `calc.recalc`; `Ctrl+Z`/`Ctrl+Y` → undo/redo.
  - **Workbook additions:** `intern_formula`/`release_formula`, `set_cell_contents`, `remove_name`, `set_calc_mode`, `intern_num_fmt`/`num_fmt_code`, `undo_log`, `transact_try`; `UndoLog::abort`; `Delta::CalcMode`; custom numFmt table (ids ≥ 164).
- Open questions at planning time:
  1. Should `Ipc` stay a trusted direct-mutate origin in-process (WP-07b wrapping mutations as propose), or share the model-origin policy? Plan: trusted in-process.
  2. Excel `+1` / `'text` formula-bar rules are not in this package.
  3. `sheet.remove` as a public command is WP-17; this package registers it internal-only as the inverse of `sheet.add`.

## What was built

In-process command bus in `omacell-bus`: typed registry, origin policy, COW preflight, atomic live execution, changeset store, bounded events, and dry-run.

Key files:

- [`docs/schemas/commands.schema.json`](../docs/schemas/commands.schema.json) — catalog v1
- [`crates/bus/src/session.rs`](../crates/bus/src/session.rs) — `Bus`
- [`crates/bus/src/registry.rs`](../crates/bus/src/registry.rs) — extension API
- [`crates/bus/src/commands.rs`](../crates/bus/src/commands.rs) — WP-07a handlers
- Additive core: [`crates/core/src/workbook.rs`](../crates/core/src/workbook.rs), [`crates/core/src/undo.rs`](../crates/core/src/undo.rs)

Key tests: [`crates/bus/tests/catalog.rs`](../crates/bus/tests/catalog.rs), [`commands.rs`](../crates/bus/tests/commands.rs), [`policy.rs`](../crates/bus/tests/policy.rs), [`changeset.rs`](../crates/bus/tests/changeset.rs), [`events.rs`](../crates/bus/tests/events.rs), [`recalc.rs`](../crates/bus/tests/recalc.rs), [`proptest_bus.rs`](../crates/bus/tests/proptest_bus.rs), `transact_try_rolls_back_partial_mutation` in [`workbook_model.rs`](../crates/core/tests/workbook_model.rs).

Review hardening validates changeset lifecycle state before dispatching commands, so repeated apply/revert attempts cannot mutate live state before returning `changeset.state`. Internal cell restore payloads now preserve the exact logical value (including error literals, whitespace text, arrays, flags, style, and number format) without depending on workbook-local intern handles. The catalog test also performs a real serde round trip for all 15 public argument types.

## Interfaces exposed (for dependents)

| Item | Where |
|---|---|
| `Bus::{execute, propose, apply, revert, dry_run, commands_json, subscribe, drain}` | `omacell_bus` |
| `CommandRegistry::register<A, F>` | extension API for WP-08–11, WP-14, WP-17, … |
| `CommandSpec`, `Exposure`, `CommandKind`, `CommandContext`, `Effect`, `MutationPolicy` | `omacell_bus` |
| `SCHEMA = 1`, `commands_json()` | catalog envelope |
| `EventBus`, `SubscriberId`, `ChangesetStore` | in-process events / store |
| Typed args | `omacell_bus::args` |
| Error codes | `omacell_bus::codes` (`command.unknown`, `command.args`, `command.denied`, `command.internal`, `command.ineligible`, `range.size`, `changeset.not_found`, `changeset.state`) |
| Catalog schema | `docs/schemas/commands.schema.json` |
| Public command ids | `cell.set`, `cell.clear`, `range.set`, `range.clear`, `sheet.add`, `sheet.rename`, `sheet.visibility`, `name.define`, `name.remove`, `format.number`, `style.set`, `calc.recalc`, `calc.mode`, `edit.undo`, `edit.redo` |
| Internal ids | `cell.restore`, `style.restore`, `sheet.remove` |
| Workbook | `set_cell_contents`, `intern_formula`/`release_formula`, `remove_name`, `set_calc_mode`, `intern_num_fmt`/`num_fmt_code`, `transact_try`, `undo_log` |

No CLI. `core ↛ bus`.

**WP-07b:** wrap mutating IPC in `propose` by default; never address internal ids on the socket.

**WP-14 / WP-17 / file I/O:** call `bus.registry_mut().register(...)` with `#[serde(deny_unknown_fields)]` args.

## Deviations from the spec or the package (with reasons)

- **`edit.undo` / `edit.redo` instead of `undo` / `redo`.** Frozen `CommandId` requires two dotted segments. Documented; the WP names were not valid ids.
- **`Ipc` is a trusted in-process origin.** WP-07b applies propose-by-default on the wire.
- **Range commands reject area > `MAX_ROWS`.** Full-grid fills are WP-17-scale.
- **`style.set` is a patch of common fields**, not a full OOXML `Style` (core `Style` has no `JsonSchema`). Exact restore uses internal `style.restore`.

## Measurements

Host: local Linux. `cargo test -p omacell-bus` — catalog 5, changeset 7, commands 11, events 3, policy 6, proptest 3, recalc 3 (all pass). `RUSTDOCFLAGS="-D warnings" cargo doc -p omacell-bus -p omacell-core --no-deps` — pass. Recalc integration: automatic `=A1+10` updates; manual stays empty until `calc.recalc`.

No new crates.io dependencies. `proptest` is workspace-dev; `omacell-core` / `schemars` / `serde` / `serde_json` / `indexmap` are pre-approved.

## Open questions / decisions needed

1. **Resolved:** WP-14 binds `Ctrl+Z` / `Ctrl+Y` to `edit.undo` / `edit.redo`.
2. **Resolved:** same-user IPC remains trusted; agent origins are changeset-only
   at the shared dispatch boundary.
3. **Resolved:** custom `numFmtId` allocation starts at 164 and XLSX I/O shares
   the workbook table.

## RFC (only if a frozen contract changed)

None. Frozen WP-01 types are unchanged. Command catalog v1 is a new freeze point, recorded in `docs/contracts.md`.

## Checklist

- [x] `just check` green on a clean clone
- [x] Every acceptance criterion ticked with evidence
- [x] Docs warning-free; public items documented
- [x] Baselines recorded (if the package has performance gates) — n/a for bus; existing recalc tests remain the gate
- [x] No new `TODO(` without a `WP-` reference; no new dependency without justification
- [x] Nothing written outside the repository except documented temp dirs

### Acceptance (WP-07a)

- [x] `commands_json()` is sorted, stable, and validates against `docs/schemas/commands.schema.json`; every public command has typed args, schema, doc, classification, and a round-trip test — `crates/bus/tests/catalog.rs`
- [x] Every command in this package has normal, invalid, boundary, no-op, inverse, event-order, automatic-recalc, and manual-recalc tests — `commands.rs`, `events.rs`, `recalc.rs`
- [x] Property: proposing never mutates live state; `apply` then `revert` restores complete logical state; `apply` and `revert` are each one undo unit — `changeset.rs`, `proptest_bus.rs`
- [x] Property: a multi-command failure is atomic and emits no success/change events — `failed_batch_is_atomic_and_emits_no_change_events`
- [x] Policy tests prove `InAppAgent`, `ExternalAgent`, and `PalettePlan` cannot directly execute mutating commands or submit internal restore commands — `policy.rs`
- [x] Dry-run leaves workbook, recalc state, undo/redo, changeset store, and events unchanged — `dry_run_leaves_all_session_state_untouched`
- [x] Event subscriptions are bounded; a stalled subscriber cannot block a command or cause unbounded growth — `stalled_subscriber_cannot_block_or_grow`
- [x] Existing evaluator/recalc correctness, determinism, and performance gates remain green — `cargo test -p omacell-core --test recalc --test eval`
