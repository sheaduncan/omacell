//! Retained GUI/TUI Lua runtime integration over the single-writer task runner.

use omacell_bus::{Bus, LongOps, TaskRunner};
use std::sync::{Arc, Mutex};

use omacell_conf::{ConfigStore, LoadOptions, Paths};
use omacell_core::command::Origin;
use omacell_core::error::{CoreError, ErrorKind};
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_lua::{InteractiveRuntime, InteractiveUi, ScriptGate, register_script_commands};
use serde_json::json;

#[derive(Default)]
struct TestUi {
    keys: Mutex<Vec<(String, String, String)>>,
}

impl InteractiveUi for TestUi {
    fn keymap_set(&self, mode: &str, keys: &str, cmd: &str) -> Result<(), CoreError> {
        self.keys
            .lock()
            .unwrap()
            .push((mode.to_string(), keys.to_string(), cmd.to_string()));
        Ok(())
    }

    fn clear_keymap(&self) {
        self.keys.lock().unwrap().clear();
    }
}

fn runtime(
    source: &str,
) -> (
    tempfile::TempDir,
    TaskRunner,
    Arc<TestUi>,
    InteractiveRuntime,
) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    std::fs::write(paths.user_config.join("init.lua"), source).unwrap();
    let trusted = paths.user_config.display().to_string();
    std::fs::write(
        paths.user_config.join("config.toml"),
        format!("[scripting]\ntrusted_dirs = [{trusted:?}]\n"),
    )
    .unwrap();
    let store = ConfigStore::load_with(paths.clone(), LoadOptions::default()).unwrap();
    let loaded = store.snapshot();
    let mut fns = FnRegistry::new();
    register_all(&mut fns);
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook
        .set_cell_contents(sheet, 0, 2, "=USER.SCALE(5)")
        .unwrap();
    let mut bus = Bus::new(workbook, RecalcEngine::new(fns)).unwrap();
    register_script_commands(bus.registry_mut(), ScriptGate::default()).unwrap();
    let runner = TaskRunner::spawn(bus, LongOps::production()).unwrap();
    let ui = Arc::new(TestUi::default());
    let scripts = InteractiveRuntime::new(
        runner.handle(),
        Arc::clone(&ui) as Arc<dyn InteractiveUi>,
        paths.user_config,
        &loaded,
    )
    .unwrap();
    (dir, runner, ui, scripts)
}

#[test]
fn startup_loads_status_keymaps_functions_and_live_events() {
    let (_dir, runner, ui, scripts) = runtime(
        r#"
        omacell.ui.status("lua loaded")
        omacell.keymap.set("classic", "Ctrl+L", "cell.clear")
        omacell.fn("USER.DOUBLE", {min = 1, max = 1}, function(x) return x * 2 end)
        local factor = 3
        omacell.fn("USER.SCALE", {min = 1, max = 1}, function(x) return x * factor end)
        omacell.on_open(function()
            omacell.fn("USER.LATE", {min = 0, max = 0}, function() return 9 end)
            omacell.ui.status("lua opened")
        end)
        changing = false
        omacell.on_change(function()
            if changing then return end
            changing = true
            omacell.cmd("cell.set", {ref = "A1", input = "4"})
            changing = false
        end)
        "#,
    );

    assert_eq!(scripts.take_messages(), vec!["lua loaded"]);
    let startup = runner.handle().snapshot();
    let sheet = startup.workbook.active_sheet();
    assert!(matches!(
        startup.workbook.get(sheet, 0, 2).unwrap().unwrap().value,
        Value::Number(value) if value == 15.0
    ));
    let outcome = runner.handle().submit_wait(
        Origin::User,
        "cell.set",
        json!({"ref": "D1", "input": "=USER.LATE()"}),
    );
    assert!(outcome.ok, "{:?}", outcome.error);
    scripts.emit_open().unwrap();
    assert_eq!(scripts.take_messages(), vec!["lua opened"]);
    let opened = runner.handle().snapshot();
    assert!(matches!(
        opened.workbook.get(sheet, 0, 3).unwrap().unwrap().value,
        Value::Number(value) if value == 9.0
    ));
    assert_eq!(
        *ui.keys.lock().unwrap(),
        vec![("classic".into(), "Ctrl+L".into(), "cell.clear".into())]
    );

    let handle = runner.handle();
    let outcome = handle.submit_wait(
        Origin::User,
        "cell.set",
        json!({"ref": "B1", "input": "=USER.DOUBLE(A1)"}),
    );
    assert!(outcome.ok, "{:?}", outcome.error);
    scripts.poll_events().unwrap();
    let snapshot = handle.snapshot();
    let sheet = snapshot.workbook.active_sheet();
    assert!(matches!(
        snapshot.workbook.get(sheet, 0, 0).unwrap().unwrap().value,
        Value::Number(value) if value == 4.0
    ));
    assert!(matches!(
        snapshot.workbook.get(sheet, 0, 1).unwrap().unwrap().value,
        Value::Number(value) if value == 8.0
    ));
}

