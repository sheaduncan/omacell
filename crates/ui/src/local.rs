//! Session-local commands that can run against a reader snapshot.

use omacell_core::command::Outcome;
use omacell_core::error::CoreError;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::sheet::{FreezePanes, SplitView};
use omacell_core::storage::CellSlot;
use omacell_core::value::Value as CellValue;
use omacell_core::workbook::Workbook;
use serde_json::Value;

use crate::edit::EditSurface;
use crate::mode::{KeyModel, Mode};
use crate::selection::{Area, ExtendMode};
use crate::session::UiSession;
use crate::viewport::Viewport;

/// Commands that only need the UI session and a read-only workbook snapshot.
#[must_use]
pub fn is_local_command(id: &str) -> bool {
    matches!(
        id,
        "nav.left"
            | "nav.right"
            | "nav.up"
            | "nav.down"
            | "nav.top"
            | "nav.bottom"
            | "nav.firstcol"
            | "nav.lastcol"
            | "nav.pagedown"
            | "nav.pageup"
            | "nav.halfpagedown"
            | "nav.halfpageup"
            | "nav.pageleft"
            | "nav.pageright"
            | "nav.screentop"
            | "nav.screenmiddle"
            | "nav.screenbottom"
            | "nav.a1"
            | "nav.nextedge"
            | "nav.prevedge"
            | "nav.edgeup"
            | "nav.edgedown"
            | "sel.visual"
            | "sel.visualrow"
            | "sel.visualcol"
            | "sel.extendleft"
            | "sel.extendright"
            | "sel.extendup"
            | "sel.extenddown"
            | "sel.edgeleft"
            | "sel.edgeright"
            | "sel.edgeup"
            | "sel.edgedown"
            | "sel.row"
            | "sel.col"
            | "sel.extendmode"
            | "sel.addmode"
            | "mode.normal"
            | "edit.cell"
            | "edit.append"
            | "edit.formula"
            | "edit.cancel"
            | "edit.cycleanchor"
            | "view.zoom"
            | "view.select"
            | "view.center"
            | "view.freeze"
            | "view.split"
            | "view.formulabar"
            | "view.formulas"
            | "palette.open"
            | "ai.assist"
            | "ai.agent"
            | "help.keys"
            | "command.line"
            | "nav.goto"
            | "edit.find"
            | "changeset.review"
            | "comments.panel"
            | "sort.panel"
            | "filter.panel"
    )
}

/// Whether a completed registered command changed workbook-owned state.
///
/// Retained frontends use this shared policy for dirty-state and reader-snapshot
/// reconciliation. An explicit `changed` count is authoritative, including
/// zero; otherwise successful mutating commands are presumed to change the
/// workbook unless they are known presentation, lifecycle, or host controls.
#[must_use]
pub fn command_changes_workbook(
    command: &str,
    outcome: &Outcome,
    registered_mutating: bool,
) -> bool {
    if let Some(changed) = outcome
        .result
        .as_ref()
        .and_then(|result| result.get("changed"))
        .and_then(Value::as_u64)
    {
        return changed > 0;
    }
    if matches!(command, "edit.undo" | "edit.redo") {
        return true;
    }
    if !registered_mutating
        || is_local_command(command)
        || matches!(
            command,
            "edit.searchnext"
                | "edit.searchprev"
                | "edit.explainerror"
                | "name.manager"
                | "name.paste"
                | "macro.record"
                | "macro.stop"
                | "macro.save"
                | "script.source"
                | "sheet.next"
                | "sheet.prev"
                | "changeset.review"
                | "file.open"
                | "file.save"
                | "file.saveas"
                | "file.new"
                | "file.close"
                | "file.export"
                | "file.print"
                | "theme.reload"
        )
    {
        return false;
    }
    true
}

fn count(args: &Value) -> i64 {
    i64::from(
        args.get("count")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as u32,
    )
}

/// Apply a session-local command using `wb` only for reads. `None` if not local.
pub fn apply_local_command(
    session: &UiSession,
    wb: &Workbook,
    cmd: &str,
    args: &Value,
) -> Option<Result<(), CoreError>> {
    if !is_local_command(cmd) {
        return None;
    }
    Some(apply(session, wb, cmd, args))
}

