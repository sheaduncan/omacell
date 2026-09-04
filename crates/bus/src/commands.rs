//! Core and internal restore command handlers.

use omacell_core::changeset::ChangeSummary;
use omacell_core::error::CoreError;
use omacell_core::event::Event;
use omacell_core::graph::CellCoord;
use omacell_core::names::{DefinedName, NameReferent, NameScope, validate_defined_name};
use omacell_core::sheet::SheetVisibility;
use omacell_core::storage::CellSlot;
use omacell_core::style::NumFmtId;
use omacell_core::style::StyleId;
use omacell_core::value::Value;
use omacell_core::workbook::CalcMode;

use crate::args::{
    CalcModeArgs, CalcRecalcArgs, CellClearArgs, CellRestoreArgs, CellSetArgs, EmptyArgs,
    FormatNumberArgs, NameCreateFromArgs, NameDefineArgs, NameLabelPosition, NameReferentArg,
    NameRemoveArgs, RangeClearArgs, RangeSetArgs, SheetAddArgs, SheetRemoveArgs, SheetRenameArgs,
    SheetVisibilityArgs, StyleRestoreArgs, StyleSetArgs,
};
use crate::error as bus_error;
use crate::handler::{CommandContext, Effect};
use crate::logical::{
    apply_stored_style, apply_style_patch, call, decode_cell_flags, inverse_contents,
    inverse_style, release_root_ref, restore_cell_value, slot_input, store_logical_value, style_of,
};
use crate::registry::{CommandKind, CommandRegistry, CommandSpec, Exposure};
use crate::resolve::{
    ResolvedCell, format_cell, format_range, resolve_cell, resolve_range, resolve_sheet,
};

/// Register the WP-07a command set. Later packages call [`CommandRegistry::register`].
pub fn register_core(registry: &mut CommandRegistry) -> Result<(), CoreError> {
    registry.register_with_local_inverse(
        CommandSpec {
            id: "cell.set",
            doc: "Set a cell's contents from formula-bar text",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        cell_set,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "cell.clear",
            doc: "Clear a cell's contents, keeping style",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Delete"],
        },
        cell_clear,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "range.set",
            doc: "Set range contents from values or a fill input",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Enter"],
        },
        range_set,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "range.clear",
            doc: "Clear range contents, keeping styles",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Delete"],
        },
        range_clear,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "sheet.add",
            doc: "Add a sheet",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        sheet_add,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "sheet.rename",
            doc: "Rename a sheet",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        sheet_rename,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "sheet.visibility",
            doc: "Set sheet visibility",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        sheet_visibility,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "name.define",
            doc: "Define a named range, constant, or formula",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+F3"],
        },
        name_define,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "name.remove",
            doc: "Remove a defined name",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        name_remove,
    )?;
    registry.register(
        CommandSpec {
            id: "name.restore",
            doc: "Internal: restore an exact logical defined name",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Internal,
            default_keys: &[],
        },
        name_restore,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "name.createfrom",
            doc: "Create workbook names from labels on selection edges",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Shift+F3"],
        },
        name_createfrom,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "format.number",
            doc: "Apply a number format to a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Shift+~"],
        },
        format_number,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "style.set",
            doc: "Patch font, fill, alignment, or protection on a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+1"],
        },
        style_set,
    )?;
    registry.register(
        CommandSpec {
            id: "calc.recalc",
            doc: "Recalculate the workbook",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["F9"],
        },
        calc_recalc,
    )?;
    registry.register_with_local_inverse(
        CommandSpec {
            id: "calc.mode",
            doc: "Set calculation mode",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        calc_mode,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.undo",
            doc: "Undo the last workbook transaction",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Z"],
        },
        undo_cmd,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.redo",
            doc: "Redo the last undone workbook transaction",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Y"],
        },
        redo_cmd,
    )?;
    registry.register(
        CommandSpec {
            id: "cell.restore",
            doc: "Internal: restore cell contents and style from logical data",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Internal,
            default_keys: &[],
        },
        cell_restore,
    )?;
    registry.register(
        CommandSpec {
            id: "style.restore",
            doc: "Internal: restore an exact style record",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Internal,
            default_keys: &[],
        },
        style_restore,
    )?;
    registry.register(
        CommandSpec {
            id: "sheet.remove",
            doc: "Remove a sheet",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        sheet_remove,
    )?;
    Ok(())
}

fn cell_set(ctx: &mut CommandContext<'_>, args: CellSetArgs) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    set_one_cell(ctx, cell, &args.input)
}

fn cell_clear(ctx: &mut CommandContext<'_>, args: CellClearArgs) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    set_one_cell(ctx, cell, "")
}

