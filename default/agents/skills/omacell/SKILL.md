---
name: omacell
description: >
  Operate Omacell workbooks through the CLI, MCP server, and command bus.
  Use when the user asks to inspect, query, edit, audit, recalc, export, or
  reconcile spreadsheet files (.xlsx, .csv, .omc) with Omacell, or when
  wiring an MCP client to `omacell mcp`.
---

# Omacell

Omacell is a spreadsheet for Omarchy Linux. Agents inspect and edit workbooks
through the **CLI** and the **MCP server**. Mutations go through the command
bus as **reviewable changesets**. Do not write `.xlsx` bytes yourself while a
GUI or TUI may have the file open.

## When to use

- Open, query, or edit `.xlsx` / `.csv` / `.omc` with Omacell
- Audit a workbook for `#REF!`, hardcoded constants, or circular references
- Propose edits for the user to apply from `omacell changeset apply`
- Hand a workbook to the user's default Omarchy agent

## CLI cheatsheet

Discover flags from help; this list is kept in lock-step by a drift test.

```bash
omacell --help
omacell query <BOOK> <RANGE> --format json
omacell query <BOOK> <RANGE> --formulas
omacell set <BOOK> <RANGE> <VALUE>
omacell eval <BOOK> <FORMULA>
omacell recalc <BOOK> --wait
omacell recalc <BOOK> --write
omacell audit <BOOK> --json
omacell commands --json
omacell diff <A> <B>
omacell convert <INPUT> <OUTPUT>
omacell changeset list
omacell changeset show <ID>
omacell changeset apply <ID>
omacell changeset discard <ID>
omacell changeset revert <ID>
omacell changeset export <ID> --omc <FILE>
omacell mcp
omacell mcp --socket <PATH>
omacell agent "<prompt>"
omacell agent --book <BOOK> --selection <RANGE> "<prompt>"
omacell agent diagnose
omacell agent diagnose --pid <PID> --book <BOOK>
omacell setup omarchy
omacell ipc ping
```

`--json` is global. `--dry-run` validates writes without changing files.

## MCP tools

Register with `claude mcp add omacell -- omacell mcp` (or the equivalent for
the user's harness). Tools:

| Tool | Role |
|---|---|
| `workbook_open` / `workbook_list` | Session files (`workbook_save` is denied; user saves) |
| `sheet_list` / `sheet_add` / `sheet_rename` | Structure |
| `range_read` | Values, formulas, formats; paginate with `offset` / `limit` |
| `range_write` / `formula_set` / `command_run` | Edits (default: **propose**) |
| `commands_list` | Same catalog as `omacell commands --json` |
| `recalc` | Live workbook only; denied while a proposal is pending |
| `audit` | Same report as `omacell audit --json` |
| `card` | Privacy-filtered summary-level workbook card |
| `changeset_propose` / `changeset_list` | Review queue |
| `changeset_apply` / `changeset_revert` | Denied for agents; user applies or discards from the CLI |
| `export` | Denied; user exports from the CLI/GUI |
| `render` | GUI only; headless returns `GUI not running` |

Resources: `omacell://<file>/card`, `omacell://<file>/<sheet>`.

Write tools default to proposing a changeset. Do **not** set `apply=true`.
Apply is a deliberate user step.

## Propose → review → apply

1. Inspect with `range_read`, `query`, `card`, or `audit`.
2. Propose with `range_write` / `formula_set` / `changeset_propose` (or
   `omacell ipc` against a running instance, which already proposes).
3. Tell the user to review: `omacell changeset list` then
   `omacell changeset apply <ID>` or `omacell changeset discard <ID>`
   (or the GUI review overlay).
4. To undo an applied changeset: `omacell changeset revert <ID>`.

A headless `omacell mcp` process is a live instance: `omacell changeset list`
talks to it over IPC.

## Done checklist

Before declaring a workbook task done:

1. `omacell recalc <BOOK> --wait`
2. `omacell audit <BOOK> --json` — fix or report remaining findings
3. `omacell diff` against the original (or `changeset show`) so the user can
   see what changed

## Pitfalls

- **No direct file writes** while a GUI has the workbook open. Respect lock
  files; mutate through the command bus / MCP / `omacell ipc`.
- **Do not edit** `~/.config/omarchy` or `/usr/share/omarchy` from this skill.
- Embedded scripts and trust (`omacell trust`) are user decisions, not agent
  tools.
- Use `omacell ai setup|card|plan|log|usage` for provider and planning tasks;
  workbook mutations still cross the normal changeset-review boundary.
