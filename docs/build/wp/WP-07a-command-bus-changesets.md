# WP-07a — In-process command bus, changesets, and events

| | |
|---|---|
| Phase | 1 — Engine |
| Lane | D — Integration |
| Size | M (≈ 3–5) |
| Depends on | WP-02, WP-03, WP-04, WP-06 |
| Unblocks | WP-07b, WP-11, WP-14, WP-17, WP-20, WP-21, WP-22 |
| Spec sections | §6.10 F-10.4, §7.9, §8.6, §11.1, §11.3 |
| Where | `crates/bus`; narrowly additive `Workbook` methods only when the existing public API cannot support a listed command |

## Goal

Establish the in-process seam used by every front-end and model: typed registered commands, deterministic metadata, atomic execution, reviewable changesets, and bounded event subscriptions. Transport is WP-07b.

## Deliverables

- `CommandRegistry` with deterministic registration/lookup, typed `serde` + `schemars` arguments, documentation, default keys, mutating/query classification, changeset eligibility, public/internal exposure, and handlers.
- `commands_json()` as the single public catalog for palette, keymaps, Lua, CLI, MCP, and AI. Commit its versioned envelope schema as `docs/schemas/commands.schema.json`; command ids and argument schemas freeze when this package merges.
- Execution validates structural JSON through typed deserialization (unknown fields rejected) and semantic constraints in the handler. Errors use stable `{code, message, hint}` values.
- `CommandContext` owns/borrows the workbook, `RecalcEngine`, origin/policy, and event sink. A successful mutation batches edit notifications and runs automatic recalculation once at the outer command/batch boundary; manual mode remains a no-op until explicit `calc.recalc`.
- Core command set for this package:
  - `cell.set`, `cell.clear`, `range.set`, `range.clear` (contents only);
  - `sheet.add`, `sheet.rename`, `sheet.visibility`;
  - `name.define`, `name.remove`;
  - `format.number`, `style.set`;
  - `calc.recalc`, `calc.mode`, `undo`, `redo`.
- A documented extension API. Later owners register without modifying the bus:
  - WP-14: `view.freeze/split/zoom/select` and other session/view commands;
  - WP-17: range copy/move/fill, sheet remove/reorder, row/column structure and geometry, clipboard/clear variants;
  - WP-08–11/13: `file.open/save/export` after real I/O services exist;
  - WP-18/19 and later packages: their data/audit commands.
- Changeset store and lifecycle: `propose`, `apply`, `revert`, `list`, `get`. A proposal executes against a copy-on-write scratch workbook to validate the full batch, compute trusted inverse commands, and build a summary; supplied inverse commands are ignored/rejected.
- Applying a validated changeset executes all forward commands as one workbook transaction and recalculates once. Revert executes stored inverses in order as one transaction and recalculates once.
- Private restore handlers where a public command cannot express exact prior state. They are registered with internal exposure, excluded from `commands_json()`, unavailable as external forward commands, and encode logical workbook data rather than workbook-local interner handles.
- Mutation policy keyed by trusted out-of-band `Origin`: model origins can propose but cannot directly execute mutating commands. Query commands remain directly callable. Undo/redo and internal restore handlers are not legal forward commands in a proposal.
- `EventBus` with deterministic event ordering and bounded subscriptions. A slow subscriber cannot block mutation/recalculation or grow memory without limit.
- In-process dry-run: run the same validation/effect path on a scratch clone, return outcome + summary, and leave the live workbook, recalc engine, undo/redo state, changeset store, and event stream untouched.

## Atomicity and inverse rules

- Preflight the entire command or changeset on the scratch model before touching the single-writer live model.
- Handlers return a typed effect record (affected ranges/counts, inverse, events), so summary construction never scans an entire workbook.
- If live execution can still fail after successful preflight, roll back the open transaction before returning an error; partial mutation is never a successful outcome.
- Recalculation cache writes are outside undo recording, as documented by WP-04, but the changed formula/input state is one undo unit.
- Changeset equivalence tests compare complete logical workbook state, including formulas, styles, names, settings, sheet order/visibility/view/geometry, side tables, and opaque custom parts as applicable to commands in scope. Undo-stack bookkeeping itself is tested separately.

## Acceptance criteria

- [ ] `commands_json()` is sorted, stable, and validates against `docs/schemas/commands.schema.json`; every public command has typed args, schema, doc, classification, and a round-trip test.
- [ ] Every command in this package has normal, invalid, boundary, no-op, inverse, event-order, automatic-recalc, and manual-recalc tests.
- [ ] Property: proposing never mutates live state; `apply` then `revert` restores complete logical state; `apply` and `revert` are each one undo unit.
- [ ] Property: a multi-command failure is atomic and emits no success/change events.
- [ ] Policy tests prove `InAppAgent`, `ExternalAgent`, and `PalettePlan` cannot directly execute mutating commands or submit internal restore commands.
- [ ] Dry-run leaves workbook, recalc state, undo/redo, changeset store, and events unchanged.
- [ ] Event subscriptions are bounded; a deliberately stalled subscriber cannot block a command or cause unbounded growth.
- [ ] Existing evaluator/recalc correctness, determinism, and performance gates remain green.

## Tests

- Schema/catalog snapshots and typed-argument rejection tests.
- `proptest` command/inverse, apply/revert, and failed-batch atomicity tests.
- Origin-policy matrix tests.
- Event ordering/backpressure tests.
- Recalc integration tests for single commands and batches.

## Out of scope

- Unix sockets, request envelopes, instance discovery, and transport fuzzing (WP-07b).
- File commands before I/O traits/implementations exist (WP-08–11/13).
- View/session commands (WP-14).
- Structural, fill, copy/move, destructive-sheet, and advanced clear semantics (WP-17).
- `.omc` encoding of changesets (WP-11 consumes the in-memory interfaces).

## Procedure

1. Read `AGENTS.md`, this file, and only the listed spec sections.
2. Read the *Interfaces exposed* sections of `reports/WP-02.md`, `WP-03.md`, `WP-04.md`, and `WP-06.md`, plus `docs/contracts.md`.
3. Write the Plan section of `reports/WP-07a.md`, including registry/handler/effect/policy types and the schema version, before code. Stop if a frozen WP-01 type must change.
4. Create branch `wp/07a-command-bus-changesets`.
5. Write schemas and acceptance/property tests first; implement until green.
6. Run `just check`, strict rustdoc, `cargo deny check`, and relevant recalc performance gates.
7. Complete the report and open `WP-07a: In-process command bus, changesets, and events`. Do not merge.

## Done when

Every acceptance box is ticked with evidence, command schema v1 is frozen and documented, CI is green, and downstream packages can register commands without redesigning the bus.