fn set_one_cell(
    ctx: &mut CommandContext<'_>,
    cell: ResolvedCell,
    input: &str,
) -> Result<Effect, CoreError> {
    let before = ctx.workbook_ref().get(cell.sheet, cell.row, cell.col)?;
    let before_input = before.map(|slot| slot_input(ctx.workbook_ref(), slot));
    let inverse = inverse_contents(ctx.workbook_ref(), cell)?;
    if before_input.as_deref() == Some(input) || (before.is_none() && input.trim().is_empty()) {
        return Ok(Effect {
            inverse: vec![inverse],
            result: serde_json::json!({"changed": 0}),
            auto_recalc: false,
            ..Effect::default()
        });
    }
    ctx.workbook()
        .set_cell_contents(cell.sheet, cell.row, cell.col, input)?;
    Ok(Effect {
        inverse: vec![inverse],
        events: vec![Event::CellChanged {
            sheet: cell.sheet,
            row: cell.row,
            col: cell.col,
        }],
        summary: ChangeSummary {
            cells: 1,
            text: format!("set {}", format_cell(ctx.workbook_ref(), cell)),
            ..ChangeSummary::default()
        },
        dirty: vec![CellCoord::new(cell.sheet, cell.row, cell.col)],
        result: serde_json::json!({"changed": 1}),
        auto_recalc: true,
        rebuild: false,
    })
}

fn range_set(ctx: &mut CommandContext<'_>, args: RangeSetArgs) -> Result<Effect, CoreError> {
    if args.input.is_some() == args.values.is_some() {
        return Err(bus_error::args(
            "range.set requires exactly one of input or values",
        ));
    }
    let range = resolve_range(ctx.workbook_ref(), &args.range)?;
    let mut effect = Effect::default();
    if let Some(input) = args.input {
        for (row, col) in range.cells() {
            let cell = ResolvedCell {
                sheet: range.sheet,
                row,
                col,
            };
            let mut cell_effect = set_one_cell(ctx, cell, &input)?;
            cell_effect.summary.text.clear();
            effect.append(cell_effect);
        }
    } else if let Some(values) = args.values {
        if values.is_empty() {
            return Err(bus_error::args("values must be a non-empty 2-D array"));
        }
        let rows = values.len() as u32;
        let cols = values.iter().map(Vec::len).max().unwrap_or(0) as u32;
        let range_rows = range
            .max_row
            .saturating_sub(range.min_row)
            .saturating_add(1);
        let range_cols = u32::from(
            range
                .max_col
                .saturating_sub(range.min_col)
                .saturating_add(1),
        );
        if rows > range_rows || cols > range_cols {
            return Err(bus_error::args(
                "values shape is larger than the target range",
            ));
        }
        for (r, row) in values.iter().enumerate() {
            for (c, value) in row.iter().enumerate() {
                let cell = ResolvedCell {
                    sheet: range.sheet,
                    row: range.min_row + r as u32,
                    col: range.min_col + c as u16,
                };
                let input = value.as_deref().unwrap_or("");
                let mut cell_effect = set_one_cell(ctx, cell, input)?;
                cell_effect.summary.text.clear();
                effect.append(cell_effect);
            }
        }
    }
    effect.result = serde_json::json!({"changed": effect.summary.cells});
    effect.summary.text = format!("set {}", format_range(ctx.workbook_ref(), range));
    Ok(effect)
}

fn range_clear(ctx: &mut CommandContext<'_>, args: RangeClearArgs) -> Result<Effect, CoreError> {
    let range = resolve_range(ctx.workbook_ref(), &args.range)?;
    let mut effect = Effect::default();
    let used = ctx.workbook_ref().used_range(range.sheet)?;
    let Some(used) = used else {
        return Ok(Effect::query(serde_json::json!({"changed": 0})));
    };
    let min_row = range.min_row.max(used.min_row);
    let max_row = range.max_row.min(used.max_row);
    let min_col = range.min_col.max(used.min_col);
    let max_col = range.max_col.min(used.max_col);
    if min_row > max_row || min_col > max_col {
        return Ok(Effect::query(serde_json::json!({"changed": 0})));
    }
    for row in min_row..=max_row {
        for col in min_col..=max_col {
            let cell = ResolvedCell {
                sheet: range.sheet,
                row,
                col,
            };
            if ctx
                .workbook_ref()
                .get(cell.sheet, cell.row, cell.col)?
                .is_none()
            {
                continue;
            }
            let mut cell_effect = set_one_cell(ctx, cell, "")?;
            cell_effect.summary.text.clear();
            effect.append(cell_effect);
        }
    }
    effect.result = serde_json::json!({"changed": effect.summary.cells});
    effect.summary.text = format!("clear {}", format_range(ctx.workbook_ref(), range));
    Ok(effect)
}

fn next_sheet_name(ctx: &CommandContext<'_>) -> String {
    let mut n = 1usize;
    loop {
        let name = format!("Sheet{n}");
        if ctx.workbook_ref().sheet_by_name(&name).is_none() {
            return name;
        }
        n += 1;
    }
}

