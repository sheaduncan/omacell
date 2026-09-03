//! Chart and sparkline commands (WP-25).

use omacell_core::addr::{CellRef, RefKind, SheetId, parse_a1};
use omacell_core::changeset::ChangeSummary;
use omacell_core::chart::{
    Axis, Chart, ChartAnchor, ChartId, ChartKind, Sparkline, SparklineKind, chart_from_range,
    parse_range,
};
use omacell_core::error::CoreError;
use omacell_core::workbook::Workbook;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::handler::{CommandContext, Effect};
use crate::logical::call;
use crate::registry::{CommandKind, CommandRegistry, CommandSpec, Exposure};
use crate::resolve::{
    ResolvedCell, ResolvedRange, format_cell, format_range, resolve_cell, resolve_range_unbounded,
};

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

/// Axis selected by `chart.axistitle`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChartAxisArg {
    /// Category or horizontal value axis.
    Category,
    /// Primary value axis.
    Value,
    /// Secondary value axis on a combo chart.
    Secondary,
}

/// `chart.move`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChartMoveArgs {
    /// Stable chart id. Omitted selects the first chart on the active sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// New top-left cell, optionally sheet-qualified.
    pub to: String,
}

/// `chart.resize`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChartResizeArgs {
    /// Stable chart id. Omitted selects the first chart on the active sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// New inclusive two-cell anchor, optionally sheet-qualified.
    pub range: String,
}

/// `chart.title`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChartTitleArgs {
    /// Stable chart id. Omitted selects the first chart on the active sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// New title. An empty string removes it.
    pub title: String,
}

