//! Recorder emits Lua that replays to an identical model.

use omacell_bus::Bus;
use omacell_core::command::Origin;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_lua::{BusHost, Profile, Recorder, Runtime, replay_lua};
use serde_json::json;

fn bus() -> Bus {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    Bus::new(Workbook::new(), RecalcEngine::new(registry)).unwrap()
}

fn cell(bus: &Bus, row: u32, col: u16) -> Option<Value> {
    bus.workbook()
        .get(bus.workbook().active_sheet(), row, col)
        .ok()
        .flatten()
        .map(|s| s.value)
}

#[test]
fn recorded_session_replays_to_identical_model() {
    let mut rec = Recorder::new();
    rec.start();
    rec.push("cell.set", json!({"ref": "A1", "input": "2"}));
    rec.push("cell.set", json!({"ref": "B1", "input": "=A1*5"}));
    rec.stop();
    let lua = rec.to_lua();
    assert!(lua.contains("omacell.cmd(\"cell.set\""));

    let mut live = bus();
    for (id, args) in rec.steps() {
        let out = live.execute(Origin::User, id, args.clone());
        assert!(out.ok, "{:?}", out.error);
    }

    let mut replayed = bus();
    replay_lua(&lua, |id, args| {
        let out = replayed.execute(Origin::Script, id, args);
        assert!(out.ok, "{:?}", out.error);
        Ok(out.result.unwrap_or(serde_json::Value::Null))
    })
    .unwrap();

    assert_eq!(cell(&live, 0, 0), cell(&replayed, 0, 0));
    assert_eq!(cell(&live, 0, 1), cell(&replayed, 0, 1));

    let host = BusHost::new(bus());
    let rt = Runtime::new(Profile::User, Box::new(host)).unwrap();
    rt.exec(&lua, "macro.lua").unwrap();
}
