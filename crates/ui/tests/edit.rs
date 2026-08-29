//! Editing, point mode, and F4.

use omacell_core::addr::CellRef;
use omacell_ui::{EditState, EditSurface};

fn cell(row: u32, col: u16) -> CellRef {
    CellRef {
        sheet: None,
        row,
        col,
        row_abs: false,
        col_abs: false,
    }
}

#[test]
fn point_mode_inserts_refs() {
    let mut edit = EditState::default();
    edit.begin(EditSurface::InCell, cell(0, 0), "=");
    assert!(edit.point);
    edit.insert_ref(cell(0, 1)).unwrap();
    assert_eq!(edit.buffer, "=B1");
}

#[test]
fn f4_cycles_excel_order() {
    let mut edit = EditState::default();
    edit.begin(EditSurface::FormulaBar, cell(0, 0), "=A1");
    edit.cursor = 3;
    edit.cycle_anchor().unwrap();
    assert_eq!(edit.buffer, "=$A$1");
    edit.cycle_anchor().unwrap();
    assert_eq!(edit.buffer, "=A$1");
    edit.cycle_anchor().unwrap();
    assert_eq!(edit.buffer, "=$A1");
    edit.cycle_anchor().unwrap();
    assert_eq!(edit.buffer, "=A1");
}

#[test]
fn reference_spans_are_indexed_0_to_7() {
    let mut edit = EditState::default();
    edit.begin(EditSurface::InCell, cell(0, 0), "=A1+B1");
    let spans = edit.reference_spans();
    assert!(spans.len() >= 2);
    assert!(spans.iter().all(|(_, _, i)| *i < 8));
}
