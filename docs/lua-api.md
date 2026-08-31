# Lua API

Generated from `omacell_lua::catalog::API`. Do not edit by hand.

## `omacell.cmd`

`omacell.cmd(id, args) -> result`

Invoke a command-bus command with a Lua table of JSON arguments.

## `omacell.book`

`omacell.book -> book`

The active workbook object.

## `book:sheet`

`book:sheet([name]) -> sheet`

Active sheet, or a sheet by name.

## `sheet:cell`

`sheet:cell(a1) -> cell`

A cell on this sheet (`A1`).

## `sheet:range`

`sheet:range(a1) -> range`

A range on this sheet (`A1:B2`).

## `cell.value`

`cell.value -> number|string|boolean|nil`

Evaluated cell value.

## `cell.input`

`cell.input -> string`

Formula-bar text.

## `cell:set`

`cell:set(input)`

Set formula-bar text via `cell.set`.

## `range:cells`

`range:cells() -> iterator`

Iterate cells in row-major order.

## `omacell.fn`

`omacell.fn(name, spec, fn)`

Register a namespaced custom function (`USER.NAME`) on the calc registry.

## `omacell.keymap.set`

`omacell.keymap.set(mode, keys, cmd)`

Bind a chord in a UI mode to a command id or Lua function name.

## `omacell.ui.prompt`

`omacell.ui.prompt(message) -> string`

Prompt the user; CLI reads a line from the host.

## `omacell.ui.status`

`omacell.ui.status(message)`

Write a status-line message.

## `omacell.ui.notify`

`omacell.ui.notify(message)`

Send a desktop/status notification.

## `omacell.on_open`

`omacell.on_open(fn)`

Register a workbook-opened handler.

## `omacell.on_change`

`omacell.on_change(fn)`

Register a cell-changed handler.

## `omacell.on_before_save`

`omacell.on_before_save(fn)`

Register a before-save handler.

## `omacell.on_recalc`

`omacell.on_recalc(fn)`

Register a recalc-done handler.

## `omacell.on_theme_change`

`omacell.on_theme_change(fn)`

Register a theme-changed handler.

## `omacell.ai.task`

`omacell.ai.task(...)`

Reserved for WP-23.

## `omacell.ai.fn`

`omacell.ai.fn(...)`

Reserved for WP-23.

