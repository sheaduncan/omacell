# Lua API

Generated from `omacell_lua::catalog::API`. Do not edit by hand.

## Runtime profiles

User-profile scripts have the documented API and Lua standard library. GUI/TUI load trusted `init.lua` and sorted plugin entry points once at startup; only the explicit `script.source` command reloads them, replacing hooks, script keymaps, and custom functions. Filesystem notifications never execute scripts, and workbook-embedded scripts never run on open. Interactive worksheet callbacks preserve their Lua closure during normal calculation; calculation that overlaps a running hook uses an isolated fallback, so callbacks used on that path must be self-contained. Embedded workbook scripts run with a strict capability set: `io`, `os`, `package`, `debug`, `require`, dynamic loading, coroutines, `pcall`, and `xpcall` are unavailable. Protected calls are deliberately removed so the hard instruction-budget error cannot be caught. Embedded scripts also cannot prompt, change keymaps, register AI extensions or AI payload hooks, and `omacell.cmd` accepts only a fixed, reviewed workbook-command allowlist. New commands remain unavailable until explicitly reviewed. In both profiles, `print(...)` writes to the Omacell status sink instead of stdout.

## `omacell.cmd`

`omacell.cmd(id, args) -> result`

Invoke a command-bus command with a Lua table of JSON arguments.

## `omacell.book`

`omacell.book() -> book`

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

## `sheet:name`

`sheet:name() -> string`

The resolved worksheet name.

## `cell.value`

`cell.value -> number|string|boolean|nil`

Evaluated cell value.

## `cell.input`

`cell.input -> string`

Formula-bar text.

## `cell.formula`

`cell.formula -> string|nil`

Formula source including the leading `=`, or nil for a literal cell.

## `cell.style`

`cell.style -> table`

The cell's font, fill, border, alignment, protection, and number-format ids.

## `cell:set`

`cell:set(input)`

Set formula-bar text via `cell.set`.

## `cell:set_style`

`cell:set_style(patch)`

Patch cell style through the `style.set` command.

## `range:cells`

`range:cells() -> cell[]`

Return cell objects in row-major order (iterate with `ipairs`).

## `omacell.fn`

`omacell.fn(name, spec, fn)`

Register a namespaced custom function (`USER.NAME`) on the calc registry.

## `omacell.keymap.set`

`omacell.keymap.set(mode, keys, cmd)`

Bind a chord in a UI mode to a registered command id.

## `omacell.ui.prompt`

`omacell.ui.prompt(message) -> string`

Prompt the user; CLI reads a line, while GUI/TUI currently return `lua.prompt`.

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

`omacell.ai.task(name, spec)`

Reserve named AI-task metadata in this user-script runtime; dispatch is not implemented.

## `omacell.ai.fn`

`omacell.ai.fn(name, spec)`

Reserve an AI worksheet-function name; calls currently return `#N/A`.

## `omacell.on_ai_request`

`omacell.on_ai_request(fn)`

Reserve a pre-request hook; AI dispatch does not invoke it yet.

## `omacell.on_ai_response`

`omacell.on_ai_response(fn)`

Reserve a post-response hook; AI dispatch does not invoke it yet.
