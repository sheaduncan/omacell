//! WP-17 editing and structure commands.

use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::changeset::ChangeSummary;
use omacell_core::error::CoreError;
use omacell_core::ops::{
    ClipCell, FillMode, PasteOp, PasteSpecial, Shift, copy_range, delete_cells, delete_cols,
    delete_rows, detect_fill, excel_xor_hash, fill_range, insert_cells, insert_cols, insert_rows,
    merge, merge_across, move_range_cells, paste_special, remove_duplicates, text_to_columns,
    unmerge,
};
use omacell_core::sheet::{Hyperlink, Note};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::handler::{CommandContext, Effect};
use crate::registry::{CommandKind, CommandRegistry, CommandSpec, Exposure};
use crate::resolve::{resolve_cell, resolve_range};

/// `edit.insert` / `edit.delcells`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditInsertArgs {
    /// A1 range.
    pub range: String,
    /// `down` / `right` / `rows` / `cols`.
    #[serde(default = "down")]
    pub shift: String,
}

fn down() -> String {
    "down".into()
}

/// `edit.copy` / fill
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeOnlyArgs {
    /// A1 range.
    pub range: String,
}

/// `edit.paste`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditPasteArgs {
    /// Destination A1.
    pub range: String,
    /// Clipboard JSON from `edit.copy`.
    pub payload: serde_json::Value,
    /// `values` / `formulas` / `transpose` / `add` / `skipblanks` / `link`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub special: Option<String>,
}

/// `edit.move`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditMoveArgs {
    /// Source A1 range.
    pub src: String,
    /// Destination A1 cell.
    pub dest: String,
}

/// `edit.fillselection`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditFillArgs {
    /// Source A1 range.
    pub src: String,
    /// Destination A1 range.
    pub dest: String,
    /// `copy` / `linear` / `growth` / `date` / `weekday` / `month` / `year`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// `sheet.reorder`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetReorderArgs {
    /// Sheet name.
    pub sheet: String,
    /// 0-based target index.
    pub index: u32,
}

/// `sheet.protect`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetProtectArgs {
    /// Sheet name; default active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    /// Password (legacy XOR hash; not security).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Enable protection.
    #[serde(default = "on")]
    pub enable: bool,
}

fn on() -> bool {
    true
}

/// `edit.note`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditNoteArgs {
    /// A1 cell.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// Note body. Empty deletes.
    pub text: String,
    /// Author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

/// `edit.hyperlink`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditHyperlinkArgs {
    /// A1 cell.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// URL or internal location. Empty deletes.
    pub target: String,
}

/// `range.removeduplicates`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveDupArgs {
    /// A1 range.
    pub range: String,
}

/// `edit.texttocolumns`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextToColumnsArgs {
    /// A1 range.
    pub range: String,
    /// Delimiter character.
    #[serde(default = "comma")]
    pub delim: String,
}

fn comma() -> String {
    ",".into()
}

/// Hide/group args.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AxisRangeArgs {
    /// A1 range whose rows or columns are targeted.
    pub range: String,
}

