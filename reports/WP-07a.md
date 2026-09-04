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

### 2026-09-02 changeset preflight follow-up plan (written before coding)

- Add a regression proving high-frequency formatting changesets retain bounded,
  command-local `style.restore` inverses instead of the workbook-wide
  `edit.restore` fallback, including exact apply/revert on a workbook with
  unrelated populated cells.
- Add an opt-in registry path for handlers that provide their own logical
  inverses. Dispatch will not clone the workbook for those commands; the
  existing `register` behavior remains the compatibility-safe fallback for
  extension handlers that still need an exact snapshot diff.
- Make the WP-07a cell, range, sheet, name, style, and calculation handlers use
  the local-inverse path. Teach WP-17 format actions to build their existing
  per-cell `style.restore` inverses before mutation, then register those actions
  on the same path.
- Preserve scratch preflight, live `transact_try` atomicity, effect limits,
  event ordering, and the frozen command/effect wire contracts. Validate with
  focused bus tests, strict bus Clippy, and the exact repository gate.

### 2026-09-04 changeset restoration follow-up plan (written before coding)

- Reproduce the remaining command-bus findings from
  `reports/review-2026-09-02.md` before implementation: structural proposals
  must restore pivot definitions as well as shifted cells, and removing an
  imported defined name must retain its exact range flags, scope, sheet
  identity, formula, comment, or logical constant rather than round-tripping
  through the narrower public `name.define` arguments.
- Extend the internal logical restore format with pivot-registry differences
  and a private `name.restore` command. Keep both commands internal and absent
  from schema-1 `commands_json()`; encode logical values instead of workbook
  interner handles, and cover apply/revert plus ordinary undo behavior.
- Replace the two full `BTreeMap` cell copies in generic inverse generation
  with a constant-auxiliary-memory merge of the stores' ordered iterators.
  Charge each cell/sheet restore record while it is built and fail with the
  stable changeset-limit error before an inverse can grow past the retained
  changeset budget.
- Add `Workbook::clone_for_scratch` as a narrow additive core method so bus
  validation and preview clones share copy-on-write workbook data without
  deep-copying the 64 MiB undo/redo history. Preserve the ordinary `Clone`
  contract, and retain full history only when preflighting `edit.undo` or
  `edit.redo`. Record this additive frozen-contract change in the RFC section
  and `docs/contracts.md` for approval by merge.
- Reconcile stale parts of the original report explicitly: proposal command
  count and forward-byte limits already run before dispatch, and PR #86 moved
  37 high-frequency commands to bounded local inverses. Run focused regression
  tests first, the complete bus/core suites, strict Clippy and rustdoc, and the
  exact `just check` gate before opening the PR.

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

The 2026-09-02 changeset-preflight follow-up makes snapshot inverses an
explicit compatibility path instead of an unconditional cost. Thirty-seven
commands with bounded handler-provided inverses now skip the redundant
per-command workbook clone; WP-17 format actions produce per-cell
`style.restore` commands before mutation. Legacy structural handlers retain
the exact snapshot fallback until they gain command-local inverses.

The 2026-09-04 restoration follow-up completes and bounds that compatibility
path. Exact structural patches now include pivot-registry differences,
including the OOXML identity fields omitted from ordinary serde payloads, so a
revert restores shifted pivot definitions byte-for-byte at the logical model
boundary. `name.remove` records a private logical `name.restore` inverse that
preserves range absolute flags and sheet ids, scope, formula source, comment,
and text/array/error constants without retaining interner handles. Both restore
commands remain internal and absent from the public command catalog.

Generic cell diffs now merge the two row-major stores directly instead of
materializing maps and a union set. Restore records are charged against the
1 MiB changeset budget as they are constructed, including cells in a removed
sheet, and construction stops with `changeset.limit` before an oversized patch
can be retained. Proposal command-count and forward-payload checks were
already performed before dispatch on current `main`; that part of the original
finding was stale and required no code change.

Bus validation, preview, and snapshot-inverse paths use the additive
`Workbook::clone_for_scratch`, which keeps logical copy-on-write state but
starts without the live 64 MiB undo/redo history. Preflight retains ordinary
`Clone` only for `edit.undo` and `edit.redo`, whose behavior depends on that
history.

## Interfaces exposed (for dependents)

| Item | Where |
|---|---|
| `Bus::{execute, propose, apply, revert, dry_run, commands_json, subscribe, drain}` | `omacell_bus` |
| `CommandRegistry::{register, register_with_local_inverse}<A, F>` | compatibility-snapshot and bounded-local-inverse extension paths |
| `CommandSpec`, `Exposure`, `CommandKind`, `CommandContext`, `Effect`, `MutationPolicy` | `omacell_bus` |
| `SCHEMA = 1`, `commands_json()` | catalog envelope |
| `EventBus`, `SubscriberId`, `ChangesetStore` | in-process events / store |
| Typed args | `omacell_bus::args` |
| Error codes | `omacell_bus::codes` (`command.unknown`, `command.args`, `command.denied`, `command.internal`, `command.ineligible`, `range.size`, `changeset.not_found`, `changeset.state`) |
| Catalog schema | `docs/schemas/commands.schema.json` |
| Public command ids | `cell.set`, `cell.clear`, `range.set`, `range.clear`, `sheet.add`, `sheet.rename`, `sheet.visibility`, `name.define`, `name.remove`, `format.number`, `style.set`, `calc.recalc`, `calc.mode`, `edit.undo`, `edit.redo` |
| Internal ids | `cell.restore`, `style.restore`, `edit.restore`, `name.restore` (plus package-specific inverse ids) |
| Workbook | `set_cell_contents`, `intern_formula`/`release_formula`, `remove_name`, `set_calc_mode`, `intern_num_fmt`/`num_fmt_code`, `transact_try`, `undo_log`, additive `clone_for_scratch` |

