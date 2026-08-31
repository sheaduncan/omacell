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
use omacell_ai::functions::{is_ai_formula, stored_input};
use omacell_ai::import_assist::parse_plan_overlay;
use omacell_ai::plan::{parse_plan, plan_schema};
use omacell_ai::runtime::{AiRuntime, completion_enabled, fast_is_local};
use omacell_bus::{Bus, CommandContext, CommandKind, CommandSpec, Effect, Exposure};
use omacell_core::addr::{col_to_letters, quote_sheet_name};
use omacell_core::changeset::CommandCall;
use omacell_core::command::CommandId;
use omacell_core::error::CoreError;
use omacell_core::event::Event;
use omacell_core::graph::CellCoord;
use omacell_core::storage::UsedRange;
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
            let cells = ai_cells(ctx, args.cell_ref.as_deref());
            refresh.runtime.refresh_cells(&cells);
            Ok(Effect::query(json!({"refreshed": cells.len()})))
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
            let cells = ai_cells(ctx, args.cell_ref.as_deref());
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
        move |ctx, args: FormulaArgs| run_formula(ctx, &explain, args, "formula"),
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
        move |ctx, args: FormulaArgs| run_formula(ctx, &fix, args, "formula"),
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
        move |ctx, args: FormulaArgs| run_formula(ctx, &refactor, args, "formula"),
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
    let targets = ai_cells(ctx, cell_ref);
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
        let input = stored_input(ctx.workbook_ref(), slot);
        let addr = cell_addr(ctx.workbook_ref(), coord);
        let inverse = CommandCall {
            id: CommandId::new("cell.set")?,
            args: json!({"ref": addr, "input": src}),
        };
        ctx.workbook()
            .set_cell_contents(coord.sheet, coord.row, coord.col, &input)?;
        effect.inverse.push(inverse);
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
    let _ = args.apply;
    let card = session
        .runtime
        .workbook_card(ctx.workbook_ref(), Some(ctx.engine_ref()), None)
        .unwrap_or_else(|_| json!({"schema": 1, "kind": "summary"}));
    let catalog: Vec<String> = session.runtime.catalog().into_iter().collect();
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
    let value: Value = serde_json::from_str(&reply.text).unwrap_or(json!({"commands":[]}));
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
    let card = session
        .runtime
        .workbook_card(
            ctx.workbook_ref(),
            Some(ctx.engine_ref()),
            args.cell_ref.clone(),
        )
        .unwrap_or_else(|_| json!({"schema": 1, "kind": "summary"}));
    let user = format!("{}\n{}", args.prompt, fence_data("workbook card", &card));
    let reply = session
        .runtime
        .chat_task(Slot::Default, task, user, Some(formula_schema()), vec![])
        .map_err(CoreError::from)?;
    let value: Value = serde_json::from_str(&reply.text).unwrap_or(json!({"formula": reply.text}));
    let cell = CellCoord::new(ctx.workbook_ref().active_sheet(), 0, 0);
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
    let cfg = session.runtime.config();
    if !completion_enabled(cfg, fast_is_local(cfg)) {
        return Ok(Effect::query(json!({"prefix": args.prefix, "text": ""})));
    }
    let card = session
        .runtime
        .workbook_card(ctx.workbook_ref(), Some(ctx.engine_ref()), None)
        .unwrap_or_else(|_| json!({}));
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
    let value: Value = serde_json::from_str(&reply.text).unwrap_or(json!({"text": reply.text}));
    Ok(Effect::query(json!({
        "prefix": args.prefix,
        "text": parse_completion(&value),
    })))
}

fn run_import(
    ctx: &mut CommandContext<'_>,
    session: &AiSession,
    args: ImportArgs,
) -> Result<Effect, CoreError> {
    let _ = ctx;
    let user = fence_data("import plan", &args.plan);
    let reply = session
        .runtime
        .chat_task(Slot::Default, "import", user, None, vec![])
        .map_err(CoreError::from)?;
    let value: Value = serde_json::from_str(&reply.text).unwrap_or(json!({"plan": args.plan}));
    let proposed = parse_plan_overlay(&value).map_err(CoreError::from)?;
    Ok(Effect::query(json!({
        "current": args.plan,
        "proposed": proposed,
        "applied": false,
    })))
}

fn run_audit(ctx: &mut CommandContext<'_>, session: &AiSession) -> Result<Effect, CoreError> {
    let findings = omacell_core::audit::audit_workbook(ctx.workbook_ref(), ctx.engine_ref());
    let findings_json = serde_json::to_value(&findings).unwrap_or(json!({"findings":[]}));
    let card = session
        .runtime
        .workbook_card(ctx.workbook_ref(), Some(ctx.engine_ref()), None)
        .unwrap_or_else(|_| json!({}));
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
    let value: Value = serde_json::from_str(&reply.text).unwrap_or(json!({"findings":[]}));
    let extra = parse_findings(&value).unwrap_or_default();
    Ok(Effect::query(json!({"findings": extra})))
}

