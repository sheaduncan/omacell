//! API tests for every documented Lua function.

use omacell_bus::Bus;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_lua::{BusHost, Profile, Runtime, catalog};

fn bus() -> Bus {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    Bus::new(Workbook::new(), RecalcEngine::new(registry)).unwrap()
}

fn run(source: &str) -> Runtime {
    let host = BusHost::new(bus());
    let rt = Runtime::new(Profile::User, Box::new(host)).unwrap();
    rt.exec(source, "test.lua").unwrap();
    rt
}

#[test]
fn lua_api_markdown_matches_catalog() {
    let generated = catalog::render_markdown();
    let on_disk = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/lua-api.md"),
    )
    .unwrap();
    assert_eq!(
        on_disk, generated,
        "docs/lua-api.md is stale; regenerate from catalog::API"
    );
}

#[test]
fn cmd_and_cell_objects() {
    let host = BusHost::new(bus());
    let rt = Runtime::new(Profile::User, Box::new(host)).unwrap();
    rt.exec(
        r#"
        omacell.cmd("cell.set", {ref = "A1", input = "2"})
        omacell.cmd("cell.set", {ref = "B1", input = "=A1*3"})
        local c = omacell.book():sheet():cell("B1")
        assert(c.input == "=A1*3")
        c:set("9")
        omacell.ui.status("ok")
        omacell.ui.notify("n")
        omacell.keymap.set("normal", "gss", "range.sort")
        omacell.on_open(function() end)
        omacell.on_change(function() end)
        omacell.on_before_save(function() end)
        omacell.on_recalc(function() end)
        omacell.on_theme_change(function() end)
        "#,
        "api.lua",
    )
    .unwrap();
}

#[test]
fn custom_function_joins_calc_graph() {
    let host = BusHost::new(bus());
    let rt = Runtime::new(Profile::User, Box::new(host)).unwrap();
    rt.exec(
        r#"
        omacell.fn("USER.DOUBLE", {min = 1, max = 1, array_lift = "none"}, function(x)
            return x * 2
        end)
        omacell.cmd("cell.set", {ref = "A1", input = "4"})
        omacell.cmd("cell.set", {ref = "B1", input = "=USER.DOUBLE(A1)"})
        "#,
        "fn.lua",
    )
    .unwrap();
}

#[test]
fn range_cells_iterate() {
    let rt = run(r#"
        omacell.cmd("cell.set", {ref = "A1", input = "1"})
        omacell.cmd("cell.set", {ref = "A2", input = "2"})
        local n = 0
        for _, c in ipairs(omacell.book():sheet():range("A1:A2"):cells()) do
            n = n + 1
        end
        assert(n == 2)
        "#);
    let _ = rt;
}

#[test]
fn ai_hooks_are_reserved() {
    let host = BusHost::new(bus());
    let rt = Runtime::new(Profile::User, Box::new(host)).unwrap();
    let err = rt.exec("omacell.ai.task()", "ai.lua").unwrap_err();
    assert!(err.message.contains("WP-23"), "{err:?}");
}

#[test]
fn prompt_uses_host_queue() {
    let mut host = BusHost::new(bus());
    host.ui.prompts.push("Ada".into());
    let rt = Runtime::new(Profile::User, Box::new(host)).unwrap();
    rt.exec(
        r#"
        local name = omacell.ui.prompt("who?")
        omacell.cmd("cell.set", {ref = "A1", input = name})
        "#,
        "prompt.lua",
    )
    .unwrap();
}