No CLI. `core ↛ bus`.

**WP-07b:** wrap mutating IPC in `propose` by default; never address internal ids on the socket.

**WP-14 / WP-17 / file I/O:** call `bus.registry_mut().register(...)` with `#[serde(deny_unknown_fields)]` args.

## Deviations from the spec or the package (with reasons)

- **`edit.undo` / `edit.redo` instead of `undo` / `redo`.** Frozen `CommandId` requires two dotted segments. Documented; the WP names were not valid ids.
- **`Ipc` is a trusted in-process origin.** WP-07b applies propose-by-default on the wire.
- **Range commands reject area > `MAX_ROWS`.** Full-grid fills are WP-17-scale.
- **`style.set` is a patch of common fields**, not a full OOXML `Style` (core `Style` has no `JsonSchema`). Exact restore uses internal `style.restore`.
- **The proposed forward-command allocation fix was already present.**
  `ChangesetStore::ensure_can_propose` rejects command count and serialized
  forward bytes before scratch dispatch, and PR #86 already converted 37
  high-frequency commands to local inverses. This follow-up changes only the
  remaining generic structural fallback and exact name removal.

## Measurements

Host: local Linux. `cargo test -p omacell-bus` — catalog 5, changeset 7, commands 11, events 3, policy 6, proptest 3, recalc 3 (all pass). `RUSTDOCFLAGS="-D warnings" cargo doc -p omacell-bus -p omacell-core --no-deps` — pass. Recalc integration: automatic `=A1+10` updates; manual stays empty until `calc.recalc`.

No new crates.io dependencies. `proptest` is workspace-dev; `omacell-core` / `schemars` / `serde` / `serde_json` / `indexmap` are pre-approved.

Changeset-preflight follow-up: the focused bounded-inverse regression first
failed because `format.bold` retained `edit.restore`, then passed with one
sub-2-KiB `style.restore` command on a workbook containing 10,000 unrelated
cells. `cargo test -p omacell-bus`, strict bus Clippy, and the exact
`CARGO_BUILD_JOBS=2 just check` gate pass.

Changeset-restoration follow-up: focused regressions first reproduced lossy
defined-name range flags, a pivot-protected structural revert that could not
restore the shifted cells, copied undo history during dry-run, and inverse
construction that accumulated a removed sheet before applying the retained
byte cap. Verification on 2026-09-04:

- `cargo test -p omacell-core -p omacell-bus` — pass, including all 17
  changeset tests, 16 command tests, 7 analysis tests, 22 IPC server tests,
  73 core unit tests, 20 workbook-model tests, and 103 core doctests.
- `cargo clippy -p omacell-core -p omacell-bus --all-targets -- -D warnings`
  — pass.
- `RUSTDOCFLAGS='-D warnings' cargo doc -p omacell-core -p omacell-bus
  --no-deps` — pass.
- Exact `CARGO_BUILD_JOBS=2 just check` — pass: workspace formatting, strict
  Clippy, all workspace tests, and workspace documentation.

## Open questions / decisions needed

1. **Resolved:** WP-14 binds `Ctrl+Z` / `Ctrl+Y` to `edit.undo` / `edit.redo`.
2. **Resolved:** same-user IPC remains trusted; agent origins are changeset-only
   at the shared dispatch boundary.
3. **Resolved:** custom `numFmtId` allocation starts at 164 and XLSX I/O shares
   the workbook table.

## RFC (only if a frozen contract changed)

The original package changed no frozen WP-01 type. This follow-up additively
exposes `Workbook::clone_for_scratch()` on the frozen WP-02 workbook API.
Ordinary `Clone`, the workbook data model, command catalog v1, changeset types,
and all wire schemas remain unchanged. The method exists solely to copy
logical state without retained undo/redo history; approval is recorded by
merge of this RFC and in `docs/contracts.md`.

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

### Acceptance (2026-09-04 changeset restoration follow-up)

- [x] A structural changeset apply/revert restores shifted pivot definitions,
  including serde-skipped OOXML identity —
  `structural_changeset_revert_restores_shifted_pivot_definition`
- [x] `name.remove` changesets restore exact imported range flags, formulas,
  comments, scope, and logical text constants —
  `name_remove_changeset_restores_exact_imported_range`,
  `name_remove_changeset_restores_formula_and_logical_text_constant`
- [x] Generic cell inverse discovery uses ordered iterator merge with constant
  auxiliary index memory, and removed-sheet construction fails at the 1 MiB
  budget before retaining the complete oversized inverse —
  `removed_sheet_inverse_stops_at_the_construction_budget`
- [x] Scratch clones retain logical state without undo/redo history, while
  dry-running `edit.undo` still sees history and does not touch live state —
  `scratch_clone_keeps_logical_state_without_undo_history`,
  `dry_run_undo_keeps_history_available_and_live_state_untouched`
- [x] `name.restore` remains internal and is excluded from schema-1
  `commands_json()` — `commands_json_is_sorted_stable_and_matches_schema`
