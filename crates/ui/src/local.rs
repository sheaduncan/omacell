//! Session-local commands that can run against a reader snapshot.

use omacell_core::error::CoreError;
use omacell_core::workbook::Workbook;
use serde_json::Value;

use crate::selection::ExtendMode;
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
            | "view.zoom"
            | "view.center"
            | "view.freeze"
            | "view.split"
            | "view.formulabar"
            | "view.formulas"
            | "palette.open"
            | "help.keys"
            | "command.line"
            | "nav.goto"
            | "edit.find"
            | "changeset.review"
    )
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
    let mut sel = session.selection();
    let mut vp = session.viewport();
    match cmd {
        "nav.left" => sel.move_by(0, -n),
        "nav.right" => sel.move_by(0, n),
        "nav.up" => sel.move_by(-n, 0),
        "nav.down" => sel.move_by(n, 0),
        "nav.pagedown" => {
            let rows = i64::from(vp.page_rows().max(1));
            sel.move_by(rows * n, 0);
        }
        "nav.pageup" => {
            let rows = i64::from(vp.page_rows().max(1));
            sel.move_by(-rows * n, 0);
        }
        "nav.halfpagedown" => {
            let rows = i64::from((vp.page_rows() / 2).max(1));
            sel.move_by(rows * n, 0);
        }
        "nav.halfpageup" => {
            let rows = i64::from((vp.page_rows() / 2).max(1));
            sel.move_by(-rows * n, 0);
        }
        "nav.pageleft" => {
            let cols = i64::from(vp.page_cols().max(1));
            sel.move_by(0, -cols * n);
        }
        "nav.pageright" => {
            let cols = i64::from(vp.page_cols().max(1));
            sel.move_by(0, cols * n);
        }
        "nav.screentop" => {
            let (top, _, _) = vp.screen_rows();
            sel.move_by(i64::from(top) - i64::from(sel.cursor.row), 0);
        }
        "nav.screenmiddle" => {
            let (_, mid, _) = vp.screen_rows();
            sel.move_by(i64::from(mid) - i64::from(sel.cursor.row), 0);
        }
        "nav.screenbottom" => {
            let (_, _, bottom) = vp.screen_rows();
            sel.move_by(i64::from(bottom) - i64::from(sel.cursor.row), 0);
        }
        "nav.a1" => {
            sel.cursor.row = 0;
            sel.cursor.col = 0;
            sel.replace(crate::selection::Area::cell(sel.cursor));
        }
        "nav.firstcol" => {
            sel.cursor.col = 0;
            sel.replace(crate::selection::Area::cell(sel.cursor));
        }
        "nav.top" => {
            sel.cursor.row = 0;
            sel.replace(crate::selection::Area::cell(sel.cursor));
        }
        "sel.extendleft" => {
            sel.extend = ExtendMode::Extend;
            sel.move_by(0, -n);
        }
        "sel.extendright" => {
            sel.extend = ExtendMode::Extend;
            sel.move_by(0, n);
        }
        "sel.extendup" => {
            sel.extend = ExtendMode::Extend;
            sel.move_by(-n, 0);
        }
        "sel.extenddown" => {
            sel.extend = ExtendMode::Extend;
            sel.move_by(n, 0);
        }
        "sel.extendmode" => sel.extend = ExtendMode::Extend,
        "sel.addmode" => sel.extend = ExtendMode::Add,
        "sel.row" => sel.select_row(),
        "sel.col" => sel.select_col(),
        "view.center" => vp.center_on(sel.cursor.row, sel.cursor.col),
        "view.zoom" => apply_zoom(&mut vp, args),
        "palette.open" => {
            let mut palette = session.palette();
            palette.open();
            session.set_palette(palette);
            return Ok(());
        }
        "help.keys" | "nav.goto" | "edit.find" | "command.line" | "changeset.review" => {
            let mut panel = session.panel();
            let id = match cmd {
                "help.keys" => "keys",
                "nav.goto" => "goto",
                "command.line" => "command",
                "changeset.review" => "changeset",
                _ => "find",
            };
            panel.open(id);
            session.set_panel(panel);
            return Ok(());
        }
        "view.formulabar" | "view.formulas" | "view.freeze" | "view.split" => {
            // These need inner fields; fall back to no-op local if we cannot lock.
            return Ok(());
        }
        _ => {
            // edges / lastcol / bottom / visual — best-effort snapshot reads
            if let Some((dr, dc)) = edge_delta(cmd) {
                if let Some(next) = snapshot_edge(wb, sel.cursor, dr, dc) {
                    sel.cursor = next;
                    sel.replace(crate::selection::Area::cell(sel.cursor));
                }
            }
        }
    }
    vp.ensure_row_visible(sel.cursor.row);
    vp.ensure_col_visible(sel.cursor.col);
    session.set_selection(sel);
    session.set_viewport(vp);
    Ok(())
}

fn apply_zoom(vp: &mut Viewport, args: &Value) {
    if let Some(factor) = args.get("factor").and_then(Value::as_f64) {
        vp.set_zoom(factor);
    } else if let Some(delta) = args.get("delta").and_then(Value::as_f64) {
        vp.set_zoom(vp.zoom + delta);
    }
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
    let mut cell = cursor;
    cell.sheet = Some(sheet);
    if drow != 0 {
        let next = cursor.row.saturating_add_signed(drow.clamp(-1, 1) as i32);
        cell.row = next;
    } else {
        let next = i32::from(cursor.col).saturating_add(dcol.clamp(-1, 1) as i32);
        cell.col = u16::try_from(next.max(0)).unwrap_or(0);
    }
    Some(cell)
}
