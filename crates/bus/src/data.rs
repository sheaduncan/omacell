//! Sort, AutoFilter, table, validation, CF, and Flash Fill commands (WP-18).

use omacell_core::changeset::ChangeSummary;
use omacell_core::condfmt::{CfDxf, CfKind, CfOp, CondFormat};
use omacell_core::error::CoreError;
use omacell_core::filter::{
    AutoFilter, FilterColumn, FilterCriteria, NumOp, TextOp, apply_filter, clear_filter,
    toggle_filter,
};
use omacell_core::flashfill::flash_fill;
use omacell_core::sort::{SortBy, SortKey, SortSpec, sort_range};
use omacell_core::style::Color;
use omacell_core::tables::TableId;
use omacell_core::validation::{DataValidation, DvErrorStyle, DvOp, DvType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::args::EmptyArgs;
use crate::handler::{CommandContext, Effect};
use crate::logical::call;
use crate::registry::{CommandKind, CommandRegistry, CommandSpec, Exposure};
use crate::resolve::resolve_range;

/// `range.sort`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeSortArgs {
    /// A1 range.
    pub range: String,
    /// Keys in priority order.
    #[serde(default)]
    pub keys: Vec<SortKeyArg>,
    /// First row/column is a header.
    #[serde(default)]
    pub header: bool,
    /// Case-sensitive text.
    #[serde(default)]
    pub case_sensitive: bool,
    /// Sort left-to-right.
    #[serde(default)]
    pub left_to_right: bool,
}

/// One sort key argument.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SortKeyArg {
    /// 0-based offset inside the range.
    #[serde(default)]
    pub offset: u16,
    /// Descending.
    #[serde(default)]
    pub descending: bool,
    /// `value` / `fill_color` / `font_color`.
    #[serde(default)]
    pub by: SortByArg,
    /// Custom list.
    #[serde(default)]
    pub custom_list: Vec<String>,
}

/// Sort comparison source.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortByArg {
    /// Values.
    #[default]
    Value,
    /// Fill colour.
    FillColor,
    /// Font colour.
    FontColor,
}

/// `filter.toggle` / `filter.set`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilterRangeArgs {
    /// A1 range.
    pub range: String,
    /// Column criteria.
    #[serde(default)]
    pub columns: Vec<FilterColumnArg>,
}

/// One AutoFilter column.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilterColumnArg {
    /// 0-based column inside the filter range.
    pub col_id: u16,
    /// Criteria.
    pub criteria: FilterCriteriaArg,
}

/// Filter criteria argument.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FilterCriteriaArg {
    /// Inclusive value list.
    Values {
        /// Display texts.
        values: Vec<String>,
    },
    /// Text operator.
    Text {
        /// `contains` / `begins` / `ends`.
        op: String,
        /// Needle.
        value: String,
    },
    /// Numeric compare.
    Number {
        /// Operator.
        op: String,
        /// Bound.
        value: f64,
        /// Second bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value2: Option<f64>,
    },
    /// Top/bottom N.
    TopN {
        /// Count or percent.
        n: u32,
        /// Percent.
        #[serde(default)]
        percent: bool,
        /// Bottom.
        #[serde(default)]
        bottom: bool,
    },
    /// Above/below average.
    Average {
        /// Below.
        #[serde(default)]
        below: bool,
    },
}

/// `table.create`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TableCreateArgs {
    /// A1 range.
    pub range: String,
    /// Table name.
    pub name: String,
}

/// `table.resize` / `table.convert`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TableIdArgs {
    /// Table id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// Table name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// New A1 range (resize).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
}

/// `validation.set`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidationSetArgs {
    /// A1 range.
    pub range: String,
    /// Kind.
    #[serde(default)]
    pub kind: String,
    /// Operator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    /// Formula / min / list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula1: Option<String>,
    /// Max.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula2: Option<String>,
    /// Allow blank.
    #[serde(default = "yes")]
    pub allow_blank: bool,
    /// Error style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_style: Option<String>,
    /// Error title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_title: Option<String>,
    /// Error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Input title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_title: Option<String>,
    /// Input message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_message: Option<String>,
}

