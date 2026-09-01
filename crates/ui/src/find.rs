//! Find / replace / go-to models (F-5.8).

use omacell_core::addr::{CellRef, SheetId};
use omacell_core::limits::{MAX_COLS, MAX_ROWS};

use crate::selection::Area;
use crate::session::UiSession;

/// Search scope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FindScope {
    /// Active sheet.
    #[default]
    Sheet,
    /// Whole workbook.
    Workbook,
}

/// Find/replace panel state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindReplace {
    /// Needle.
    pub find: String,
    /// Replacement.
    pub replace: String,
    /// Values vs formulas.
    pub in_formulas: bool,
    /// Whole cell.
    pub whole_cell: bool,
    /// Case sensitive.
    pub case: bool,
    /// Regex (extension).
    pub regex: bool,
    /// Scope.
    pub scope: FindScope,
    /// Last preview count.
    pub preview: u32,
}

/// Go To target.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GoTo {
    /// Address or name.
    pub target: String,
}

/// Restore the selected match from an `edit.searchnext` / `edit.searchprev` result.
///
/// Frontends call this after adopting the writer's workbook snapshot so a
/// cross-sheet transition cannot replace the matched cell with the sheet's
/// previously saved view selection.
pub fn apply_search_result(session: &UiSession, result: &serde_json::Value) -> bool {
    let Some(sheet) = result
        .get("sheet")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .map(SheetId::new)
    else {
        return false;
    };
    let Some(row) = result
        .get("row")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
    else {
        return false;
    };
    let Some(col) = result
        .get("col")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
    else {
        return false;
    };
    if row >= MAX_ROWS || col >= MAX_COLS {
        return false;
    }
    let cell = CellRef {
        sheet: Some(sheet),
        row,
        col,
        row_abs: false,
        col_abs: false,
    };
    let mut selection = session.selection();
    selection.replace(Area::cell(cell));
    session.set_selection(selection);
    let mut viewport = session.viewport();
    viewport.ensure_row_visible(row);
    viewport.ensure_col_visible(col);
    session.set_viewport(viewport);
    true
}