fn run_describe(ctx: &mut CommandContext<'_>, session: &AiSession) -> Result<Effect, CoreError> {
    let card = session
        .runtime
        .workbook_card(ctx.workbook_ref(), Some(ctx.engine_ref()), None)
        .unwrap_or_else(|_| json!({}));
    let user = fence_data("workbook card", &card);
    let reply = session
        .runtime
        .chat_task(Slot::Default, "describe", user, None, vec![])
        .map_err(CoreError::from)?;
    let value: Value = serde_json::from_str(&reply.text).unwrap_or(json!({"summary": reply.text}));
    Ok(Effect::query(value))
}

fn run_agent(
    ctx: &mut CommandContext<'_>,
    session: &AiSession,
    args: PlanArgs,
) -> Result<Effect, CoreError> {
    let cfg = session.runtime.config();
    let autopilot = cfg.ai.agent.review != "always";
    let max_ops = cfg.ai.agent.autopilot_max_ops.max(1);
    let skills = load_skills(std::path::Path::new(&cfg.ai.agent.skills_dir));
    let skill_names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    let mut conv = load_conversation(session.runtime.state_dir());
    let card = session
        .runtime
        .workbook_card(ctx.workbook_ref(), Some(ctx.engine_ref()), None)
        .unwrap_or_else(|_| json!({}));
    let user = format!(
        "{}\n{}\n{}",
        args.prompt,
        fence_data("skills", &json!(skill_names)),
        fence_data("workbook card", &card)
    );
    let reply = session
        .runtime
        .chat_task(Slot::Agent, "agent", user, None, agent_tools())
        .map_err(CoreError::from)?;
    let mut proposed = Vec::new();
    for (i, call) in reply.tool_calls.iter().enumerate() {
        if autopilot && i as u32 >= max_ops {
            break;
        }
        match validate_tool(&call.name, &call.arguments, autopilot) {
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
    conv.turns
        .push(json!({"prompt": args.prompt, "proposed": proposed, "applied": false}));
    let _ = save_conversation(session.runtime.state_dir(), &conv);
    Ok(Effect::query(json!({
        "prompt": args.prompt,
        "proposed": proposed,
        "applied": false,
        "autopilot": autopilot,
    })))
}

fn ai_cells(ctx: &CommandContext<'_>, cell_ref: Option<&str>) -> Vec<CellCoord> {
    let wb = ctx.workbook_ref();
    let mut out = Vec::new();
    let sheets: Vec<(omacell_core::addr::SheetId, UsedRange)> = if let Some(raw) = cell_ref {
        match omacell_core::addr::parse_a1(raw) {
            Ok(parsed) => {
                let sheet = parsed
                    .sheet
                    .as_ref()
                    .and_then(|spec| wb.sheet_by_name(&spec.start).map(|s| s.id))
                    .unwrap_or_else(|| wb.active_sheet());
                let used = match parsed.kind {
                    omacell_core::addr::RefKind::Cell(cell) => UsedRange {
                        min_row: cell.row,
                        min_col: cell.col,
                        max_row: cell.row,
                        max_col: cell.col,
                    },
                    omacell_core::addr::RefKind::Range(range) => UsedRange {
                        min_row: range.start.row.min(range.end.row),
                        min_col: range.start.col.min(range.end.col),
                        max_row: range.start.row.max(range.end.row),
                        max_col: range.start.col.max(range.end.col),
                    },
                };
                vec![(sheet, used)]
            }
            Err(_) => return out,
        }
    } else {
        wb.sheets()
            .filter_map(|sheet| sheet.used_range().map(|used| (sheet.id, used)))
            .collect()
    };
    for (sheet, used) in sheets {
        for row in used.min_row..=used.max_row {
            for col in used.min_col..=used.max_col {
                let Some(slot) = wb.get(sheet, row, col).ok().flatten() else {
                    continue;
                };
                let Some(fid) = slot.formula else {
                    continue;
                };
                let src = wb.intern().formulas.get(fid).unwrap_or("");
                if is_ai_formula(src) {
                    out.push(CellCoord::new(sheet, row, col));
                }
            }
        }
    }
    out
}

fn cell_addr(wb: &omacell_core::workbook::Workbook, coord: CellCoord) -> String {
    let name = wb
        .sheet(coord.sheet)
        .map(|s| s.name.as_str())
        .unwrap_or("Sheet1");
    let letters = col_to_letters(coord.col).unwrap_or_else(|_| "A".into());
    format!("{}!{}{}", quote_sheet_name(name), letters, coord.row + 1)
}

#[cfg(test)]
mod tests {
    use omacell_bus::Bus;
    use omacell_core::command::Origin;
    use omacell_core::eval::FnRegistry;
    use omacell_core::recalc::RecalcEngine;
    use omacell_core::workbook::Workbook;
    use omacell_fn::register_all;
    use serde_json::json;

    use omacell_ai::plan::{parse_plan, to_calls};

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
}