fn yes() -> bool {
    true
}

/// `condfmt.add`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CondFmtAddArgs {
    /// A1 range.
    pub range: String,
    /// Rule kind name.
    pub kind: String,
    /// Operator for cell-is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    /// First formula/value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula1: Option<String>,
    /// Second formula/value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula2: Option<String>,
    /// Priority (1 wins).
    #[serde(default = "one")]
    pub priority: u32,
    /// Stop if true.
    #[serde(default)]
    pub stop_if_true: bool,
    /// Fill ARGB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<u32>,
    /// Font ARGB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<u32>,
}

fn one() -> u32 {
    1
}

/// `edit.flashfill`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlashFillArgs {
    /// Destination A1 range (adjacent source column is inferred).
    pub range: String,
}

/// Register WP-18 data-tool commands.
pub fn register_data_commands(registry: &mut CommandRegistry) -> Result<(), CoreError> {
    registry.register(
        CommandSpec {
            id: "range.sort",
            doc: "Sort a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        range_sort,
    )?;
    registry.register(
        CommandSpec {
            id: "filter.toggle",
            doc: "Toggle AutoFilter on the selection",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Shift+L"],
        },
        filter_toggle,
    )?;
    registry.register(
        CommandSpec {
            id: "filter.clear",
            doc: "Clear AutoFilter",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        filter_clear,
    )?;
    registry.register(
        CommandSpec {
            id: "filter.set",
            doc: "Apply AutoFilter criteria",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        filter_set,
    )?;
    registry.register(
        CommandSpec {
            id: "table.create",
            doc: "Create a table",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+T"],
        },
        table_create,
    )?;
    registry.register(
        CommandSpec {
            id: "table.resize",
            doc: "Resize a table",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        table_resize,
    )?;
    registry.register(
        CommandSpec {
            id: "table.convert",
            doc: "Convert a table to a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        table_convert,
    )?;
    registry.register(
        CommandSpec {
            id: "validation.set",
            doc: "Set data validation on a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        validation_set,
    )?;
    registry.register(
        CommandSpec {
            id: "condfmt.add",
            doc: "Add a conditional format rule",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        condfmt_add,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.flashfill",
            doc: "Flash Fill from examples in the adjacent column",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+E"],
        },
        edit_flashfill,
    )?;
    Ok(())
}

fn range_sort(ctx: &mut CommandContext<'_>, args: RangeSortArgs) -> Result<Effect, CoreError> {
    let resolved = resolve_range(ctx.workbook_ref(), &args.range)?;
    let range = omacell_core::addr::RangeRef::from_corners(
        omacell_core::addr::CellRef::new(resolved.min_row, resolved.min_col)?,
        omacell_core::addr::CellRef::new(resolved.max_row, resolved.max_col)?,
    );
    let spec = SortSpec {
        keys: args
            .keys
            .iter()
            .map(|k| SortKey {
                offset: k.offset,
                descending: k.descending,
                by: match k.by {
                    SortByArg::Value => SortBy::Value,
                    SortByArg::FillColor => SortBy::FillColor,
                    SortByArg::FontColor => SortBy::FontColor,
                },
                custom_list: k.custom_list.clone(),
            })
            .collect(),
        header: args.header,
        case_sensitive: args.case_sensitive,
        left_to_right: args.left_to_right,
    };
    let spec = if spec.keys.is_empty() {
        SortSpec {
            keys: vec![SortKey {
                offset: 0,
                descending: false,
                by: SortBy::Value,
                custom_list: Vec::new(),
            }],
            ..spec
        }
    } else {
        spec
    };
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    let moved = sort_range(ctx.workbook(), resolved.sheet, range, &spec)?;
    Ok(Effect {
        summary: ChangeSummary {
            text: format!("sort {moved} rows"),
            cells: u64::from(moved),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"moved": moved}),
        ..Effect::default()
    })
}

fn filter_toggle(ctx: &mut CommandContext<'_>, args: FilterRangeArgs) -> Result<Effect, CoreError> {
    let resolved = resolve_range(ctx.workbook_ref(), &args.range)?;
    let range = omacell_core::addr::RangeRef::from_corners(
        omacell_core::addr::CellRef::new(resolved.min_row, resolved.min_col)?,
        omacell_core::addr::CellRef::new(resolved.max_row, resolved.max_col)?,
    );
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    let on = toggle_filter(ctx.workbook(), resolved.sheet, range)?;
    Ok(Effect {
        summary: ChangeSummary {
            text: if on {
                "filter on".into()
            } else {
                "filter off".into()
            },
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"on": on}),
        ..Effect::default()
    })
}

fn filter_clear(ctx: &mut CommandContext<'_>, _args: EmptyArgs) -> Result<Effect, CoreError> {
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    let sheet = ctx.workbook_ref().active_sheet();
    clear_filter(ctx.workbook(), sheet)?;
    Ok(Effect {
        summary: ChangeSummary {
            text: "clear filter".into(),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"ok": true}),
        ..Effect::default()
    })
}

fn filter_set(ctx: &mut CommandContext<'_>, args: FilterRangeArgs) -> Result<Effect, CoreError> {
    let resolved = resolve_range(ctx.workbook_ref(), &args.range)?;
    let range = omacell_core::addr::RangeRef::from_corners(
        omacell_core::addr::CellRef::new(resolved.min_row, resolved.min_col)?,
        omacell_core::addr::CellRef::new(resolved.max_row, resolved.max_col)?,
    );
    let filter = AutoFilter {
        range,
        columns: args
            .columns
            .iter()
            .map(|c| FilterColumn {
                col_id: c.col_id,
                criteria: criteria_from(&c.criteria),
            })
            .collect(),
    };
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    let hidden = apply_filter(ctx.workbook(), resolved.sheet, &filter)?;
    Ok(Effect {
        summary: ChangeSummary {
            text: format!("filter hid {hidden} rows"),
            rows: u64::from(hidden),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"hidden": hidden}),
        ..Effect::default()
    })
}

fn criteria_from(arg: &FilterCriteriaArg) -> FilterCriteria {
    match arg {
        FilterCriteriaArg::Values { values } => FilterCriteria::Values(values.clone()),
        FilterCriteriaArg::Text { op, value } => FilterCriteria::Text {
            op: match op.as_str() {
                "begins" => TextOp::Begins,
                "ends" => TextOp::Ends,
                _ => TextOp::Contains,
            },
            value: value.clone(),
        },
        FilterCriteriaArg::Number { op, value, value2 } => FilterCriteria::Number {
            op: match op.as_str() {
                "greater_eq" | ">=" => NumOp::GreaterEq,
                "less" | "<" => NumOp::Less,
                "less_eq" | "<=" => NumOp::LessEq,
                "equal" | "=" => NumOp::Equal,
                "between" => NumOp::Between,
                _ => NumOp::Greater,
            },
            value: *value,
            value2: *value2,
        },
        FilterCriteriaArg::TopN { n, percent, bottom } => FilterCriteria::TopN {
            n: *n,
            percent: *percent,
            bottom: *bottom,
        },
        FilterCriteriaArg::Average { below } => FilterCriteria::Average { below: *below },
    }
}

fn table_create(ctx: &mut CommandContext<'_>, args: TableCreateArgs) -> Result<Effect, CoreError> {
    let resolved = resolve_range(ctx.workbook_ref(), &args.range)?;
    let range = omacell_core::addr::RangeRef::from_corners(
        omacell_core::addr::CellRef::new(resolved.min_row, resolved.min_col)?,
        omacell_core::addr::CellRef::new(resolved.max_row, resolved.max_col)?,
    );
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    let id = ctx
        .workbook()
        .create_table(resolved.sheet, range, args.name.clone())?;
    Ok(Effect {
        inverse: vec![call(
            "table.convert",
            serde_json::json!({"id": id.index()}),
        )?],
        summary: ChangeSummary {
            text: format!("create table {}", args.name),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"id": id.index()}),
        ..Effect::default()
    })
}

fn table_id(ctx: &CommandContext<'_>, args: &TableIdArgs) -> Result<TableId, CoreError> {
    if let Some(id) = args.id {
        return Ok(TableId::new(id));
    }
    if let Some(name) = &args.name {
        return ctx
            .workbook_ref()
            .tables()
            .get_by_name(name)
            .map(|t| t.id)
            .ok_or_else(|| CoreError::table_name(format!("unknown table {name}")));
    }
    Err(CoreError::table_name("table id or name is required"))
}

fn table_resize(ctx: &mut CommandContext<'_>, args: TableIdArgs) -> Result<Effect, CoreError> {
    let id = table_id(ctx, &args)?;
    let range_text = args
        .range
        .as_deref()
        .ok_or_else(|| CoreError::new("args.range", "table.resize requires range"))?;
    let resolved = resolve_range(ctx.workbook_ref(), range_text)?;
    let range = omacell_core::addr::RangeRef::from_corners(
        omacell_core::addr::CellRef::new(resolved.min_row, resolved.min_col)?,
        omacell_core::addr::CellRef::new(resolved.max_row, resolved.max_col)?,
    );
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    ctx.workbook().resize_table(id, range)?;
    Ok(Effect {
        summary: ChangeSummary {
            text: "resize table".into(),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"id": id.index()}),
        ..Effect::default()
    })
}

