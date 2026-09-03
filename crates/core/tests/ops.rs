//! Structural-edit, fill, paste-special, and protection corpora (WP-17).

use omacell_core::addr::{CellRef, RangeRef, RefKind, parse_a1, parse_a1_cell};
use omacell_core::intern::RichTextRun;
use omacell_core::ops::{
    FillMode, PasteOp, PasteSpecial, Shift, TextColumnType, TextToColumnsMode, TextToColumnsPlan,
    copy_range, delete_cells, delete_cols, delete_rows, detect_fill, excel_xor_hash, extend_fill,
    fill_custom_list, fill_range, formula_src, insert_cells, insert_cols, insert_rows, merge,
    merge_across, move_range_cells, move_range_cells_between, paste_special, remove_duplicates,
    text_to_columns, text_to_columns_with_plan, unmerge,
};
use omacell_core::sheet::Note;
use omacell_core::style::Font;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;

fn range(r0: u32, c0: u16, r1: u32, c1: u16) -> RangeRef {
    RangeRef::from_corners(CellRef::new(r0, c0).unwrap(), CellRef::new(r1, c1).unwrap())
}

fn cell(row: u32, col: u16) -> CellRef {
    CellRef::new(row, col).unwrap()
}

fn text(wb: &Workbook, row: u32, col: u16) -> String {
    let sheet = wb.active_sheet();
    let slot = wb.get(sheet, row, col).unwrap().unwrap();
    let Value::Text(id) = slot.value else {
        panic!("expected text cell");
    };
    wb.intern().strings.get(id).unwrap_or_default().to_string()
}

fn a1_range(value: &str) -> RangeRef {
    match parse_a1(value).unwrap().kind {
        RefKind::Cell(cell) => RangeRef::from_corners(cell, cell),
        RefKind::Range(range) => range,
    }
}

#[test]
fn structural_formula_corpus_matches_documented_excel_rules() {
    let corpus = include_str!("../../../tests/corpus/ops/structure.tsv");
    for line in corpus
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 8, "malformed corpus row: {line}");
        let [
            case,
            operation,
            formula_cell,
            before,
            op_range,
            shift_or_dest,
            expected,
            _rule,
        ] = fields.as_slice()
        else {
            unreachable!()
        };
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        let formula_cell = parse_a1_cell(formula_cell).unwrap();
        wb.set_cell_contents(sheet, formula_cell.row, formula_cell.col, before)
            .unwrap();
        let target = a1_range(op_range);
        match (*operation, *shift_or_dest) {
            ("insert", "rows") => {
                insert_rows(
                    &mut wb,
                    sheet,
                    target.start.row,
                    target.end.row - target.start.row + 1,
                )
                .unwrap();
            }
            ("delete", "rows") => {
                delete_rows(
                    &mut wb,
                    sheet,
                    target.start.row,
                    target.end.row - target.start.row + 1,
                )
                .unwrap();
            }
            ("insert", "cols") => {
                insert_cols(
                    &mut wb,
                    sheet,
                    target.start.col,
                    target.end.col - target.start.col + 1,
                )
                .unwrap();
            }
            ("delete", "cols") => {
                delete_cols(
                    &mut wb,
                    sheet,
                    target.start.col,
                    target.end.col - target.start.col + 1,
                )
                .unwrap();
            }
            ("insert", "down") => {
                insert_cells(&mut wb, sheet, target, Shift::Down).unwrap();
            }
            ("insert", "right") => {
                insert_cells(&mut wb, sheet, target, Shift::Right).unwrap();
            }
            ("delete", "up") => {
                delete_cells(&mut wb, sheet, target, Shift::Down).unwrap();
            }
            ("delete", "left") => {
                delete_cells(&mut wb, sheet, target, Shift::Right).unwrap();
            }
            ("move", destination) => {
                move_range_cells(&mut wb, sheet, target, parse_a1_cell(destination).unwrap())
                    .unwrap();
            }
            other => panic!("{case}: unsupported corpus operation {other:?}"),
        }
        assert_eq!(
            formula_src(&wb, sheet, formula_cell.row, formula_cell.col),
            *expected,
            "{case}: {}",
            fields[7]
        );
    }
}

