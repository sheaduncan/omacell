//! Editing, point mode, and F4.

use omacell_core::addr::CellRef;
use omacell_core::locale::LocaleSeparators;
use omacell_ui::{EditState, EditSurface, canonicalize_entry};

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
fn f4_cycles_both_ends_without_destroying_a_range() {
    let mut edit = EditState::default();
    edit.begin(EditSurface::FormulaBar, cell(0, 0), "=A1:B2");
    edit.cursor = edit.buffer.len();
    edit.cycle_anchor().unwrap();
    assert_eq!(edit.buffer, "=$A$1:$B$2");
}

#[test]
fn f4_preserves_a_quoted_sheet_qualifier() {
    let mut edit = EditState::default();
    edit.begin(EditSurface::FormulaBar, cell(0, 0), "='Data Set'!A1");
    edit.cursor = edit.buffer.len();
    edit.cycle_anchor().unwrap();
    assert_eq!(edit.buffer, "='Data Set'!$A$1");
}

#[test]
fn reference_spans_are_indexed_0_to_7() {
    let mut edit = EditState::default();
    edit.begin(EditSurface::InCell, cell(0, 0), "=A1+B1");
    let spans = edit.reference_spans();
    assert!(spans.len() >= 2);
    assert!(spans.iter().all(|(_, _, i)| *i < 8));
}

#[test]
fn backspace_respects_utf8_boundaries() {
    let mut edit = EditState::default();
    edit.begin(EditSurface::InCell, cell(0, 0), "aé");
    edit.backspace();
    assert_eq!(edit.buffer, "a");
    assert_eq!(edit.cursor, 1);
}

#[test]
fn caret_navigation_respects_utf8_boundaries() {
    let mut edit = EditState::default();
    edit.begin(EditSurface::InCell, cell(0, 0), "aéb");
    edit.move_left();
    edit.move_left();
    edit.insert_char('X');
    assert_eq!(edit.buffer, "aXéb");
    edit.move_home();
    assert_eq!(edit.cursor, 0);
    edit.move_end();
    assert_eq!(edit.cursor, edit.buffer.len());

    edit.begin(EditSurface::FormulaBar, cell(0, 0), "aé\nxyz");
    edit.move_up();
    assert_eq!(edit.cursor, 3);
    edit.move_down();
    assert_eq!(edit.cursor, 6);
}

#[test]
fn localized_entry_becomes_canonical_without_touching_strings() {
    let de = LocaleSeparators {
        decimal: ',',
        thousands: '.',
        list: ';',
    };
    assert_eq!(canonicalize_entry("1.234,5", de), "1234.5");
    assert_eq!(canonicalize_entry("1.23,5", de), "1.23,5");
    assert_eq!(
        canonicalize_entry("=SUM(1,5;2,5;\"1,5\")", de),
        "=SUM(1.5,2.5,\"1,5\")"
    );
}
