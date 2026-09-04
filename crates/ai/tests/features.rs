//! WP-23 evals: plans, injection, async cells, budget, formula scratch eval.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use omacell_ai::agent::validate_tool;
use omacell_ai::formula::parse_and_eval;
use omacell_ai::functions::{is_ai_formula, register_ai_functions, strip_ai_formulas};
use omacell_ai::http::{HttpRequest, HttpResponse, SharedTransport, Transport};
use omacell_ai::import_assist::parse_plan_overlay;
use omacell_ai::plan::{forbidden, parse_plan, to_calls};
use omacell_ai::prompts::PromptSet;
use omacell_ai::runtime::AiRuntime;
use omacell_ai::{
    AiHookRequest, AiHookResponse, AiHooks, AiTaskSpec, PolicySnapshot, SendLevel, Slot, ToolSpec,
};
use omacell_conf::schema::package_defaults;
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, ArrayLift, DynamicFn, DynamicFnBody, FnRegistry, RuntimeValue};
use omacell_core::graph::CellCoord;
use omacell_core::recalc::RecalcEngine;
use omacell_core::value::Value as CellValue;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_io::xlsx::{open_bytes, save_workbook_bytes};

#[path = "../../../tests/support/libreoffice.rs"]
mod libreoffice;
use serde_json::{Value, json};

struct CountingTransport {
    hits: AtomicU32,
    body: Value,
    requests: Mutex<Vec<HttpRequest>>,
}

#[async_trait::async_trait]
impl Transport for CountingTransport {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, omacell_ai::AiError> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push(req);
        Ok(HttpResponse {
            status: 200,
            body: json!({
                "choices": [{"message": {"content": serde_json::to_string(&self.body).unwrap()}}]
            }),
            chunks: Vec::new(),
        })
    }
}

struct BlockingTransport {
    hits: AtomicU32,
    started: SyncSender<()>,
    release: Mutex<Receiver<()>>,
}

#[async_trait::async_trait]
impl Transport for BlockingTransport {
    async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, omacell_ai::AiError> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        self.started.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(HttpResponse {
            status: 200,
            body: json!({
                "choices": [{
                    "message": {
                        "content": r#"{"results":[{"i":0,"value":"stale result"}]}"#
                    }
                }]
            }),
            chunks: Vec::new(),
        })
    }
}