/// `chart.axistitle`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChartAxisTitleArgs {
    /// Stable chart id. Omitted selects the first chart on the active sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// Axis to edit.
    pub axis: ChartAxisArg,
    /// New title. An empty string removes it.
    pub title: String,
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
    registry.register_with_local_inverse(
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
    registry.register_with_local_inverse(
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
    registry.register_with_local_inverse(
        CommandSpec {
            id: "chart.move",
            doc: "Move a chart to a new top-left cell",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        chart_move,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "chart.resize",
            doc: "Resize a chart to an inclusive cell range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        chart_resize,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "chart.title",
            doc: "Set or clear a chart title",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        chart_title,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "chart.axistitle",
            doc: "Set or clear a chart axis title",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        chart_axis_title,
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

fn chart_move(ctx: &mut CommandContext<'_>, args: ChartMoveArgs) -> Result<Effect, CoreError> {
    let before = find_chart(ctx.workbook_ref(), args.id)?;
    let qualified = parse_a1(&args.to)?.sheet.is_some();
    let mut target = resolve_cell(ctx.workbook_ref(), &args.to)?;
    if !qualified {
        target.sheet = before.sheet;
    }
    if target.sheet != before.sheet {
        return Err(CoreError::new(
            "chart.anchor",
            "a chart cannot be moved to a different sheet",
        )
        .with_hint("move the chart within its current sheet"));
    }
    let row_span = before.anchor.to_row - before.anchor.from_row;
    let col_span = before.anchor.to_col - before.anchor.from_col;
    let to_row = target
        .row
        .checked_add(row_span)
        .ok_or_else(chart_anchor_overflow)?;
    let to_col = target
        .col
        .checked_add(col_span)
        .ok_or_else(chart_anchor_overflow)?;
    CellRef::new(to_row, to_col).map_err(|_| chart_anchor_overflow())?;
    let mut after = before.clone();
    after.anchor = ChartAnchor {
        from_row: target.row,
        from_col: target.col,
        to_row,
        to_col,
    };
    let inverse = serde_json::json!({
        "id": before.id.index(),
        "to": qualified_anchor_start(ctx.workbook_ref(), &before),
    });
    apply_chart_edit(ctx, before, after, "chart.move", inverse, "move chart")
}

fn chart_resize(ctx: &mut CommandContext<'_>, args: ChartResizeArgs) -> Result<Effect, CoreError> {
    let before = find_chart(ctx.workbook_ref(), args.id)?;
    let parsed = parse_a1(&args.range)?;
    if matches!(
        parsed.kind,
        RefKind::Range(range) if range.whole_row || range.whole_col
    ) {
        return Err(CoreError::new(
            "chart.anchor",
            "chart anchors require two cell corners, not a whole row or column",
        ));
    }
    let qualified = parsed.sheet.is_some();
    let mut target = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    if !qualified {
        target.sheet = before.sheet;
    }
    if target.sheet != before.sheet {
        return Err(CoreError::new(
            "chart.anchor",
            "a chart anchor cannot span a different sheet",
        )
        .with_hint("resize the chart within its current sheet"));
    }
    let mut after = before.clone();
    after.anchor = ChartAnchor {
        from_row: target.min_row,
        from_col: target.min_col,
        to_row: target.max_row,
        to_col: target.max_col,
    };
    let inverse = serde_json::json!({
        "id": before.id.index(),
        "range": qualified_anchor_range(ctx.workbook_ref(), &before),
    });
    apply_chart_edit(ctx, before, after, "chart.resize", inverse, "resize chart")
}

fn chart_title(ctx: &mut CommandContext<'_>, args: ChartTitleArgs) -> Result<Effect, CoreError> {
    let before = find_chart(ctx.workbook_ref(), args.id)?;
    let mut after = before.clone();
    after.title = checked_title(args.title)?;
    let inverse = serde_json::json!({
        "id": before.id.index(),
        "title": before.title.clone().unwrap_or_default(),
    });
    apply_chart_edit(
        ctx,
        before,
        after,
        "chart.title",
        inverse,
        "edit chart title",
    )
}

fn chart_axis_title(
    ctx: &mut CommandContext<'_>,
    args: ChartAxisTitleArgs,
) -> Result<Effect, CoreError> {
    let before = find_chart(ctx.workbook_ref(), args.id)?;
    let title = checked_title(args.title)?;
    let mut after = before.clone();
    let previous = match args.axis {
        ChartAxisArg::Category => std::mem::replace(&mut after.category_axis.title, title),
        ChartAxisArg::Value => std::mem::replace(&mut after.value_axis.title, title),
        ChartAxisArg::Secondary => {
            if after.secondary_axis.is_none() && after.kind != ChartKind::Combo {
                return Err(
                    CoreError::new("chart.axis", "this chart has no secondary value axis")
                        .with_hint("secondary axis titles are available on combo charts"),
                );
            }
            std::mem::replace(
                &mut after.secondary_axis.get_or_insert_with(Axis::default).title,
                title,
            )
        }
    };
    let inverse = serde_json::json!({
        "id": before.id.index(),
        "axis": args.axis,
        "title": previous.unwrap_or_default(),
    });
    apply_chart_edit(
        ctx,
        before,
        after,
        "chart.axistitle",
        inverse,
        "edit chart axis title",
    )
}

fn find_chart(workbook: &Workbook, id: Option<u32>) -> Result<Chart, CoreError> {
    if let Some(id) = id {
        return workbook
            .sheets()
            .flat_map(|sheet| sheet.charts.iter())
            .find(|chart| chart.id.index() == id)
            .cloned()
            .ok_or_else(|| CoreError::new("chart.id", format!("unknown chart {id}")));
    }
    workbook
        .sheet(workbook.active_sheet())
        .and_then(|sheet| sheet.charts.first())
        .cloned()
        .ok_or_else(|| {
            CoreError::new("chart.id", "the active sheet has no chart")
                .with_hint("create a chart first or provide its id")
        })
}

fn checked_title(title: String) -> Result<Option<String>, CoreError> {
    if title.len() > 4_096 {
        return Err(CoreError::new(
            "chart.title",
            "chart titles are limited to 4096 UTF-8 bytes",
        ));
    }
    Ok((!title.is_empty()).then_some(title))
}

fn apply_chart_edit(
    ctx: &mut CommandContext<'_>,
    before: Chart,
    after: Chart,
    inverse_id: &str,
    inverse_args: serde_json::Value,
    summary: &str,
) -> Result<Effect, CoreError> {
    after.values_valid()?;
    let changed = before != after;
    let result = serde_json::json!({
        "changed": changed,
        "id": after.id.index(),
        "sheet": after.sheet.index(),
        "anchor": anchor_range(&after.anchor),
        "title": after.title.clone(),
    });
    if !changed {
        return Ok(Effect::query(result));
    }
    let inverse = call(inverse_id, inverse_args)?;
    if !ctx.is_preflight() {
        let _ = ctx
            .workbook()
            .replace_chart(after.sheet, after.id, after.clone())?;
    }
    Ok(Effect {
        inverse: vec![inverse],
        summary: ChangeSummary {
            text: summary.into(),
            ..ChangeSummary::default()
        },
        result,
        auto_recalc: false,
        ..Effect::default()
    })
}

fn anchor_start(anchor: &ChartAnchor) -> String {
    CellRef::new(anchor.from_row, anchor.from_col)
        .map(|cell| cell.to_a1())
        .unwrap_or_default()
}

fn anchor_range(anchor: &ChartAnchor) -> String {
    let start = anchor_start(anchor);
    let end = CellRef::new(anchor.to_row, anchor.to_col)
        .map(|cell| cell.to_a1())
        .unwrap_or_default();
    format!("{start}:{end}")
}

fn qualified_anchor_start(workbook: &Workbook, chart: &Chart) -> String {
    format_cell(
        workbook,
        ResolvedCell {
            sheet: chart.sheet,
            row: chart.anchor.from_row,
            col: chart.anchor.from_col,
        },
    )
}

fn qualified_anchor_range(workbook: &Workbook, chart: &Chart) -> String {
    format_range(
        workbook,
        ResolvedRange {
            sheet: chart.sheet,
            min_row: chart.anchor.from_row,
            min_col: chart.anchor.from_col,
            max_row: chart.anchor.to_row,
            max_col: chart.anchor.to_col,
        },
    )
}

fn chart_anchor_overflow() -> CoreError {
    CoreError::new(
        "chart.anchor",
        "moving this chart would exceed the worksheet grid",
    )
    .with_hint("choose a top-left cell that leaves room for the current chart size")
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
