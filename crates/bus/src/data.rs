//! Sort, AutoFilter, table, validation, CF, and Flash Fill commands (WP-18).

use omacell_core::changeset::ChangeSummary;
use omacell_core::condfmt::{CfDxf, CfKind, CfOp, CfTimePeriod, CondFormat};
use omacell_core::error::CoreError;
use omacell_core::filter::{
    AutoFilter, FilterColumn, FilterCriteria, NumOp, TextOp, apply_filter, clear_filter,
    filter_value_options, toggle_filter,
};
use omacell_core::flashfill::flash_fill;
use omacell_core::sort::{SortBy, SortKey, SortSpec, detect_header, sort_range};
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
    pub header: Option<bool>,
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
    /// Resolved conditional-format icon bucket.
    Icon,
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

/// `filter.values`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilterValuesArgs {
    /// A1 filter range, including its header row.
    pub range: String,
    /// 0-based column inside the filter range.
    pub col_id: u16,
    /// Case-insensitive dropdown search text.
    #[serde(default)]
    pub search: String,
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
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
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
    /// Fill or font colour.
    Color {
        /// Compare fill (`true`) or font (`false`).
        #[serde(default = "yes")]
        fill: bool,
        /// Packed ARGB.
        argb: u32,
    },
    /// Calendar date period.
    Period {
        /// Optional year.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        year: Option<i32>,
        /// Optional month (1–12).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        month: Option<u32>,
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

/// `table.rename`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TableRenameArgs {
    /// Table id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// Existing table name when no id is supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// New table name.
    pub new_name: String,
}

/// `table.totals`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TableTotalsArgs {
    /// Table id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// Table name when no id is supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Show the totals row.
    #[serde(default = "yes")]
    pub show: bool,
    /// Per-column OOXML totals functions; `null` leaves a column without one.
    #[serde(default)]
    pub functions: Vec<Option<String>>,
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
    /// Top/bottom rank or percentage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    /// Treat `n` as a percentage.
    #[serde(default)]
    pub percent: bool,
    /// Select the bottom values.
    #[serde(default)]
    pub bottom: bool,
    /// Select values below the average.
    #[serde(default)]
    pub below: bool,
    /// Packed ARGB scale stops (two or three).
    #[serde(default)]
    pub colors: Vec<u32>,
    /// Packed ARGB for a data bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_color: Option<u32>,
    /// Use a gradient data bar.
    #[serde(default = "yes")]
    pub gradient: bool,
    /// Icon-set size (three to five).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icons: Option<u8>,
    /// Relative date period for `time_period` rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<String>,
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
            id: "filter.values",
            doc: "List distinct values for an AutoFilter dropdown",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        filter_values,
    )?;
    registry.register_with_local_inverse(
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
            id: "table.rename",
            doc: "Rename a table and update structured references",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        table_rename,
    )?;
    registry.register(
        CommandSpec {
            id: "table.totals",
            doc: "Show or hide a table totals row",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        table_totals,
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
    let max_key_offset = if args.left_to_right {
        resolved.max_row - resolved.min_row
    } else {
        u32::from(resolved.max_col - resolved.min_col)
    };
    if let Some(key) = args
        .keys
        .iter()
        .find(|key| u32::from(key.offset) > max_key_offset)
    {
        return Err(CoreError::new(
            "sort.key",
            format!(
                "sort key offset {} is outside the selected range",
                key.offset
            ),
        ));
    }
    let range = omacell_core::addr::RangeRef::from_corners(
        omacell_core::addr::CellRef::new(resolved.min_row, resolved.min_col)?,
        omacell_core::addr::CellRef::new(resolved.max_row, resolved.max_col)?,
    );
    let header = match args.header {
        Some(header) => header,
        None => detect_header(
            ctx.workbook_ref(),
            resolved.sheet,
            range,
            args.left_to_right,
        )?,
    };
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
                    SortByArg::Icon => SortBy::Icon,
                },
                custom_list: k.custom_list.clone(),
            })
            .collect(),
        header,
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