fn sheet_add(ctx: &mut CommandContext<'_>, args: SheetAddArgs) -> Result<Effect, CoreError> {
    let name = match args.name {
        Some(name) => name,
        None => next_sheet_name(ctx),
    };
    if ctx.workbook_ref().sheet_by_name(&name).is_some() {
        return Err(CoreError::sheet_name(format!(
            "sheet name {name:?} already exists"
        )));
    }
    let id = ctx.workbook().add_sheet(&name)?;
    let stored = ctx
        .workbook_ref()
        .sheet(id)
        .map(|s| s.name.clone())
        .unwrap_or(name);
    Ok(Effect {
        inverse: vec![call("sheet.remove", serde_json::json!({"sheet": stored}))?],
        events: Vec::new(),
        summary: ChangeSummary {
            sheets: 1,
            text: "add sheet".into(),
            ..ChangeSummary::default()
        },
        dirty: Vec::new(),
        result: serde_json::json!({"sheet": stored}),
        auto_recalc: true,
        rebuild: true,
    })
}

fn sheet_rename(ctx: &mut CommandContext<'_>, args: SheetRenameArgs) -> Result<Effect, CoreError> {
    let id = resolve_sheet(ctx.workbook_ref(), &args.sheet)?;
    let before = ctx
        .workbook_ref()
        .sheet(id)
        .map(|s| s.name.clone())
        .ok_or_else(|| CoreError::sheet_id("sheet vanished"))?;
    if before == args.name {
        return Ok(Effect::query(serde_json::json!({"changed": 0})));
    }
    ctx.workbook().rename_sheet(id, &args.name)?;
    Ok(Effect {
        inverse: vec![call(
            "sheet.rename",
            serde_json::json!({"sheet": args.name, "name": before}),
        )?],
        events: Vec::new(),
        summary: ChangeSummary {
            sheets: 1,
            text: "rename sheet".into(),
            ..ChangeSummary::default()
        },
        dirty: Vec::new(),
        result: serde_json::json!({"sheet": args.name}),
        auto_recalc: true,
        rebuild: true,
    })
}

fn parse_visibility(name: &str) -> Result<SheetVisibility, CoreError> {
    Ok(match name {
        "visible" => SheetVisibility::Visible,
        "hidden" => SheetVisibility::Hidden,
        "very_hidden" => SheetVisibility::VeryHidden,
        other => {
            return Err(bus_error::args(format!(
                "visibility must be visible, hidden, or very_hidden (got {other:?})"
            )));
        }
    })
}

fn visibility_name(v: SheetVisibility) -> &'static str {
    match v {
        SheetVisibility::Visible => "visible",
        SheetVisibility::Hidden => "hidden",
        SheetVisibility::VeryHidden => "very_hidden",
    }
}

fn sheet_visibility(
    ctx: &mut CommandContext<'_>,
    args: SheetVisibilityArgs,
) -> Result<Effect, CoreError> {
    let id = resolve_sheet(ctx.workbook_ref(), &args.sheet)?;
    let next = parse_visibility(&args.visibility)?;
    let before = ctx
        .workbook_ref()
        .sheet(id)
        .map(|s| s.visibility)
        .ok_or_else(|| CoreError::sheet_id("sheet vanished"))?;
    if before == next {
        return Ok(Effect::query(serde_json::json!({"changed": 0})));
    }
    let name = ctx
        .workbook_ref()
        .sheet(id)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| args.sheet.clone());
    ctx.workbook().set_visibility(id, next)?;
    Ok(Effect {
        inverse: vec![call(
            "sheet.visibility",
            serde_json::json!({"sheet": name, "visibility": visibility_name(before)}),
        )?],
        events: Vec::new(),
        summary: ChangeSummary {
            sheets: 1,
            text: "set sheet visibility".into(),
            ..ChangeSummary::default()
        },
        dirty: Vec::new(),
        result: serde_json::json!({"visibility": args.visibility}),
        auto_recalc: false,
        rebuild: false,
    })
}

fn sheet_remove(ctx: &mut CommandContext<'_>, args: SheetRemoveArgs) -> Result<Effect, CoreError> {
    let id = resolve_sheet(ctx.workbook_ref(), &args.sheet)?;
    let _ = ctx.workbook().remove_sheet(id)?;
    Ok(Effect {
        inverse: Vec::new(),
        events: Vec::new(),
        summary: ChangeSummary {
            sheets: 1,
            text: "remove sheet".into(),
            ..ChangeSummary::default()
        },
        dirty: Vec::new(),
        result: serde_json::json!({"removed": args.sheet}),
        auto_recalc: true,
        rebuild: true,
    })
}

fn name_scope(ctx: &CommandContext<'_>, sheet: Option<&str>) -> Result<NameScope, CoreError> {
    match sheet {
        None => Ok(NameScope::Workbook),
        Some(name) => Ok(NameScope::Sheet(resolve_sheet(ctx.workbook_ref(), name)?)),
    }
}