/// Register WP-17 commands.
pub fn register_edit_commands(registry: &mut CommandRegistry) -> Result<(), CoreError> {
    registry.register(
        CommandSpec {
            id: "edit.insert",
            doc: "Insert cells, rows, or columns",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Shift+="],
        },
        edit_insert,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.delcells",
            doc: "Delete cells, rows, or columns",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+-"],
        },
        edit_delcells,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.copy",
            doc: "Copy a range to an internal clipboard payload",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+C"],
        },
        edit_copy,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.cut",
            doc: "Cut a range (copy payload marked for move)",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+X"],
        },
        edit_cut,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.paste",
            doc: "Paste a clipboard payload",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+V"],
        },
        edit_paste,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.pastespecial",
            doc: "Paste special",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Alt+V"],
        },
        edit_paste,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.move",
            doc: "Move a range in one atomic operation",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_move,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.fillselection",
            doc: "Fill the destination from a source range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Enter"],
        },
        edit_fill,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.filldown",
            doc: "Fill down",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+D"],
        },
        edit_filldown,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.fillright",
            doc: "Fill right",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+R"],
        },
        edit_fillright,
    )?;
    registry.register(
        CommandSpec {
            id: "range.merge",
            doc: "Merge a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        range_merge,
    )?;
    registry.register(
        CommandSpec {
            id: "range.mergeacross",
            doc: "Merge each row of a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        range_mergeacross,
    )?;
    registry.register(
        CommandSpec {
            id: "range.unmerge",
            doc: "Unmerge overlapping merges",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        range_unmerge,
    )?;
    registry.register(
        CommandSpec {
            id: "view.hiderows",
            doc: "Hide rows in a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+9"],
        },
        hide_rows,
    )?;
    registry.register(
        CommandSpec {
            id: "view.hidecols",
            doc: "Hide columns in a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+0"],
        },
        hide_cols,
    )?;
    registry.register(
        CommandSpec {
            id: "view.unhiderows",
            doc: "Unhide rows",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Shift+9"],
        },
        unhide_rows,
    )?;
    registry.register(
        CommandSpec {
            id: "view.unhidecols",
            doc: "Unhide columns",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Shift+0"],
        },
        unhide_cols,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.group",
            doc: "Group rows or columns",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Alt+Shift+Right"],
        },
        edit_group,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.ungroup",
            doc: "Ungroup rows or columns",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Alt+Shift+Left"],
        },
        edit_ungroup,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.note",
            doc: "Set or clear a legacy note",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Shift+F2"],
        },
        edit_note,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.hyperlink",
            doc: "Set or clear a hyperlink",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+K"],
        },
        edit_hyperlink,
    )?;
    registry.register(
        CommandSpec {
            id: "sheet.protect",
            doc: "Protect or unprotect a sheet (legacy XOR hash)",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        sheet_protect,
    )?;
    registry.register(
        CommandSpec {
            id: "sheet.reorder",
            doc: "Move a sheet to a new tab index",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        sheet_reorder,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.texttocolumns",
            doc: "Split text into columns",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_texttocolumns,
    )?;
    registry.register(
        CommandSpec {
            id: "range.removeduplicates",
            doc: "Clear duplicate rows in a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        range_removeduplicates,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.clearcell",
            doc: "Clear cell contents in a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Delete"],
        },
        edit_clearcell,
    )?;
    Ok(())
}

fn to_range(r: crate::resolve::ResolvedRange) -> RangeRef {
    RangeRef::from_corners(
        CellRef::new(r.min_row, r.min_col).unwrap(),
        CellRef::new(r.max_row, r.max_col).unwrap(),
    )
}

fn parse_shift(s: &str) -> Shift {
    if s.eq_ignore_ascii_case("right") || s.eq_ignore_ascii_case("cols") {
        Shift::Right
    } else {
        Shift::Down
    }
}

fn whole_rows(r: crate::resolve::ResolvedRange) -> bool {
    r.min_col == 0 && r.max_col == omacell_core::limits::MAX_COLS - 1
}

fn whole_cols(r: crate::resolve::ResolvedRange) -> bool {
    r.min_row == 0 && r.max_row == omacell_core::limits::MAX_ROWS - 1
}

fn edit_insert(ctx: &mut CommandContext<'_>, args: EditInsertArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let shift = parse_shift(&args.shift);
    if whole_rows(r) || args.shift.eq_ignore_ascii_case("rows") {
        insert_rows(
            ctx.workbook(),
            r.sheet,
            r.min_row,
            r.max_row - r.min_row + 1,
        )?;
    } else if whole_cols(r) || args.shift.eq_ignore_ascii_case("cols") {
        insert_cols(
            ctx.workbook(),
            r.sheet,
            r.min_col,
            r.max_col - r.min_col + 1,
        )?;
    } else {
        insert_cells(ctx.workbook(), r.sheet, to_range(r), shift)?;
    }
    Ok(changed("insert"))
}

fn edit_delcells(ctx: &mut CommandContext<'_>, args: EditInsertArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let shift = parse_shift(&args.shift);
    if whole_rows(r) || args.shift.eq_ignore_ascii_case("rows") {
        delete_rows(
            ctx.workbook(),
            r.sheet,
            r.min_row,
            r.max_row - r.min_row + 1,
        )?;
    } else if whole_cols(r) || args.shift.eq_ignore_ascii_case("cols") {
        delete_cols(
            ctx.workbook(),
            r.sheet,
            r.min_col,
            r.max_col - r.min_col + 1,
        )?;
    } else {
        delete_cells(ctx.workbook(), r.sheet, to_range(r), shift)?;
    }
    Ok(changed("delete"))
}