fn number_matrix(value: &str) -> Vec<Vec<Option<f64>>> {
    value
        .split(';')
        .map(|row| {
            row.split(',')
                .map(|cell| {
                    if cell == "blank" {
                        None
                    } else {
                        Some(cell.parse::<f64>().unwrap())
                    }
                })
                .collect()
        })
        .collect()
}

#[test]
fn fill_and_paste_special_matrix_corpus_passes() {
    let corpus = include_str!("../../../tests/corpus/ops/fill-paste.tsv");
    for line in corpus
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 6, "malformed corpus row: {line}");
        let case = fields[0];
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        let destination = a1_range(fields[3]);
        if fields[1] == "fill" {
            let seed = number_matrix(fields[2]).remove(0);
            let upward = case.ends_with("_up");
            let start = if upward {
                destination.end.row + 1 - seed.len() as u32
            } else {
                destination.start.row
            };
            for (offset, value) in seed.iter().enumerate() {
                wb.set_number(
                    sheet,
                    start + offset as u32,
                    destination.start.col,
                    value.unwrap(),
                )
                .unwrap();
            }
            let source = range(
                start,
                destination.start.col,
                start + seed.len() as u32 - 1,
                destination.start.col,
            );
            let mode = match fields[4] {
                "linear" => FillMode::Linear,
                "growth" => FillMode::Growth,
                "weekday" => FillMode::Weekday,
                other => panic!("{case}: unknown fill mode {other}"),
            };
            fill_range(&mut wb, sheet, source, destination, mode).unwrap();
        } else {
            let source_values = number_matrix(fields[2]);
            for (row, values) in source_values.iter().enumerate() {
                for (col, value) in values.iter().enumerate() {
                    if let Some(value) = value {
                        wb.set_number(sheet, 20 + row as u32, 20 + col as u16, *value)
                            .unwrap();
                    }
                }
            }
            if fields[4] != "transpose" {
                wb.set_number(sheet, destination.start.row, destination.start.col, 3.0)
                    .unwrap();
            }
            let source = copy_range(
                &wb,
                sheet,
                range(
                    20,
                    20,
                    20 + source_values.len() as u32 - 1,
                    20 + source_values.iter().map(Vec::len).max().unwrap_or(1) as u16 - 1,
                ),
            );
            let special = match fields[4] {
                "add" => PasteSpecial {
                    operation: PasteOp::Add,
                    ..PasteSpecial::default()
                },
                "subtract" => PasteSpecial {
                    operation: PasteOp::Sub,
                    ..PasteSpecial::default()
                },
                "multiply" => PasteSpecial {
                    operation: PasteOp::Mul,
                    ..PasteSpecial::default()
                },
                "divide" => PasteSpecial {
                    operation: PasteOp::Div,
                    ..PasteSpecial::default()
                },
                "skip_blanks" => PasteSpecial {
                    values: true,
                    skip_blanks: true,
                    ..PasteSpecial::default()
                },
                "transpose" => PasteSpecial {
                    values: true,
                    transpose: true,
                    ..PasteSpecial::default()
                },
                other => panic!("{case}: unknown paste mode {other}"),
            };
            paste_special(&mut wb, sheet, destination.start, &source, special, None).unwrap();
        }
        let expected_matrix = if fields[1] == "fill" {
            fields[5]
                .split(',')
                .map(|value| vec![Some(value.parse::<f64>().unwrap())])
                .collect()
        } else {
            number_matrix(fields[5])
        };
        for (row, expected_row) in expected_matrix.iter().enumerate() {
            for (col, expected) in expected_row.iter().enumerate() {
                let actual = wb
                    .get(
                        sheet,
                        destination.start.row + row as u32,
                        destination.start.col + col as u16,
                    )
                    .unwrap()
                    .map(|slot| slot.value);
                match expected {
                    Some(expected) => assert_eq!(
                        actual,
                        Some(Value::Number(*expected)),
                        "{case} at ({row},{col})"
                    ),
                    None => assert!(actual.is_none(), "{case} at ({row},{col})"),
                }
            }
        }
    }
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

