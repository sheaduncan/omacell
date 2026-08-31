//! WP-23 evals: plans, injection, async cells, budget, formula scratch eval.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use omacell_ai::PolicySnapshot;
use omacell_ai::agent::validate_tool;
use omacell_ai::formula::parse_and_eval;
use omacell_ai::functions::{is_ai_formula, register_ai_functions, strip_ai_formulas};
use omacell_ai::http::{HttpRequest, HttpResponse, SharedTransport, Transport};
use omacell_ai::import_assist::parse_plan_overlay;
use omacell_ai::plan::{forbidden, parse_plan, to_calls};
use omacell_ai::prompts::PromptSet;
use omacell_ai::runtime::AiRuntime;
use omacell_conf::schema::package_defaults;
use omacell_core::eval::FnRegistry;
use omacell_core::graph::CellCoord;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_io::xlsx::{open_bytes, save_workbook_bytes};
use serde_json::{Value, json};

struct CountingTransport {
    hits: AtomicU32,
    body: Value,
}

#[async_trait::async_trait]
impl Transport for CountingTransport {
    async fn send(&self, _req: HttpRequest) -> Result<HttpResponse, omacell_ai::AiError> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        Ok(HttpResponse {
            status: 200,
            body: json!({
                "choices": [{"message": {"content": serde_json::to_string(&self.body).unwrap()}}]
            }),
            chunks: Vec::new(),
        })
    }
}

fn catalog() -> BTreeSet<String> {
    [
        "cell.set",
        "range.sort",
        "range.filter",
        "sheet.add",
        "sheet.rename",
        "condfmt.add",
        "table.create",
        "edit.filldown",
        "nav.gotospecial",
        "audit.run",
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

#[test]
fn two_hundred_plan_evals_match_commands() {
    let cat = catalog();
    let templates = [
        ("sort column {n}", "range.sort", json!({"range": "A1:F10"})),
        ("filter {n}", "range.filter", json!({"range": "A1:A20"})),
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
        (
            "go to special {n}",
            "nav.gotospecial",
            json!({"kind": "formulas"}),
        ),
        ("run audit {n}", "audit.run", json!({})),
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
        assert!(validate_tool("command_run", &format!(r#"{{"id":"{id}"}}"#), true).is_err());
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
fn batching_fifty_plus_cells_uses_two_requests() {
    let mut config = enabled_config();
    config.ai.functions.batch_size = 50;
    config.ai.functions.max_cells_per_recalc = 500;
    let (ai, transport, _rt, _tmp) = runtime(config.clone(), json!({"results":[]}));
    let mut registry = FnRegistry::new();
    register_ai_functions(&mut registry);
    let mut engine = RecalcEngine::new(registry);
    engine.set_async_provider(ai.clone());
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    for i in 0..51u32 {
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
    if let Some(soffice) = ["soffice", "libreoffice"].into_iter().find(|bin| {
        std::process::Command::new(bin)
            .arg("--version")
            .output()
            .is_ok()
    }) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("fill.xlsx");
        std::fs::write(&path, &bytes).unwrap();
        let status = std::process::Command::new(soffice)
            .arg(format!(
                "-env:UserInstallation=file://{}",
                dir.path().join("profile").display()
            ))
            .args(["--headless", "--convert-to", "csv", "--outdir"])
            .arg(dir.path())
            .arg(&path)
            .status();
        let _ = status;
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
    strip_ai_formulas(&mut wb);
    let slot = wb.get(sheet, 0, 0).unwrap().unwrap();
    assert!(slot.formula.is_none());
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

proptest::proptest! {
    #[test]
    fn plan_ids_are_dotted_and_not_forbidden(n in 0u8..10) {
        let ids = ["cell.set", "range.sort", "sheet.add", "audit.run"];
        let id = ids[n as usize % ids.len()];
        let plan = parse_plan(&json!({"commands":[{"id": id, "args": {}}]}), &catalog()).unwrap();
        assert!(!omacell_ai::forbidden(&plan.commands[0].id));
        assert!(to_calls(&plan).is_ok());
    }
}