fn edit_copy(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let grid = copy_range(ctx.workbook_ref(), r.sheet, to_range(r));
    let cells: Vec<Vec<String>> = grid
        .iter()
        .map(|row| row.iter().map(|c| c.input.clone()).collect())
        .collect();
    Ok(Effect::query(serde_json::json!({
        "payload": {
            "cut": false,
            "sheet": r.sheet.index(),
            "row": r.min_row,
            "col": r.min_col,
            "cells": cells,
        }
    })))
}

fn edit_cut(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let mut effect = edit_copy(ctx, args)?;
    if let Some(obj) = effect.result.get_mut("payload") {
        obj["cut"] = serde_json::json!(true);
    }
    Ok(effect)
}

fn edit_paste(ctx: &mut CommandContext<'_>, args: EditPasteArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let cells = args
        .payload
        .get("cells")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut grid: Vec<Vec<ClipCell>> = Vec::new();
    for row in cells {
        let mut out = Vec::new();
        if let Some(arr) = row.as_array() {
            for cell in arr {
                let input = cell.as_str().unwrap_or("").to_string();
                out.push(ClipCell {
                    number: input.parse().ok(),
                    input,
                });
            }
        }
        grid.push(out);
    }
    let mut spec = PasteSpecial {
        values: true,
        formulas: true,
        ..PasteSpecial::default()
    };
    if let Some(kind) = args.special.as_deref() {
        spec = match kind {
            "values" => PasteSpecial {
                values: true,
                ..PasteSpecial::default()
            },
            "formulas" => PasteSpecial {
                formulas: true,
                ..PasteSpecial::default()
            },
            "transpose" => PasteSpecial {
                values: true,
                transpose: true,
                ..PasteSpecial::default()
            },
            "add" => PasteSpecial {
                operation: PasteOp::Add,
                ..PasteSpecial::default()
            },
            "skipblanks" => PasteSpecial {
                values: true,
                skip_blanks: true,
                ..PasteSpecial::default()
            },
            "link" => PasteSpecial {
                paste_link: true,
                ..PasteSpecial::default()
            },
            _ => spec,
        };
    }
    let origin = args
        .payload
        .get("row")
        .and_then(|v| v.as_u64())
        .and_then(|row| {
            args.payload
                .get("col")
                .and_then(|v| v.as_u64())
                .map(|col| (row as u32, col as u16))
        });
    let n = paste_special(
        ctx.workbook(),
        r.sheet,
        CellRef::new(r.min_row, r.min_col).unwrap(),
        &grid,
        spec,
        origin,
    )?;
    if args
        .payload
        .get("cut")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        && let (Some(sr), Some(sc), Some(sid)) = (
            args.payload.get("row").and_then(|v| v.as_u64()),
            args.payload.get("col").and_then(|v| v.as_u64()),
            args.payload.get("sheet").and_then(|v| v.as_u64()),
        )
    {
        let h = grid.len() as u32;
        let w = grid.first().map(|row| row.len() as u16).unwrap_or(0);
        let src = RangeRef::from_corners(
            CellRef::new(sr as u32, sc as u16).unwrap(),
            CellRef::new(
                sr as u32 + h.saturating_sub(1),
                sc as u16 + w.saturating_sub(1),
            )
            .unwrap(),
        );
        let sheet = omacell_core::addr::SheetId::new(sid as u32);
        let _ = move_range_cells(
            ctx.workbook(),
            sheet,
            src,
            CellRef::new(r.min_row, r.min_col).unwrap(),
        );
        let _ = n;
    }
    Ok(changed("paste"))
}

fn edit_move(ctx: &mut CommandContext<'_>, args: EditMoveArgs) -> Result<Effect, CoreError> {
    let src = resolve_range(ctx.workbook_ref(), &args.src)?;
    let dest = resolve_cell(ctx.workbook_ref(), &args.dest)?;
    let n = move_range_cells(
        ctx.workbook(),
        src.sheet,
        to_range(src),
        CellRef::new(dest.row, dest.col).unwrap(),
    )?;
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        auto_recalc: true,
        rebuild: true,
        summary: ChangeSummary {
            cells: u64::from(n),
            text: "move".into(),
            ..ChangeSummary::default()
        },
        ..Effect::default()
    })
}

