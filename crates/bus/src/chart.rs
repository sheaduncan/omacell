//! Chart and sparkline commands (WP-25).

use omacell_core::chart::{ChartKind, Sparkline, SparklineKind, chart_from_range, parse_range};
use omacell_core::error::CoreError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::handler::{CommandContext, Effect};
use crate::registry::{CommandKind, CommandRegistry, CommandSpec, Exposure};

/// `chart.fromselection`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChartFromSelectionArgs {
    /// A1 range.
    pub range: String,
    /// Kind name (`column`, `line`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
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
    pub kind: Option<String>,
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
    Ok(())
}

fn chart_fromselection(
    ctx: &mut CommandContext<'_>,
    args: ChartFromSelectionArgs,
) -> Result<Effect, CoreError> {
    let kind = args
        .kind
        .as_deref()
        .and_then(ChartKind::parse)
        .unwrap_or(ChartKind::Column);
    let (sheet, range) = parse_range(ctx.workbook_ref(), &args.range)?;
    let chart = chart_from_range(ctx.workbook_ref(), sheet, range, kind, args.title)?;
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    let id = ctx.workbook().add_chart(chart)?;
    Ok(Effect {
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
        .as_deref()
        .and_then(SparklineKind::parse)
        .unwrap_or(SparklineKind::Line);
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    ctx.workbook().add_sparkline(Sparkline {
        kind,
        data: range,
        row: parsed.row,
        col: parsed.col,
        sheet,
    })?;
    Ok(Effect {
        result: serde_json::json!({"ok": true}),
        auto_recalc: false,
        ..Effect::default()
    })
}
