//! `ai.*` command-bus handlers (composition root; `bus` does not depend on `ai`).

use std::sync::Arc;

use omacell_ai::Slot;
use omacell_ai::agent::{
    agent_tools, load_conversation, load_skills, save_conversation, validate_tool,
};
use omacell_ai::audit_ai::{findings_schema, parse_findings};
use omacell_ai::complete::{complete_schema, parse_completion};
use omacell_ai::fence_data;
use omacell_ai::formula::{formula_schema, parse_and_eval};
use omacell_ai::functions::is_ai_formula;
use omacell_ai::import_assist::{import_request_payload, parse_plan_overlay};
use omacell_ai::plan::{parse_plan, plan_schema};
use omacell_ai::policy::PolicySnapshot;
use omacell_ai::runtime::{AiRuntime, completion_enabled, fast_is_local};
use omacell_bus::{Bus, CommandContext, CommandKind, CommandSpec, Effect, Exposure};
use omacell_core::addr::RefKind;
use omacell_core::error::CoreError;
use omacell_core::event::Event;
use omacell_core::graph::CellCoord;
use omacell_core::storage::{CellFlags, CellSlot, UsedRange};
use omacell_io::csv::{ImportPlan, PreviewRows};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Shared runtime for async cells and `ai.*` commands.
#[derive(Clone)]
pub struct AiSession {
    /// Runtime.
    pub runtime: Arc<AiRuntime>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RangeArgs {
    /// Optional A1 range.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "ref")]
    cell_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlanArgs {
    /// Natural-language request.
    prompt: String,
    /// Unused (plans are proposed, not auto-applied).
    #[serde(default)]
    apply: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FormulaArgs {
    /// Description or current formula.
    prompt: String,
    /// Optional cell.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "ref")]
    cell_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CompleteArgs {
    /// Formula-bar prefix.
    prefix: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportArgs {
    /// Current import plan JSON.
    plan: Value,
    /// Bounded raw/converted rows from the retained import preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview: Option<Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}

/// Register AI commands.
pub fn register_ai_commands(bus: &mut Bus, session: AiSession) -> Result<(), CoreError> {
    let refresh = session.clone();
    bus.registry_mut().register::<RangeArgs, _>(
        CommandSpec {
            id: "ai.refresh",
            doc: "Force AI cells to re-query",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args: RangeArgs| {
            if ctx.is_preflight() {
                return Ok(ai_preflight(ctx));
            }
            let cells = ai_cells(ctx, args.cell_ref.as_deref())?;
            if cells.is_empty() {
                return Ok(Effect::query(json!({"refreshed": 0, "pending": 0})));
            }
            refresh.runtime.refresh_cells(&cells);
            let recalc = ctx.recalc_full();
            if recalc.cancelled {
                return Err(CoreError::new(
                    omacell_bus::codes::TASK_CANCELLED,
                    "operation cancelled",
                ));
            }
            ctx.report_progress(
                recalc.cells_evaluated,
                Some(recalc.cells_evaluated),
                "AI refresh",
            );
            Ok(Effect {
                events: vec![Event::RecalcDone {
                    cells: recalc.cells_evaluated,
                    elapsed_ms: recalc.elapsed_ms,
                }],
                result: json!({
                    "refreshed": cells.len(),
                    "pending": recalc.pending_async.len(),
                }),
                auto_recalc: false,
                ..Effect::default()
            })
        },
    )?;
    let pin = session.clone();
    bus.registry_mut().register::<RangeArgs, _>(
        CommandSpec {
            id: "ai.pin",
            doc: "Pin cached AI results",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args: RangeArgs| {
            if ctx.is_preflight() {
                return Ok(ai_preflight(ctx));
            }
            let cells = ai_cells(ctx, args.cell_ref.as_deref())?;
            if cells.is_empty() {
                return Ok(Effect::query(json!({"pinned": 0})));
            }
            pin.runtime.pin_cells(&cells);
            Ok(Effect::query(json!({"pinned": cells.len()})))
        },
    )?;
    let freeze = session.clone();
    bus.registry_mut().register::<RangeArgs, _>(
        CommandSpec {
            id: "ai.freeze",
            doc: "Convert AI formulas to values",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args: RangeArgs| freeze_ai(ctx, &freeze, args.cell_ref.as_deref()),
    )?;
    let plan_s = session.clone();
    bus.registry_mut().register::<PlanArgs, _>(
        CommandSpec {
            id: "ai.plan",
            doc: "Natural-language command plan",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Shift+A"],
        },
        move |ctx, args: PlanArgs| run_plan(ctx, &plan_s, args),
    )?;
    let generate = session.clone();
    bus.registry_mut().register::<FormulaArgs, _>(
        CommandSpec {
            id: "ai.formula.generate",
            doc: "Generate a formula from a description",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args: FormulaArgs| run_formula(ctx, &generate, args, "formula"),
    )?;
    let explain = session.clone();
    bus.registry_mut().register::<FormulaArgs, _>(
        CommandSpec {
            id: "ai.formula.explain",
            doc: "Explain the formula in a cell",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args: FormulaArgs| run_formula(ctx, &explain, args, "formula_explain"),
    )?;
    let fix = session.clone();
    bus.registry_mut().register::<FormulaArgs, _>(
        CommandSpec {
            id: "ai.formula.fix",
            doc: "Fix a formula error",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args: FormulaArgs| run_formula(ctx, &fix, args, "formula_fix"),
    )?;
    let refactor = session.clone();
    bus.registry_mut().register::<FormulaArgs, _>(
        CommandSpec {
            id: "ai.formula.refactor",
            doc: "Refactor a formula",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args: FormulaArgs| run_formula(ctx, &refactor, args, "formula_refactor"),
    )?;
    let complete = session.clone();
    bus.registry_mut().register::<CompleteArgs, _>(
        CommandSpec {
            id: "ai.complete",
            doc: "Ghost-text formula completion",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args: CompleteArgs| run_complete(ctx, &complete, args),
    )?;
    let import = session.clone();
    bus.registry_mut().register::<ImportArgs, _>(
        CommandSpec {
            id: "ai.import.assist",
            doc: "Propose ImportPlan changes (never auto-applies)",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args: ImportArgs| run_import(ctx, &import, args),
    )?;
    let audit = session.clone();
    bus.registry_mut().register::<Empty, _>(
        CommandSpec {
            id: "ai.audit",
            doc: "AI judgments on the deterministic audit",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, _args: Empty| run_audit(ctx, &audit),
    )?;
    let describe = session.clone();
    bus.registry_mut().register::<Empty, _>(
        CommandSpec {
            id: "ai.describe",
            doc: "Summarize the sheet or selection",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, _args: Empty| run_describe(ctx, &describe),
    )?;
    let agent = session.clone();
    bus.registry_mut().register::<PlanArgs, _>(
        CommandSpec {
            id: "ai.agent.turn",
            doc: "One in-app agent turn (review by default)",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args: PlanArgs| run_agent(ctx, &agent, args),
    )?;
    Ok(())
}

fn freeze_ai(
    ctx: &mut CommandContext<'_>,
    session: &AiSession,
    cell_ref: Option<&str>,
) -> Result<Effect, CoreError> {
    let _ = session;
    let targets = ai_cells(ctx, cell_ref)?;
    let mut effect = Effect {
        auto_recalc: false,
        ..Effect::default()
    };
    for coord in targets {
        let Some(slot) = ctx
            .workbook_ref()
            .get(coord.sheet, coord.row, coord.col)
            .ok()
            .flatten()
        else {
            continue;
        };
        let Some(fid) = slot.formula else {
            continue;
        };
        let src = ctx
            .workbook_ref()
            .intern()
            .formulas
            .get(fid)
            .unwrap_or("")
            .to_string();
        if !is_ai_formula(&src) {
            continue;
        }
        let fixed_array = ctx
            .workbook_ref()
            .sheet(coord.sheet)
            .and_then(|sheet| sheet.array_formula_at(coord.row, coord.col))
            .is_some();
        if fixed_array {
            ctx.workbook()
                .detach_array_formula(coord.sheet, coord.row, coord.col)?;
            effect.events.push(Event::CellChanged {
                sheet: coord.sheet,
                row: coord.row,
                col: coord.col,
            });
            effect.dirty.push(coord);
            effect.summary.cells += 1;
            effect.auto_recalc = true;
            continue;
        }
        let mut frozen = *slot;
        frozen.formula = None;
        frozen.flags = frozen
            .flags
            .with(CellFlags::DIRTY, false)
            .with(CellFlags::STALE, false)
            .with(CellFlags::ARRAY, false);
        ctx.workbook()
            .set_slot(coord.sheet, coord.row, coord.col, frozen)?;
        effect.events.push(Event::CellChanged {
            sheet: coord.sheet,
            row: coord.row,
            col: coord.col,
        });
        effect.dirty.push(coord);
        effect.summary.cells += 1;
        effect.auto_recalc = true;
    }
    if effect.summary.cells > 0 {
        effect.summary.text = format!("froze {} AI cells", effect.summary.cells);
    }
    effect.result = json!({"changed": effect.summary.cells});
    Ok(effect)
}

fn run_plan(
    ctx: &mut CommandContext<'_>,
    session: &AiSession,
    args: PlanArgs,
) -> Result<Effect, CoreError> {
    if ctx.is_preflight() {
        return Ok(ai_preflight(ctx));
    }
    let _ = args.apply;
    let card = session
        .runtime
        .workbook_card(ctx.workbook_ref(), Some(ctx.engine_ref()), None)
        .map_err(CoreError::from)?;
    let catalog = session.runtime.catalog_payload();
    let user = format!(
        "{}\n{}\n{}",
        args.prompt,
        fence_data("command registry", &json!(catalog)),
        fence_data("workbook card", &card)
    );
    let reply = session
        .runtime
        .chat_task(Slot::Default, "plan", user, Some(plan_schema()), vec![])
        .map_err(CoreError::from)?;
    let value = structured_reply("plan", &reply.text)?;
    let catalog = session.runtime.catalog();
    let plan = parse_plan(&value, &catalog).map_err(CoreError::from)?;
    Ok(Effect::query(json!(plan)))
}

fn run_formula(
    ctx: &mut CommandContext<'_>,
    session: &AiSession,
    args: FormulaArgs,
    task: &str,
) -> Result<Effect, CoreError> {
    if ctx.is_preflight() {
        return Ok(ai_preflight(ctx));
    }
    let card = session
        .runtime
        .workbook_card(
            ctx.workbook_ref(),
            Some(ctx.engine_ref()),
            args.cell_ref.clone(),
        )
        .map_err(CoreError::from)?;
    let user = format!("{}\n{}", args.prompt, fence_data("workbook card", &card));
    let reply = session
        .runtime
        .chat_task(
            Slot::Default,
            task,
            user,
            Some(if task == "formula_explain" {
                json!({
                    "type": "object",
                    "required": ["explanation"],
                    "additionalProperties": false,
                    "properties": {"explanation": {"type": "string"}}
                })
            } else {
                formula_schema()
            }),
            vec![],
        )
        .map_err(CoreError::from)?;
    let value = structured_reply(task, &reply.text)?;
    if task == "formula_explain" {
        let explanation = value
            .get("explanation")
            .and_then(Value::as_str)
            .ok_or_else(|| CoreError::new("ai.payload", "formula explanation is missing"))?;
        return Ok(Effect::query(json!({"explanation": explanation})));
    }
    let cell = formula_cell(ctx.workbook_ref(), args.cell_ref.as_deref())?;
    match parse_and_eval(&value, ctx.workbook_ref(), ctx.engine_ref(), cell) {
        Ok((src, runtime)) => Ok(Effect::query(json!({
            "formula": src,
            "scratch": format!("{runtime:?}"),
        }))),
        Err(err) if task == "formula" && args.prompt.starts_with('=') => Ok(Effect::query(
            json!({"error": err.message, "prompt": args.prompt}),
        )),
        Err(err) => Err(err.into()),
    }
}

fn run_complete(
    ctx: &mut CommandContext<'_>,
    session: &AiSession,
    args: CompleteArgs,
) -> Result<Effect, CoreError> {
    if ctx.is_preflight() {
        return Ok(ai_preflight(ctx));
    }
    let cfg = session.runtime.config();
    if !completion_enabled(cfg, fast_is_local(cfg)) {
        return Ok(Effect::query(json!({"prefix": args.prefix, "text": ""})));
    }
    let card = session
        .runtime
        .workbook_card_for(Slot::Fast, ctx.workbook_ref(), Some(ctx.engine_ref()), None)
        .map_err(CoreError::from)?;
    let user = format!("{}\n{}", args.prefix, fence_data("workbook card", &card));
    let reply = session
        .runtime
        .chat_task(
            Slot::Fast,
            "complete",
            user,
            Some(complete_schema()),
            vec![],
        )
        .map_err(CoreError::from)?;
    let value = structured_reply("complete", &reply.text)?;
    Ok(Effect::query(json!({
        "prefix": args.prefix,
        "text": parse_completion(&value).map_err(CoreError::from)?,
    })))
}

fn run_import(
    ctx: &mut CommandContext<'_>,
    session: &AiSession,
    args: ImportArgs,
) -> Result<Effect, CoreError> {
    if ctx.is_preflight() {
        return Ok(ai_preflight(ctx));
    }
    let policy = session
        .runtime
        .policy_for(Slot::Default, Some(ctx.workbook_ref()));
    let (current, request) = import_request(&args, &policy)?;
    let user = fence_data("import preview", &request);
    let reply = session
        .runtime
        .chat_task(Slot::Default, "import", user, None, vec![])
        .map_err(CoreError::from)?;
    let value = structured_reply("import", &reply.text)?;
    let proposed = parse_plan_overlay(&value).map_err(CoreError::from)?;
    Ok(Effect::query(json!({
        "current": current,
        "proposed": proposed,
        "applied": false,
    })))
}

fn import_request(
    args: &ImportArgs,
    policy: &PolicySnapshot,
) -> Result<(ImportPlan, Value), CoreError> {
    let current = parse_plan_overlay(&args.plan).map_err(CoreError::from)?;
    let preview = args
        .preview
        .as_ref()
        .map(|preview| {
            serde_json::from_value(preview.clone()).map_err(|error| {
                CoreError::new("ai.payload", format!("invalid import preview: {error}"))
            })
        })
        .transpose()?
        .unwrap_or(PreviewRows {
            header: None,
            rows: Vec::new(),
        });
    let request =
        import_request_payload(current.clone(), preview, policy).map_err(CoreError::from)?;
    Ok((current, request))
}

fn run_audit(ctx: &mut CommandContext<'_>, session: &AiSession) -> Result<Effect, CoreError> {
    if ctx.is_preflight() {
        return Ok(ai_preflight(ctx));
    }
    let findings = omacell_core::audit::audit_workbook(ctx.workbook_ref(), ctx.engine_ref());
    let findings_json = serde_json::to_value(&findings).map_err(|err| {
        CoreError::new(
            "ai.payload",
            format!("cannot serialize deterministic audit findings: {err}"),
        )
    })?;
    let card = session
        .runtime
        .workbook_card(ctx.workbook_ref(), Some(ctx.engine_ref()), None)
        .map_err(CoreError::from)?;
    let user = format!(
        "{}\n{}",
        fence_data("audit findings", &findings_json),
        fence_data("workbook card", &card)
    );
    let reply = session
        .runtime
        .chat_task(
            Slot::Default,
            "audit",
            user,
            Some(findings_schema()),
            vec![],
        )
        .map_err(CoreError::from)?;
    let value = structured_reply("audit", &reply.text)?;
    let extra = parse_findings(&value).map_err(CoreError::from)?;
    Ok(Effect::query(json!({
        "deterministic": findings,
        "ai": extra,
    })))
}

fn run_describe(ctx: &mut CommandContext<'_>, session: &AiSession) -> Result<Effect, CoreError> {
    if ctx.is_preflight() {
        return Ok(ai_preflight(ctx));
    }
    let card = session
        .runtime
        .workbook_card(ctx.workbook_ref(), Some(ctx.engine_ref()), None)
        .map_err(CoreError::from)?;
    let user = fence_data("workbook card", &card);
    let reply = session
        .runtime
        .chat_task(Slot::Default, "describe", user, None, vec![])
        .map_err(CoreError::from)?;
    let value = structured_reply("describe", &reply.text)?;
    Ok(Effect::query(value))
}

fn run_agent(
    ctx: &mut CommandContext<'_>,
    session: &AiSession,
    args: PlanArgs,
) -> Result<Effect, CoreError> {
    if ctx.is_preflight() {
        return Ok(ai_preflight(ctx));
    }
    let cfg = session.runtime.config();
    // `autopilot_opt_in` permits a future per-session toggle; it is not itself consent.
    let autopilot = false;
    let max_ops = cfg.ai.agent.autopilot_max_ops.max(1);
    let skills = load_skills(std::path::Path::new(&cfg.ai.agent.skills_dir));
    let mut conv = load_conversation(session.runtime.state_dir());
    let card = session
        .runtime
        .workbook_card_for(
            Slot::Agent,
            ctx.workbook_ref(),
            Some(ctx.engine_ref()),
            None,
        )
        .map_err(CoreError::from)?;
    let user = format!(
        "{}\n{}\n{}\n{}\n{}",
        args.prompt,
        fence_data(
            "command registry",
            &json!(session.runtime.catalog_payload())
        ),
        fence_data("skills", &json!(skills)),
        fence_data("prior conversation", &json!(&conv.turns)),
        fence_data("workbook card", &card)
    );
    let reply = session
        .runtime
        .chat_task(Slot::Agent, "agent", user, None, agent_tools())
        .map_err(CoreError::from)?;
    let mut proposed = Vec::new();
    let catalog = session.runtime.catalog();
    for (i, call) in reply.tool_calls.iter().enumerate() {
        if i as u32 >= max_ops {
            break;
        }
        match validate_tool(&call.name, &call.arguments, autopilot, &catalog) {
            Ok(tool_args) => proposed.push(tool_args),
            Err(err) => {
                return Err(err.into());
            }
        }
    }
    if proposed.is_empty()
        && let Ok(value) = serde_json::from_str::<Value>(&reply.text)
        && let Ok(plan) = parse_plan(&value, &session.runtime.catalog())
    {
        proposed = plan
            .commands
            .into_iter()
            .map(|c| json!({"id": c.id, "args": c.args}))
            .collect();
    }
    conv.push_bounded(json!({
        "prompt": args.prompt,
        "proposed": proposed,
        "applied": false,
    }))
    .map_err(CoreError::from)?;
    save_conversation(session.runtime.state_dir(), &conv).map_err(CoreError::from)?;
    Ok(Effect::query(json!({
        "prompt": args.prompt,
        "proposed": proposed,
        "applied": false,
        "autopilot": autopilot,
    })))
}

fn ai_cells(ctx: &CommandContext<'_>, cell_ref: Option<&str>) -> Result<Vec<CellCoord>, CoreError> {
    let wb = ctx.workbook_ref();
    let mut out = Vec::new();
    let sheets: Vec<(omacell_core::addr::SheetId, Option<UsedRange>)> = if let Some(raw) = cell_ref
    {
        let parsed = omacell_core::addr::parse_a1(raw)?;
        let sheet = match parsed.sheet.as_ref() {
            Some(spec) if spec.end.is_some() => {
                return Err(CoreError::addr_ref("AI commands do not accept 3-D ranges"));
            }
            Some(spec) => wb.resolve_sheet_name(&spec.start)?,
            None => wb.active_sheet(),
        };
        let used = match parsed.kind {
            RefKind::Cell(cell) => UsedRange {
                min_row: cell.row,
                min_col: cell.col,
                max_row: cell.row,
                max_col: cell.col,
            },
            RefKind::Range(range) => UsedRange {
                min_row: range.start.row.min(range.end.row),
                min_col: range.start.col.min(range.end.col),
                max_row: range.start.row.max(range.end.row),
                max_col: range.start.col.max(range.end.col),
            },
        };
        vec![(sheet, Some(used))]
    } else {
        wb.sheets().map(|sheet| (sheet.id, None)).collect()
    };
    for (sheet, used) in sheets {
        let store = &wb
            .sheet(sheet)
            .ok_or_else(|| CoreError::sheet_id("AI command sheet disappeared"))?
            .store;
        let cells: Vec<(u32, u16, CellSlot)> = match used {
            Some(used) => store
                .iter_region(used.min_row, used.min_col, used.max_row, used.max_col)
                .collect(),
            None => store.iter().collect(),
        };
        for (row, col, slot) in cells {
            let Some(fid) = slot.formula else {
                continue;
            };
            let src = wb.intern().formulas.get(fid).unwrap_or("");
            if is_ai_formula(src) {
                out.push(CellCoord::new(sheet, row, col));
            }
        }
    }
    Ok(out)
}

fn formula_cell(
    wb: &omacell_core::workbook::Workbook,
    cell_ref: Option<&str>,
) -> Result<CellCoord, CoreError> {
    let Some(raw) = cell_ref else {
        return Ok(CellCoord::new(wb.active_sheet(), 0, 0));
    };
    let parsed = omacell_core::addr::parse_a1(raw)?;
    let sheet = match parsed.sheet.as_ref() {
        Some(spec) if spec.end.is_some() => {
            return Err(CoreError::addr_ref(
                "formula assist does not accept a 3-D reference",
            ));
        }
        Some(spec) => wb.resolve_sheet_name(&spec.start)?,
        None => wb.active_sheet(),
    };
    match parsed.kind {
        RefKind::Cell(cell) => Ok(CellCoord::new(sheet, cell.row, cell.col)),
        RefKind::Range(_) => Err(CoreError::addr_ref(
            "formula assist requires a single-cell reference",
        )),
    }
}

fn structured_reply(task: &str, text: &str) -> Result<Value, CoreError> {
    serde_json::from_str(text).map_err(|err| {
        CoreError::new(
            "ai.payload",
            format!("{task} returned invalid structured JSON: {err}"),
        )
    })
}

fn ai_preflight(ctx: &CommandContext<'_>) -> Effect {
    Effect::query(json!({
        "preflight": true,
        "dry_run": ctx.is_dry_run(),
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use omacell_ai::http::{HttpRequest, HttpResponse, SharedTransport, Transport};
    use omacell_ai::{AiRuntime, PromptSet, register_ai_functions};
    use omacell_bus::Bus;
    use omacell_conf::schema::package_defaults;
    use omacell_core::command::Origin;
    use omacell_core::eval::FnRegistry;
    use omacell_core::recalc::RecalcEngine;
    use omacell_core::workbook::Workbook;
    use omacell_fn::register_all;
    use serde_json::json;

    use omacell_ai::plan::{parse_plan, to_calls};

    struct PlanTransport;

    #[async_trait::async_trait]
    impl Transport for PlanTransport {
        async fn send(&self, _req: HttpRequest) -> Result<HttpResponse, omacell_ai::AiError> {
            Ok(HttpResponse {
                status: 200,
                body: json!({
                    "choices": [{"message": {"content": "{\"commands\":[]}"}}]
                }),
                chunks: Vec::new(),
            })
        }
    }

    #[test]
    fn plan_changeset_apply_revert_is_inverse() {
        let mut registry = FnRegistry::new();
        register_all(&mut registry);
        let engine = RecalcEngine::new(registry);
        let mut bus = Bus::new(Workbook::new(), engine).unwrap();
        let sheet = bus.workbook().active_sheet();
        let start = format!("{:?}", bus.workbook().get(sheet, 0, 0));
        let plan = parse_plan(
            &json!({"commands":[{"id":"cell.set","args":{"ref":"A1","input":"1"}}]}),
            &["cell.set".into()].into_iter().collect(),
        )
        .unwrap();
        let calls = to_calls(&plan).unwrap();
        let cs = bus.propose(Origin::PalettePlan, calls).unwrap();
        bus.apply(Origin::User, &cs.id).unwrap();
        let v = bus.workbook().get(sheet, 0, 0).unwrap().unwrap().value;
        assert_eq!(v, omacell_core::value::Value::Number(1.0));
        bus.revert(Origin::User, &cs.id).unwrap();
        let after = format!("{:?}", bus.workbook().get(sheet, 0, 0));
        assert_eq!(after, start);
    }

    #[test]
    fn ai_queries_do_not_send_during_preflight_or_dry_run() {
        let mut config = package_defaults().unwrap();
        config.ai.enabled = true;
        config.ai.providers.insert(
            "test".into(),
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
        config.ai.models.default = "test:model".into();
        let tokio = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let transport: SharedTransport = Arc::new(PlanTransport);
        let runtime = AiRuntime::new(
            tokio.handle().clone(),
            config,
            transport,
            PromptSet::builtin(),
            temp.path().join("cache"),
            temp.path().join("state"),
            Default::default(),
        );
        runtime.set_catalog(vec![(
            "cell.set".into(),
            json!({"id":"cell.set","doc":"Set a cell","args":{"type":"object"}}),
        )]);
        let mut registry = FnRegistry::new();
        register_all(&mut registry);
        let engine = RecalcEngine::new(registry);
        let mut bus = Bus::new(Workbook::new(), engine).unwrap();
        super::register_ai_commands(
            &mut bus,
            super::AiSession {
                runtime: Arc::clone(&runtime),
            },
        )
        .unwrap();

        let dry = bus
            .dry_run(
                Origin::User,
                "ai.plan",
                json!({"prompt":"set A1","apply":false}),
            )
            .unwrap();
        assert!(dry.outcome.ok);
        assert_eq!(runtime.session_stats().requests, 0);

        let live = bus.execute(
            Origin::User,
            "ai.plan",
            json!({"prompt":"set A1","apply":false}),
        );
        assert!(live.ok, "{:?}", live.error);
        assert_eq!(runtime.session_stats().requests, 1);
    }

    #[test]
    fn ai_refresh_schedules_auto_disabled_cells_for_settlement() {
        let mut config = package_defaults().unwrap();
        config.ai.enabled = true;
        config.ai.functions.auto = false;
        config.ai.providers.insert(
            "test".into(),
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
        config.ai.models.default = "test:model".into();
        let tokio = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let transport: SharedTransport = Arc::new(PlanTransport);
        let runtime = AiRuntime::new(
            tokio.handle().clone(),
            config,
            transport,
            PromptSet::builtin(),
            temp.path().join("cache"),
            temp.path().join("state"),
            Default::default(),
        );
        let mut registry = FnRegistry::new();
        register_all(&mut registry);
        register_ai_functions(&mut registry);
        let mut engine = RecalcEngine::new(registry);
        engine.set_async_provider(runtime.clone());
        let mut bus = Bus::new(Workbook::new(), engine).unwrap();
        super::register_ai_commands(
            &mut bus,
            super::AiSession {
                runtime: runtime.clone(),
            },
        )
        .unwrap();

        let set = bus.execute(
            Origin::User,
            "cell.set",
            json!({"ref":"A1","input":"=AI(\"name\")"}),
        );
        assert!(set.ok, "{:?}", set.error);
        assert!(runtime.pending_generation().is_none());

        let empty = bus.execute(Origin::User, "ai.refresh", json!({"ref":"B1"}));
        assert!(empty.ok, "{:?}", empty.error);
        assert_eq!(empty.result.unwrap()["refreshed"], 0);
        assert!(runtime.pending_generation().is_none());

        let refresh = bus.execute(Origin::User, "ai.refresh", json!({"ref":"A1"}));
        assert!(refresh.ok, "{:?}", refresh.error);
        assert_eq!(refresh.result.unwrap()["pending"], 1);
        assert!(runtime.pending_generation().is_some());
    }

    #[test]
    fn import_request_includes_the_bounded_preview() {
        let policy = omacell_ai::PolicySnapshot {
            enabled: true,
            send: omacell_ai::SendLevel::Full,
            suggest_redaction: false,
            log_content: false,
            marks: Vec::new(),
            local: true,
        };
        let (current, payload) = super::import_request(
            &super::ImportArgs {
                plan: json!({"delimiter": ",", "has_header": true}),
                preview: Some(json!({
                    "header": ["sample"],
                    "rows": [[{
                        "raw": "007",
                        "would_become": "007",
                        "kind": "text",
                        "changed": false
                    }]]
                })),
            },
            &policy,
        )
        .unwrap();

        assert!(current.has_header);
        assert_eq!(payload["plan"]["has_header"], true);
        assert_eq!(payload["preview"]["rows"][0][0]["raw"], "007");
    }
}