fn parse_mode(s: Option<&str>, values: &[f64]) -> FillMode {
    match s.unwrap_or("") {
        "linear" => FillMode::Linear,
        "growth" => FillMode::Growth,
        "date" => FillMode::Date,
        "weekday" => FillMode::Weekday,
        "month" => FillMode::Month,
        "year" => FillMode::Year,
        "copy" => FillMode::Copy,
        _ => detect_fill(values),
    }
}

fn edit_fill(ctx: &mut CommandContext<'_>, args: EditFillArgs) -> Result<Effect, CoreError> {
    let src = resolve_range(ctx.workbook_ref(), &args.src)?;
    let dest = resolve_range(ctx.workbook_ref(), &args.dest)?;
    let mut nums = Vec::new();
    for r in src.min_row..=src.max_row {
        if let Ok(Some(slot)) = ctx.workbook_ref().get(src.sheet, r, src.min_col)
            && let omacell_core::value::Value::Number(n) = slot.value
        {
            nums.push(n);
        }
    }
    let mode = parse_mode(args.mode.as_deref(), &nums);
    let n = fill_range(
        ctx.workbook(),
        src.sheet,
        to_range(src),
        to_range(dest),
        mode,
    )?;
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        auto_recalc: true,
        ..Effect::default()
    })
}

fn edit_filldown(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    if r.max_row == r.min_row {
        return Ok(changed("fill"));
    }
    let src = RangeRef::from_corners(
        CellRef::new(r.min_row, r.min_col).unwrap(),
        CellRef::new(r.min_row, r.max_col).unwrap(),
    );
    let n = fill_range(ctx.workbook(), r.sheet, src, to_range(r), FillMode::Copy)?;
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        auto_recalc: true,
        ..Effect::default()
    })
}

fn edit_fillright(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let src = RangeRef::from_corners(
        CellRef::new(r.min_row, r.min_col).unwrap(),
        CellRef::new(r.max_row, r.min_col).unwrap(),
    );
    let n = fill_range(ctx.workbook(), r.sheet, src, to_range(r), FillMode::Copy)?;
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        auto_recalc: true,
        ..Effect::default()
    })
}

fn range_merge(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    merge(ctx.workbook(), r.sheet, to_range(r))?;
    Ok(changed("merge"))
}

fn range_mergeacross(
    ctx: &mut CommandContext<'_>,
    args: RangeOnlyArgs,
) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    merge_across(ctx.workbook(), r.sheet, to_range(r))?;
    Ok(changed("merge"))
}

fn range_unmerge(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let n = unmerge(ctx.workbook(), r.sheet, to_range(r))?;
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        ..Effect::default()
    })
}

fn hide_rows(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    for row in r.min_row..=r.max_row {
        ctx.workbook().set_row_hidden(r.sheet, row, true)?;
    }
    Ok(changed("hide"))
}

fn hide_cols(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    for col in r.min_col..=r.max_col {
        ctx.workbook()
            .sheet_mut_public(r.sheet)?
            .geometry
            .cols
            .set_hidden(u32::from(col), true)?;
    }
    Ok(changed("hide"))
}

fn unhide_rows(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    for row in r.min_row..=r.max_row {
        ctx.workbook().set_row_hidden(r.sheet, row, false)?;
    }
    Ok(changed("unhide"))
}

fn unhide_cols(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    for col in r.min_col..=r.max_col {
        ctx.workbook()
            .sheet_mut_public(r.sheet)?
            .geometry
            .cols
            .set_hidden(u32::from(col), false)?;
    }
    Ok(changed("unhide"))
}

fn edit_group(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let sheet = ctx.workbook().sheet_mut_public(r.sheet)?;
    if whole_cols(r) {
        for c in r.min_col..=r.max_col {
            let lvl = sheet
                .geometry
                .cols
                .outline_level(u32::from(c))
                .saturating_add(1);
            sheet.geometry.cols.set_outline_level(u32::from(c), lvl)?;
        }
    } else {
        for row in r.min_row..=r.max_row {
            let lvl = sheet.geometry.rows.outline_level(row).saturating_add(1);
            sheet.geometry.rows.set_outline_level(row, lvl)?;
        }
    }
    Ok(changed("group"))
}

