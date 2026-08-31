//! MCP session contract: every tool/resource, pagination, and error cases.

use omacell_bus::mcp::{
    McpCtx, McpSession, TOOLS, catalog_json, parse_resource_uri, render_markdown, stub_card,
    tool_names,
};
use omacell_bus::{
    Bus, CommandKind, CommandSpec, Effect, Exposure, codes, register_audit_commands,
};
use omacell_core::command::Origin;
use omacell_core::eval::FnRegistry;
use omacell_core::event::Event;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use serde_json::{Value, json};

fn bus() -> Bus {
    let mut bus = Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())).unwrap();
    register_audit_commands(bus.registry_mut()).unwrap();
    bus
}

fn call(
    bus: &mut Bus,
    ctx: &mut McpCtx,
    tool: &str,
    args: Value,
) -> Result<Value, omacell_core::error::CoreError> {
    McpSession::call(bus, ctx, tool, args)
}

fn call_ok(bus: &mut Bus, ctx: &mut McpCtx, tool: &str, args: Value) -> Value {
    call(bus, ctx, tool, args).unwrap_or_else(|err| panic!("{tool} failed: {err:?}"))
}

#[test]
fn catalog_is_sorted_complete_and_matches_docs() {
    let names = tool_names();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    for required in [
        "workbook_open",
        "workbook_list",
        "workbook_save",
        "sheet_list",
        "sheet_add",
        "sheet_rename",
        "range_read",
        "range_write",
        "formula_set",
        "command_run",
        "commands_list",
        "recalc",
        "audit",
        "card",
        "changeset_propose",
        "changeset_apply",
        "changeset_revert",
        "changeset_list",
        "export",
        "render",
    ] {
        assert!(names.contains(&required), "missing tool {required}");
    }
    let catalog = catalog_json();
    assert_eq!(catalog["schema"], 1);
    assert_eq!(catalog["tools"].as_array().unwrap().len(), TOOLS.len());
    let docs =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/mcp.md")).unwrap();
    assert_eq!(docs, render_markdown());
    let schema_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/schemas/mcp.schema.json"
    );
    let expected = serde_json::to_string_pretty(&catalog).unwrap() + "\n";
    if std::env::var("UPDATE_MCP_SCHEMA").is_ok() {
        std::fs::write(schema_path, &expected).unwrap();
    }
    let schema = std::fs::read_to_string(schema_path).unwrap();
    assert_eq!(schema, expected);
}

