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
use crate::resolve::{ResolvedCell, format_cell, resolve_cell};

/// `edit.findall` options.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindArgs {
    /// Needle.
    pub query: String,
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
}

/// `edit.replacepreview` / `edit.replaceall` options.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceArgs {
    /// Find options.
    #[serde(flatten)]
    pub find: FindArgs,
    /// Replacement text.
    pub replacement: String,
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
    /// Selection kind.
    pub kind: GotoSpecialKind,
    /// Visible cells only.
    #[serde(default)]
    pub visible: bool,
    /// Origin cell for precedent/dependent selection.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub cell_ref: Option<String>,
    /// Walk the full dependency chain.
    #[serde(default)]
    pub transitive: bool,
}

/// Typed Go To Special selection.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GotoSpecialKind {
    /// Empty cells.
    Blanks,
    /// Numeric constants.
    Numbers,
    /// Text constants.
    Text,
    /// Logical constants.
    Logicals,
    /// Error constants.
    Errors,
    /// All formulas.
    Formulas,
    /// Formulas with numeric results.
    FormulaNumbers,
    /// Formulas with text results.
    FormulaText,
    /// Formulas with logical results.
    FormulaLogicals,
    /// Formulas with error results.
    FormulaErrors,
    /// Visible stored cells.
    Visible,
    /// Direct or transitive precedents of `ref`.
    Precedents,
    /// Direct or transitive dependents of `ref`.
    Dependents,
    /// Cells covered by conditional formatting.
    CondFormats,
    /// Cells covered by data validation.
    Validation,
}

/// One cell argument.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellArgs {
    /// A1 cell.
    #[serde(rename = "ref")]
    pub cell_ref: String,
}

/// One cell plus dependency traversal options.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceCellArgs {
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
            id: "edit.replacepreview",
            doc: "Preview replacements without changing the workbook",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_replace_preview,
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
    Ok(Effect::query(to_json(&report)?))
}

fn audit_diagnose(ctx: &mut CommandContext<'_>, args: CellArgs) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let bundle = diagnose(
        ctx.workbook_ref(),
        ctx.engine_ref(),
        CellCoord::new(cell.sheet, cell.row, cell.col),
    );
    Ok(Effect::query(to_json(&bundle)?))
}

fn formula_explain(ctx: &mut CommandContext<'_>, args: CellArgs) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let exp = explain_error(
        ctx.workbook_ref(),
        ctx.engine_ref(),
        CellCoord::new(cell.sheet, cell.row, cell.col),
    );
    Ok(Effect::query(to_json(&exp)?))
}

fn formula_trace(ctx: &mut CommandContext<'_>, args: CellArgs) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let steps = eval_steps(
        ctx.workbook_ref(),
        ctx.engine_ref(),
        CellCoord::new(cell.sheet, cell.row, cell.col),
    );
    Ok(Effect::query(to_json(&steps)?))
}

fn formula_precedents(
    ctx: &mut CommandContext<'_>,
    args: TraceCellArgs,
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
    args: TraceCellArgs,
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
        .map(|hit| format_hit(ctx.workbook_ref(), hit))
        .collect();
    Ok(Effect::query(
        serde_json::json!({"count": cells.len(), "cells": cells}),
    ))
}

fn edit_replace_preview(
    ctx: &mut CommandContext<'_>,
    args: ReplaceArgs,
) -> Result<Effect, CoreError> {
    let spec = spec_from(&args.find);
    let sheet = ctx.workbook_ref().active_sheet();
    let count = replace_preview(ctx.workbook_ref(), sheet, &spec, &args.replacement)?;
    Ok(Effect::query(
        serde_json::json!({"count": count, "preview": true}),
    ))
}

fn edit_replace(ctx: &mut CommandContext<'_>, args: ReplaceArgs) -> Result<Effect, CoreError> {
    let spec = spec_from(&args.find);
    let sheet = ctx.workbook_ref().active_sheet();
    let hits = find_cells(ctx.workbook_ref(), sheet, &spec)?;
    let n = replace_apply(ctx.workbook(), sheet, &spec, &args.replacement)?;
    let dirty = if n == 0 {
        Vec::new()
    } else {
        hits.into_iter()
            .map(|hit| CellCoord::new(hit.sheet, hit.row, hit.col))
            .collect()
    };
    Ok(Effect {
        result: serde_json::json!({"count": n}),
        summary: omacell_core::changeset::ChangeSummary {
            text: format!("replace {n} cells"),
            cells: u64::from(n),
            ..omacell_core::changeset::ChangeSummary::default()
        },
        dirty,
        rebuild: spec.formulas,
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
    match args.kind {
        GotoSpecialKind::Precedents | GotoSpecialKind::Dependents => {
            let cell_ref = args.cell_ref.as_deref().ok_or_else(|| {
                CoreError::new("goto.ref", "precedents and dependents require ref")
            })?;
            let cell = resolve_cell(ctx.workbook_ref(), cell_ref)?;
            let coord = CellCoord::new(cell.sheet, cell.row, cell.col);
            let cells = if matches!(args.kind, GotoSpecialKind::Precedents) {
                precedents_of(ctx.workbook_ref(), ctx.engine_ref(), coord, args.transitive)
            } else {
                dependents_of(ctx.workbook_ref(), ctx.engine_ref(), coord, args.transitive)
            };
            return Ok(Effect::query(
                serde_json::json!({"count": cells.len(), "cells": cells}),
            ));
        }
        _ => {}
    }
    let kind = match args.kind {
        GotoSpecialKind::Blanks => GotoKind::Blanks,
        GotoSpecialKind::Numbers => GotoKind::Numbers,
        GotoSpecialKind::Text => GotoKind::Text,
        GotoSpecialKind::Logicals => GotoKind::Logicals,
        GotoSpecialKind::Errors => GotoKind::Errors,
        GotoSpecialKind::Formulas => GotoKind::Formulas,
        GotoSpecialKind::FormulaNumbers => GotoKind::FormulaNumbers,
        GotoSpecialKind::FormulaText => GotoKind::FormulaText,
        GotoSpecialKind::FormulaLogicals => GotoKind::FormulaLogicals,
        GotoSpecialKind::FormulaErrors => GotoKind::FormulaErrors,
        GotoSpecialKind::Visible => GotoKind::Visible,
        GotoSpecialKind::CondFormats => GotoKind::CondFormats,
        GotoSpecialKind::Validation => GotoKind::Validation,
        GotoSpecialKind::Precedents | GotoSpecialKind::Dependents => {
            return Err(CoreError::new(
                "goto.kind",
                "dependency selection was not resolved",
            ));
        }
    };
    let hits = goto_special(
        ctx.workbook_ref(),
        ctx.workbook_ref().active_sheet(),
        kind,
        args.visible,
    )?;
    let cells: Vec<_> = hits
        .iter()
        .map(|hit| format_hit(ctx.workbook_ref(), hit))
        .collect();
    Ok(Effect::query(
        serde_json::json!({"count": cells.len(), "cells": cells}),
    ))
}

fn format_hit(wb: &omacell_core::workbook::Workbook, hit: &omacell_core::find::FindHit) -> String {
    format_cell(
        wb,
        ResolvedCell {
            sheet: hit.sheet,
            row: hit.row,
            col: hit.col,
        },
    )
}

fn to_json<T: Serialize>(value: &T) -> Result<serde_json::Value, CoreError> {
    serde_json::to_value(value)
        .map_err(|error| CoreError::new("audit.serialize", error.to_string()))
}