fn name_define(ctx: &mut CommandContext<'_>, args: NameDefineArgs) -> Result<Effect, CoreError> {
    let scope = name_scope(ctx, args.sheet.as_deref())?;
    if ctx.workbook_ref().names().get(scope, &args.name).is_some() {
        return Err(CoreError::name_defined(format!(
            "defined name {:?} already exists in this scope",
            args.name
        )));
    }
    let referent = match args.referent {
        NameReferentArg::Range { range } => {
            let resolved = resolve_range(ctx.workbook_ref(), &range)?;
            let start = omacell_core::addr::CellRef::new(resolved.min_row, resolved.min_col)?
                .on_sheet(resolved.sheet);
            let end = omacell_core::addr::CellRef::new(resolved.max_row, resolved.max_col)?
                .on_sheet(resolved.sheet);
            NameReferent::Range(omacell_core::addr::RangeRef::from_corners(start, end))
        }
        NameReferentArg::Constant { value } => NameReferent::Constant(json_to_value(ctx, value)?),
        NameReferentArg::Formula { formula } => NameReferent::Formula(formula),
    };
    let defined = DefinedName {
        name: args.name.clone(),
        scope,
        referent,
        comment: args.comment,
    };
    ctx.workbook().define_name(defined)?;
    let mut inverse_args = serde_json::json!({"name": args.name});
    if let Some(sheet) = args.sheet {
        inverse_args["sheet"] = serde_json::Value::String(sheet);
    }
    Ok(Effect {
        inverse: vec![call("name.remove", inverse_args)?],
        events: Vec::new(),
        summary: ChangeSummary {
            text: format!("define {}", args.name),
            ..ChangeSummary::default()
        },
        dirty: Vec::new(),
        result: serde_json::json!({"name": args.name}),
        auto_recalc: true,
        rebuild: true,
    })
}

fn json_to_value(
    ctx: &mut CommandContext<'_>,
    value: serde_json::Value,
) -> Result<Value, CoreError> {
    Ok(match value {
        serde_json::Value::Null => Value::Empty,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            let f = n
                .as_f64()
                .ok_or_else(|| bus_error::args("constant number must be finite"))?;
            if !f.is_finite() {
                return Err(bus_error::args("constant number must be finite"));
            }
            Value::Number(f)
        }
        serde_json::Value::String(s) => Value::Text(ctx.workbook().intern_text(&s)),
        other => {
            return Err(bus_error::args(format!(
                "constant must be null, bool, number, or string (got {other})"
            )));
        }
    })
}