#[test]
fn band_insert_shifts_only_metadata_inside_the_band() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let inside = Note {
        author: Some("Ada".into()),
        text: "inside".into(),
    };
    let outside = Note {
        author: Some("Lin".into()),
        text: "outside".into(),
    };
    wb.set_note(sheet, 4, 4, Some(inside.clone())).unwrap();
    wb.set_note(sheet, 2, 7, Some(outside.clone())).unwrap();
    insert_cells(&mut wb, sheet, range(4, 4, 4, 4), Shift::Down).unwrap();
    let sheet_ref = wb.sheet(sheet).unwrap();
    assert_eq!(sheet_ref.notes.get(&(5, 4)), Some(&inside));
    assert_eq!(sheet_ref.notes.get(&(2, 7)), Some(&outside));
    assert!(!sheet_ref.notes.contains_key(&(4, 4)));
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
fn fill_copy_works_up_and_left_with_formula_deltas() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_cell_contents(s, 2, 2, "=D3").unwrap();
    fill_range(
        &mut wb,
        s,
        range(2, 2, 2, 2),
        range(0, 2, 2, 2),
        FillMode::Copy,
    )
    .unwrap();
    assert_eq!(formula_src(&wb, s, 0, 2), "=D1");

    fill_range(
        &mut wb,
        s,
        range(0, 2, 0, 2),
        range(0, 0, 0, 2),
        FillMode::Copy,
    )
    .unwrap();
    assert_eq!(formula_src(&wb, s, 0, 0), "=B1");
}

#[test]
fn fill_linear_series_extends_backwards() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 2, 0, 3.0).unwrap();
    wb.set_number(s, 3, 0, 5.0).unwrap();
    fill_range(
        &mut wb,
        s,
        range(2, 0, 3, 0),
        range(0, 0, 3, 0),
        FillMode::Linear,
    )
    .unwrap();
    assert_eq!(wb.get(s, 1, 0).unwrap().unwrap().value, Value::Number(1.0));
    assert_eq!(wb.get(s, 0, 0).unwrap().unwrap().value, Value::Number(-1.0));
}

#[test]
fn fill_custom_list_wraps_in_both_directions() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 1, 0, "Tue").unwrap();
    let list = ["Mon".to_string(), "Tue".to_string(), "Wed".to_string()];
    fill_custom_list(&mut wb, s, range(1, 0, 1, 0), range(0, 0, 3, 0), &list).unwrap();
    let text = |wb: &Workbook, row| {
        let slot = wb.get(s, row, 0).unwrap().unwrap();
        let Value::Text(id) = slot.value else {
            panic!("expected text")
        };
        wb.intern().strings.get(id).unwrap().to_string()
    };
    assert_eq!(text(&wb, 0), "Mon");
    assert_eq!(text(&wb, 2), "Wed");
    assert_eq!(text(&wb, 3), "Mon");
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
fn ordinary_internal_paste_preserves_rich_text_runs() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let font = Font {
        bold: true,
        ..Font::default()
    };
    wb.set_rich_text(
        sheet,
        0,
        0,
        "Bold plain",
        vec![RichTextRun {
            start: 0,
            len: 4,
            font: font.clone(),
        }],
    )
    .unwrap();
    let grid = copy_range(&wb, sheet, range(0, 0, 0, 0));
    paste_special(
        &mut wb,
        sheet,
        cell(0, 1),
        &grid,
        PasteSpecial::default(),
        Some((0, 0)),
    )
    .unwrap();
    let Value::Text(id) = wb.get(sheet, 0, 1).unwrap().unwrap().value else {
        panic!("expected rich text")
    };
    assert_eq!(wb.intern().strings.get_rich(id).unwrap()[0].font, font);
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

#[test]
fn overlapping_move_retains_unique_text_when_undo_is_disabled() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_text(s, 0, 0, "alpha").unwrap();
    wb.set_text(s, 1, 0, "bravo").unwrap();

    move_range_cells(&mut wb, s, range(0, 0, 1, 0), cell(1, 0)).unwrap();

    assert!(wb.get(s, 0, 0).unwrap().is_none());
    assert_eq!(text(&wb, 1, 0), "alpha");
    assert_eq!(text(&wb, 2, 0), "bravo");
}