#[test]
fn every_tool_and_resource_against_fixture() {
    let mut bus = bus();
    let mut ctx = McpCtx {
        open_path: Some("/workbooks/fixture.xlsx".into()),
        ..McpCtx::default()
    };
    bus.execute(Origin::User, "cell.set", json!({"ref": "A1", "input": "1"}));
    bus.execute(
        Origin::User,
        "cell.set",
        json!({"ref": "A2", "input": "=A1+1"}),
    );

    let sheets = call_ok(&mut bus, &mut ctx, "sheet_list", json!({}));
    assert!(
        sheets["sheets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == "Sheet1")
    );

    let listed = call_ok(&mut bus, &mut ctx, "workbook_list", json!({}));
    assert_eq!(listed["files"][0], "/workbooks/fixture.xlsx");

    let read = call_ok(
        &mut bus,
        &mut ctx,
        "range_read",
        json!({"range": "Sheet1!A1:A2"}),
    );
    assert_eq!(read["rows"].as_array().unwrap().len(), 2);
    assert_eq!(read["rows"][0][0]["value"], "1");
    assert_eq!(read["rows"][1][0]["formula"], "=A1+1");

    let page = call_ok(
        &mut bus,
        &mut ctx,
        "range_read",
        json!({"range": "A1:A2", "offset": 1, "limit": 1, "fields": ["values"]}),
    );
    assert_eq!(page["rows"].as_array().unwrap().len(), 1);
    assert_eq!(page["truncated"], false);
    assert!(page["rows"][0][0].get("formula").is_none());

    let commands = call_ok(&mut bus, &mut ctx, "commands_list", json!({}));
    assert!(commands.get("commands").is_some() || commands.get("schema").is_some());

    let card = call_ok(&mut bus, &mut ctx, "card", json!({}));
    assert_eq!(card["kind"], "summary");
    assert_eq!(card["schema"], 1);

    let audit = call_ok(&mut bus, &mut ctx, "audit", json!({}));
    assert!(audit.get("findings").is_some());

    let recalc = call_ok(&mut bus, &mut ctx, "recalc", json!({"wait": true}));
    assert_eq!(recalc["wait"], true);
    assert_eq!(recalc["settled"], true);

    let proposed = call_ok(
        &mut bus,
        &mut ctx,
        "range_write",
        json!({"range": "B1:B1", "values": [["9"]]}),
    );
    assert_eq!(proposed["status"], "proposed");
    assert_eq!(
        bus.workbook()
            .get(bus.workbook().active_sheet(), 0, 1)
            .unwrap(),
        None,
        "propose must not mutate live cells"
    );

    let formula = call_ok(
        &mut bus,
        &mut ctx,
        "formula_set",
        json!({"ref": "C1", "formula": "A1*2"}),
    );
    assert_eq!(formula["status"], "proposed");

    let add = call_ok(&mut bus, &mut ctx, "sheet_add", json!({"name": "Data"}));
    assert_eq!(add["status"], "proposed");

    let rename = call_ok(
        &mut bus,
        &mut ctx,
        "sheet_rename",
        json!({"sheet": "Sheet1", "name": "Main"}),
    );
    assert_eq!(rename["status"], "proposed");

    let run = call_ok(
        &mut bus,
        &mut ctx,
        "command_run",
        json!({"id": "cell.set", "args": {"ref": "D1", "input": "x"}}),
    );
    assert_eq!(run["status"], "proposed");

    let batch = call_ok(
        &mut bus,
        &mut ctx,
        "changeset_propose",
        json!({"commands": [{"id": "cell.set", "args": {"ref": "E1", "input": "1"}}]}),
    );
    let id = batch["id"].as_str().unwrap();
    let list = call_ok(&mut bus, &mut ctx, "changeset_list", json!({}));
    assert!(list.as_array().unwrap().iter().any(|cs| cs["id"] == id));

    let resources = McpSession::list_resources(&bus, &ctx);
    assert!(resources.iter().any(|r| r["name"] == "card"));
    let card_uri = resources.iter().find(|r| r["name"] == "card").unwrap()["uri"]
        .as_str()
        .unwrap()
        .to_string();
    let sheet_uri = resources.iter().find(|r| r["name"] == "Sheet1").unwrap()["uri"]
        .as_str()
        .unwrap()
        .to_string();
    let card_res = McpSession::read_resource(&bus, &ctx, &card_uri).unwrap();
    assert_eq!(card_res["kind"], "summary");
    let sheet_res = McpSession::read_resource(&bus, &ctx, &sheet_uri).unwrap();
    assert_eq!(sheet_res["name"], "Sheet1");
}

#[test]
fn errors_unknown_tool_bad_args_pagination_and_policy() {
    let mut bus = bus();
    let mut ctx = McpCtx::default();
    let err = call(&mut bus, &mut ctx, "nope", json!({})).unwrap_err();
    assert_eq!(err.code, codes::MCP_UNKNOWN);

    let err = call(
        &mut bus,
        &mut ctx,
        "range_read",
        json!({"range": "A1", "fields": ["nope"]}),
    )
    .unwrap_err();
    assert_eq!(err.code, codes::MCP_ARGS);

    let err = call(&mut bus, &mut ctx, "range_read", json!({"extra": true})).unwrap_err();
    assert_eq!(err.code, codes::MCP_ARGS);

    let err = call(&mut bus, &mut ctx, "card", json!({"extra": true})).unwrap_err();
    assert_eq!(err.code, codes::MCP_ARGS);

    let err = call(
        &mut bus,
        &mut ctx,
        "range_write",
        json!({"range": "A1", "values": [["1"]], "apply": true}),
    )
    .unwrap_err();
    assert_eq!(err.code, codes::COMMAND_DENIED);

    let proposed = call_ok(
        &mut bus,
        &mut ctx,
        "range_write",
        json!({"range": "A1", "values": [["1"]]}),
    );
    let id = proposed["id"].as_str().unwrap();
    let err = call(&mut bus, &mut ctx, "changeset_apply", json!({"id": id})).unwrap_err();
    assert_eq!(err.code, codes::COMMAND_DENIED);

    let err = call(&mut bus, &mut ctx, "render", json!({"range": "A1"})).unwrap_err();
    assert_eq!(err.code, codes::MCP_RENDER);
    assert!(err.message.contains("GUI not running"));

    let err = call(
        &mut bus,
        &mut ctx,
        "command_run",
        json!({"id": "edit.undo", "args": {}}),
    )
    .unwrap_err();
    assert_eq!(err.code, codes::COMMAND_DENIED);

    parse_resource_uri("http://x").unwrap_err();
}

#[test]
fn card_named_sheet_has_a_distinct_resource_uri() {
    let mut bus = bus();
    let renamed = bus.execute(
        Origin::User,
        "sheet.rename",
        json!({"sheet": "Sheet1", "name": "card"}),
    );
    assert!(renamed.ok, "{:?}", renamed.error);
    let ctx = McpCtx {
        open_path: Some("book.xlsx".into()),
        ..McpCtx::default()
    };
    let resources = McpSession::list_resources(&bus, &ctx);
    assert_eq!(resources.len(), 2);
    assert_ne!(resources[0]["uri"], resources[1]["uri"]);
    let sheet_uri = resources[1]["uri"].as_str().unwrap();
    assert!(sheet_uri.ends_with("/%63ard"));
    let sheet = McpSession::read_resource(&bus, &ctx, sheet_uri).unwrap();
    assert_eq!(sheet["name"], "card");
}

#[test]
fn opening_a_workbook_discards_changesets_from_the_previous_workbook() {
    let mut bus = bus();
    bus.registry_mut()
        .register::<omacell_bus::args::EmptyArgs, _>(
            CommandSpec {
                id: "file.open",
                doc: "test workbook open",
                kind: CommandKind::Mutating,
                changeset_eligible: false,
                exposure: Exposure::Public,
                default_keys: &[],
            },
            |_ctx, _args| {
                Ok(Effect {
                    events: vec![Event::WorkbookOpened {
                        path: Some("next.xlsx".into()),
                    }],
                    ..Effect::default()
                })
            },
        )
        .unwrap();
    let proposed = bus
        .propose(
            Origin::ExternalAgent,
            vec![omacell_core::changeset::CommandCall {
                id: omacell_core::command::CommandId::new("cell.set").unwrap(),
                args: json!({"ref": "A1", "input": "old"}),
            }],
        )
        .unwrap();
    assert_eq!(
        proposed.status,
        omacell_core::changeset::ChangesetStatus::Proposed
    );
    assert_eq!(bus.list_changesets().len(), 1);
    let opened = bus.execute(Origin::User, "file.open", json!({}));
    assert!(opened.ok, "{:?}", opened.error);
    assert!(bus.list_changesets().is_empty());
}

#[test]
fn range_read_caps_serialized_page_bytes_without_stalling_pagination() {
    let mut bus = bus();
    let value = "x".repeat(32_768);
    let values: Vec<Vec<Option<String>>> = (0..40).map(|_| vec![Some(value.clone())]).collect();
    let written = bus.execute(
        Origin::User,
        "range.set",
        json!({"range": "A1:A40", "values": values}),
    );
    assert!(written.ok, "{:?}", written.error);
    let mut ctx = McpCtx::default();
    let first = call_ok(
        &mut bus,
        &mut ctx,
        "range_read",
        json!({"range": "A1:A40", "fields": ["values"], "limit": 40}),
    );
    let returned = first["rows"].as_array().unwrap().len();
    assert!(returned > 0 && returned < 40);
    assert_eq!(first["truncated"], true);
    let second = call_ok(
        &mut bus,
        &mut ctx,
        "range_read",
        json!({
            "range": "A1:A40",
            "fields": ["values"],
            "offset": returned,
            "limit": 40
        }),
    );
    assert_eq!(second["rows"].as_array().unwrap().len(), 40 - returned);
    assert_eq!(second["truncated"], false);
}

#[test]
fn stub_card_is_summary_level() {
    let wb = Workbook::new();
    let card = stub_card(&wb, Some("book.xlsx"));
    assert_eq!(card["kind"], "summary");
    assert_eq!(card["file"], "book.xlsx");
    assert!(card.get("sample").is_none());
}
