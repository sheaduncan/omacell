//! Audit, find/replace, and Go To commands (WP-19).

use omacell_core::audit::{
    audit_workbook, dependents_of, diagnose, eval_steps, explain_error, precedents_of,
};
use omacell_core::error::CoreError;
use omacell_core::find::{
    FindSpec, GotoKind, find_cells, goto_spec, goto_special, replace_apply, replace_preview,
};
use omacell_core::graph::CellCoord;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::args::EmptyArgs;
use crate::handler::{CommandContext, Effect};
use crate::registry::{CommandKind, CommandRegistry, CommandSpec, Exposure};
use crate::resolve::resolve_cell;

/// `edit.find` / `edit.replace`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindArgs {
    /// Needle.
    pub query: String,
    /// Replacement (replace only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<String>,
    /// Search formulas.
    #[serde(default)]
    pub formulas: bool,
    /// Whole cell.
    #[serde(default)]
    pub whole: bool,
    /// Case-sensitive.
    #[serde(default)]
    pub case: bool,
    /// Regex.
    #[serde(default)]
    pub regex: bool,
    /// Whole workbook.
    #[serde(default)]
    pub workbook: bool,
    /// Apply replacements (else preview).
    #[serde(default)]
    pub apply: bool,
}

/// `nav.goto`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GotoArgs {
    /// A1 or defined name.
    pub spec: String,
}

/// `nav.gotospecial`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GotoSpecialArgs {
    /// Kind name.
    pub kind: String,
    /// Visible cells only.
    #[serde(default)]
    pub visible: bool,
}

/// `formula.explain` / `formula.trace` / precedents
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellRefArgs {
    /// A1 cell.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// Walk the full chain.
    #[serde(default)]
    pub transitive: bool,
}

