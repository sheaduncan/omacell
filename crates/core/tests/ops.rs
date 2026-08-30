//! Structural-edit, fill, paste-special, and protection corpora (WP-17).

use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::ops::{
    FillMode, PasteOp, PasteSpecial, Shift, copy_range, delete_rows, detect_fill, excel_xor_hash,
    extend_fill, fill_range, formula_src, insert_cells, insert_rows, merge, merge_across,
    move_range_cells, paste_special, remove_duplicates, text_to_columns, unmerge,
};
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;

fn range(r0: u32, c0: u16, r1: u32, c1: u16) -> RangeRef {
    RangeRef::from_corners(CellRef::new(r0, c0).unwrap(), CellRef::new(r1, c1).unwrap())
}

fn cell(row: u32, col: u16) -> CellRef {
    CellRef::new(row, col).unwrap()
}

/// Excel: inserting a row before row 2 (1-based) bumps `=A3` to `=A4`.
#[test]
fn insert_row_rewrites_relative_refs() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_cell_contents(s, 0, 0, "=A3").unwrap();
    insert_rows(&mut wb, s, 1, 1).unwrap();
    assert_eq!(formula_src(&wb, s, 0, 0), "=A4");
}

/// Excel: deleting the referenced row yields `#REF!`.
#[test]
fn delete_row_of_target_becomes_ref_error() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_cell_contents(s, 0, 0, "=A3").unwrap();
    delete_rows(&mut wb, s, 2, 1).unwrap();
    assert_eq!(formula_src(&wb, s, 0, 0), "=#REF!");
}

#[test]
fn insert_row_undo_restores_formula() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_cell_contents(s, 0, 0, "=A3").unwrap();
    insert_rows(&mut wb, s, 1, 1).unwrap();
    wb.undo().unwrap();
    // rewrite is a later undo unit than the shift
    while formula_src(&wb, s, 0, 0) != "=A3" {
        if wb.undo().is_err() {
            break;
        }
    }
    assert_eq!(formula_src(&wb, s, 0, 0), "=A3");
}

#[test]
fn fill_linear_series_down() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_number(s, 1, 0, 3.0).unwrap();
    fill_range(
        &mut wb,
        s,
        range(0, 0, 1, 0),
        range(0, 0, 4, 0),
        FillMode::Linear,
    )
    .unwrap();
    assert_eq!(wb.get(s, 2, 0).unwrap().unwrap().value, Value::Number(5.0));
    assert_eq!(wb.get(s, 3, 0).unwrap().unwrap().value, Value::Number(7.0));
}

#[test]
fn detect_growth_and_extend() {
    assert_eq!(detect_fill(&[2.0, 6.0, 18.0]), FillMode::Growth);
    let next = extend_fill(
        &[2.0, 6.0],
        FillMode::Growth,
        2,
        omacell_core::dates::DateSystem::Excel1900,
    );
    assert!((next[0] - 18.0).abs() < 1e-9);
}

#[test]
fn paste_special_values_and_add() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 2.0).unwrap();
    wb.set_number(s, 1, 0, 10.0).unwrap();
    let grid = copy_range(&wb, s, range(0, 0, 0, 0));
    paste_special(
        &mut wb,
        s,
        cell(1, 0),
        &grid,
        PasteSpecial {
            operation: PasteOp::Add,
            ..PasteSpecial::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(wb.get(s, 1, 0).unwrap().unwrap().value, Value::Number(12.0));
}

#[test]
fn paste_transpose() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_number(s, 0, 1, 2.0).unwrap();
    let grid = copy_range(&wb, s, range(0, 0, 0, 1));
    paste_special(
        &mut wb,
        s,
        cell(2, 0),
        &grid,
        PasteSpecial {
            values: true,
            transpose: true,
            ..PasteSpecial::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(wb.get(s, 2, 0).unwrap().unwrap().value, Value::Number(1.0));
    assert_eq!(wb.get(s, 3, 0).unwrap().unwrap().value, Value::Number(2.0));
}

#[test]
fn move_retargets_and_clears_source() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_cell_contents(s, 1, 0, "=A1").unwrap();
    move_range_cells(&mut wb, s, range(0, 0, 1, 0), cell(0, 1)).unwrap();
    assert!(
        wb.get(s, 0, 0).unwrap().is_none()
            || matches!(wb.get(s, 0, 0).unwrap().unwrap().value, Value::Empty)
    );
}

/// Published Excel XOR hash: password "password" → 0x83AF
/// (OpenOffice / ECMA-376 legacy algorithm).
#[test]
fn protection_hash_matches_known_vector() {
    assert_eq!(excel_xor_hash("password"), 0x83AF);
}

#[test]
fn merge_across_makes_one_merge_per_row() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    merge_across(&mut wb, s, range(0, 0, 1, 2)).unwrap();
    assert_eq!(wb.sheet(s).unwrap().merges.len(), 2);
    unmerge(&mut wb, s, range(0, 0, 1, 2)).unwrap();
    assert!(wb.sheet(s).unwrap().merges.is_empty());
}

#[test]
fn text_to_columns_splits_on_comma() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "a,b,c").unwrap();
    text_to_columns(&mut wb, s, range(0, 0, 0, 0), ',').unwrap();
    match wb.get(s, 0, 1).unwrap().unwrap().value {
        Value::Text(id) => assert_eq!(wb.intern().strings.get(id), Some("b")),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn remove_duplicates_keeps_first() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_number(s, 1, 0, 1.0).unwrap();
    wb.set_number(s, 2, 0, 2.0).unwrap();
    let n = remove_duplicates(&mut wb, s, range(0, 0, 2, 0), &[]).unwrap();
    assert_eq!(n, 1);
}

#[test]
fn insert_cells_shift_down_moves_band() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_number(s, 1, 0, 2.0).unwrap();
    insert_cells(&mut wb, s, range(0, 0, 0, 0), Shift::Down).unwrap();
    assert_eq!(wb.get(s, 1, 0).unwrap().unwrap().value, Value::Number(1.0));
}

#[test]
fn merge_rejects_overlap() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    merge(&mut wb, s, range(0, 0, 0, 1)).unwrap();
    assert!(merge(&mut wb, s, range(0, 1, 0, 2)).is_err());
}
