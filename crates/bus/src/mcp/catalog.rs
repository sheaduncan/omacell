//! Frozen MCP tool table (spec A-5.2). Generates `docs/mcp.md`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Catalog envelope version. Frozen at WP-21 merge.
pub const SCHEMA: u32 = 1;

/// One MCP tool.
#[derive(Clone, Copy, Debug)]
pub struct ToolSpec {
    /// Wire name (`range_read`).
    pub name: &'static str,
    /// One-line documentation.
    pub doc: &'static str,
    /// JSON Schema for arguments.
    pub schema: fn() -> Value,
}

fn schema_of<T: JsonSchema>() -> Value {
    let schema = schemars::schema_for!(T);
    serde_json::to_value(schema).unwrap_or_else(|_| serde_json::json!({"type": "object"}))
}

/// `workbook_open`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkbookOpenArgs {
    /// Path to open.
    pub path: String,
}

/// `workbook_save`
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkbookSaveArgs {
    /// Destination; default is the path from `workbook_open`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// `workbook_list` takes no arguments.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyArgs {}

/// `sheet_add`
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetAddToolArgs {
    /// Sheet name. Generated when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Apply immediately (denied for external agents).
    #[serde(default)]
    pub apply: bool,
}

/// `sheet_rename`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetRenameToolArgs {
    /// Current name.
    pub sheet: String,
    /// New name.
    pub name: String,
    /// Apply immediately (denied for external agents).
    #[serde(default)]
    pub apply: bool,
}

/// `range_read`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeReadArgs {
    /// A1 range (`Sheet!A1:D20`).
    pub range: String,
    /// `values`, `formulas`, `formats`. Default is all three.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
    /// Row offset into the range.
    #[serde(default)]
    pub offset: u32,
    /// Max rows to return (default 256, max 1024).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `range_write`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeWriteArgs {
    /// A1 range.
    pub range: String,
    /// Row-major formula-bar values. `null` clears a cell.
    pub values: Vec<Vec<Option<String>>>,
    /// Apply immediately (denied for external agents).
    #[serde(default)]
    pub apply: bool,
}

/// `formula_set`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormulaSetArgs {
    /// A1 cell.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// Formula source (`=SUM(A1:A10)` or formula-bar text).
    pub formula: String,
    /// Apply immediately (denied for external agents).
    #[serde(default)]
    pub apply: bool,
}

/// `command_run`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandRunArgs {
    /// Registry command id.
    pub id: String,
    /// JSON arguments.
    #[serde(default)]
    pub args: Value,
    /// Apply immediately when the command is mutating (denied for external agents).
    #[serde(default)]
    pub apply: bool,
}

/// `recalc`
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecalcArgs {
    /// Wait for async AI cells to settle (no-op until WP-22).
    #[serde(default)]
    pub wait: bool,
}

/// `changeset_propose`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangesetProposeArgs {
    /// Ordered command-bus calls.
    pub commands: Vec<CommandCallArgs>,
}

/// One command in a proposed batch.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandCallArgs {
    /// Registry id.
    pub id: String,
    /// JSON arguments.
    #[serde(default)]
    pub args: Value,
}

/// `changeset_apply` / `changeset_revert` / `changeset_show`-style id.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangesetIdArgs {
    /// Changeset id.
    pub id: String,
}

/// `export`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportArgs {
    /// Destination path (extension selects format).
    pub path: String,
    /// Sheet name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    /// A1 range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
}

/// `render`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenderArgs {
    /// A1 range to rasterize.
    pub range: String,
}

/// `card`
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CardArgs {}