fn table_convert(ctx: &mut CommandContext<'_>, args: TableIdArgs) -> Result<Effect, CoreError> {
    let id = table_id(ctx, &args)?;
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    let table = ctx.workbook().convert_table(id)?;
    Ok(Effect {
        summary: ChangeSummary {
            text: format!("convert table {}", table.name),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"name": table.name}),
        ..Effect::default()
    })
}

fn validation_set(
    ctx: &mut CommandContext<'_>,
    args: ValidationSetArgs,
) -> Result<Effect, CoreError> {
    let resolved = resolve_range(ctx.workbook_ref(), &args.range)?;
    let range = omacell_core::addr::RangeRef::from_corners(
        omacell_core::addr::CellRef::new(resolved.min_row, resolved.min_col)?,
        omacell_core::addr::CellRef::new(resolved.max_row, resolved.max_col)?,
    );
    let dv = DataValidation {
        range,
        kind: match args.kind.as_str() {
            "whole" => DvType::Whole,
            "decimal" => DvType::Decimal,
            "list" => DvType::List,
            "date" => DvType::Date,
            "time" => DvType::Time,
            "text_length" => DvType::TextLength,
            "custom" => DvType::Custom,
            _ => DvType::Any,
        },
        op: match args.op.as_deref().unwrap_or("between") {
            "not_between" => DvOp::NotBetween,
            "equal" => DvOp::Equal,
            "not_equal" => DvOp::NotEqual,
            "greater" => DvOp::Greater,
            "less" => DvOp::Less,
            "greater_eq" => DvOp::GreaterEq,
            "less_eq" => DvOp::LessEq,
            _ => DvOp::Between,
        },
        formula1: args.formula1,
        formula2: args.formula2,
        allow_blank: args.allow_blank,
        error_style: match args.error_style.as_deref().unwrap_or("stop") {
            "warning" => DvErrorStyle::Warning,
            "information" => DvErrorStyle::Information,
            _ => DvErrorStyle::Stop,
        },
        error_title: args.error_title,
        error_message: args.error_message,
        input_title: args.input_title,
        input_message: args.input_message,
    };
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    let mut rules = ctx
        .workbook_ref()
        .sheet(resolved.sheet)
        .map(|s| s.validations.clone())
        .unwrap_or_default();
    rules.retain(|r| r.range != range);
    rules.push(dv);
    ctx.workbook().set_validations(resolved.sheet, rules)?;
    Ok(Effect {
        summary: ChangeSummary {
            text: "set validation".into(),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"ok": true}),
        ..Effect::default()
    })
}