fn apply(session: &UiSession, wb: &Workbook, cmd: &str, args: &Value) -> Result<(), CoreError> {
    let n = count(args);
    let mut inner = session.inner.lock().unwrap_or_else(|p| p.into_inner());
    match cmd {
        "nav.left" => inner.selection.move_by(0, -n),
        "nav.right" => inner.selection.move_by(0, n),
        "nav.up" => inner.selection.move_by(-n, 0),
        "nav.down" => inner.selection.move_by(n, 0),
        "nav.pagedown" => {
            let rows = i64::from(inner.viewport.page_rows().max(1));
            inner.selection.move_by(rows * n, 0);
        }
        "nav.pageup" => {
            let rows = i64::from(inner.viewport.page_rows().max(1));
            inner.selection.move_by(-rows * n, 0);
        }
        "nav.halfpagedown" => {
            let rows = i64::from((inner.viewport.page_rows() / 2).max(1));
            inner.selection.move_by(rows * n, 0);
        }
        "nav.halfpageup" => {
            let rows = i64::from((inner.viewport.page_rows() / 2).max(1));
            inner.selection.move_by(-rows * n, 0);
        }
        "nav.pageleft" => {
            let cols = i64::from(inner.viewport.page_cols().max(1));
            inner.selection.move_by(0, -cols * n);
        }
        "nav.pageright" => {
            let cols = i64::from(inner.viewport.page_cols().max(1));
            inner.selection.move_by(0, cols * n);
        }
        "nav.screentop" => {
            let (top, _, _) = inner.viewport.screen_rows();
            let row = inner.selection.cursor.row;
            inner.selection.move_by(i64::from(top) - i64::from(row), 0);
        }
        "nav.screenmiddle" => {
            let (_, mid, _) = inner.viewport.screen_rows();
            let row = inner.selection.cursor.row;
            inner.selection.move_by(i64::from(mid) - i64::from(row), 0);
        }
        "nav.screenbottom" => {
            let (_, _, bottom) = inner.viewport.screen_rows();
            let row = inner.selection.cursor.row;
            inner
                .selection
                .move_by(i64::from(bottom) - i64::from(row), 0);
        }
        "nav.a1" => {
            let mut cursor = inner.selection.cursor;
            cursor.row = 0;
            cursor.col = 0;
            inner.selection.replace(Area::cell(cursor));
        }
        "nav.firstcol" => {
            let mut cursor = inner.selection.cursor;
            cursor.col = 0;
            inner.selection.replace(Area::cell(cursor));
        }
        "nav.top" => {
            let mut cursor = inner.selection.cursor;
            cursor.row = 0;
            inner.selection.replace(Area::cell(cursor));
        }
        "nav.bottom" => move_to_used_edge(wb, &mut inner.selection, true),
        "nav.lastcol" => move_to_used_edge(wb, &mut inner.selection, false),
        "sel.extendleft" => {
            inner.selection.extend = ExtendMode::Extend;
            inner.selection.move_by(0, -n);
        }
        "sel.extendright" => {
            inner.selection.extend = ExtendMode::Extend;
            inner.selection.move_by(0, n);
        }
        "sel.extendup" => {
            inner.selection.extend = ExtendMode::Extend;
            inner.selection.move_by(-n, 0);
        }
        "sel.extenddown" => {
            inner.selection.extend = ExtendMode::Extend;
            inner.selection.move_by(n, 0);
        }
        "sel.extendmode" => {
            inner.selection.extend = match inner.selection.extend {
                ExtendMode::Extend => ExtendMode::Replace,
                _ => ExtendMode::Extend,
            };
        }
        "sel.addmode" => inner.selection.extend = ExtendMode::Add,
        "sel.row" => inner.selection.select_row(),
        "sel.col" => inner.selection.select_col(),
        "sel.visual" => {
            inner.mode = Mode::Visual;
            inner.selection.extend = ExtendMode::Extend;
        }
        "sel.visualrow" => {
            inner.mode = Mode::VisualRow;
            inner.selection.select_row();
        }
        "sel.visualcol" => {
            inner.mode = Mode::VisualCol;
            inner.selection.select_col();
        }
        "view.center" => {
            let cursor = inner.selection.cursor;
            inner.viewport.center_on(cursor.row, cursor.col);
        }
        "view.zoom" => apply_zoom(&mut inner.viewport, args)?,
        "view.freeze" => {
            inner.viewport.freeze = FreezePanes {
                rows: inner.selection.cursor.row,
                cols: inner.selection.cursor.col,
            };
            inner.viewport.split = None;
        }
        "view.split" => {
            let cursor = inner.selection.cursor;
            let x = inner.viewport.cols.index_to_pixel(u32::from(cursor.col));
            let y = inner.viewport.rows.index_to_pixel(cursor.row);
            inner.viewport.split = Some(SplitView {
                x_px: scaled_coordinate(x, inner.viewport.zoom),
                y_px: scaled_coordinate(y, inner.viewport.zoom),
            });
            inner.viewport.freeze = FreezePanes::default();
            inner.viewport.first_row = cursor.row;
            inner.viewport.first_col = cursor.col;
        }
        "view.formulabar" => inner.formula_bar_expanded = !inner.formula_bar_expanded,
        "view.formulas" => inner.show_formulas = !inner.show_formulas,
        "view.select" => apply_select(wb, &mut inner.selection, args)?,
        "palette.open" => inner.palette.open(),
        "ai.assist" => crate::assist::open(&mut inner.palette),
        "ai.agent" => {
            inner.pending_agent = Some(crate::session::AgentHandoff {
                prompt: args
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                diagnose: args
                    .get("diagnose")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
        }
        "help.keys" | "nav.goto" | "edit.find" | "command.line" | "changeset.review" => {
            let id = match cmd {
                "help.keys" => "keys",
                "nav.goto" => "goto",
                "command.line" => "command",
                "changeset.review" => "changeset",
                _ => "find",
            };
            if cmd == "command.line" {
                inner.mode = Mode::Command;
            }
            inner.panel.open(id);
        }
        "comments.panel" | "sort.panel" | "filter.panel" => {
            let kind = match cmd {
                "comments.panel" => crate::panel::WorkbookPanel::Comments,
                "sort.panel" => crate::panel::WorkbookPanel::Sort,
                _ => crate::panel::WorkbookPanel::Filter,
            };
            let selection = inner.selection.clone();
            crate::panel::open_workbook_panel(&mut inner.panel, &selection, wb, kind);
        }
        "mode.normal" => {
            inner.mode = Mode::Normal;
            inner.edit.cancel();
            inner.panel.dismiss();
        }
        "edit.cancel" => {
            inner.edit.cancel();
            inner.panel.dismiss();
        }
        "edit.formula" => {
            let origin = inner.selection.cursor;
            inner.edit.begin(EditSurface::InCell, origin, "=");
            if inner.model == KeyModel::Modal {
                inner.mode = Mode::Insert;
            }
        }
        "edit.cell" | "edit.append" => {
            let origin = inner.selection.cursor;
            let initial = cell_input(wb, origin)?;
            inner.edit.begin(EditSurface::InCell, origin, &initial);
            if inner.model == KeyModel::Modal {
                inner.mode = Mode::Insert;
            }
        }
        "edit.cycleanchor" => inner.edit.cycle_anchor()?,
        _ => {
            if let Some((dr, dc)) = edge_delta(cmd)
                && let Some(next) = snapshot_edge(wb, inner.selection.cursor, dr, dc)
            {
                if cmd.starts_with("sel.") {
                    let current = inner.selection.cursor;
                    inner.selection.extend = ExtendMode::Extend;
                    inner.selection.move_by(
                        i64::from(next.row) - i64::from(current.row),
                        i64::from(next.col) - i64::from(current.col),
                    );
                } else {
                    inner.selection.replace(Area::cell(next));
                }
            }
        }
    }
    let cursor = inner.selection.cursor;
    inner.viewport.ensure_row_visible(cursor.row);
    inner.viewport.ensure_col_visible(cursor.col);
    Ok(())
}

fn apply_select(
    wb: &Workbook,
    selection: &mut crate::selection::Selection,
    args: &Value,
) -> Result<(), CoreError> {
    let range = args
        .get("range")
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::new("command.args", "view.select requires range"))?;
    let resolved = wb.resolve_parsed(omacell_core::addr::parse_a1(range)?)?;
    let (sheet, mut reference) = match resolved {
        omacell_core::addr::RefKind::Cell(cell) => (
            cell.sheet.unwrap_or_else(|| wb.active_sheet()),
            Area::cell(cell),
        ),
        omacell_core::addr::RefKind::Range(range) => {
            if range.sheet_end.is_some() {
                return Err(CoreError::new(
                    "ui.view",
                    "view.select does not support 3-D ranges",
                ));
            }
            (
                range.start.sheet.unwrap_or_else(|| wb.active_sheet()),
                Area {
                    start: range.start,
                    end: range.end,
                },
            )
        }
    };
    reference.start.sheet = Some(sheet);
    reference.end.sheet = Some(sheet);
    selection.sheet = sheet;
    selection.replace(reference);
    Ok(())
}

fn move_to_used_edge(wb: &Workbook, selection: &mut crate::selection::Selection, row: bool) {
    let used = wb
        .sheet(selection.sheet)
        .and_then(|sheet| sheet.used_range());
    let mut cursor = selection.cursor;
    if row {
        cursor.row = used.map_or(0, |range| range.max_row);
    } else {
        cursor.col = used.map_or(0, |range| range.max_col);
    }
    selection.replace(Area::cell(cursor));
}

fn scaled_coordinate(value: u64, zoom: f64) -> u32 {
    (value as f64 * zoom)
        .round()
        .clamp(0.0, f64::from(u32::MAX)) as u32
}

fn cell_input(wb: &Workbook, cell: omacell_core::addr::CellRef) -> Result<String, CoreError> {
    let sheet = cell.sheet.unwrap_or_else(|| wb.active_sheet());
    let Some(slot) = wb.get(sheet, cell.row, cell.col)? else {
        return Ok(String::new());
    };
    if let Some(formula) = slot.formula {
        return wb
            .intern()
            .formulas
            .get(formula)
            .map(str::to_string)
            .ok_or_else(|| CoreError::new("ui.edit", "cell formula handle is not interned"));
    }
    Ok(match slot.value {
        CellValue::Text(id) => wb
            .intern()
            .strings
            .get(id)
            .map(str::to_string)
            .ok_or_else(|| CoreError::new("ui.edit", "cell text handle is not interned"))?,
        value => value.to_string(),
    })
}

fn apply_zoom(vp: &mut Viewport, args: &Value) -> Result<(), CoreError> {
    let zoom = match (
        args.get("factor").and_then(Value::as_f64),
        args.get("delta").and_then(Value::as_f64),
    ) {
        (Some(factor), None) => factor,
        (None, Some(delta)) => vp.zoom + delta,
        _ => {
            return Err(CoreError::new(
                "ui.view",
                "view.zoom requires exactly one of factor or delta",
            ));
        }
    };
    if !zoom.is_finite() || zoom <= 0.0 {
        return Err(CoreError::new(
            "ui.view",
            "view.zoom must resolve to a finite positive factor",
        ));
    }
    vp.set_zoom(zoom);
    Ok(())
}

fn edge_delta(cmd: &str) -> Option<(i64, i64)> {
    match cmd {
        "nav.nextedge" | "sel.edgeright" => Some((0, 1)),
        "nav.prevedge" | "sel.edgeleft" => Some((0, -1)),
        "nav.edgedown" | "sel.edgedown" => Some((1, 0)),
        "nav.edgeup" | "sel.edgeup" => Some((-1, 0)),
        _ => None,
    }
}

fn snapshot_edge(
    wb: &Workbook,
    cursor: omacell_core::addr::CellRef,
    drow: i64,
    dcol: i64,
) -> Option<omacell_core::addr::CellRef> {
    let sheet = cursor.sheet.unwrap_or_else(|| wb.active_sheet());
    let worksheet = wb.sheet(sheet)?;
    let (current, maximum, mut occupied) = if drow != 0 {
        (
            cursor.row,
            MAX_ROWS - 1,
            worksheet
                .store
                .iter_col(cursor.col)
                .filter(|(_, slot)| cell_has_contents(*slot))
                .map(|(row, _)| row)
                .collect::<Vec<_>>(),
        )
    } else {
        (
            u32::from(cursor.col),
            u32::from(MAX_COLS - 1),
            worksheet
                .store
                .iter_row(cursor.row)
                .filter(|(_, slot)| cell_has_contents(*slot))
                .map(|(col, _)| u32::from(col))
                .collect::<Vec<_>>(),
        )
    };
    occupied.sort_unstable();
    let target = edge_index(&occupied, current, maximum, drow > 0 || dcol > 0);
    let mut cell = cursor;
    cell.sheet = Some(sheet);
    if drow != 0 {
        cell.row = target;
    } else {
        cell.col = u16::try_from(target).unwrap_or(MAX_COLS - 1);
    }
    Some(cell)
}

fn edge_index(occupied: &[u32], current: u32, maximum: u32, forward: bool) -> u32 {
    if forward {
        let adjacent = current.saturating_add(1).min(maximum);
        if occupied.binary_search(&current).is_ok()
            && let Ok(mut index) = occupied.binary_search(&adjacent)
        {
            let mut target = adjacent;
            while let Some(next) = occupied.get(index + 1)
                && *next == target.saturating_add(1)
            {
                target = *next;
                index += 1;
            }
            return target;
        }
        let index = occupied.partition_point(|position| *position <= current);
        occupied.get(index).copied().unwrap_or(maximum)
    } else {
        let adjacent = current.saturating_sub(1);
        if occupied.binary_search(&current).is_ok()
            && current > 0
            && let Ok(mut index) = occupied.binary_search(&adjacent)
        {
            let mut target = adjacent;
            while index > 0 && occupied[index - 1].saturating_add(1) == target {
                index -= 1;
                target = occupied[index];
            }
            return target;
        }
        let index = occupied.partition_point(|position| *position < current);
        index
            .checked_sub(1)
            .and_then(|previous| occupied.get(previous))
            .copied()
            .unwrap_or(0)
    }
}

fn cell_has_contents(slot: CellSlot) -> bool {
    slot.formula.is_some() || !matches!(slot.value, CellValue::Empty)
}