/// Published Excel XOR worksheet-protection hash vectors
/// (OpenOffice / ECMA-376 legacy algorithm).
#[test]
fn protection_hash_matches_known_vectors() {
    let corpus = include_str!("../../../tests/corpus/ops/protection_hash.tsv");
    for (line_number, line) in corpus.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            3,
            "protection hash corpus line {} must have three fields",
            line_number + 1
        );
        let expected = u16::from_str_radix(fields[1], 16).unwrap_or_else(|error| {
            panic!(
                "invalid expected hash on protection hash corpus line {}: {error}",
                line_number + 1
            )
        });
        assert_eq!(
            excel_xor_hash(fields[0]),
            expected,
            "protection hash corpus line {}: {}",
            line_number + 1,
            fields[2]
        );
    }
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
fn insert_cells_retains_unique_text_when_undo_is_disabled() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_text(s, 0, 0, "alpha").unwrap();
    wb.set_text(s, 1, 0, "bravo").unwrap();

    insert_cells(&mut wb, s, range(0, 0, 0, 0), Shift::Down).unwrap();

    assert!(wb.get(s, 0, 0).unwrap().is_none());
    assert_eq!(text(&wb, 1, 0), "alpha");
    assert_eq!(text(&wb, 2, 0), "bravo");
}

#[test]
fn insert_cells_rewrites_only_references_in_shifted_band() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_cell_contents(s, 0, 3, "=B3+C3").unwrap();
    insert_cells(&mut wb, s, range(1, 1, 1, 1), Shift::Down).unwrap();
    assert_eq!(formula_src(&wb, s, 0, 3), "=B4+C3");
}

#[test]
fn paste_formula_uses_source_to_destination_delta() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_cell_contents(s, 0, 0, "=B1").unwrap();
    let grid = copy_range(&wb, s, range(0, 0, 0, 0));
    paste_special(
        &mut wb,
        s,
        cell(2, 2),
        &grid,
        PasteSpecial {
            formulas: true,
            ..PasteSpecial::default()
        },
        Some((0, 0)),
    )
    .unwrap();
    assert_eq!(formula_src(&wb, s, 2, 2), "=D3");
}

#[test]
fn move_retargets_formulas_outside_the_moved_range() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_cell_contents(s, 1, 0, "=A1").unwrap();
    wb.set_cell_contents(s, 0, 3, "=A1").unwrap();
    move_range_cells(&mut wb, s, range(0, 0, 1, 0), cell(0, 1)).unwrap();
    assert_eq!(formula_src(&wb, s, 1, 1), "=B1");
    assert_eq!(formula_src(&wb, s, 0, 3), "=B1");
}

#[test]
fn cross_sheet_move_retargets_local_references_but_not_external_workbooks() {
    let mut wb = Workbook::new();
    let source = wb.active_sheet();
    let target = wb.add_sheet("Target").unwrap();
    wb.set_number(source, 0, 0, 1.0).unwrap();
    wb.set_number(source, 0, 1, 2.0).unwrap();
    wb.set_cell_contents(source, 1, 0, "=B1").unwrap();
    wb.set_cell_contents(target, 0, 3, "=Sheet1!A1").unwrap();
    wb.set_cell_contents(target, 0, 4, "=[Book.xlsx]Sheet1!A1")
        .unwrap();
    move_range_cells_between(&mut wb, source, range(0, 0, 1, 0), target, cell(2, 2)).unwrap();
    assert_eq!(formula_src(&wb, target, 0, 3), "=C3");
    assert_eq!(formula_src(&wb, target, 0, 4), "=[Book.xlsx]Sheet1!A1");
    assert_eq!(formula_src(&wb, target, 3, 2), "=Sheet1!B1");
}

#[test]
fn remove_duplicates_compacts_kept_rows() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "a").unwrap();
    wb.set_text(s, 1, 0, "a").unwrap();
    wb.set_text(s, 2, 0, "b").unwrap();
    assert_eq!(
        remove_duplicates(&mut wb, s, range(0, 0, 2, 0), &[]).unwrap(),
        1
    );
    let moved = wb.get(s, 1, 0).unwrap().unwrap();
    let Value::Text(id) = moved.value else {
        panic!("expected compacted text row");
    };
    assert_eq!(wb.intern().strings.get(id), Some("b"));
    assert!(wb.get(s, 2, 0).unwrap().is_none());
}