/// Register WP-19 commands.
pub fn register_audit_commands(registry: &mut CommandRegistry) -> Result<(), CoreError> {
    registry.register(
        CommandSpec {
            id: "audit.run",
            doc: "Run the deterministic workbook audit",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        audit_run,
    )?;
    registry.register(
        CommandSpec {
            id: "audit.diagnose",
            doc: "Build a diagnostic bundle around a cell",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        audit_diagnose,
    )?;
    registry.register(
        CommandSpec {
            id: "formula.explain",
            doc: "Explain a cell error",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        formula_explain,
    )?;
    registry.register(
        CommandSpec {
            id: "formula.trace",
            doc: "Evaluate-Formula step trace",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        formula_trace,
    )?;
    registry.register(
        CommandSpec {
            id: "formula.precedents",
            doc: "List formula precedents",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        formula_precedents,
    )?;
    registry.register(
        CommandSpec {
            id: "formula.dependents",
            doc: "List formula dependents",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        formula_dependents,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.findall",
            doc: "Find matching cells",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_find,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.replaceall",
            doc: "Replace matching cells",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_replace,
    )?;
    registry.register(
        CommandSpec {
            id: "nav.address",
            doc: "Resolve Go To by A1 or defined name",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        nav_goto,
    )?;
    registry.register(
        CommandSpec {
            id: "nav.gotospecial",
            doc: "Go To Special",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        nav_gotospecial,
    )?;
    Ok(())
}

fn spec_from(args: &FindArgs) -> FindSpec {
    FindSpec {
        query: args.query.clone(),
        formulas: args.formulas,
        whole: args.whole,
        case: args.case,
        regex: args.regex,
        workbook: args.workbook,
    }
}

fn audit_run(ctx: &mut CommandContext<'_>, _args: EmptyArgs) -> Result<Effect, CoreError> {
    let report = audit_workbook(ctx.workbook_ref(), ctx.engine_ref());
    Ok(Effect::query(
        serde_json::to_value(&report).unwrap_or_default(),
    ))
}

fn audit_diagnose(ctx: &mut CommandContext<'_>, args: CellRefArgs) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let bundle = diagnose(
        ctx.workbook_ref(),
        ctx.engine_ref(),
        CellCoord::new(cell.sheet, cell.row, cell.col),
    );
    Ok(Effect::query(
        serde_json::to_value(&bundle).unwrap_or_default(),
    ))
}

fn formula_explain(ctx: &mut CommandContext<'_>, args: CellRefArgs) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let exp = explain_error(
        ctx.workbook_ref(),
        ctx.engine_ref(),
        CellCoord::new(cell.sheet, cell.row, cell.col),
    );
    Ok(Effect::query(
        serde_json::to_value(&exp).unwrap_or_default(),
    ))
}

fn formula_trace(ctx: &mut CommandContext<'_>, args: CellRefArgs) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let steps = eval_steps(
        ctx.workbook_ref(),
        ctx.engine_ref(),
        CellCoord::new(cell.sheet, cell.row, cell.col),
    );
    Ok(Effect::query(
        serde_json::to_value(&steps).unwrap_or_default(),
    ))
}

fn formula_precedents(
    ctx: &mut CommandContext<'_>,
    args: CellRefArgs,
) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let list = precedents_of(
        ctx.workbook_ref(),
        ctx.engine_ref(),
        CellCoord::new(cell.sheet, cell.row, cell.col),
        args.transitive,
    );
    Ok(Effect::query(serde_json::json!({"cells": list})))
}

fn formula_dependents(
    ctx: &mut CommandContext<'_>,
    args: CellRefArgs,
) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let list = dependents_of(
        ctx.workbook_ref(),
        ctx.engine_ref(),
        CellCoord::new(cell.sheet, cell.row, cell.col),
        args.transitive,
    );
    Ok(Effect::query(serde_json::json!({"cells": list})))
}

fn edit_find(ctx: &mut CommandContext<'_>, args: FindArgs) -> Result<Effect, CoreError> {
    let spec = spec_from(&args);
    let hits = find_cells(ctx.workbook_ref(), ctx.workbook_ref().active_sheet(), &spec)?;
    let cells: Vec<String> = hits
        .iter()
        .map(|h| {
            format!(
                "{}{}",
                omacell_core::addr::col_to_letters(h.col).unwrap_or_else(|_| "A".into()),
                h.row + 1
            )
        })
        .collect();
    Ok(Effect::query(
        serde_json::json!({"count": cells.len(), "cells": cells}),
    ))
}

fn edit_replace(ctx: &mut CommandContext<'_>, args: FindArgs) -> Result<Effect, CoreError> {
    let spec = spec_from(&args);
    let replacement = args.replace.clone().unwrap_or_default();
    let sheet = ctx.workbook_ref().active_sheet();
    if ctx.is_preflight() || !args.apply {
        let n = replace_preview(ctx.workbook_ref(), sheet, &spec, &replacement)?;
        return Ok(Effect::query(
            serde_json::json!({"count": n, "preview": true}),
        ));
    }
    let n = replace_apply(ctx.workbook(), sheet, &spec, &replacement)?;
    Ok(Effect {
        result: serde_json::json!({"count": n}),
        summary: omacell_core::changeset::ChangeSummary {
            text: format!("replace {n} cells"),
            cells: u64::from(n),
            ..omacell_core::changeset::ChangeSummary::default()
        },
        ..Effect::default()
    })
}

fn nav_goto(ctx: &mut CommandContext<'_>, args: GotoArgs) -> Result<Effect, CoreError> {
    let (sheet, row, col) = goto_spec(ctx.workbook_ref(), &args.spec)?;
    Ok(Effect::query(serde_json::json!({
        "sheet": sheet.index(),
        "row": row,
        "col": col,
    })))
}

fn nav_gotospecial(
    ctx: &mut CommandContext<'_>,
    args: GotoSpecialArgs,
) -> Result<Effect, CoreError> {
    let kind = match args.kind.as_str() {
        "blanks" => GotoKind::Blanks,
        "numbers" => GotoKind::Numbers,
        "text" => GotoKind::Text,
        "logicals" => GotoKind::Logicals,
        "errors" => GotoKind::Errors,
        "formulas" => GotoKind::Formulas,
        "formula_errors" => GotoKind::FormulaErrors,
        "visible" => GotoKind::Visible,
        "cond_formats" => GotoKind::CondFormats,
        "validation" => GotoKind::Validation,
        other => {
            return Err(CoreError::new(
                "goto.kind",
                format!("unknown Go To Special kind {other}"),
            ));
        }
    };
    let hits = goto_special(
        ctx.workbook_ref(),
        ctx.workbook_ref().active_sheet(),
        kind,
        args.visible,
    )?;
    Ok(Effect::query(serde_json::json!({"count": hits.len()})))
}