fn catalog() -> BTreeSet<String> {
    [
        "cell.set",
        "range.sort",
        "filter.set",
        "sheet.add",
        "sheet.rename",
        "condfmt.add",
        "table.create",
        "edit.filldown",
        "format.bold",
        "range.removeduplicates",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn enabled_config() -> omacell_conf::schema::Config {
    let mut config = package_defaults().unwrap();
    config.ai.enabled = true;
    config.ai.providers.insert(
        "ollama".into(),
        omacell_conf::schema::AiProvider {
            kind: "openai_compatible".into(),
            endpoint: "http://127.0.0.1:9/v1".into(),
            local: true,
            secret_env: None,
            secret_cmd: None,
            timeout: 0,
            headers: Default::default(),
        },
    );
    config.ai.models.default = "ollama:qwen".into();
    config.ai.models.fast = "ollama:qwen".into();
    config.ai.functions.batch_size = 50;
    config.ai.functions.max_cells_per_recalc = 500;
    config
}

fn runtime(
    config: omacell_conf::schema::Config,
    body: Value,
) -> (
    Arc<AiRuntime>,
    Arc<CountingTransport>,
    tokio::runtime::Runtime,
    tempfile::TempDir,
) {
    let transport = Arc::new(CountingTransport {
        hits: AtomicU32::new(0),
        body,
        requests: Mutex::new(Vec::new()),
    });
    let shared: SharedTransport = transport.clone();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let keep = tmp.path().to_path_buf();
    let ai = AiRuntime::new(
        rt.handle().clone(),
        config,
        shared,
        PromptSet::builtin(),
        keep.clone(),
        keep,
        omacell_ai::cache::AiCache::default(),
    );
    (ai, transport, rt, tmp)
}

struct RewritingHooks;

impl AiHooks for RewritingHooks {
    fn cache_version(&self) -> String {
        "rewrite-v1".into()
    }

    fn on_request(&self, mut request: AiHookRequest) -> Result<AiHookRequest, omacell_ai::AiError> {
        request
            .messages
            .last_mut()
            .unwrap()
            .content
            .push_str("\nhooked request");
        request.provider = "gateway".into();
        request.model = "corporate-model".into();
        Ok(request)
    }

    fn on_response(
        &self,
        mut response: AiHookResponse,
    ) -> Result<AiHookResponse, omacell_ai::AiError> {
        response.response.text = r#"{"hooked":true}"#.into();
        Ok(response)
    }
}

#[test]
fn custom_tasks_and_hooks_are_applied_inside_the_ai_runtime() {
    let mut config = enabled_config();
    config.ai.providers.insert(
        "gateway".into(),
        omacell_conf::schema::AiProvider {
            kind: "openai_compatible".into(),
            endpoint: "http://127.0.0.1:8/v1".into(),
            local: true,
            secret_env: None,
            secret_cmd: None,
            timeout: 0,
            headers: Default::default(),
        },
    );
    let (ai, transport, _rt, _tmp) = runtime(config, json!({"raw": true}));
    ai.replace_extensions(
        vec![AiTaskSpec {
            name: "summarize".into(),
            prompt: "CUSTOM TASK PROMPT".into(),
            schema: Some(json!({"type": "object"})),
            tools: vec![ToolSpec {
                name: "lookup".into(),
                description: "Look up a local value".into(),
                parameters: json!({"type": "object"}),
            }],
        }],
        Some(Arc::new(RewritingHooks)),
    )
    .unwrap();

    let response = ai
        .chat_task(Slot::Default, "summarize", "payload".into(), None, vec![])
        .unwrap();
    assert_eq!(response.text, r#"{"hooked":true}"#);
    let requests = transport.requests.lock().unwrap();
    let body = &requests[0].body;
    assert!(requests[0].url.starts_with("http://127.0.0.1:8/"));
    assert_eq!(body["model"], "corporate-model");
    assert!(body.to_string().contains("CUSTOM TASK PROMPT"));
    assert!(body.to_string().contains("hooked request"));
    assert!(body.to_string().contains("lookup"));
    assert_eq!(
        body["response_format"]["json_schema"]["schema"]["type"],
        "object"
    );
}

struct AsyncStub;

impl DynamicFnBody for AsyncStub {
    fn async_node(&self) -> bool {
        true
    }

    fn eval(&self, _args: &[ArgVal]) -> RuntimeValue {
        RuntimeValue::error(ErrorKind::Na)
    }
}

#[test]
fn custom_async_function_settles_and_reuses_its_cache() {
    let config = enabled_config();
    let (ai, transport, _rt, _tmp) = runtime(
        config.clone(),
        json!({"results":[{"i":0,"value":"custom result"}]}),
    );
    ai.replace_extensions(
        vec![AiTaskSpec {
            name: "MY.AI".into(),
            prompt: "Use the custom worksheet task.".into(),
            schema: Some(json!({"type": "string"})),
            tools: Vec::new(),
        }],
        None,
    )
    .unwrap();
    let mut registry = FnRegistry::new();
    registry.register_dynamic(DynamicFn {
        name: "MY.AI".into(),
        min_args: 1,
        max_args: 1,
        volatile: false,
        array_lift: ArrayLift::None,
        body: Arc::new(AsyncStub),
    });
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook
        .set_cell_contents(sheet, 0, 0, r#"=MY.AI("input")"#)
        .unwrap();

    let first = engine.recalc_rebuild(&mut workbook);
    assert_eq!(first.pending_async, vec![CellCoord::new(sheet, 0, 0)]);
    let policy = PolicySnapshot::capture(&config, Some(&workbook), true);
    assert_eq!(ai.settle(&policy).unwrap(), 1);
    let second = engine.recalc_rebuild(&mut workbook);
    assert!(second.pending_async.is_empty());
    let value = workbook.get(sheet, 0, 0).unwrap().unwrap().value;
    let CellValue::Text(text) = value else {
        panic!("expected custom text result, got {value:?}");
    };
    assert_eq!(workbook.intern().strings.get(text), Some("custom result"));
    assert_eq!(ai.settle(&policy).unwrap(), 0);
    assert_eq!(transport.hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        transport.requests.lock().unwrap()[0].body.pointer(
            "/response_format/json_schema/schema/properties/results/items/properties/value/type"
        ),
        Some(&json!("string"))
    );
}

#[test]
fn extension_reload_discards_an_in_flight_response() {
    let config = enabled_config();
    let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let transport = Arc::new(BlockingTransport {
        hits: AtomicU32::new(0),
        started: started_tx,
        release: Mutex::new(release_rx),
    });
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let shared: SharedTransport = transport.clone();
    let ai = AiRuntime::new(
        tokio.handle().clone(),
        config.clone(),
        shared,
        PromptSet::builtin(),
        tmp.path().join("cache"),
        tmp.path().join("state"),
        Default::default(),
    );
    ai.replace_extensions(
        vec![AiTaskSpec {
            name: "MY.AI".into(),
            prompt: "old prompt".into(),
            schema: None,
            tools: Vec::new(),
        }],
        None,
    )
    .unwrap();
    let mut registry = FnRegistry::new();
    registry.register_dynamic(DynamicFn {
        name: "MY.AI".into(),
        min_args: 1,
        max_args: 1,
        volatile: false,
        array_lift: ArrayLift::None,
        body: Arc::new(AsyncStub),
    });
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    let cell = CellCoord::new(sheet, 0, 0);
    workbook
        .set_cell_contents(sheet, 0, 0, r#"=MY.AI("input")"#)
        .unwrap();
    assert_eq!(
        engine.recalc_rebuild(&mut workbook).pending_async,
        vec![cell]
    );

    let policy = ai.policy(Some(&workbook));
    let settling = {
        let ai = ai.clone();
        std::thread::spawn(move || ai.settle(&policy))
    };
    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    ai.replace_extensions(
        vec![AiTaskSpec {
            name: "MY.AI".into(),
            prompt: "new prompt".into(),
            schema: None,
            tools: Vec::new(),
        }],
        None,
    )
    .unwrap();
    assert_eq!(
        engine.recalc_rebuild(&mut workbook).pending_async,
        vec![cell]
    );
    release_tx.send(()).unwrap();

    assert_eq!(settling.join().unwrap().unwrap(), 0);
    assert_eq!(transport.hits.load(Ordering::SeqCst), 1);
    assert!(ai.provenance(cell).is_none());
    assert!(ai.pending_generation().is_some());
}

#[test]
fn two_hundred_plan_evals_match_commands() {
    let cat = catalog();
    let templates = [
        ("sort column {n}", "range.sort", json!({"range": "A1:F10"})),
        ("filter {n}", "filter.set", json!({"range": "A1:A20"})),
        ("add a sheet {n}", "sheet.add", json!({"name": "S"})),
        (
            "rename sheet {n}",
            "sheet.rename",
            json!({"from": "Sheet1", "to": "Data"}),
        ),
        ("fill down {n}", "edit.filldown", json!({"range": "A1:A10"})),
        (
            "set A1 to {n}",
            "cell.set",
            json!({"ref": "A1", "input": "1"}),
        ),
        ("add cf {n}", "condfmt.add", json!({"range": "A1:A10"})),
        (
            "make a table {n}",
            "table.create",
            json!({"range": "A1:C10"}),
        ),
        ("bold cells {n}", "format.bold", json!({"range": "A1:A10"})),
        (
            "remove duplicates {n}",
            "range.removeduplicates",
            json!({"range": "A1:C10"}),
        ),
    ];
    let mut n = 0u32;
    for i in 0..200 {
        let (prompt_t, id, args) = &templates[i % templates.len()];
        let prompt = prompt_t.replace("{n}", &i.to_string());
        let model = json!({"commands":[{"id": id, "args": args}]});
        let plan = parse_plan(&model, &cat).unwrap();
        assert_eq!(plan.commands[0].id, *id, "{prompt}");
        let calls = to_calls(&plan).unwrap();
        assert_eq!(calls[0].id.as_str(), *id);
        n += 1;
    }
    assert_eq!(n, 200);
}

#[test]
fn injection_suite_rejects_policy_commands() {
    let cat = catalog();
    for payload in [
        json!({"commands":[{"id":"trust.add","args":{}}]}),
        json!({"commands":[{"id":"script.run","args":{}}]}),
        json!({"commands":[{"id":"scripting.enable","args":{}}]}),
        json!({"commands":[{"id":"file.save","args":{"path":"/tmp/x.xlsx"}}]}),
        json!({"commands":[{"id":"network.enable","args":{}}]}),
        json!({"commands":[{"id":"config.set","args":{}}]}),
        json!({"commands":[{"id":"ai.agent.turn","args":{}}]}),
    ] {
        let id = payload["commands"][0]["id"].as_str().unwrap();
        let err = parse_plan(&payload, &cat).unwrap_err();
        assert_eq!(err.code, "ai.payload");
        assert!(
            forbidden(id) || err.message.contains("unknown") || err.message.contains("forbidden"),
            "{id}: {err:?}"
        );
        assert!(validate_tool("command_run", &format!(r#"{{"id":"{id}"}}"#), true, &cat,).is_err());
    }
}

#[test]
fn async_cells_batch_and_cache_skips_http() {
    let config = enabled_config();
    let (ai, transport, _rt, _tmp) = runtime(
        config.clone(),
        json!({"results":[{"i":0,"value":"Ada"},{"i":1,"value":"Bob"}]}),
    );
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, r#"=AI("name")"#).unwrap();
    wb.set_cell_contents(sheet, 1, 0, r#"=AI("name2")"#)
        .unwrap();
    let first = engine.recalc_rebuild(&mut wb);
    assert!(!first.pending_async.is_empty());
    let policy = PolicySnapshot::capture(&config, Some(&wb), true);
    let n = ai.settle(&policy).unwrap();
    assert!(n >= 1);
    let hits_after_first = transport.hits.load(Ordering::SeqCst);
    assert!(hits_after_first >= 1);
    let second = engine.recalc_rebuild(&mut wb);
    assert!(second.pending_async.is_empty());
    let _ = ai.settle(&policy).unwrap();
    assert_eq!(transport.hits.load(Ordering::SeqCst), hits_after_first);
}

#[test]
fn schema_policy_filters_ai_cell_arguments_before_transport() {
    let mut config = enabled_config();
    config.ai.privacy.send = "schema".into();
    config.ai.privacy.local_full = false;
    config.ai.privacy.suggest_redaction = false;
    let (ai, transport, _rt, _tmp) = runtime(
        config.clone(),
        json!({"results":[{"i":0,"value":"classified"}]}),
    );
    let mut registry = FnRegistry::new();
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook
        .set_cell_contents(sheet, 0, 0, "private-cell-value")
        .unwrap();
    workbook
        .set_cell_contents(sheet, 0, 1, "=AI.CLASSIFY(A1,\"category\")")
        .unwrap();
    let _ = engine.recalc_rebuild(&mut workbook);
    let policy = PolicySnapshot::capture(&config, Some(&workbook), false);
    assert_eq!(ai.settle(&policy).unwrap(), 1);
    let requests = transport.requests.lock().unwrap();
    let payload = requests[0].body.to_string();
    assert!(!payload.contains("private-cell-value"), "{payload}");
    assert!(payload.contains("category"), "{payload}");
}

#[test]
fn disabled_live_policy_blocks_a_queued_ai_cell_before_transport() {
    let config = enabled_config();
    let (ai, transport, _rt, _tmp) = runtime(
        config.clone(),
        json!({"results":[{"i":0,"value":"unused"}]}),
    );
    let mut registry = FnRegistry::new();
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook
        .set_cell_contents(sheet, 0, 0, "=AI(\"queued\")")
        .unwrap();
    let _ = engine.recalc_rebuild(&mut workbook);
    let mut disabled = config;
    disabled.ai.enabled = false;
    let policy = ai.policy_for_config(&disabled, Slot::Default, Some(&workbook));
    assert_eq!(ai.settle(&policy).unwrap_err().code, "ai.disabled");
    assert_eq!(transport.hits.load(Ordering::SeqCst), 0);
}

#[test]
fn audit_log_is_preflighted_before_provider_transport() {
    let config = enabled_config();
    let transport = Arc::new(CountingTransport {
        hits: AtomicU32::new(0),
        body: json!({"ok": true}),
        requests: Mutex::new(Vec::new()),
    });
    let shared: SharedTransport = transport.clone();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let state_file = tmp.path().join("not-a-directory");
    std::fs::write(&state_file, b"occupied").unwrap();
    let ai = AiRuntime::new(
        rt.handle().clone(),
        config,
        shared,
        PromptSet::builtin(),
        tmp.path().join("cache"),
        state_file,
        Default::default(),
    );
    let error = ai
        .chat_task(Slot::Default, "plan", "payload".into(), None, vec![])
        .unwrap_err();
    assert_eq!(error.code, "ai.log");
    assert_eq!(transport.hits.load(Ordering::SeqCst), 0);
}

#[test]
fn auto_false_waits_for_refresh_and_keeps_the_prior_value_stale() {
    let mut config = enabled_config();
    config.ai.functions.auto = false;
    let (ai, transport, _rt, _tmp) =
        runtime(config.clone(), json!({"results":[{"i":0,"value":"Ada"}]}));
    let mut registry = FnRegistry::new();
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    let cell = CellCoord::new(sheet, 0, 1);
    workbook.set_cell_contents(sheet, 0, 0, "first").unwrap();
    workbook.set_cell_contents(sheet, 0, 1, "=AI(A1)").unwrap();

    let initial = engine.recalc_rebuild(&mut workbook);
    assert_eq!(initial.pending_async, vec![cell]);
    assert!(ai.pending_generation().is_none());
    let policy = PolicySnapshot::capture(&config, Some(&workbook), true);
    assert_eq!(ai.settle(&policy).unwrap(), 0);
    assert_eq!(transport.hits.load(Ordering::SeqCst), 0);

    ai.refresh_cells(&[cell]);
    assert_eq!(engine.recalc_full(&mut workbook).pending_async, vec![cell]);
    assert_eq!(ai.settle(&policy).unwrap(), 1);
    assert!(
        engine
            .recalc_incremental(&mut workbook)
            .pending_async
            .is_empty()
    );
    let first_value = workbook.get(sheet, 0, 1).unwrap().unwrap().value;
    let CellValue::Text(first_text) = first_value else {
        panic!("expected text result, got {first_value:?}");
    };
    assert_eq!(workbook.intern().strings.get(first_text), Some("Ada"));

    workbook.set_cell_contents(sheet, 0, 0, "second").unwrap();
    engine.notify_edit(&workbook, CellCoord::new(sheet, 0, 0));
    let changed = engine.recalc_incremental(&mut workbook);
    assert_eq!(changed.pending_async, vec![cell]);
    assert!(workbook.get(sheet, 0, 1).unwrap().unwrap().flags.stale());
    let stale_value = workbook.get(sheet, 0, 1).unwrap().unwrap().value;
    let CellValue::Text(stale_text) = stale_value else {
        panic!("expected stale text result, got {stale_value:?}");
    };
    assert_eq!(workbook.intern().strings.get(stale_text), Some("Ada"));
    assert_eq!(ai.settle(&policy).unwrap(), 0);
    assert_eq!(transport.hits.load(Ordering::SeqCst), 1);

    ai.refresh_cells(&[cell]);
    let _ = engine.recalc_full(&mut workbook);
    assert_eq!(ai.settle(&policy).unwrap(), 1);
    assert_eq!(transport.hits.load(Ordering::SeqCst), 2);
}

#[test]
fn workbook_load_never_queues_a_cache_miss_but_a_later_input_edit_does() {
    let config = enabled_config();
    let (ai, transport, _rt, _tmp) =
        runtime(config.clone(), json!({"results":[{"i":0,"value":"Ada"}]}));
    let mut registry = FnRegistry::new();
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    let cell = CellCoord::new(sheet, 0, 1);
    workbook.set_cell_contents(sheet, 0, 0, "first").unwrap();
    workbook.set_cell_contents(sheet, 0, 1, "=AI(A1)").unwrap();

    ai.begin_workbook_load();
    let opened = engine.recalc_rebuild(&mut workbook);
    ai.end_workbook_load();
    assert_eq!(opened.pending_async, vec![cell]);
    assert!(ai.pending_generation().is_none());
    let policy = PolicySnapshot::capture(&config, Some(&workbook), true);
    assert_eq!(ai.settle(&policy).unwrap(), 0);
    assert_eq!(transport.hits.load(Ordering::SeqCst), 0);

    workbook.set_cell_contents(sheet, 0, 0, "second").unwrap();
    engine.notify_edit(&workbook, CellCoord::new(sheet, 0, 0));
    assert_eq!(
        engine.recalc_incremental(&mut workbook).pending_async,
        vec![cell]
    );
    assert_eq!(ai.settle(&policy).unwrap(), 1);
    assert_eq!(transport.hits.load(Ordering::SeqCst), 1);
}

#[test]
fn explicit_full_recalc_uses_the_live_refresh_setting() {
    let mut config = enabled_config();
    config.ai.functions.refresh_on_full_recalc = false;
    let (ai, transport, _rt, _tmp) =
        runtime(config.clone(), json!({"results":[{"i":0,"value":"Ada"}]}));
    let mut registry = FnRegistry::new();
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    let cell = CellCoord::new(sheet, 0, 0);
    workbook
        .set_cell_contents(sheet, 0, 0, "=AI(\"name\")")
        .unwrap();
    let policy = PolicySnapshot::capture(&config, Some(&workbook), true);

    let _ = engine.recalc_rebuild(&mut workbook);
    assert_eq!(ai.settle(&policy).unwrap(), 1);
    let _ = engine.recalc_incremental(&mut workbook);
    assert!(
        engine
            .recalc_explicit_full(&mut workbook)
            .pending_async
            .is_empty()
    );
    assert_eq!(ai.settle(&policy).unwrap(), 0);
    assert_eq!(transport.hits.load(Ordering::SeqCst), 1);

    let mut functions = config.ai.functions.clone();
    functions.refresh_on_full_recalc = true;
    ai.update_function_config(functions);
    assert_eq!(
        engine.recalc_explicit_full(&mut workbook).pending_async,
        vec![cell]
    );
    assert_eq!(ai.settle(&policy).unwrap(), 1);
    assert_eq!(transport.hits.load(Ordering::SeqCst), 2);
}

#[test]
fn malformed_cell_batch_is_rejected_and_requeued() {
    let config = enabled_config();
    let (ai, transport, _rt, _tmp) = runtime(config.clone(), json!({"value": "wrong shape"}));
    let mut registry = FnRegistry::new();
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, r#"=AI("name")"#).unwrap();
    let _ = engine.recalc_rebuild(&mut wb);
    let policy = PolicySnapshot::capture(&config, Some(&wb), true);
    let err = ai.settle(&policy).unwrap_err();
    assert_eq!(err.code, "ai.payload");
    assert_eq!(transport.hits.load(Ordering::SeqCst), 1);
    assert!(ai.settle(&policy).is_err());
    assert_eq!(transport.hits.load(Ordering::SeqCst), 2);
}

#[test]
fn ai_table_result_spills_a_rectangular_array() {
    let config = enabled_config();
    let (ai, _transport, _rt, _tmp) = runtime(
        config.clone(),
        json!({"results":[{"i":0,"value":[[1,2],[3,4]]}]}),
    );
    let mut registry = FnRegistry::new();
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, r#"=AI.TABLE("two by two")"#)
        .unwrap();
    let _ = engine.recalc_rebuild(&mut wb);
    let policy = PolicySnapshot::capture(&config, Some(&wb), true);
    ai.settle(&policy).unwrap();
    let result = engine.recalc_rebuild(&mut wb);
    assert!(result.pending_async.is_empty());
    assert_eq!(
        wb.get(sheet, 0, 1).unwrap().unwrap().value,
        CellValue::Number(2.0)
    );
    assert_eq!(
        wb.get(sheet, 1, 0).unwrap().unwrap().value,
        CellValue::Number(3.0)
    );
    assert_eq!(
        wb.get(sheet, 1, 1).unwrap().unwrap().value,
        CellValue::Number(4.0)
    );
}

#[test]
fn privacy_policy_follows_the_routed_provider_slot() {
    let mut config = enabled_config();
    config.ai.privacy.send = "schema".into();
    config.ai.privacy.local_full = true;
    config.ai.providers.insert(
        "gateway".into(),
        omacell_conf::schema::AiProvider {
            kind: "openai_compatible".into(),
            endpoint: "https://example.invalid/v1".into(),
            local: false,
            secret_env: None,
            secret_cmd: None,
            timeout: 0,
            headers: Default::default(),
        },
    );
    config.ai.models.fast = "gateway:fast".into();
    let (ai, _transport, _rt, _tmp) = runtime(config, json!({}));
    assert_eq!(ai.policy_for(Slot::Default, None).send, SendLevel::Full);
    assert_eq!(ai.policy_for(Slot::Fast, None).send, SendLevel::Schema);
    ai.replace_extensions(Vec::new(), Some(Arc::new(RewritingHooks)))
        .unwrap();
    let error = ai
        .chat_task(Slot::Default, "plan", "payload".into(), None, vec![])
        .unwrap_err();
    assert_eq!(error.code, "ai.payload");
    assert!(error.message.contains("local-provider payload"));
}

#[test]
fn policy_snapshot_uses_the_live_config_route() {
    let config = enabled_config();
    let (ai, _transport, _rt, _tmp) = runtime(config.clone(), json!({}));
    let mut reloaded = config;
    reloaded.ai.privacy.send = "schema".into();
    reloaded.ai.privacy.local_full = true;
    reloaded.ai.providers.insert(
        "gateway".into(),
        omacell_conf::schema::AiProvider {
            kind: "openai_compatible".into(),
            endpoint: "https://models.example.test/v1".into(),
            local: false,
            secret_env: None,
            secret_cmd: None,
            timeout: 0,
            headers: Default::default(),
        },
    );
    reloaded.ai.models.fast = "gateway:fast".into();
    assert_eq!(
        ai.policy_for_config(&reloaded, Slot::Fast, None).send,
        SendLevel::Schema
    );
}

#[test]
fn opening_another_workbook_discards_per_workbook_ai_state() {
    let config = enabled_config();
    let (ai, _transport, _rt, _tmp) =
        runtime(config.clone(), json!({"results":[{"i":0,"value":"Ada"}]}));
    let mut registry = FnRegistry::new();
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, r#"=AI("name")"#).unwrap();
    let _ = engine.recalc_rebuild(&mut wb);
    let policy = PolicySnapshot::capture(&config, Some(&wb), true);
    ai.settle(&policy).unwrap();
    ai.replace_workbook_cache(Default::default());
    let mut next = Workbook::new();
    ai.write_workbook_cache(&mut next).unwrap();
    let cache = omacell_ai::cache::AiCache::from_bytes(
        next.custom_parts
            .get(omacell_ai::cache::AICACHE_PART)
            .unwrap(),
    );
    assert!(cache.entries.is_empty());
}

#[test]
fn batching_fifty_plus_cells_uses_two_requests() {
    let mut config = enabled_config();
    config.ai.functions.batch_size = 50;
    config.ai.functions.max_cells_per_recalc = 500;
    let results: Vec<_> = (0..50).map(|i| json!({"i": i, "value": i})).collect();
    let (ai, transport, _rt, _tmp) = runtime(config.clone(), json!({"results": results}));
    let mut registry = FnRegistry::new();
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    for i in 0..100u32 {
        wb.set_cell_contents(sheet, i, 0, &format!(r#"=AI("{i}")"#))
            .unwrap();
    }
    let _ = engine.recalc_rebuild(&mut wb);
    let policy = PolicySnapshot::capture(&config, Some(&wb), true);
    ai.settle(&policy).unwrap();
    assert_eq!(transport.hits.load(Ordering::SeqCst), 2);
}

#[test]
fn budget_confirmation_trips() {
    let mut config = enabled_config();
    config.ai.functions.max_cells_per_recalc = 1;
    let (ai, _transport, _rt, _tmp) = runtime(config.clone(), json!({"results":[]}));
    let mut registry = FnRegistry::new();
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, r#"=AI("a")"#).unwrap();
    wb.set_cell_contents(sheet, 1, 0, r#"=AI("b")"#).unwrap();
    let _ = engine.recalc_rebuild(&mut wb);
    let policy = PolicySnapshot::capture(&config, Some(&wb), true);
    let err = ai.settle(&policy).unwrap_err();
    assert_eq!(err.code, "ai.budget");
    assert!(ai.confirmation().is_some());
}

#[test]
fn formula_scratch_eval_rejects_garbage() {
    let wb = Workbook::new();
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    let engine = RecalcEngine::new(registry);
    let cell = CellCoord::new(wb.active_sheet(), 0, 0);
    let err = parse_and_eval(&json!({"formula": "=ZZZ("}), &wb, &engine, cell).unwrap_err();
    assert_eq!(err.code, "ai.payload");
    let (src, _) = parse_and_eval(&json!({"formula": "=1+1"}), &wb, &engine, cell).unwrap();
    assert_eq!(src, "=1+1");
}

#[test]
fn fill_round_trip_custom_part_and_xlsx() {
    let config = enabled_config();
    let (ai, _transport, _rt, _tmp) =
        runtime(config.clone(), json!({"results":[{"i":0,"value":"Ada"}]}));
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, r#"=AI.FILL("Ada","Ada Lovelace")"#)
        .unwrap();
    let _ = engine.recalc_rebuild(&mut wb);
    let policy = PolicySnapshot::capture(&config, Some(&wb), true);
    ai.settle(&policy).unwrap();
    let _ = engine.recalc_rebuild(&mut wb);
    ai.write_workbook_cache(&mut wb).unwrap();
    assert!(
        wb.custom_parts
            .contains_key(omacell_ai::cache::AICACHE_PART)
    );
    let src = wb
        .get(sheet, 0, 0)
        .unwrap()
        .unwrap()
        .formula
        .and_then(|id| wb.intern().formulas.get(id).map(str::to_string));
    assert!(src.as_deref().is_some_and(is_ai_formula));
    let bytes = save_workbook_bytes(&wb).unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), &bytes).unwrap();
    let mut cal = calamine::open_workbook::<calamine::Xlsx<_>, _>(tmp.path()).unwrap();
    use calamine::Reader;
    let range = cal.worksheet_range("Sheet1").unwrap();
    let cell = range.get((0, 0)).cloned().unwrap();
    match cell {
        calamine::Data::String(s) => assert_eq!(s, "Ada"),
        other => panic!("expected cached string, got {other:?}"),
    }
    let again = open_bytes(&bytes).unwrap();
    assert!(
        again
            .workbook
            .custom_parts
            .contains_key(omacell_ai::cache::AICACHE_PART)
    );
    let reopened_slot = again.workbook.get(sheet, 0, 0).unwrap().unwrap();
    let reopened_formula = reopened_slot
        .formula
        .and_then(|id| again.workbook.intern().formulas.get(id));
    assert!(reopened_formula.is_some_and(is_ai_formula));
    let cache = omacell_ai::cache::AiCache::from_bytes(
        again
            .workbook
            .custom_parts
            .get(omacell_ai::cache::AICACHE_PART)
            .unwrap(),
    );
    assert_eq!(cache.entries.len(), 1);
    let provenance = cache.entries.values().next().unwrap();
    assert_eq!(provenance.provider, "ollama");
    assert_eq!(provenance.model, "qwen");
    assert!(!provenance.prompt_hash.is_empty());
    assert!(!provenance.input_hash.is_empty());
    assert!(provenance.ts > 0);
    if let Some(soffice) = libreoffice::find_calc() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("fill.xlsx");
        let profile = dir.path().join("profile");
        let output_dir = dir.path().join("output");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(&output_dir).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let output = std::process::Command::new(&soffice)
            .arg(format!(
                "-env:UserInstallation=file://{}",
                profile.display()
            ))
            .env("HOME", dir.path())
            .env("XDG_CACHE_HOME", dir.path().join("cache"))
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("SAL_USE_VCLPLUGIN", "svp")
            .args(["--headless", "--convert-to", "csv", "--outdir"])
            .arg(&output_dir)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "LibreOffice could not reopen fill.xlsx: status={:?} stdout={} stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let csv = std::fs::read_to_string(output_dir.join("fill.csv")).unwrap();
        assert!(
            csv.contains("Ada"),
            "LibreOffice did not expose the cached value: {csv:?}"
        );
    }
}

#[test]
fn strip_ai_formulas_keeps_values() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, r#"=AI("x")"#).unwrap();
    assert!(is_ai_formula(
        wb.get(sheet, 0, 0)
            .unwrap()
            .unwrap()
            .formula
            .and_then(|id| wb.intern().formulas.get(id).map(str::to_string))
            .unwrap()
            .as_str()
    ));
    strip_ai_formulas(&mut wb).unwrap();
    let slot = wb.get(sheet, 0, 0).unwrap().unwrap();
    assert!(slot.formula.is_none());
}

#[test]
fn strip_ai_formulas_detaches_fixed_array_metadata() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let range = omacell_core::addr::RangeRef::from_corners(
        omacell_core::addr::CellRef::new(0, 0).unwrap(),
        omacell_core::addr::CellRef::new(0, 1).unwrap(),
    );
    wb.set_array_formula_text(sheet, range, r#"=AI("x")"#)
        .unwrap();

    strip_ai_formulas(&mut wb).unwrap();

    assert!(wb.sheet(sheet).unwrap().array_formula_at(0, 0).is_none());
    let anchor = wb.get(sheet, 0, 0).unwrap().unwrap();
    assert!(anchor.formula.is_none());
    assert!(!anchor.flags.array());
}

#[test]
fn strip_ai_formulas_preserves_formula_like_text_exactly() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, r#"=AI("x")"#).unwrap();
    let text = wb.intern_text("=not a formula");
    let mut slot = *wb.get(sheet, 0, 0).unwrap().unwrap();
    slot.value = CellValue::Text(text);
    wb.set_slot(sheet, 0, 0, slot).unwrap();
    wb.release_text(text);
    strip_ai_formulas(&mut wb).unwrap();
    let slot = wb.get(sheet, 0, 0).unwrap().unwrap();
    assert!(slot.formula.is_none());
    let CellValue::Text(id) = slot.value else {
        panic!("expected text, got {:?}", slot.value);
    };
    assert_eq!(wb.intern().strings.get(id), Some("=not a formula"));
}

#[test]
fn import_overlay_never_requires_apply() {
    let plan = parse_plan_overlay(&json!({
        "plan": {
            "delimiter": ",",
            "has_header": true
        }
    }))
    .unwrap();
    assert!(plan.has_header);
}

#[test]
fn unknown_plan_command_is_rejected() {
    let err = parse_plan(
        &json!({"commands":[{"id":"not.a.command","args":{}}]}),
        &catalog(),
    )
    .unwrap_err();
    assert!(err.message.contains("unknown"));
}

#[test]
fn empty_plan_catalog_fails_closed() {
    let err = parse_plan(
        &json!({"commands":[{"id":"cell.set","args":{}}]}),
        &BTreeSet::new(),
    )
    .unwrap_err();
    assert!(err.message.contains("unknown"));
}

proptest::proptest! {
    #[test]
    fn plan_ids_are_dotted_and_not_forbidden(n in 0u8..10) {
        let ids = ["cell.set", "range.sort", "sheet.add", "format.bold"];
        let id = ids[n as usize % ids.len()];
        let plan = parse_plan(&json!({"commands":[{"id": id, "args": {}}]}), &catalog()).unwrap();
        assert!(!omacell_ai::forbidden(&plan.commands[0].id));
        assert!(to_calls(&plan).is_ok());
    }
}
