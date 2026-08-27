# WP-07 — Command bus, changesets, events, and IPC

| | |
|---|---|
| Phase | 1 — Engine |
| Lane | D — Integration (bus, CLI, Lua, MCP, AI, release) |
| Size | L (≈ 6–10) |
| Depends on | WP-02, WP-03, WP-04, WP-06 |
| Unblocks | WP-11, WP-13, WP-14, WP-17, WP-20, WP-21, WP-22 |
| Spec sections | §6.10 F-10.4, F-10.6, §7.9, §8.6, §11.1, §11.3 |
| Where | `crates/bus` |

## Goal

Make every mutation a named command with a JSON schema, make every AI or agent mutation a changeset, and expose both over a Unix socket. This is the seam all front-ends and all models go through.

## Deliverables

- `CommandRegistry`: registration with `schemars` schemas, docs, default keys, `mutating` flag; `execute(call, ctx)`; `commands_json()`; validation errors as `{code, message, hint}`.
- Core commands v1 (each with schema and doc): `cell.set`, `cell.clear`, `range.set`, `range.copy`, `range.move`, `range.fill`, `range.clear`, `sheet.add/rename/remove/reorder/visibility`, `row.insert/delete/hide/unhide/height`, `col.insert/delete/hide/unhide/width`, `name.define/remove`, `format.number`, `style.set`, `view.freeze/split/zoom/select`, `calc.recalc`, `calc.mode`, `file.open/save/export` (through `io` traits), `undo`, `redo`. (Data tools land in WP-17/18/19 as further commands.)
- Changeset engine: build a `Changeset` from a command list by executing against a scratch clone to compute inverses and a summary; `propose`, `apply` (one undo unit), `revert`, `list`, `get`; origin tracking; export/import via `.omc` `change` records (format in WP-11).
- `--dry-run` semantics: execute on a clone, return the summary, touch nothing.
- `EventBus` with subscriptions; events from §6.10 and §8.6.
- IPC: JSON-lines server on `$XDG_RUNTIME_DIR/omacell/<pid>.sock` (permissions 0600) with request `{id, cmd, args}` / reply `{id, ok, result|error}` and `subscribe`; client library; instance discovery (`omacell ipc` targets the focused window later; for now, newest instance).

## Interface sketch

```jsonc
// one command call
{"id": "range.sort", "args": {"range": "Data!A1:F400", "keys": [{"col": "F", "order": "desc"}], "header": true}}
// IPC frames (one JSON object per line)
{"id": 7, "cmd": "cell.set", "args": {"ref": "Inputs!B3", "input": "0.07"}}
{"id": 7, "ok": true, "result": {"changed": ["Inputs!B3"], "recalc": {"cells": 42, "ms": 3}}}
```

## Implementation notes

- The registry is the single source of truth for the palette, keymaps, Lua, CLI, MCP, and AI plans; anything not in it does not exist.
- Use std threads and blocking I/O for IPC; async is confined to `ai` and MCP.

## Acceptance criteria

- [ ] `commands_json()` validates against `docs/schemas/commands.schema.json`; every command has schema, doc, and a round-trip test.
- [ ] Property: `apply(changeset)` then `revert` restores the model exactly; `apply` is one undo unit.
- [ ] IPC integration tests: request/response, subscription events, two concurrent clients, malformed input rejected without crashing, socket permissions.
- [ ] `--dry-run` leaves the workbook and file byte-identical.

## Tests

- Schema validation tests; `proptest` for changesets; socket integration tests.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-07.md` before writing code.
4. Create branch `wp/07-command-bus-ipc`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-07: Command bus, changesets, events, and IPC`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
