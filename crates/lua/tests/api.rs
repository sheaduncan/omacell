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
        r##"
        local result = omacell.cmd("cell.set", {ref = "A1", input = "2"})
        assert(result.changed == 1)
        omacell.cmd("cell.set", {ref = "B1", input = "=A1*3"})
        local c = omacell.book():sheet():cell("B1")
        assert(omacell.book():sheet():name() == "Sheet1")
        assert(c.input == "=A1*3")
        assert(c.formula == "=A1*3")
        c:set_style({bold = true, wrap = true})
        assert(c.style.font.bold == true)
        assert(c.style.alignment.wrap == true)
        c:set("9")
        omacell.ui.status("ok")
        omacell.ui.notify("n")
        omacell.keymap.set("normal", "gss", "range.sort")
        omacell.on_open(function() end)
        omacell.on_change(function() end)
        omacell.on_before_save(function() end)
        omacell.on_recalc(function() end)
        omacell.on_theme_change(function() end)
        "##,
        "api.lua",
    )
    .unwrap();
}

#[test]
fn command_arguments_preserve_nested_arrays() {
    run(r#"
        local result = omacell.cmd("range.set", {
            range = "A1:B2",
            values = {{"1", "2"}, {"3", "4"}},
        })
        assert(result.changed == 4)
        assert(omacell.book():sheet():cell("B2").value == 4)
        "#);
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
        assert(omacell.book():sheet():cell("B1").value == 8)
        "#,
        "fn.lua",
    )
    .unwrap();
}

#[test]
fn custom_functions_support_text_and_array_results() {
    run(r#"
        omacell.fn("USER.ECHO", {min = 1, max = 1}, function(x) return x end)
        omacell.fn("USER.PAIR", {min = 1, max = 1}, function(x) return {x, x * 2} end)
        omacell.cmd("cell.set", {ref = "A1", input = "hello"})
        omacell.cmd("cell.set", {ref = "B1", input = "=USER.ECHO(A1)"})
        omacell.cmd("cell.set", {ref = "C1", input = "=INDEX(USER.PAIR(3),1,2)"})
        assert(omacell.book():sheet():cell("B1").value == "hello")
        assert(omacell.book():sheet():cell("C1").value == 6)
        "#);
}

#[test]
fn custom_functions_cannot_reenter_mutating_host_apis() {
    run(r##"
        omacell.fn("USER.BAD", {min = 0, max = 0}, function()
            omacell.cmd("cell.set", {ref = "A1", input = "reentrant"})
            return 1
        end)
        omacell.cmd("cell.set", {ref = "B1", input = "=USER.BAD()"})
        assert(omacell.book():sheet():cell("B1").value == "#VALUE!")
        assert(omacell.book():sheet():cell("A1").value == nil)
        "##);
}

#[test]
fn custom_function_specs_fail_closed() {
    let host = BusHost::new(bus());
    let rt = Runtime::new(Profile::User, Box::new(host)).unwrap();
    for source in [
        r#"omacell.fn("BAD", {}, function() end)"#,
        r#"omacell.fn("USER.BAD", {min = "one"}, function() end)"#,
        r#"omacell.fn("USER.BAD", {min = 2, max = 1}, function() end)"#,
        r#"omacell.fn("USER.BAD", {array_lift = "maybe"}, function() end)"#,
        r#"omacell.fn("USER.BAD", {surprise = true}, function() end)"#,
    ] {
        assert!(rt.exec(source, "bad-spec.lua").is_err(), "{source}");
    }
}

#[test]
fn event_hooks_keep_every_registered_handler() {
    let rt = run(r#"
        count = 0
        omacell.on_open(function() count = count + 1 end)
        omacell.on_open(function() count = count + 10 end)
        "#);
    rt.emit_hook("on_open").unwrap();
    rt.exec("assert(count == 11)", "assert-hook.lua").unwrap();
}

#[test]
fn command_events_dispatch_to_registered_hooks() {
    run(r#"
        changes = 0
        recalcs = 0
        omacell.on_change(function() changes = changes + 1 end)
        omacell.on_recalc(function() recalcs = recalcs + 1 end)
        omacell.cmd("cell.set", {ref = "A1", input = "2"})
        omacell.book():sheet():cell("A2"):set("3")
        assert(changes == 2)
        assert(recalcs == 2)
        "#);
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
fn sheet_objects_resolve_names_and_range_iteration_is_bounded() {
    let host = BusHost::new(bus());
    let rt = Runtime::new(Profile::User, Box::new(host)).unwrap();
    rt.exec(
        r#"
        omacell.cmd("sheet.add", {name = "Data Sheet"})
        local sheet = omacell.book():sheet("data sheet")
        assert(sheet:name() == "Data Sheet")
        sheet:cell("A1"):set("7")
        sheet:cell("A2"):set("8")
        local total = 0
        for _, cell in ipairs(sheet:range("A1:A2"):cells()) do
            total = total + cell.value
        end
        assert(total == 15)
        "#,
        "sheet-names.lua",
    )
    .unwrap();
    let err = rt
        .exec(
            r#"omacell.book():sheet("Data Sheet"):range("A:XFD"):cells()"#,
            "huge-range.lua",
        )
        .unwrap_err();
    assert!(err.message.contains("maximum is 100000"), "{err:?}");
    assert!(
        rt.exec(r#"omacell.book():sheet("missing")"#, "missing-sheet.lua")
            .is_err()
    );
}

#[test]
fn ai_hooks_register() {
    let host = BusHost::new(bus());
    let rt = Runtime::new(Profile::User, Box::new(host)).unwrap();
    rt.exec("omacell.ai.task('summarize', {prompt = 'x'})", "ai.lua")
        .unwrap();
    rt.exec("omacell.ai.fn('MY.AI', {prompt = 'y'})", "ai.lua")
        .unwrap();
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
