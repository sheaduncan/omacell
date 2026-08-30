//! Pivot, Goal Seek, and statistics commands (WP-24).

use std::collections::BTreeMap;

use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::changeset::ChangeSummary;
use omacell_core::error::CoreError;
use omacell_core::graph::CellCoord;
use omacell_core::pivot::{
    DateGroup, PivotAgg, PivotDataField, PivotGroup, PivotId, PivotLayout, PivotTable, ShowAs,
};
use omacell_core::stats::describe_range;
use omacell_core::whatif::{DEFAULT_MAX_ITER, DEFAULT_TOL, goal_seek};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::handler::{CommandContext, Effect};
use crate::logical::{call, slot_input};
use crate::registry::{CommandKind, CommandRegistry, CommandSpec, Exposure};
use crate::resolve::{format_cell, resolve_cell, resolve_range};

/// `pivot.create`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PivotCreateArgs {
    /// Source A1 range including the header row.
    pub source: String,
    /// Output origin A1 cell.
    pub dest: String,
    /// Display name. Assigned as `PivotN` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Row fields.
    #[serde(default)]
    pub rows: Vec<String>,
    /// Column fields.
    #[serde(default)]
    pub cols: Vec<String>,
    /// Data fields.
    #[serde(default)]
    pub data: Vec<PivotDataArg>,
    /// Page/filter fields.
    #[serde(default)]
    pub filters: Vec<PivotFilterArg>,
    /// Grouping keyed by source field name.
    #[serde(default)]
    pub groups: BTreeMap<String, PivotGroupArg>,
    /// Layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<PivotLayoutArg>,
    /// Grand totals for rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grand_rows: Option<bool>,
    /// Grand totals for columns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grand_cols: Option<bool>,
    /// Subtotals for outer row fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtotals: Option<bool>,
    /// Refresh when the file opens.
    #[serde(default)]
    pub refresh_on_load: bool,
}

/// One data field argument.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PivotDataArg {
    /// Source column name.
    pub source: String,
    /// Aggregation.
    #[serde(default)]
    pub agg: PivotAggArg,
    /// Show-values-as.
    #[serde(default)]
    pub show_as: ShowAsArg,
}

/// Aggregation argument.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PivotAggArg {
    /// Sum.
    #[default]
    Sum,
    /// Count of numbers.
    Count,
    /// Average.
    Average,
    /// Minimum.
    Min,
    /// Maximum.
    Max,
    /// Count of non-empty.
    Counta,
    /// Distinct count.
    DistinctCount,
    /// Sample standard deviation.
    Stdev,
    /// Sample variance.
    Var,
}

impl From<PivotAggArg> for PivotAgg {
    fn from(value: PivotAggArg) -> Self {
        match value {
            PivotAggArg::Sum => Self::Sum,
            PivotAggArg::Count => Self::Count,
            PivotAggArg::Average => Self::Average,
            PivotAggArg::Min => Self::Min,
            PivotAggArg::Max => Self::Max,
            PivotAggArg::Counta => Self::CountA,
            PivotAggArg::DistinctCount => Self::DistinctCount,
            PivotAggArg::Stdev => Self::Stdev,
            PivotAggArg::Var => Self::Var,
        }
    }
}

/// Show-as argument.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShowAsArg {
    /// Raw aggregate.
    #[default]
    Normal,
    /// Percent of grand total.
    PctOfTotal,
    /// Percent of row total.
    PctOfRow,
    /// Percent of column total.
    PctOfCol,
    /// Running total.
    RunningTotal,
    /// Difference from previous row.
    DifferenceFrom,
}

impl From<ShowAsArg> for ShowAs {
    fn from(value: ShowAsArg) -> Self {
        match value {
            ShowAsArg::Normal => Self::Normal,
            ShowAsArg::PctOfTotal => Self::PctOfTotal,
            ShowAsArg::PctOfRow => Self::PctOfRow,
            ShowAsArg::PctOfCol => Self::PctOfCol,
            ShowAsArg::RunningTotal => Self::RunningTotal,
            ShowAsArg::DifferenceFrom => Self::DifferenceFrom,
        }
    }
}

/// Layout argument.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PivotLayoutArg {
    /// Compact.
    #[default]
    Compact,
    /// Outline.
    Outline,
    /// Tabular.
    Tabular,
}

impl From<PivotLayoutArg> for PivotLayout {
    fn from(value: PivotLayoutArg) -> Self {
        match value {
            PivotLayoutArg::Compact => Self::Compact,
            PivotLayoutArg::Outline => Self::Outline,
            PivotLayoutArg::Tabular => Self::Tabular,
        }
    }
}

/// Filter field argument.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PivotFilterArg {
    /// Source column name.
    pub source: String,
    /// Allowed display values. Empty means all.
    #[serde(default)]
    pub values: Vec<String>,
}

