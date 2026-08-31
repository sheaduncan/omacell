# MCP tools

Generated from `omacell_bus::mcp::TOOLS`. Do not edit by hand.

Write tools default to proposing a changeset. `apply=true` is denied for external agents; apply from `omacell changeset apply`.

## `audit`

Run the deterministic workbook audit (`omacell audit --json`).

## `card`

Workbook card. WP-21 returns summary level; WP-22 replaces the payload.

## `changeset_apply`

Apply a proposed changeset. External agents are denied; use the CLI.

## `changeset_list`

List stored changesets for this session.

## `changeset_propose`

Propose an ordered list of command-bus calls without mutating live state.

## `changeset_revert`

Revert an applied changeset. External agents are denied; use the CLI.

## `command_run`

Invoke any public registry command. Mutating calls default to a changeset proposal.

## `commands_list`

The command-bus catalog (`omacell commands --json`).

## `export`

Export the open workbook (`file.export`).

## `formula_set`

Set one cell's formula. Defaults to proposing a changeset.

## `range_read`

Read values, formulas, and/or formats from an A1 range (paginated by row).

## `range_write`

Write formula-bar values into an A1 range. Defaults to proposing a changeset.

## `recalc`

Recalculate the workbook. `wait` is reserved for async AI cells (WP-22).

## `render`

Rasterize a range. Headless servers return 'GUI not running'.

## `sheet_add`

Add a worksheet. Defaults to proposing a changeset.

## `sheet_list`

List worksheet names in the open workbook.

## `sheet_rename`

Rename a worksheet. Defaults to proposing a changeset.

## `workbook_list`

List workbooks open in this MCP session.

## `workbook_open`

Open a workbook from disk, replacing the current session workbook.

## `workbook_save`

Save the open workbook.

## Resources

- `omacell://<file>/card` — workbook card (summary until WP-22)
- `omacell://<file>/<sheet>` — sheet summary
