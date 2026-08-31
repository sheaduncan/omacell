//! Fuzz smoke: MCP tool-argument JSON over the sync session.
#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_bus::mcp::{MAX_MCP_JSON_BYTES, McpCtx, McpSession, tool_names};
use omacell_bus::Bus;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_MCP_JSON_BYTES + 32 {
        return;
    }
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let mut bus = Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())).unwrap();
    let mut ctx = McpCtx::default();
    for name in tool_names() {
        let _ = McpSession::call(&mut bus, &mut ctx, name, value.clone());
    }
});