#[test]
fn remove_duplicates_compacts_side_records_with_their_rows() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    for (row, value) in [(0, "a"), (1, "a"), (2, "b")] {
        wb.set_text(sheet, row, 0, value).unwrap();
    }
    let note = Note {
        author: None,
        text: "kept row".into(),
    };
    wb.set_note(sheet, 2, 0, Some(note.clone())).unwrap();
    remove_duplicates(&mut wb, sheet, range(0, 0, 2, 0), &[]).unwrap();
    assert_eq!(wb.sheet(sheet).unwrap().notes.get(&(1, 0)), Some(&note));
    assert!(!wb.sheet(sheet).unwrap().notes.contains_key(&(2, 0)));
}

#[test]
fn text_to_columns_preserves_field_whitespace() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "a, b ").unwrap();
    text_to_columns(&mut wb, s, range(0, 0, 0, 0), ',').unwrap();
    let value = wb.get(s, 0, 1).unwrap().unwrap();
    let Value::Text(id) = value.value else {
        panic!("expected text");
    };
    assert_eq!(wb.intern().strings.get(id), Some(" b "));
}

#[test]
fn text_to_columns_supports_fixed_width_and_column_types() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "00742skip").unwrap();
    text_to_columns_with_plan(
        &mut wb,
        s,
        range(0, 0, 0, 0),
        &TextToColumnsPlan {
            mode: TextToColumnsMode::Fixed { breaks: vec![3, 5] },
            columns: vec![
                TextColumnType::Text,
                TextColumnType::General,
                TextColumnType::Skip,
            ],
        },
    )
    .unwrap();
    let first = wb.get(s, 0, 0).unwrap().unwrap();
    let Value::Text(id) = first.value else {
        panic!("expected text")
    };
    assert_eq!(wb.intern().strings.get(id), Some("007"));
    assert_eq!(wb.get(s, 0, 1).unwrap().unwrap().value, Value::Number(42.0));
    assert!(wb.get(s, 0, 2).unwrap().is_none());
}

#[test]
fn merge_rejects_overlap() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    merge(&mut wb, s, range(0, 0, 0, 1)).unwrap();
    assert!(merge(&mut wb, s, range(0, 1, 0, 2)).is_err());
}

#[test]
fn content_and_structural_edits_do_not_partially_mutate_fixed_cse_ranges() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    let cse = range(1, 1, 2, 2);
    wb.set_array_formula_text(s, cse, "={1,2;3,4}").unwrap();
    wb.set_number(s, 0, 3, 7.0).unwrap();
    wb.set_number(s, 0, 4, 8.0).unwrap();
    let source = copy_range(&wb, s, range(0, 3, 0, 4));

    let err = paste_special(
        &mut wb,
        s,
        cell(1, 0),
        &source,
        PasteSpecial::default(),
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, "formula.array");
    assert!(wb.get(s, 1, 0).unwrap().is_none());

    let err = insert_rows(&mut wb, s, 0, 1).unwrap_err();
    assert_eq!(err.code, "formula.array");
    assert_eq!(
        wb.sheet(s).unwrap().array_formula_at(2, 2).unwrap().range,
        cse
    );
    insert_rows(&mut wb, s, 3, 1).unwrap();
}

#[test]
fn copy_and_fill_do_not_silently_convert_fixed_cse_formulas() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let cse = range(0, 0, 0, 1);
    wb.set_array_formula_text(sheet, cse, "={1,2}").unwrap();
    let grid = copy_range(&wb, sheet, cse);

    let paste = paste_special(
        &mut wb,
        sheet,
        cell(2, 0),
        &grid,
        PasteSpecial::default(),
        Some((0, 0)),
    )
    .unwrap_err();
    assert_eq!(paste.code, "formula.array");
    assert!(wb.get(sheet, 2, 0).unwrap().is_none());

    let fill = fill_range(&mut wb, sheet, cse, range(0, 0, 1, 1), FillMode::Copy).unwrap_err();
    assert_eq!(fill.code, "formula.array");
    assert!(wb.get(sheet, 1, 0).unwrap().is_none());
}