fn filter_values(
    ctx: &mut CommandContext<'_>,
    args: FilterValuesArgs,
) -> Result<Effect, CoreError> {
    let resolved = resolve_range(ctx.workbook_ref(), &args.range)?;
    let range = omacell_core::addr::RangeRef::from_corners(
        omacell_core::addr::CellRef::new(resolved.min_row, resolved.min_col)?,
        omacell_core::addr::CellRef::new(resolved.max_row, resolved.max_col)?,
    );
    let values = filter_value_options(
        ctx.workbook_ref(),
        resolved.sheet,
        range,
        args.col_id,
        &args.search,
    )?;
    Ok(Effect::query(serde_json::json!({"values": values})))
}

fn filter_set(ctx: &mut CommandContext<'_>, args: FilterRangeArgs) -> Result<Effect, CoreError> {
    let resolved = resolve_range(ctx.workbook_ref(), &args.range)?;
    let range = omacell_core::addr::RangeRef::from_corners(
        omacell_core::addr::CellRef::new(resolved.min_row, resolved.min_col)?,
        omacell_core::addr::CellRef::new(resolved.max_row, resolved.max_col)?,
    );
    let columns = args
        .columns
        .iter()
        .map(|column| {
            let max_offset = resolved.max_col - resolved.min_col;
            if column.col_id > max_offset {
                return Err(CoreError::new(
                    "filter.column",
                    format!(
                        "filter column {} is outside the {}-column range",
                        column.col_id,
                        u32::from(max_offset) + 1
                    ),
                ));
            }
            Ok(FilterColumn {
                col_id: column.col_id,
                criteria: criteria_from(&column.criteria)?,
            })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    let filter = AutoFilter { range, columns };
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

fn criteria_from(arg: &FilterCriteriaArg) -> Result<FilterCriteria, CoreError> {
    Ok(match arg {
        FilterCriteriaArg::Values { values } => FilterCriteria::Values(values.clone()),
        FilterCriteriaArg::Text { op, value } => FilterCriteria::Text {
            op: match op.as_str() {
                "begins" => TextOp::Begins,
                "ends" => TextOp::Ends,
                "contains" => TextOp::Contains,
                other => {
                    return Err(CoreError::new(
                        "filter.operator",
                        format!("unsupported text filter operator {other}"),
                    ));
                }
            },
            value: value.clone(),
        },
        FilterCriteriaArg::Number { op, value, value2 } => FilterCriteria::Number {
            op: match op.as_str() {
                "greater_eq" | ">=" => NumOp::GreaterEq,
                "less" | "<" => NumOp::Less,
                "less_eq" | "<=" => NumOp::LessEq,
                "equal" | "=" => NumOp::Equal,
                "not_equal" | "!=" | "<>" => NumOp::NotEqual,
                "between" => NumOp::Between,
                "greater" | ">" => NumOp::Greater,
                other => {
                    return Err(CoreError::new(
                        "filter.operator",
                        format!("unsupported numeric filter operator {other}"),
                    ));
                }
            },
            value: *value,
            value2: *value2,
        },
        FilterCriteriaArg::TopN { n, percent, bottom } => {
            if *n == 0 || (*percent && *n > 100) {
                return Err(CoreError::new(
                    "filter.top_n",
                    "top-N count must be positive and percent must be at most 100",
                ));
            }
            FilterCriteria::TopN {
                n: *n,
                percent: *percent,
                bottom: *bottom,
            }
        }
        FilterCriteriaArg::Average { below } => FilterCriteria::Average { below: *below },
        FilterCriteriaArg::Color { fill, argb } => FilterCriteria::Color {
            fill: *fill,
            argb: *argb,
        },
        FilterCriteriaArg::Period { year, month } => {
            if year.is_none() && month.is_none() {
                return Err(CoreError::new(
                    "filter.period",
                    "date-period filter requires year and/or month",
                ));
            }
            if month.is_some_and(|month| !(1..=12).contains(&month)) {
                return Err(CoreError::new(
                    "filter.period",
                    "date-period month must be in 1..=12",
                ));
            }
            FilterCriteria::Period {
                year: *year,
                month: *month,
            }
        }
    })
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
    resolve_table_id(ctx, args.id, args.name.as_deref())
}

fn resolve_table_id(
    ctx: &CommandContext<'_>,
    id: Option<u32>,
    name: Option<&str>,
) -> Result<TableId, CoreError> {
    if let Some(id) = id {
        return Ok(TableId::new(id));
    }
    if let Some(name) = name {
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

fn table_rename(ctx: &mut CommandContext<'_>, args: TableRenameArgs) -> Result<Effect, CoreError> {
    let id = resolve_table_id(ctx, args.id, args.name.as_deref())?;
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    ctx.workbook().rename_table(id, args.new_name.clone())?;
    Ok(Effect {
        summary: ChangeSummary {
            text: format!("rename table to {}", args.new_name),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"id": id.index(), "name": args.new_name}),
        ..Effect::default()
    })
}

fn table_totals(ctx: &mut CommandContext<'_>, args: TableTotalsArgs) -> Result<Effect, CoreError> {
    let id = resolve_table_id(ctx, args.id, args.name.as_deref())?;
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({"ok": true})));
    }
    ctx.workbook()
        .set_table_totals(id, args.show, args.functions)?;
    Ok(Effect {
        summary: ChangeSummary {
            text: if args.show {
                "show table totals".into()
            } else {
                "hide table totals".into()
            },
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"id": id.index(), "show": args.show}),
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
            "" | "any" => DvType::Any,
            "whole" => DvType::Whole,
            "decimal" => DvType::Decimal,
            "list" => DvType::List,
            "date" => DvType::Date,
            "time" => DvType::Time,
            "text_length" => DvType::TextLength,
            "custom" => DvType::Custom,
            other => {
                return Err(CoreError::new(
                    "validation.kind",
                    format!("unsupported validation kind {other}"),
                ));
            }
        },
        op: match args.op.as_deref().unwrap_or("between") {
            "not_between" => DvOp::NotBetween,
            "equal" => DvOp::Equal,
            "not_equal" => DvOp::NotEqual,
            "greater" => DvOp::Greater,
            "less" => DvOp::Less,
            "greater_eq" => DvOp::GreaterEq,
            "less_eq" => DvOp::LessEq,
            "between" => DvOp::Between,
            other => {
                return Err(CoreError::new(
                    "validation.operator",
                    format!("unsupported validation operator {other}"),
                ));
            }
        },
        formula1: args.formula1,
        formula2: args.formula2,
        allow_blank: args.allow_blank,
        error_style: match args.error_style.as_deref().unwrap_or("stop") {
            "warning" => DvErrorStyle::Warning,
            "information" => DvErrorStyle::Information,
            "stop" => DvErrorStyle::Stop,
            other => {
                return Err(CoreError::new(
                    "validation.error_style",
                    format!("unsupported validation error style {other}"),
                ));
            }
        },
        error_title: args.error_title,
        error_message: args.error_message,
        input_title: args.input_title,
        input_message: args.input_message,
    };
    if dv.kind != DvType::Any && dv.formula1.as_deref().is_none_or(str::is_empty) {
        return Err(CoreError::new(
            "validation.formula1",
            "this validation kind requires formula1",
        ));
    }
    if matches!(dv.op, DvOp::Between | DvOp::NotBetween)
        && !matches!(dv.kind, DvType::Any | DvType::List | DvType::Custom)
        && dv.formula2.as_deref().is_none_or(str::is_empty)
    {
        return Err(CoreError::new(
            "validation.formula2",
            "between validation requires formula2",
        ));
    }
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
    if args.priority == 0 {
        return Err(CoreError::new(
            "condfmt.priority",
            "conditional-format priority starts at 1",
        ));
    }
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
    for existing in &mut rules {
        if existing.priority >= args.priority {
            existing.priority = existing.priority.saturating_add(1);
        }
    }
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
        "cell_is" | "cellIs" => {
            let op = match args.op.as_deref().unwrap_or("greater") {
                "greater" => CfOp::Greater,
                "less" => CfOp::Less,
                "equal" => CfOp::Equal,
                "between" => CfOp::Between,
                "not_between" => CfOp::NotBetween,
                "greater_eq" => CfOp::GreaterEq,
                "less_eq" => CfOp::LessEq,
                "not_equal" => CfOp::NotEqual,
                other => {
                    return Err(CoreError::new(
                        "condfmt.operator",
                        format!("unsupported conditional-format operator {other}"),
                    ));
                }
            };
            if matches!(op, CfOp::Between | CfOp::NotBetween)
                && args.formula2.as_deref().is_none_or(str::is_empty)
            {
                return Err(CoreError::new(
                    "condfmt.formula2",
                    "between conditional format requires formula2",
                ));
            }
            CfKind::CellIs {
                op,
                formula1: required_cf_text(args.formula1.as_deref(), "formula1")?.to_string(),
                formula2: args.formula2.clone(),
            }
        }
        "contains_text" => CfKind::ContainsText(
            required_cf_text(args.formula1.as_deref(), "formula1")?.to_string(),
        ),
        "blanks" => CfKind::Blanks,
        "errors" => CfKind::Errors,
        "duplicate" => CfKind::Duplicate,
        "unique" => CfKind::Unique,
        "top_n" => CfKind::TopN {
            n: args.n.unwrap_or(10),
            percent: args.percent,
            bottom: args.bottom,
        },
        "average" => CfKind::Average { below: args.below },
        "time_period" | "date" => CfKind::TimePeriod(match args.period.as_deref() {
            None | Some("today") => CfTimePeriod::Today,
            Some("yesterday") => CfTimePeriod::Yesterday,
            Some("tomorrow") => CfTimePeriod::Tomorrow,
            Some("last_7_days") => CfTimePeriod::Last7Days,
            Some("this_week") => CfTimePeriod::ThisWeek,
            Some("last_week") => CfTimePeriod::LastWeek,
            Some("next_week") => CfTimePeriod::NextWeek,
            Some("this_month") => CfTimePeriod::ThisMonth,
            Some("last_month") => CfTimePeriod::LastMonth,
            Some("next_month") => CfTimePeriod::NextMonth,
            Some(other) => {
                return Err(CoreError::new(
                    "condfmt.period",
                    format!("unsupported conditional-format date period {other}"),
                ));
            }
        }),
        "color_scale" => CfKind::ColorScale {
            colors: if args.colors.is_empty() {
                vec![
                    Color::Theme {
                        theme: 5,
                        tint: 0.0,
                    },
                    Color::Theme {
                        theme: 7,
                        tint: 0.0,
                    },
                    Color::Theme {
                        theme: 9,
                        tint: 0.0,
                    },
                ]
            } else {
                args.colors
                    .iter()
                    .map(|argb| Color::Rgb { argb: *argb })
                    .collect()
            },
        },
        "data_bar" => CfKind::DataBar {
            color: args.visual_color.or(args.fill).map_or(
                Color::Theme {
                    theme: 4,
                    tint: 0.0,
                },
                |argb| Color::Rgb { argb },
            ),
            gradient: args.gradient,
        },
        "icon_set" => CfKind::IconSet {
            icons: args.icons.unwrap_or(3),
        },
        "formula" => {
            CfKind::Formula(required_cf_text(args.formula1.as_deref(), "formula1")?.to_string())
        }
        other => {
            return Err(CoreError::new(
                "condfmt.kind",
                format!("unsupported conditional format kind {other}"),
            ));
        }
    })
}

fn required_cf_text<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, CoreError> {
    value.filter(|value| !value.is_empty()).ok_or_else(|| {
        CoreError::new(
            format!("condfmt.{field}"),
            format!("conditional format requires {field}"),
        )
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
