//! Retained GUI/TUI Lua runtime integration over the single-writer task runner.

use std::time::{Duration, Instant};

use omacell_ai::cache::AiCache;
use omacell_ai::http::{HttpRequest, HttpResponse, SharedTransport, Transport};
use omacell_ai::{AiError, AiRuntime, PromptSet, Slot};
use omacell_bus::{Bus, LongOps, TaskRunner};
use std::sync::{Arc, Mutex};

use omacell_conf::schema::AiProvider;
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
struct RecordingTransport {
    requests: Mutex<Vec<HttpRequest>>,
}

#[async_trait::async_trait]
impl Transport for RecordingTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, AiError> {
        self.requests.lock().unwrap().push(request);
        Ok(HttpResponse {
            status: 200,
            body: json!({
                "choices": [{
                    "message": {
                        "content": r#"{"results":[{"i":0,"value":"provider result"}]}"#
                    }
                }]
            }),
            chunks: Vec::new(),
        })
    }
}

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

#[test]
fn retained_lua_ai_function_settles_through_hooks_and_the_task_runner() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    let source = r##"
        omacell.ai.fn("MY.AI", {
            prompt = "Use the retained Lua task.",
            min = 1,
            max = 1,
        })
        omacell.on_ai_request(function(request)
            request.provider = "gateway"
            request.model = "hooked-model"
            request.messages[#request.messages].content =
                request.messages[#request.messages].content .. ":request-hook"
            return request
        end)
        omacell.on_ai_response(function(response)
            response.response.text =
                '{"results":[{"i":0,"value":"hooked response"}]}'
            return response
        end)
        "##;
    std::fs::write(paths.user_config.join("init.lua"), source).unwrap();
    let trusted = paths.user_config.display().to_string();
    std::fs::write(
        paths.user_config.join("config.toml"),
        format!("[scripting]\ntrusted_dirs = [{trusted:?}]\n"),
    )
    .unwrap();
    let store = ConfigStore::load_with(paths.clone(), LoadOptions::default()).unwrap();
    let mut loaded = store.snapshot();
    loaded.config.ai.enabled = true;
    for (name, endpoint) in [
        ("local", "http://127.0.0.1:9/v1"),
        ("gateway", "http://127.0.0.1:8/v1"),
    ] {
        loaded.config.ai.providers.insert(
            name.into(),
            AiProvider {
                kind: "openai_compatible".into(),
                endpoint: endpoint.into(),
                local: true,
                secret_env: None,
                secret_cmd: None,
                timeout: 0,
                headers: Default::default(),
            },
        );
    }
    loaded.config.ai.models.default = "local:test-model".into();
    loaded.config.ai.models.fast = "local:test-model".into();

    let tokio = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let transport = Arc::new(RecordingTransport::default());
    let shared: SharedTransport = transport.clone();
    let ai = AiRuntime::new(
        tokio.handle().clone(),
        loaded.config.clone(),
        shared,
        PromptSet::builtin(),
        dir.path().join("cache"),
        dir.path().join("state"),
        AiCache::default(),
    );
    let mut fns = FnRegistry::new();
    register_all(&mut fns);
    let mut engine = RecalcEngine::new(fns);
    engine.set_async_provider(ai.clone());
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook
        .set_cell_contents(sheet, 0, 0, r#"=MY.AI("input")"#)
        .unwrap();
    engine.recalc_rebuild(&mut workbook);
    let mut bus = Bus::new(workbook, engine).unwrap();
    register_script_commands(bus.registry_mut(), ScriptGate::default()).unwrap();
    let runner = TaskRunner::spawn(bus, LongOps::production()).unwrap();
    let ui = Arc::new(TestUi::default());
    let mut scripts = InteractiveRuntime::new_with_ai(
        runner.handle(),
        ui as Arc<dyn InteractiveUi>,
        paths.user_config.clone(),
        &loaded,
        Some(ai.clone()),
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        scripts.poll_ai().unwrap();
        let snapshot = runner.handle().snapshot();
        let value = &snapshot.workbook.get(sheet, 0, 0).unwrap().unwrap().value;
        if let Value::Text(text) = value
            && snapshot.workbook.intern().strings.get(*text) == Some("hooked response")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "AI cell did not settle: {value:?}; messages: {:?}",
            scripts.take_messages()
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.starts_with("http://127.0.0.1:8/"));
    assert_eq!(requests[0].body["model"], "hooked-model");
    assert!(requests[0].body.to_string().contains("request-hook"));
    assert!(
        requests[0]
            .body
            .to_string()
            .contains("Use the retained Lua task.")
    );
    drop(requests);

    std::fs::write(
        paths.user_config.join("init.lua"),
        "-- extensions intentionally removed\n",
    )
    .unwrap();
    scripts.source().unwrap();
    let snapshot = runner.handle().snapshot();
    assert_eq!(
        snapshot.workbook.get(sheet, 0, 0).unwrap().unwrap().value,
        Value::Error(ErrorKind::Name)
    );
    let response = ai
        .chat_task(Slot::Default, "MY.AI", "payload".into(), None, vec![])
        .unwrap();
    assert!(response.text.contains("provider result"));
    let requests = transport.requests.lock().unwrap();
    assert!(
        requests
            .last()
            .unwrap()
            .url
            .starts_with("http://127.0.0.1:9/")
    );
    assert!(
        !requests
            .last()
            .unwrap()
            .body
            .to_string()
            .contains("Use the retained Lua task.")
    );
    drop(requests);

    std::fs::write(paths.user_config.join("init.lua"), source).unwrap();
    scripts.source().unwrap();
    std::fs::write(
        paths.user_config.join("init.lua"),
        "omacell.ai.fn('MY.BROKEN', {",
    )
    .unwrap();
    assert!(scripts.source().is_err());
    let response = ai
        .chat_task(Slot::Default, "MY.AI", "payload".into(), None, vec![])
        .unwrap();
    assert!(response.text.contains("hooked response"));
    let requests = transport.requests.lock().unwrap();
    assert!(
        requests
            .last()
            .unwrap()
            .url
            .starts_with("http://127.0.0.1:8/")
    );
    drop(requests);

    std::fs::write(
        paths.user_config.join("config.toml"),
        "[scripting]\nenabled = false\n",
    )
    .unwrap();
    let disabled = ConfigStore::load_with(paths, LoadOptions::default()).unwrap();
    scripts.tighten(&disabled.snapshot()).unwrap();
    let response = ai
        .chat_task(Slot::Default, "MY.AI", "payload".into(), None, vec![])
        .unwrap();
    assert!(response.text.contains("provider result"));
    let requests = transport.requests.lock().unwrap();
    assert!(
        requests
            .last()
            .unwrap()
            .url
            .starts_with("http://127.0.0.1:9/")
    );
}