/// Every MCP tool, sorted by name. Tests freeze this table.
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "audit",
        doc: "Run the deterministic workbook audit (`omacell audit --json`).",
        schema: schema_of::<EmptyArgs>,
    },
    ToolSpec {
        name: "card",
        doc: "Workbook card. WP-21 returns summary level; WP-22 replaces the payload.",
        schema: schema_of::<CardArgs>,
    },
    ToolSpec {
        name: "changeset_apply",
        doc: "Apply a proposed changeset. External agents are denied; use the CLI.",
        schema: schema_of::<ChangesetIdArgs>,
    },
    ToolSpec {
        name: "changeset_list",
        doc: "List stored changesets for this session.",
        schema: schema_of::<EmptyArgs>,
    },
    ToolSpec {
        name: "changeset_propose",
        doc: "Propose an ordered list of command-bus calls without mutating live state.",
        schema: schema_of::<ChangesetProposeArgs>,
    },
    ToolSpec {
        name: "changeset_revert",
        doc: "Revert an applied changeset. External agents are denied; use the CLI.",
        schema: schema_of::<ChangesetIdArgs>,
    },
    ToolSpec {
        name: "command_run",
        doc: "Invoke any public registry command. Mutating calls default to a changeset proposal.",
        schema: schema_of::<CommandRunArgs>,
    },
    ToolSpec {
        name: "commands_list",
        doc: "The command-bus catalog (`omacell commands --json`).",
        schema: schema_of::<EmptyArgs>,
    },
    ToolSpec {
        name: "export",
        doc: "Export the open workbook (`file.export`).",
        schema: schema_of::<ExportArgs>,
    },
    ToolSpec {
        name: "formula_set",
        doc: "Set one cell's formula. Defaults to proposing a changeset.",
        schema: schema_of::<FormulaSetArgs>,
    },
    ToolSpec {
        name: "range_read",
        doc: "Read values, formulas, and/or formats from an A1 range (paginated by row).",
        schema: schema_of::<RangeReadArgs>,
    },
    ToolSpec {
        name: "range_write",
        doc: "Write formula-bar values into an A1 range. Defaults to proposing a changeset.",
        schema: schema_of::<RangeWriteArgs>,
    },
    ToolSpec {
        name: "recalc",
        doc: "Recalculate the workbook. `wait` is reserved for async AI cells (WP-22).",
        schema: schema_of::<RecalcArgs>,
    },
    ToolSpec {
        name: "render",
        doc: "Rasterize a range. Headless servers return 'GUI not running'.",
        schema: schema_of::<RenderArgs>,
    },
    ToolSpec {
        name: "sheet_add",
        doc: "Add a worksheet. Defaults to proposing a changeset.",
        schema: schema_of::<SheetAddToolArgs>,
    },
    ToolSpec {
        name: "sheet_list",
        doc: "List worksheet names in the open workbook.",
        schema: schema_of::<EmptyArgs>,
    },
    ToolSpec {
        name: "sheet_rename",
        doc: "Rename a worksheet. Defaults to proposing a changeset.",
        schema: schema_of::<SheetRenameToolArgs>,
    },
    ToolSpec {
        name: "workbook_list",
        doc: "List workbooks open in this MCP session.",
        schema: schema_of::<EmptyArgs>,
    },
    ToolSpec {
        name: "workbook_open",
        doc: "Open a workbook from disk, replacing the current session workbook.",
        schema: schema_of::<WorkbookOpenArgs>,
    },
    ToolSpec {
        name: "workbook_save",
        doc: "Save the open workbook.",
        schema: schema_of::<WorkbookSaveArgs>,
    },
];

/// Tool names in catalog order.
#[must_use]
pub fn tool_names() -> Vec<&'static str> {
    TOOLS.iter().map(|t| t.name).collect()
}

/// JSON Schema for one tool, or `None` if unknown.
#[must_use]
pub fn schema_for_tool(name: &str) -> Option<Value> {
    TOOLS.iter().find(|t| t.name == name).map(|t| (t.schema)())
}

/// Frozen catalog envelope written to `docs/schemas/mcp.schema.json`.
#[must_use]
pub fn catalog_json() -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.doc,
                "inputSchema": (t.schema)(),
            })
        })
        .collect();
    serde_json::json!({
        "schema": SCHEMA,
        "tools": tools,
        "resources": crate::mcp::uri::templates_json(),
    })
}

/// Markdown reference generated from [`TOOLS`].
#[must_use]
pub fn render_markdown() -> String {
    let mut out = String::from("# MCP tools\n\n");
    out.push_str("Generated from `omacell_bus::mcp::TOOLS`. Do not edit by hand.\n\n");
    out.push_str("Write tools default to proposing a changeset. `apply=true` is denied for external agents; apply from `omacell changeset apply`.\n\n");
    for tool in TOOLS {
        out.push_str(&format!("## `{}`\n\n{}\n\n", tool.name, tool.doc));
    }
    out.push_str("## Resources\n\n");
    out.push_str("- `omacell://<file>/card` — workbook card (summary until WP-22)\n");
    out.push_str("- `omacell://<file>/<sheet>` — sheet summary\n");
    out
}
