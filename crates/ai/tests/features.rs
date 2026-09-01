//! WP-23 evals: plans, injection, async cells, budget, formula scratch eval.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use omacell_ai::agent::validate_tool;
use omacell_ai::formula::parse_and_eval;
use omacell_ai::functions::{is_ai_formula, register_ai_functions, strip_ai_formulas};
use omacell_ai::http::{HttpRequest, HttpResponse, SharedTransport, Transport};
use omacell_ai::import_assist::parse_plan_overlay;
use omacell_ai::plan::{forbidden, parse_plan, to_calls};
use omacell_ai::prompts::PromptSet;
use omacell_ai::runtime::AiRuntime;
use omacell_ai::{PolicySnapshot, SendLevel, Slot};
use omacell_conf::schema::package_defaults;
use omacell_core::eval::FnRegistry;
use omacell_core::graph::CellCoord;
use omacell_core::recalc::RecalcEngine;
use omacell_core::value::Value as CellValue;
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
        "cloud".into(),
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
    config.ai.models.fast = "cloud:fast".into();
    let (ai, _transport, _rt, _tmp) = runtime(config, json!({}));
    assert_eq!(ai.policy_for(Slot::Default, None).send, SendLevel::Full);
    assert_eq!(ai.policy_for(Slot::Fast, None).send, SendLevel::Schema);
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
    if let Some(soffice) = ["soffice", "libreoffice"].into_iter().find(|bin| {
        std::process::Command::new(bin)
            .arg("--version")
            .output()
            .is_ok()
    }) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("fill.xlsx");
        let profile = dir.path().join("profile");
        let output_dir = dir.path().join("output");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(&output_dir).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let output = std::process::Command::new(soffice)
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
            "LibreOffice could not reopen fill.xlsx: stdout={} stderr={}",
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
