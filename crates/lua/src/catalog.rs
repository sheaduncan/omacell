//! Registration table that generates `docs/lua-api.md`.

/// One documented API entry.
#[derive(Clone, Copy, Debug)]
pub struct ApiEntry {
    /// Dotted name (`omacell.cmd`).
    pub name: &'static str,
    /// Lua signature.
    pub signature: &'static str,
    /// One-line documentation.
    pub doc: &'static str,
}

/// Every public Lua API function. Tests assert `docs/lua-api.md` matches this table.
pub const API: &[ApiEntry] = &[
    ApiEntry {
        name: "omacell.cmd",
        signature: "omacell.cmd(id, args) -> result",
        doc: "Invoke a command-bus command with a Lua table of JSON arguments.",
    },
    ApiEntry {
        name: "omacell.book",
        signature: "omacell.book() -> book",
        doc: "The active workbook object.",
    },
    ApiEntry {
        name: "book:sheet",
        signature: "book:sheet([name]) -> sheet",
        doc: "Active sheet, or a sheet by name.",
    },
    ApiEntry {
        name: "sheet:cell",
        signature: "sheet:cell(a1) -> cell",
        doc: "A cell on this sheet (`A1`).",
    },
    ApiEntry {
        name: "sheet:range",
        signature: "sheet:range(a1) -> range",
        doc: "A range on this sheet (`A1:B2`).",
    },
    ApiEntry {
        name: "sheet:name",
        signature: "sheet:name() -> string",
        doc: "The resolved worksheet name.",
    },
    ApiEntry {
        name: "cell.value",
        signature: "cell.value -> number|string|boolean|nil",
        doc: "Evaluated cell value.",
    },
    ApiEntry {
        name: "cell.input",
        signature: "cell.input -> string",
        doc: "Formula-bar text.",
    },
    ApiEntry {
        name: "cell.formula",
        signature: "cell.formula -> string|nil",
        doc: "Formula source including the leading `=`, or nil for a literal cell.",
    },
    ApiEntry {
        name: "cell.style",
        signature: "cell.style -> table",
        doc: "The cell's font, fill, border, alignment, protection, and number-format ids.",
    },
    ApiEntry {
        name: "cell:set",
        signature: "cell:set(input)",
        doc: "Set formula-bar text via `cell.set`.",
    },
    ApiEntry {
        name: "cell:set_style",
        signature: "cell:set_style(patch)",
        doc: "Patch cell style through the `style.set` command.",
    },
    ApiEntry {
        name: "range:cells",
        signature: "range:cells() -> cell[]",
        doc: "Return cell objects in row-major order (iterate with `ipairs`).",
    },
    ApiEntry {
        name: "omacell.fn",
        signature: "omacell.fn(name, spec, fn)",
        doc: "Register a namespaced custom function (`USER.NAME`) on the calc registry.",
    },
    ApiEntry {
        name: "omacell.keymap.set",
        signature: "omacell.keymap.set(mode, keys, cmd)",
        doc: "Bind a chord in a UI mode to a command id or Lua function name.",
    },
    ApiEntry {
        name: "omacell.ui.prompt",
        signature: "omacell.ui.prompt(message) -> string",
        doc: "Prompt the user; CLI reads a line from the host.",
    },
    ApiEntry {
        name: "omacell.ui.status",
        signature: "omacell.ui.status(message)",
        doc: "Write a status-line message.",
    },
    ApiEntry {
        name: "omacell.ui.notify",
        signature: "omacell.ui.notify(message)",
        doc: "Send a desktop/status notification.",
    },
    ApiEntry {
        name: "omacell.on_open",
        signature: "omacell.on_open(fn)",
        doc: "Register a workbook-opened handler.",
    },
    ApiEntry {
        name: "omacell.on_change",
        signature: "omacell.on_change(fn)",
        doc: "Register a cell-changed handler.",
    },
    ApiEntry {
        name: "omacell.on_before_save",
        signature: "omacell.on_before_save(fn)",
        doc: "Register a before-save handler.",
    },
    ApiEntry {
        name: "omacell.on_recalc",
        signature: "omacell.on_recalc(fn)",
        doc: "Register a recalc-done handler.",
    },
    ApiEntry {
        name: "omacell.on_theme_change",
        signature: "omacell.on_theme_change(fn)",
        doc: "Register a theme-changed handler.",
    },
    ApiEntry {
        name: "omacell.ai.task",
        signature: "omacell.ai.task(...)",
        doc: "Reserved for WP-23.",
    },
    ApiEntry {
        name: "omacell.ai.fn",
        signature: "omacell.ai.fn(...)",
        doc: "Reserved for WP-23.",
    },
];

/// Render the Markdown API reference from [`API`].
#[must_use]
pub fn render_markdown() -> String {
    let mut out = String::from("# Lua API\n\n");
    out.push_str("Generated from `omacell_lua::catalog::API`. Do not edit by hand.\n\n");
    for entry in API {
        out.push_str(&format!(
            "## `{}`\n\n`{}`\n\n{}\n\n",
            entry.name, entry.signature, entry.doc
        ));
    }
    let _ = out.pop();
    out
}