#[test]
fn source_replaces_hooks_and_keymap_overlay() {
    let (dir, runner, ui, mut scripts) = runtime(
        r#"
        omacell.keymap.set("classic", "Ctrl+L", "cell.clear")
        omacell.fn("USER.OLD", {min = 0, max = 0}, function() return 7 end)
        omacell.on_change(function() omacell.ui.status("old") end)
        "#,
    );
    let outcome = runner.handle().submit_wait(
        Origin::User,
        "cell.set",
        json!({"ref": "D1", "input": "=USER.OLD()"}),
    );
    assert!(outcome.ok, "{:?}", outcome.error);
    let before = runner.handle().snapshot();
    let sheet = before.workbook.active_sheet();
    assert!(matches!(
        before.workbook.get(sheet, 0, 3).unwrap().unwrap().value,
        Value::Number(value) if value == 7.0
    ));
    scripts.poll_events().unwrap();
    assert_eq!(scripts.take_messages(), vec!["old"]);
    std::fs::write(
        dir.path().join(".config/omacell/init.lua"),
        r#"
        omacell.keymap.set("classic", "Ctrl+J", "cell.clear")
        omacell.on_change(function() omacell.ui.status("new") end)
        "#,
    )
    .unwrap();

    scripts.source().unwrap();
    assert_eq!(
        *ui.keys.lock().unwrap(),
        vec![("classic".into(), "Ctrl+J".into(), "cell.clear".into())]
    );

    let outcome =
        runner
            .handle()
            .submit_wait(Origin::User, "cell.set", json!({"ref": "A1", "input": "1"}));
    assert!(outcome.ok, "{:?}", outcome.error);
    scripts.poll_events().unwrap();
    assert_eq!(scripts.take_messages(), vec!["new"]);
    let after = runner.handle().snapshot();
    let sheet = after.workbook.active_sheet();
    assert_eq!(
        after.workbook.get(sheet, 0, 3).unwrap().unwrap().value,
        Value::Error(ErrorKind::Name)
    );
}

#[test]
fn stricter_policy_drops_runtime_keymaps_and_functions() {
    let (dir, runner, ui, mut scripts) = runtime(
        r#"
        omacell.keymap.set("classic", "Ctrl+L", "cell.clear")
        omacell.fn("USER.OLD", {min = 0, max = 0}, function() return 7 end)
        "#,
    );
    let outcome = runner.handle().submit_wait(
        Origin::User,
        "cell.set",
        json!({"ref": "D1", "input": "=USER.OLD()"}),
    );
    assert!(outcome.ok, "{:?}", outcome.error);

    let paths = Paths::from_home(dir.path());
    std::fs::write(
        paths.user_config.join("config.toml"),
        "[scripting]\nenabled = false\n",
    )
    .unwrap();
    let store = ConfigStore::load_with(paths, LoadOptions::default()).unwrap();
    scripts.tighten(&store.snapshot()).unwrap();

    assert!(ui.keys.lock().unwrap().is_empty());
    let snapshot = runner.handle().snapshot();
    let sheet = snapshot.workbook.active_sheet();
    assert_eq!(
        snapshot.workbook.get(sheet, 0, 3).unwrap().unwrap().value,
        Value::Error(ErrorKind::Name)
    );
}