/// Grouping argument.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PivotGroupArg {
    /// Date grain (`days` / `months` / `quarters` / `years`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Numeric bin origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    /// Numeric bin width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
}

/// Identify a pivot by id or name.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PivotIdArgs {
    /// Numeric id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `whatif.goalseek`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GoalSeekArgs {
    /// Target formula cell.
    pub target: String,
    /// Desired value.
    pub goal: f64,
    /// Input cell to vary.
    pub input: String,
    /// Iteration cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iter: Option<u32>,
    /// Absolute residual tolerance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tol: Option<f64>,
}

/// `stats.describe`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatsDescribeArgs {
    /// A1 range.
    pub range: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PivotRestoreArgs {
    table: serde_json::Value,
}

/// Register WP-24 analysis commands.
pub fn register_analysis_commands(registry: &mut CommandRegistry) -> Result<(), CoreError> {
    registry.register(
        CommandSpec {
            id: "pivot.create",
            doc: "Create a pivot table from a source range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        pivot_create,
    )?;
    registry.register(
        CommandSpec {
            id: "pivot.refresh",
            doc: "Refresh a pivot table from its source range",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        pivot_refresh,
    )?;
    registry.register(
        CommandSpec {
            id: "pivot.remove",
            doc: "Remove a pivot table and clear its output",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        pivot_remove,
    )?;
    registry.register(
        CommandSpec {
            id: "pivot.restore",
            doc: "Internal: restore a pivot table removed by pivot.remove",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Internal,
            default_keys: &[],
        },
        pivot_restore,
    )?;
    registry.register(
        CommandSpec {
            id: "whatif.goalseek",
            doc: "Goal Seek: vary an input cell until a formula reaches a goal",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        whatif_goalseek,
    )?;
    registry.register(
        CommandSpec {
            id: "stats.describe",
            doc: "Descriptive statistics and histogram for a range",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        stats_describe,
    )?;
    Ok(())
}

fn pivot_create(ctx: &mut CommandContext<'_>, args: PivotCreateArgs) -> Result<Effect, CoreError> {
    let source = resolve_range(ctx.workbook_ref(), &args.source)?;
    let dest = resolve_cell(ctx.workbook_ref(), &args.dest)?;
    let name = args
        .name
        .clone()
        .unwrap_or_else(|| default_pivot_name(ctx.workbook_ref()));
    let source_range = RangeRef::from_corners(
        CellRef::new(source.min_row, source.min_col)?,
        CellRef::new(source.max_row, source.max_col)?,
    );
    let mut table = PivotTable::new(
        name.clone(),
        source.sheet,
        source_range,
        dest.sheet,
        dest.row,
        dest.col,
    );
    table.rows = args.rows.clone();
    table.cols = args.cols.clone();
    table.data = args
        .data
        .iter()
        .map(|d| PivotDataField {
            source: d.source.clone(),
            agg: PivotAgg::from(d.agg),
            show_as: ShowAs::from(d.show_as),
        })
        .collect();
    table.filters = args
        .filters
        .iter()
        .map(|f| (f.source.clone(), f.values.clone()))
        .collect();
    table.groups = args
        .groups
        .iter()
        .map(|(name, spec)| Ok((name.clone(), parse_group(spec)?)))
        .collect::<Result<_, CoreError>>()?;
    if let Some(layout) = args.layout {
        table.layout = PivotLayout::from(layout);
    }
    if let Some(v) = args.grand_rows {
        table.grand_rows = v;
    }
    if let Some(v) = args.grand_cols {
        table.grand_cols = v;
    }
    if let Some(v) = args.subtotals {
        table.subtotals = v;
    }
    table.refresh_on_load = args.refresh_on_load;
    let id = ctx.workbook().add_pivot(table)?;
    Ok(Effect {
        inverse: vec![call("pivot.remove", serde_json::json!({"id": id.index()}))?],
        summary: ChangeSummary {
            text: format!("create pivot {name}"),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"id": id.index(), "name": name}),
        auto_recalc: false,
        ..Effect::default()
    })
}

fn pivot_refresh(ctx: &mut CommandContext<'_>, args: PivotIdArgs) -> Result<Effect, CoreError> {
    let id = resolve_pivot_id(ctx, &args)?;
    ctx.workbook().refresh_pivot(id)?;
    Ok(Effect {
        summary: ChangeSummary {
            text: "refresh pivot".into(),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"id": id.index()}),
        auto_recalc: false,
        ..Effect::default()
    })
}

fn pivot_remove(ctx: &mut CommandContext<'_>, args: PivotIdArgs) -> Result<Effect, CoreError> {
    let id = resolve_pivot_id(ctx, &args)?;
    let table = ctx.workbook().remove_pivot(id)?;
    Ok(Effect {
        inverse: vec![call("pivot.restore", serde_json::json!({"table": table}))?],
        summary: ChangeSummary {
            text: format!("remove pivot {}", table.name),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"id": id.index()}),
        auto_recalc: false,
        ..Effect::default()
    })
}

fn pivot_restore(
    ctx: &mut CommandContext<'_>,
    args: PivotRestoreArgs,
) -> Result<Effect, CoreError> {
    if ctx.is_preflight() {
        ctx.recalc_rebuild();
    }
    let table: PivotTable = serde_json::from_value(args.table)
        .map_err(|error| CoreError::new("pivot.restore", error.to_string()))?;
    let id = table.id;
    ctx.workbook().restore_pivot(table)?;
    ctx.workbook().refresh_pivot(id)?;
    Ok(Effect::query(serde_json::json!({"id": id.index()})))
}

fn whatif_goalseek(ctx: &mut CommandContext<'_>, args: GoalSeekArgs) -> Result<Effect, CoreError> {
    let target = resolve_cell(ctx.workbook_ref(), &args.target)?;
    let input = resolve_cell(ctx.workbook_ref(), &args.input)?;
    let original = ctx
        .workbook_ref()
        .get(input.sheet, input.row, input.col)?
        .copied();
    let original_input = original
        .map(|slot| slot_input(ctx.workbook_ref(), &slot))
        .unwrap_or_default();
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    let (workbook, engine) = ctx.workbook_and_engine();
    let result = goal_seek(
        workbook,
        engine,
        CellCoord::new(target.sheet, target.row, target.col),
        args.goal,
        CellCoord::new(input.sheet, input.row, input.col),
        args.max_iter.unwrap_or(DEFAULT_MAX_ITER),
        args.tol.unwrap_or(DEFAULT_TOL),
    )?;
    Ok(Effect {
        inverse: vec![call(
            "cell.set",
            serde_json::json!({
                "ref": format_cell(ctx.workbook_ref(), input),
                "input": original_input,
            }),
        )?],
        summary: ChangeSummary {
            text: if result.converged {
                "goal seek".into()
            } else {
                "goal seek (did not converge)".into()
            },
            cells: 1,
            ..ChangeSummary::default()
        },
        dirty: vec![CellCoord::new(input.sheet, input.row, input.col)],
        result: serde_json::json!({
            "converged": result.converged,
            "input": result.input,
            "output": result.output,
            "iterations": result.iterations,
        }),
        ..Effect::default()
    })
}

fn stats_describe(
    ctx: &mut CommandContext<'_>,
    args: StatsDescribeArgs,
) -> Result<Effect, CoreError> {
    let resolved = resolve_range(ctx.workbook_ref(), &args.range)?;
    let range = RangeRef::from_corners(
        CellRef::new(resolved.min_row, resolved.min_col)?,
        CellRef::new(resolved.max_row, resolved.max_col)?,
    );
    let summary = describe_range(ctx.workbook_ref(), resolved.sheet, range)?;
    Ok(Effect::query(serde_json::to_value(&summary).map_err(
        |error| CoreError::new("stats.serialize", error.to_string()),
    )?))
}

fn parse_group(spec: &PivotGroupArg) -> Result<PivotGroup, CoreError> {
    if let Some(grain) = &spec.date {
        let group = DateGroup::parse(grain).ok_or_else(|| {
            CoreError::new("pivot.group", format!("unknown date grouping {grain:?}"))
        })?;
        return Ok(PivotGroup::Date(group));
    }
    match (spec.start, spec.size) {
        (Some(start), Some(size)) => Ok(PivotGroup::Numeric { start, size }),
        (None, None) => Ok(PivotGroup::None),
        _ => Err(CoreError::new(
            "pivot.group",
            "numeric grouping requires start and size",
        )),
    }
}

fn resolve_pivot_id(ctx: &CommandContext<'_>, args: &PivotIdArgs) -> Result<PivotId, CoreError> {
    if let Some(id) = args.id {
        let id = PivotId::new(id);
        if ctx.workbook_ref().pivots().get(id).is_some() {
            return Ok(id);
        }
        return Err(CoreError::new("pivot.id", format!("unknown pivot {id:?}")));
    }
    let name = args
        .name
        .as_deref()
        .ok_or_else(|| CoreError::new("pivot.id", "pivot id or name is required"))?;
    ctx.workbook_ref()
        .pivots()
        .get_by_name(name)
        .map(|t| t.id)
        .ok_or_else(|| CoreError::new("pivot.name", format!("unknown pivot {name:?}")))
}

fn default_pivot_name(wb: &omacell_core::workbook::Workbook) -> String {
    let mut i = 1u32;
    loop {
        let name = format!("Pivot{i}");
        if wb.pivots().get_by_name(&name).is_none() {
            return name;
        }
        i = i.saturating_add(1);
        if i == 0 {
            return "Pivot".into();
        }
    }
}