fn condfmt_add(ctx: &mut CommandContext<'_>, args: CondFmtAddArgs) -> Result<Effect, CoreError> {
    let resolved = resolve_range(ctx.workbook_ref(), &args.range)?;
    let range = omacell_core::addr::RangeRef::from_corners(
        omacell_core::addr::CellRef::new(resolved.min_row, resolved.min_col)?,
        omacell_core::addr::CellRef::new(resolved.max_row, resolved.max_col)?,
    );
    let kind = parse_cf_kind(&args)?;
    let rule = CondFormat {
        range,
        priority: args.priority,
        stop_if_true: args.stop_if_true,
        kind,
        dxf: CfDxf {
            fill: args.fill.map(|argb| Color::Rgb { argb }),
            font: args.font.map(|argb| Color::Rgb { argb }),
        },
    };
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    let mut rules = ctx
        .workbook_ref()
        .sheet(resolved.sheet)
        .map(|s| s.cond_formats.clone())
        .unwrap_or_default();
    rules.push(rule);
    ctx.workbook().set_cond_formats(resolved.sheet, rules)?;
    Ok(Effect {
        summary: ChangeSummary {
            text: "add conditional format".into(),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"ok": true}),
        ..Effect::default()
    })
}

fn parse_cf_kind(args: &CondFmtAddArgs) -> Result<CfKind, CoreError> {
    Ok(match args.kind.as_str() {
        "cell_is" | "cellIs" => CfKind::CellIs {
            op: match args.op.as_deref().unwrap_or("greater") {
                "less" => CfOp::Less,
                "equal" => CfOp::Equal,
                "between" => CfOp::Between,
                "not_between" => CfOp::NotBetween,
                "greater_eq" => CfOp::GreaterEq,
                "less_eq" => CfOp::LessEq,
                "not_equal" => CfOp::NotEqual,
                _ => CfOp::Greater,
            },
            formula1: args.formula1.clone().unwrap_or_else(|| "0".into()),
            formula2: args.formula2.clone(),
        },
        "contains_text" => CfKind::ContainsText(args.formula1.clone().unwrap_or_default()),
        "blanks" => CfKind::Blanks,
        "errors" => CfKind::Errors,
        "duplicate" => CfKind::Duplicate,
        "unique" => CfKind::Unique,
        "top_n" => CfKind::TopN {
            n: args
                .formula1
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(10),
            percent: false,
            bottom: false,
        },
        "average" => CfKind::Average { below: false },
        "formula" => CfKind::Formula(args.formula1.clone().unwrap_or_else(|| "TRUE".into())),
        other => {
            return Err(CoreError::new(
                "condfmt.kind",
                format!("unsupported conditional format kind {other}"),
            ));
        }
    })
}

fn edit_flashfill(ctx: &mut CommandContext<'_>, args: FlashFillArgs) -> Result<Effect, CoreError> {
    let resolved = resolve_range(ctx.workbook_ref(), &args.range)?;
    let range = omacell_core::addr::RangeRef::from_corners(
        omacell_core::addr::CellRef::new(resolved.min_row, resolved.min_col)?,
        omacell_core::addr::CellRef::new(resolved.max_row, resolved.max_col)?,
    );
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    let filled = flash_fill(ctx.workbook(), resolved.sheet, range)?;
    Ok(Effect {
        summary: ChangeSummary {
            text: format!("flash fill {filled} cells"),
            cells: u64::from(filled),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"filled": filled}),
        ..Effect::default()
    })
}