fn edit_ungroup(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let sheet = ctx.workbook().sheet_mut_public(r.sheet)?;
    for row in r.min_row..=r.max_row {
        let lvl = sheet.geometry.rows.outline_level(row).saturating_sub(1);
        sheet.geometry.rows.set_outline_level(row, lvl)?;
    }
    Ok(changed("ungroup"))
}

fn edit_note(ctx: &mut CommandContext<'_>, args: EditNoteArgs) -> Result<Effect, CoreError> {
    let c = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let sheet = ctx.workbook().sheet_mut_public(c.sheet)?;
    if args.text.is_empty() {
        sheet.notes.remove(&(c.row, c.col));
    } else {
        sheet.notes.insert(
            (c.row, c.col),
            Note {
                author: args.author,
                text: args.text,
            },
        );
    }
    Ok(changed("note"))
}

fn edit_hyperlink(
    ctx: &mut CommandContext<'_>,
    args: EditHyperlinkArgs,
) -> Result<Effect, CoreError> {
    let c = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let sheet = ctx.workbook().sheet_mut_public(c.sheet)?;
    if args.target.is_empty() {
        sheet.hyperlinks.remove(&(c.row, c.col));
    } else {
        sheet.hyperlinks.insert(
            (c.row, c.col),
            Hyperlink {
                target: args.target,
                tooltip: None,
                display: None,
            },
        );
    }
    Ok(changed("hyperlink"))
}

fn sheet_protect(
    ctx: &mut CommandContext<'_>,
    args: SheetProtectArgs,
) -> Result<Effect, CoreError> {
    let id = match args.sheet.as_deref() {
        Some(name) => ctx
            .workbook_ref()
            .sheet_by_name(name)
            .map(|s| s.id)
            .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?,
        None => ctx.workbook_ref().active_sheet(),
    };
    let hash = args.password.as_deref().map(excel_xor_hash);
    let sheet = ctx.workbook().sheet_mut_public(id)?;
    sheet.protection.enabled = args.enable;
    sheet.protection.password = hash.map(|h| h.to_be_bytes().to_vec());
    Ok(Effect {
        result: serde_json::json!({
            "enabled": args.enable,
            "hash": hash,
        }),
        ..Effect::default()
    })
}

fn sheet_reorder(
    ctx: &mut CommandContext<'_>,
    args: SheetReorderArgs,
) -> Result<Effect, CoreError> {
    let id = ctx
        .workbook_ref()
        .sheet_by_name(&args.sheet)
        .map(|s| s.id)
        .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?;
    ctx.workbook().reorder_sheet(id, args.index as usize)?;
    Ok(changed("reorder"))
}

fn edit_texttocolumns(
    ctx: &mut CommandContext<'_>,
    args: TextToColumnsArgs,
) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let delim = args.delim.chars().next().unwrap_or(',');
    let n = text_to_columns(ctx.workbook(), r.sheet, to_range(r), delim)?;
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        auto_recalc: true,
        ..Effect::default()
    })
}

fn range_removeduplicates(
    ctx: &mut CommandContext<'_>,
    args: RemoveDupArgs,
) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let n = remove_duplicates(ctx.workbook(), r.sheet, to_range(r), &[])?;
    Ok(Effect {
        result: serde_json::json!({"removed": n}),
        auto_recalc: true,
        ..Effect::default()
    })
}

fn edit_clearcell(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let mut n = 0u32;
    for row in r.min_row..=r.max_row {
        for col in r.min_col..=r.max_col {
            ctx.workbook().set_cell_contents(r.sheet, row, col, "")?;
            n += 1;
        }
    }
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        auto_recalc: true,
        ..Effect::default()
    })
}

fn changed(text: &str) -> Effect {
    Effect {
        result: serde_json::json!({"ok": true}),
        auto_recalc: true,
        rebuild: true,
        summary: ChangeSummary {
            text: text.into(),
            ..ChangeSummary::default()
        },
        inverse: Vec::new(),
        ..Effect::default()
    }
}
