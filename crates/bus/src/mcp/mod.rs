//! MCP tool catalog, resource URIs, and a sync session over [`crate::Bus`].
//!
//! The `rmcp` transport (stdio / Unix socket) lives in `omacell-cli` so this
//! crate stays synchronous. Tool names, argument schemas, and resource URI
//! templates freeze with WP-21 (`docs/schemas/mcp.schema.json`).

mod catalog;
mod session;
mod uri;

pub use catalog::{
    SCHEMA, TOOLS, ToolSpec, catalog_json, render_markdown, schema_for_tool, tool_names,
};
pub use session::{
    CardHook, DEFAULT_PAGE_ROWS, MAX_MCP_JSON_BYTES, MAX_MCP_JSON_DEPTH, MAX_PAGE_ROWS, McpCtx,
    McpSession, ProposeHook, stub_card,
};
pub use uri::{
    RESOURCE_TEMPLATES, ResourceKind, card_uri, parse_resource_uri, sheet_uri, templates_json,
};
