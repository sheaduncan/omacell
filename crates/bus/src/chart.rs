//! Chart and sparkline commands (WP-25).

use omacell_core::addr::SheetId;
use omacell_core::changeset::ChangeSummary;
use omacell_core::chart::{
    ChartId, ChartKind, Sparkline, SparklineKind, chart_from_range, parse_range,
};
use omacell_core::error::CoreError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::handler::{CommandContext, Effect};
use crate::logical::call;
use crate::registry::{CommandKind, CommandRegistry, CommandSpec, Exposure};

/// Public chart-kind argument. Keeping this an enum makes the command schema
/// advertise exactly the values callers may send.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChartKindArg {
    /// Line.
    Line,
    /// Clustered column.
    Column,
    /// Clustered bar.
    Bar,
    /// Stacked column.
    ColumnStacked,
    /// Stacked bar.
    BarStacked,
    /// 100% stacked column.
    ColumnPct,
    /// 100% stacked bar.
    BarPct,
    /// Area.
    Area,
    /// Pie.
    Pie,
    /// Donut.
    Donut,
    /// Scatter.
    Scatter,
    /// Bubble.
    Bubble,
    /// Column plus secondary-axis line.
    Combo,
    /// Histogram.
    Histogram,
}

impl From<ChartKindArg> for ChartKind {
    fn from(value: ChartKindArg) -> Self {
        ChartKind::parse(match value {
            ChartKindArg::Line => "line",
            ChartKindArg::Column => "column",
            ChartKindArg::Bar => "bar",
            ChartKindArg::ColumnStacked => "column_stacked",
            ChartKindArg::BarStacked => "bar_stacked",
            ChartKindArg::ColumnPct => "column_pct",
            ChartKindArg::BarPct => "bar_pct",
            ChartKindArg::Area => "area",
            ChartKindArg::Pie => "pie",
            ChartKindArg::Donut => "donut",
            ChartKindArg::Scatter => "scatter",
            ChartKindArg::Bubble => "bubble",
            ChartKindArg::Combo => "combo",
            ChartKindArg::Histogram => "histogram",
        })
        .unwrap_or(ChartKind::Column)
    }
}

/// Public sparkline-kind argument.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SparklineKindArg {
    /// Line.
    Line,
    /// Column.
    Column,
    /// Win/loss.
    #[serde(alias = "winloss")]
    WinLoss,
}

impl From<SparklineKindArg> for SparklineKind {
    fn from(value: SparklineKindArg) -> Self {
        match value {
            SparklineKindArg::Line => SparklineKind::Line,
            SparklineKindArg::Column => SparklineKind::Column,
            SparklineKindArg::WinLoss => SparklineKind::WinLoss,
        }
    }
}

/// `chart.fromselection`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChartFromSelectionArgs {
    /// A1 range.
    pub range: String,
    /// Kind name (`column`, `line`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ChartKindArg>,
    /// Title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// `sparkline.set`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SparklineSetArgs {
    /// Source A1 range.
    pub range: String,
    /// Display cell A1.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// `line` / `column` / `winloss`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<SparklineKindArg>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChartRemoveArgs {
    sheet: u32,
    id: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SparklineRemoveArgs {
    sheet: u32,
    index: u32,
}

/// Register chart commands.
pub fn register_chart_commands(registry: &mut CommandRegistry) -> Result<(), CoreError> {
    registry.register(
        CommandSpec {
            id: "chart.fromselection",
            doc: "Create a chart from a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["F11"],
        },
        chart_fromselection,
    )?;
    registry.register(
        CommandSpec {
            id: "sparkline.set",
            doc: "Place a sparkline in a cell",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        sparkline_set,
    )?;
    registry.register(
        CommandSpec {
            id: "chart.remove",
            doc: "Internal: remove a chart added by chart.fromselection",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Internal,
            default_keys: &[],
        },
        chart_remove,
    )?;
    registry.register(
        CommandSpec {
            id: "sparkline.remove",
            doc: "Internal: remove a sparkline added by sparkline.set",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Internal,
            default_keys: &[],
        },
        sparkline_remove,
    )?;
    Ok(())
}

fn chart_fromselection(
    ctx: &mut CommandContext<'_>,
    args: ChartFromSelectionArgs,
) -> Result<Effect, CoreError> {
    let kind = args.kind.map(ChartKind::from).unwrap_or(ChartKind::Column);
    let (sheet, range) = parse_range(ctx.workbook_ref(), &args.range)?;
    if ctx.workbook_ref().sheet(sheet).is_some_and(|target| {
        target
            .charts
            .iter()
            .any(|chart| chart.kind == ChartKind::Unsupported)
    }) {
        return Err(CoreError::new(
            "chart.unsupported",
            "cannot add a chart while this sheet contains an opaque imported drawing",
        )
        .with_hint("move the new chart to another sheet so the imported drawing stays intact"));
    }
    let chart = chart_from_range(ctx.workbook_ref(), sheet, range, kind, args.title)?;
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    let id = ctx.workbook().add_chart(chart)?;
    Ok(Effect {
        inverse: vec![call(
            "chart.remove",
            serde_json::json!({"sheet": sheet.index(), "id": id.index()}),
        )?],
        summary: ChangeSummary {
            text: format!("add {} chart", kind.as_str()),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"id": id.index()}),
        auto_recalc: false,
        ..Effect::default()
    })
}

fn sparkline_set(
    ctx: &mut CommandContext<'_>,
    args: SparklineSetArgs,
) -> Result<Effect, CoreError> {
    let (sheet, range) = parse_range(ctx.workbook_ref(), &args.range)?;
    let parsed = omacell_core::addr::parse_a1_cell(&args.cell_ref)?;
    let kind = args
        .kind
        .map(SparklineKind::from)
        .unwrap_or(SparklineKind::Line);
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    let index = ctx
        .workbook_ref()
        .sheet(sheet)
        .map(|target| target.sparklines.len())
        .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", sheet.index())))?;
    ctx.workbook().add_sparkline(Sparkline {
        kind,
        data: range,
        row: parsed.row,
        col: parsed.col,
        sheet,
    })?;
    Ok(Effect {
        inverse: vec![call(
            "sparkline.remove",
            serde_json::json!({"sheet": sheet.index(), "index": index}),
        )?],
        summary: ChangeSummary {
            text: "set sparkline".into(),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"ok": true}),
        auto_recalc: false,
        ..Effect::default()
    })
}

fn chart_remove(ctx: &mut CommandContext<'_>, args: ChartRemoveArgs) -> Result<Effect, CoreError> {
    let _ = ctx
        .workbook()
        .remove_chart(SheetId::new(args.sheet), ChartId::new(args.id))?;
    Ok(Effect::query(serde_json::json!({"removed": true})))
}

fn sparkline_remove(
    ctx: &mut CommandContext<'_>,
    args: SparklineRemoveArgs,
) -> Result<Effect, CoreError> {
    let index = usize::try_from(args.index)
        .map_err(|_| CoreError::new("sparkline.id", "sparkline index does not fit this host"))?;
    let _ = ctx
        .workbook()
        .remove_sparkline(SheetId::new(args.sheet), index)?;
    Ok(Effect::query(serde_json::json!({"removed": true})))
}
