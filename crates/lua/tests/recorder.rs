//! Recorder emits Lua that replays to an identical model.

use omacell_bus::Bus;
use omacell_core::command::Origin;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_lua::{
    BusHost, MAX_RECORDED_STEPS, Profile, Recorder, Runtime, ScriptGate, attach_recorder,
    register_script_commands, replay_lua,
};
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
    rec.push(
        "cell.set",
        json!({"ref": "B1", "input": "nil = [\"quoted\"]"}),
    );
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

#[test]
fn bus_recorder_captures_only_successful_live_commands() {
    let mut live = bus();
    let gate = ScriptGate::default();
    register_script_commands(live.registry_mut(), gate.clone()).unwrap();
    attach_recorder(&mut live, &gate);

    assert!(live.execute(Origin::User, "macro.record", json!({})).ok);
    assert!(
        live.execute(Origin::User, "cell.set", json!({"ref": "A1", "input": "7"}),)
            .ok
    );
    assert!(
        !live
            .execute(
                Origin::User,
                "cell.set",
                json!({"ref": "bad", "input": "8"})
            )
            .ok
    );
    assert!(live.execute(Origin::User, "macro.stop", json!({})).ok);

    let recorder = gate.recorder.lock().unwrap();
    assert_eq!(recorder.steps().len(), 1);
    assert_eq!(recorder.steps()[0].0, "cell.set");
}

#[test]
fn macro_save_has_no_preflight_or_dry_run_side_effect() {
    let dir = tempfile::Builder::new()
        .prefix("macro-dry-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap();
    let path = dir.path().join("macro.lua");
    let mut live = bus();
    let gate = ScriptGate::default();
    register_script_commands(live.registry_mut(), gate.clone()).unwrap();
    attach_recorder(&mut live, &gate);

    let dry = live
        .dry_run(
            Origin::User,
            "macro.save",
            json!({"path": path.display().to_string()}),
        )
        .unwrap();
    assert!(dry.outcome.ok, "{:?}", dry.outcome.error);
    assert!(!path.exists());

    assert!(live.execute(Origin::User, "macro.record", json!({})).ok);
    assert!(
        live.execute(Origin::User, "cell.set", json!({"ref": "A1", "input": "7"}))
            .ok
    );
    assert!(live.execute(Origin::User, "macro.stop", json!({})).ok);
    let saved = live.execute(
        Origin::User,
        "macro.save",
        json!({"path": path.display().to_string()}),
    );
    assert!(saved.ok, "{:?}", saved.error);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("omacell.cmd(\"cell.set\"")
    );
}

#[test]
fn recorder_stops_at_its_retention_limit() {
    let mut recorder = Recorder::new();
    recorder.start();
    for _ in 0..=MAX_RECORDED_STEPS {
        recorder.push("cell.set", json!({"ref": "A1", "input": "1"}));
    }
    assert_eq!(recorder.steps().len(), MAX_RECORDED_STEPS);
    assert!(!recorder.is_recording());
    assert!(recorder.overflowed());
}

#[test]
fn models_cannot_control_recording_or_source_user_code() {
    let mut live = bus();
    register_script_commands(live.registry_mut(), ScriptGate::default()).unwrap();
    for id in ["macro.record", "macro.stop", "macro.save", "script.source"] {
        let args = if id == "macro.save" {
            json!({"path": "model-macro.lua"})
        } else {
            json!({})
        };
        let outcome = live.execute(Origin::ExternalAgent, id, args);
        assert!(!outcome.ok, "model unexpectedly executed {id}");
        assert_eq!(outcome.error.unwrap().code, "command.denied");
    }
}