fn name_remove(ctx: &mut CommandContext<'_>, args: NameRemoveArgs) -> Result<Effect, CoreError> {
    let scope = name_scope(ctx, args.sheet.as_deref())?;
    let existing = ctx
        .workbook_ref()
        .names()
        .get(scope, &args.name)
        .cloned()
        .ok_or_else(|| {
            CoreError::name_defined(format!(
                "defined name {:?} does not exist in this scope",
                args.name
            ))
        })?;
    let definition = store_defined_name(ctx.workbook_ref(), &existing)?;
    ctx.workbook().remove_name(scope, &args.name)?;
    Ok(Effect {
        inverse: vec![call(
            "name.restore",
            serde_json::json!({"definition": definition}),
        )?],
        events: Vec::new(),
        summary: ChangeSummary {
            text: format!("remove {}", args.name),
            ..ChangeSummary::default()
        },
        dirty: Vec::new(),
        result: serde_json::json!({"removed": args.name}),
        auto_recalc: true,
        rebuild: true,
    })
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum StoredNameReferent {
    Range { range: omacell_core::addr::RangeRef },
    Constant { value: serde_json::Value },
    Formula { formula: String },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDefinedName {
    name: String,
    scope: NameScope,
    referent: StoredNameReferent,
    comment: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct NameRestoreArgs {
    definition: serde_json::Value,
}

fn store_defined_name(
    workbook: &omacell_core::workbook::Workbook,
    definition: &DefinedName,
) -> Result<serde_json::Value, CoreError> {
    let referent = match &definition.referent {
        NameReferent::Range(range) => StoredNameReferent::Range { range: *range },
        NameReferent::Constant(value) => StoredNameReferent::Constant {
            value: store_logical_value(workbook, *value)?,
        },
        NameReferent::Formula(formula) => StoredNameReferent::Formula {
            formula: formula.clone(),
        },
    };
    serde_json::to_value(StoredDefinedName {
        name: definition.name.clone(),
        scope: definition.scope,
        referent,
        comment: definition.comment.clone(),
    })
    .map_err(|error| bus_error::args(format!("cannot encode defined-name inverse: {error}")))
}

fn name_restore(ctx: &mut CommandContext<'_>, args: NameRestoreArgs) -> Result<Effect, CoreError> {
    let stored: StoredDefinedName = serde_json::from_value(args.definition)
        .map_err(|error| bus_error::args(format!("invalid defined-name inverse: {error}")))?;
    let (referent, owned) = match stored.referent {
        StoredNameReferent::Range { range } => (NameReferent::Range(range), None),
        StoredNameReferent::Constant { value } => {
            let (value, owned) = restore_cell_value(ctx.workbook(), value)?;
            (NameReferent::Constant(value), owned)
        }
        StoredNameReferent::Formula { formula } => (NameReferent::Formula(formula), None),
    };
    let result = ctx.workbook().define_name(DefinedName {
        name: stored.name,
        scope: stored.scope,
        referent,
        comment: stored.comment,
    });
    if result.is_err() {
        release_root_ref(ctx.workbook(), owned);
    }
    result?;
    Ok(Effect {
        summary: ChangeSummary {
            text: "restore defined name".into(),
            ..ChangeSummary::default()
        },
        auto_recalc: true,
        rebuild: true,
        result: serde_json::json!({"restored": true}),
        ..Effect::default()
    })
}

fn name_createfrom(
    ctx: &mut CommandContext<'_>,
    args: NameCreateFromArgs,
) -> Result<Effect, CoreError> {
    if args.positions.is_empty() {
        return Err(bus_error::args(
            "name.createfrom requires at least one label position",
        ));
    }
    let positions = args
        .positions
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let range = resolve_range(ctx.workbook_ref(), &args.range)?;
    let top = positions.contains(&NameLabelPosition::Top);
    let left = positions.contains(&NameLabelPosition::Left);
    let bottom = positions.contains(&NameLabelPosition::Bottom);
    let right = positions.contains(&NameLabelPosition::Right);
    let data_min_row = range.min_row + u32::from(top);
    let data_max_row = range.max_row.saturating_sub(u32::from(bottom));
    let data_min_col = range.min_col + u16::from(left);
    let data_max_col = range.max_col.saturating_sub(u16::from(right));
    if data_min_row > data_max_row || data_min_col > data_max_col {
        return Err(bus_error::args(
            "label edges must leave at least one data cell in the selected range",
        ));
    }

    let mut definitions = Vec::new();
    if top {
        collect_column_names(
            ctx,
            &mut definitions,
            range.sheet,
            range.min_row,
            data_min_col,
            data_max_col,
            data_min_row,
            data_max_row,
        )?;
    }
    if bottom {
        collect_column_names(
            ctx,
            &mut definitions,
            range.sheet,
            range.max_row,
            data_min_col,
            data_max_col,
            data_min_row,
            data_max_row,
        )?;
    }
    if left {
        collect_row_names(
            ctx,
            &mut definitions,
            range.sheet,
            range.min_col,
            data_min_row,
            data_max_row,
            data_min_col,
            data_max_col,
        )?;
    }
    if right {
        collect_row_names(
            ctx,
            &mut definitions,
            range.sheet,
            range.max_col,
            data_min_row,
            data_max_row,
            data_min_col,
            data_max_col,
        )?;
    }
    if definitions.is_empty() {
        return Err(bus_error::args(
            "selected label edges do not contain any text labels",
        ));
    }

    let mut planned = std::collections::BTreeSet::new();
    for definition in &definitions {
        let key = definition.name.to_lowercase();
        if !planned.insert(key)
            || ctx
                .workbook_ref()
                .names()
                .get(NameScope::Workbook, &definition.name)
                .is_some()
        {
            return Err(CoreError::name_defined(format!(
                "defined name {:?} already exists in this selection or workbook",
                definition.name
            )));
        }
    }

    let mut inverse = Vec::with_capacity(definitions.len());
    for definition in definitions {
        let name = definition.name.clone();
        ctx.workbook().define_name(definition)?;
        inverse.push(call("name.remove", serde_json::json!({"name": name}))?);
    }
    let created = inverse.len();
    Ok(Effect {
        inverse,
        summary: ChangeSummary {
            text: format!("create {created} names from selection"),
            ..ChangeSummary::default()
        },
        result: serde_json::json!({"created": created}),
        auto_recalc: true,
        rebuild: true,
        ..Effect::default()
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_column_names(
    ctx: &CommandContext<'_>,
    definitions: &mut Vec<DefinedName>,
    sheet: omacell_core::addr::SheetId,
    label_row: u32,
    min_col: u16,
    max_col: u16,
    data_min_row: u32,
    data_max_row: u32,
) -> Result<(), CoreError> {
    for col in min_col..=max_col {
        if let Some(name) = label_name(ctx, sheet, label_row, col)? {
            definitions.push(DefinedName {
                name,
                scope: NameScope::Workbook,
                referent: absolute_range(sheet, data_min_row, col, data_max_row, col)?,
                comment: None,
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn collect_row_names(
    ctx: &CommandContext<'_>,
    definitions: &mut Vec<DefinedName>,
    sheet: omacell_core::addr::SheetId,
    label_col: u16,
    min_row: u32,
    max_row: u32,
    data_min_col: u16,
    data_max_col: u16,
) -> Result<(), CoreError> {
    for row in min_row..=max_row {
        if let Some(name) = label_name(ctx, sheet, row, label_col)? {
            definitions.push(DefinedName {
                name,
                scope: NameScope::Workbook,
                referent: absolute_range(sheet, row, data_min_col, row, data_max_col)?,
                comment: None,
            });
        }
    }
    Ok(())
}

fn label_name(
    ctx: &CommandContext<'_>,
    sheet: omacell_core::addr::SheetId,
    row: u32,
    col: u16,
) -> Result<Option<String>, CoreError> {
    let Some(slot) = ctx.workbook_ref().get(sheet, row, col)? else {
        return Ok(None);
    };
    let Value::Text(id) = slot.value else {
        return Ok(None);
    };
    let label = ctx.workbook_ref().intern().strings.get(id).unwrap_or("");
    if label.trim().is_empty() {
        return Ok(None);
    }
    normalize_label(label).map(Some)
}

fn normalize_label(label: &str) -> Result<String, CoreError> {
    let label = label.trim();
    let mut normalized = String::with_capacity(label.len() + 1);
    for character in label.chars().take(255) {
        if character.is_alphanumeric() || matches!(character, '_' | '.' | '?') {
            normalized.push(character);
        } else if !normalized.ends_with('_') {
            normalized.push('_');
        }
    }
    if normalized
        .chars()
        .next()
        .is_none_or(|first| !(first.is_alphabetic() || first == '_' || first == '\\'))
    {
        normalized.insert(0, '_');
    }
    if validate_defined_name(&normalized).is_err() {
        normalized.insert(0, '_');
    }
    while normalized.chars().count() > 255 {
        normalized.pop();
    }
    validate_defined_name(&normalized)?;
    Ok(normalized)
}

fn absolute_range(
    sheet: omacell_core::addr::SheetId,
    start_row: u32,
    start_col: u16,
    end_row: u32,
    end_col: u16,
) -> Result<NameReferent, CoreError> {
    let start =
        omacell_core::addr::CellRef::with_abs(start_row, start_col, true, true)?.on_sheet(sheet);
    let end = omacell_core::addr::CellRef::with_abs(end_row, end_col, true, true)?.on_sheet(sheet);
    Ok(NameReferent::Range(
        omacell_core::addr::RangeRef::from_corners(start, end),
    ))
}

fn format_number(
    ctx: &mut CommandContext<'_>,
    args: FormatNumberArgs,
) -> Result<Effect, CoreError> {
    let id = match (&args.format, args.num_fmt_id) {
        (Some(code), None) => ctx.workbook().intern_num_fmt(code)?,
        (None, Some(id)) => {
            if ctx.workbook_ref().num_fmt_code(NumFmtId::new(id)).is_none() && id > 49 {
                return Err(bus_error::args(format!("unknown num_fmt_id {id}")));
            }
            NumFmtId::new(id)
        }
        _ => {
            return Err(bus_error::args(
                "format.number requires exactly one of format or num_fmt_id",
            ));
        }
    };
    let range = resolve_range(ctx.workbook_ref(), &args.range)?;
    let mut effect = Effect::default();
    for (row, col) in range.cells() {
        let cell = ResolvedCell {
            sheet: range.sheet,
            row,
            col,
        };
        let inverse = inverse_style(ctx.workbook_ref(), cell)?;
        let existed = ctx
            .workbook_ref()
            .get(cell.sheet, cell.row, cell.col)?
            .is_some();
        let mut style = ctx
            .workbook_ref()
            .get(cell.sheet, cell.row, cell.col)?
            .map(|slot| style_of(ctx.workbook_ref(), slot.style))
            .unwrap_or_default();
        if style.num_fmt == id && existed {
            continue;
        }
        style.num_fmt = id;
        ctx.workbook()
            .set_cell_style(cell.sheet, cell.row, cell.col, style)?;
        effect.inverse.push(inverse);
        effect.summary.styles += 1;
        effect.summary.cells += 1;
        effect.events.push(Event::CellChanged {
            sheet: cell.sheet,
            row: cell.row,
            col: cell.col,
        });
        effect
            .dirty
            .push(CellCoord::new(cell.sheet, cell.row, cell.col));
    }
    if effect.summary.styles == 0 {
        return Ok(Effect::query(serde_json::json!({"changed": 0})));
    }
    effect.summary.text = "format number".into();
    effect.result = serde_json::json!({"changed": effect.summary.styles});
    effect.auto_recalc = false;
    Ok(effect)
}

fn style_set(ctx: &mut CommandContext<'_>, args: StyleSetArgs) -> Result<Effect, CoreError> {
    if args.bold.is_none()
        && args.italic.is_none()
        && args.underline.is_none()
        && args.strike.is_none()
        && args.size_pt.is_none()
        && args.font_name.is_none()
        && args.font_color_argb.is_none()
        && args.fill_argb.is_none()
        && args.wrap.is_none()
        && args.horizontal.is_none()
        && args.vertical.is_none()
        && args.locked.is_none()
        && args.hidden.is_none()
        && args.format.is_none()
    {
        return Err(bus_error::args(
            "style.set requires at least one style field",
        ));
    }
    let range = resolve_range(ctx.workbook_ref(), &args.range)?;
    let mut effect = Effect::default();
    for (row, col) in range.cells() {
        let cell = ResolvedCell {
            sheet: range.sheet,
            row,
            col,
        };
        let inverse = inverse_style(ctx.workbook_ref(), cell)?;
        let existed = ctx
            .workbook_ref()
            .get(cell.sheet, cell.row, cell.col)?
            .is_some();
        let current = ctx
            .workbook_ref()
            .get(cell.sheet, cell.row, cell.col)?
            .map(|slot| style_of(ctx.workbook_ref(), slot.style))
            .unwrap_or_default();
        let patched = apply_style_patch(ctx.workbook(), current.clone(), &args)?;
        if patched == current && existed {
            continue;
        }
        ctx.workbook()
            .set_cell_style(cell.sheet, cell.row, cell.col, patched)?;
        effect.inverse.push(inverse);
        effect.summary.styles += 1;
        effect.events.push(Event::CellChanged {
            sheet: cell.sheet,
            row: cell.row,
            col: cell.col,
        });
    }
    if effect.summary.styles == 0 {
        return Ok(Effect::query(serde_json::json!({"changed": 0})));
    }
    effect.summary.text = "set style".into();
    effect.result = serde_json::json!({"changed": effect.summary.styles});
    effect.auto_recalc = false;
    Ok(effect)
}

fn calc_recalc(ctx: &mut CommandContext<'_>, args: CalcRecalcArgs) -> Result<Effect, CoreError> {
    if ctx.is_preflight() && !ctx.is_dry_run() {
        return Ok(Effect::query(serde_json::json!({"queued": true})));
    }
    let manual = ctx.workbook_ref().settings().calc_mode == CalcMode::Manual;
    let result = match args.mode.as_deref() {
        None | Some("full" | "rebuild") => ctx.recalc_explicit_full(),
        Some("incremental") => {
            // Explicit calc.recalc must calculate even in Manual.
            if manual {
                // This is an incremental settlement/input-change wave even
                // though Manual mode requires a full engine pass. It must not
                // opt into the user's full-refresh policy.
                ctx.recalc_full()
            } else {
                ctx.recalc_incremental()
            }
        }
        Some(other) => {
            return Err(bus_error::args(format!(
                "calc.recalc mode must be incremental, full, or rebuild (got {other:?})"
            )));
        }
    };
    if result.cancelled {
        return Err(crate::error::task_cancelled());
    }
    ctx.report_progress(
        result.cells_evaluated,
        Some(result.cells_evaluated),
        "recalc",
    );
    Ok(Effect {
        inverse: Vec::new(),
        events: vec![Event::RecalcDone {
            cells: result.cells_evaluated,
            elapsed_ms: result.elapsed_ms,
        }],
        summary: ChangeSummary {
            text: "recalc".into(),
            ..ChangeSummary::default()
        },
        dirty: Vec::new(),
        result: serde_json::json!({
            "cells": result.cells_evaluated,
            "elapsed_ms": result.elapsed_ms
        }),
        auto_recalc: false,
        rebuild: false,
    })
}

fn calc_mode(ctx: &mut CommandContext<'_>, args: CalcModeArgs) -> Result<Effect, CoreError> {
    let mode = parse_calc_mode(&args.mode)?;
    let before = ctx.workbook_ref().settings().calc_mode;
    if before == mode {
        return Ok(Effect::query(serde_json::json!({"changed": 0})));
    }
    ctx.workbook().set_calc_mode(mode)?;
    Ok(Effect {
        inverse: vec![call(
            "calc.mode",
            serde_json::json!({"mode": calc_mode_name(before)}),
        )?],
        events: Vec::new(),
        summary: ChangeSummary {
            text: format!("calc mode {}", args.mode),
            ..ChangeSummary::default()
        },
        dirty: Vec::new(),
        result: serde_json::json!({"mode": args.mode}),
        auto_recalc: mode != CalcMode::Manual,
        rebuild: false,
    })
}

fn parse_calc_mode(name: &str) -> Result<CalcMode, CoreError> {
    Ok(match name {
        "automatic" => CalcMode::Automatic,
        "automatic_except_tables" => CalcMode::AutomaticExceptTables,
        "manual" => CalcMode::Manual,
        other => {
            return Err(bus_error::args(format!(
                "calc.mode must be automatic, automatic_except_tables, or manual (got {other:?})"
            )));
        }
    })
}

fn calc_mode_name(mode: CalcMode) -> &'static str {
    match mode {
        CalcMode::Automatic => "automatic",
        CalcMode::AutomaticExceptTables => "automatic_except_tables",
        CalcMode::Manual => "manual",
    }
}

fn undo_cmd(ctx: &mut CommandContext<'_>, _args: EmptyArgs) -> Result<Effect, CoreError> {
    let affected = ctx.workbook().undo()?;
    let mut dirty = Vec::new();
    for range in &affected {
        dirty.push(CellCoord::new(range.sheet, range.min_row, range.min_col));
    }
    Ok(Effect {
        inverse: Vec::new(),
        events: Vec::new(),
        summary: ChangeSummary {
            text: "undo".into(),
            ..ChangeSummary::default()
        },
        dirty,
        result: serde_json::json!({"ok": true}),
        auto_recalc: true,
        rebuild: true,
    })
}

fn redo_cmd(ctx: &mut CommandContext<'_>, _args: EmptyArgs) -> Result<Effect, CoreError> {
    let affected = ctx.workbook().redo()?;
    let mut dirty = Vec::new();
    for range in &affected {
        dirty.push(CellCoord::new(range.sheet, range.min_row, range.min_col));
    }
    Ok(Effect {
        inverse: Vec::new(),
        events: Vec::new(),
        summary: ChangeSummary {
            text: "redo".into(),
            ..ChangeSummary::default()
        },
        dirty,
        result: serde_json::json!({"ok": true}),
        auto_recalc: true,
        rebuild: true,
    })
}

pub(crate) fn cell_restore(
    ctx: &mut CommandContext<'_>,
    args: CellRestoreArgs,
) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    if args.absent {
        let _ = ctx.workbook().clear_cell(cell.sheet, cell.row, cell.col)?;
        return Ok(Effect {
            inverse: Vec::new(),
            events: vec![Event::CellChanged {
                sheet: cell.sheet,
                row: cell.row,
                col: cell.col,
            }],
            summary: ChangeSummary {
                cells: 1,
                text: "restore cell".into(),
                ..ChangeSummary::default()
            },
            dirty: vec![CellCoord::new(cell.sheet, cell.row, cell.col)],
            result: serde_json::json!({"restored": true}),
            auto_recalc: true,
            rebuild: false,
        });
    }
    let encoded = args
        .value
        .ok_or_else(|| bus_error::args("cell.restore requires a stored value"))?;
    let (value, owned) = restore_cell_value(ctx.workbook(), encoded)?;
    let formula = match args.formula {
        Some(source) => Some(ctx.workbook().intern_formula(&source)?),
        None => None,
    };
    let slot = CellSlot {
        value,
        formula,
        style: StyleId::DEFAULT,
        flags: decode_cell_flags(args.flags)?,
    };
    let result = ctx
        .workbook()
        .set_slot(cell.sheet, cell.row, cell.col, slot);
    if let Some(id) = formula {
        ctx.workbook().release_formula(id);
    }
    release_root_ref(ctx.workbook(), owned);
    result?;
    if let Some(style_json) = args.style {
        let style = apply_stored_style(ctx.workbook(), style_json, args.format.as_deref())?;
        ctx.workbook()
            .set_cell_style(cell.sheet, cell.row, cell.col, style)?;
    }
    Ok(Effect {
        inverse: Vec::new(),
        events: vec![Event::CellChanged {
            sheet: cell.sheet,
            row: cell.row,
            col: cell.col,
        }],
        summary: ChangeSummary {
            cells: 1,
            text: "restore cell".into(),
            ..ChangeSummary::default()
        },
        dirty: vec![CellCoord::new(cell.sheet, cell.row, cell.col)],
        result: serde_json::json!({"restored": true}),
        auto_recalc: true,
        rebuild: false,
    })
}

fn style_restore(
    ctx: &mut CommandContext<'_>,
    args: StyleRestoreArgs,
) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    if args.absent {
        let _ = ctx.workbook().clear_cell(cell.sheet, cell.row, cell.col)?;
    } else {
        let style = apply_stored_style(ctx.workbook(), args.style, args.format.as_deref())?;
        ctx.workbook()
            .set_cell_style(cell.sheet, cell.row, cell.col, style)?;
    }
    Ok(Effect {
        inverse: Vec::new(),
        events: vec![Event::CellChanged {
            sheet: cell.sheet,
            row: cell.row,
            col: cell.col,
        }],
        summary: ChangeSummary {
            styles: 1,
            text: "restore style".into(),
            ..ChangeSummary::default()
        },
        dirty: vec![CellCoord::new(cell.sheet, cell.row, cell.col)],
        result: serde_json::json!({"restored": true}),
        auto_recalc: false,
        rebuild: false,
    })
}
