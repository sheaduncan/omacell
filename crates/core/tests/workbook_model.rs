//! Sheet naming, defined names, undo/redo, snapshot, geometry (WP-02).

use std::path::PathBuf;

use omacell_core::addr::{SheetId, parse_a1};
use omacell_core::error::codes;
use omacell_core::geometry::AxisGeometry;
use omacell_core::names::{DefinedName, NameReferent, NameScope, validate_defined_name};
use omacell_core::sheet::{SheetVisibility, validate_sheet_name};
use omacell_core::storage::CellSlot;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, FileFailurePersistence};

fn corpus(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(rel)
}

fn read_tsv(path: &std::path::Path) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|line| {
            let t = line.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .map(|line| line.split('\t').map(ToOwned::to_owned).collect())
        .collect()
}

#[test]
fn sheet_name_corpus() {
    let rows = read_tsv(&corpus("workbook/sheet_names.tsv"));
    assert!(!rows.is_empty());
    for row in rows {
        assert!(row.len() >= 4, "{row:?}");
        let name = &row[0];
        let ok = row[1] == "true";
        let code = &row[2];
        let got = validate_sheet_name(name);
        assert_eq!(got.is_ok(), ok, "name={name:?} note={}", row[3]);
        if !ok {
            assert_eq!(got.unwrap_err().code, *code, "{name:?}");
        }
    }
}

#[test]
fn defined_name_corpus() {
    let rows = read_tsv(&corpus("workbook/defined_names.tsv"));
    assert!(!rows.is_empty());
    for row in rows {
        assert!(row.len() >= 4, "{row:?}");
        let name = &row[0];
        let ok = row[1] == "true";
        let code = &row[2];
        let got = validate_defined_name(name);
        assert_eq!(got.is_ok(), ok, "name={name:?} note={}", row[3]);
        if !ok {
            assert_eq!(got.unwrap_err().code, *code, "{name:?}");
        }
    }
}

#[test]
fn sheet_duplicate_and_visibility_rules() {
    let mut wb = Workbook::new();
    let a = wb.active_sheet();
    assert_eq!(wb.add_sheet("sheet1").unwrap_err().code, codes::SHEET_NAME);
    let b = wb.add_sheet("Data").unwrap();
    wb.set_visibility(b, SheetVisibility::Hidden).unwrap();
    assert_eq!(
        wb.set_visibility(a, SheetVisibility::Hidden)
            .unwrap_err()
            .code,
        codes::SHEET_NAME
    );
    assert_eq!(wb.remove_sheet(a).unwrap_err().code, codes::SHEET_NAME);
    wb.set_visibility(b, SheetVisibility::Visible).unwrap();
    wb.remove_sheet(b).unwrap();
    assert_eq!(wb.remove_sheet(a).unwrap_err().code, codes::SHEET_NAME);
}

#[test]
fn resolve_sheet_spec() {
    let mut wb = Workbook::new();
    let s2 = wb.add_sheet("Data").unwrap();
    let parsed = parse_a1("Data!B2").unwrap();
    let kind = wb.resolve_parsed(parsed).unwrap();
    match kind {
        omacell_core::addr::RefKind::Cell(c) => {
            assert_eq!(c.sheet, Some(s2));
            assert_eq!((c.row, c.col), (1, 1));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn snapshot_isolated_from_writer() {
    let mut wb = Workbook::new();
    let id = wb.active_sheet();
    wb.set_number(id, 0, 0, 1.0).unwrap();
    let snap = wb.snapshot();
    wb.set_number(id, 0, 0, 2.0).unwrap();
    wb.set_number(id, 1, 0, 3.0).unwrap();
    assert_eq!(
        snap.get(id, 0, 0).unwrap().unwrap().value,
        Value::Number(1.0)
    );
    assert!(snap.get(id, 1, 0).unwrap().is_none());
}

#[test]
fn undo_redo_restores_numbers() {
    let mut wb = Workbook::new();
    let id = wb.active_sheet();
    wb.transact(|wb| {
        wb.set_number(id, 0, 0, 1.0).unwrap();
        wb.set_number(id, 0, 1, 2.0).unwrap();
    });
    wb.set_number(id, 0, 0, 9.0).unwrap();
    wb.undo().unwrap();
    assert_eq!(wb.get(id, 0, 0).unwrap().unwrap().value, Value::Number(1.0));
    assert_eq!(wb.get(id, 0, 1).unwrap().unwrap().value, Value::Number(2.0));
    wb.undo().unwrap();
    assert!(wb.get(id, 0, 0).unwrap().is_none());
    wb.redo().unwrap();
    assert_eq!(wb.get(id, 0, 0).unwrap().unwrap().value, Value::Number(1.0));
    wb.redo().unwrap();
    assert_eq!(wb.get(id, 0, 0).unwrap().unwrap().value, Value::Number(9.0));
}

#[test]
fn undo_redo_restores_structurally_deleted_cells() {
    let mut wb = Workbook::new();
    let id = wb.active_sheet();
    for (row, value) in [(0, 10.0), (1, 20.0), (2, 30.0), (3, 40.0)] {
        wb.set_number(id, row, 0, value).unwrap();
    }

    wb.delete_rows(id, 1, 2).unwrap();
    assert_eq!(
        wb.get(id, 1, 0).unwrap().unwrap().value,
        Value::Number(40.0)
    );
    wb.undo().unwrap();
    for (row, value) in [(0, 10.0), (1, 20.0), (2, 30.0), (3, 40.0)] {
        assert_eq!(
            wb.get(id, row, 0).unwrap().unwrap().value,
            Value::Number(value)
        );
    }
    wb.redo().unwrap();
    assert_eq!(
        wb.get(id, 1, 0).unwrap().unwrap().value,
        Value::Number(40.0)
    );
    assert!(wb.get(id, 2, 0).unwrap().is_none());

    let mut wb = Workbook::new();
    let id = wb.active_sheet();
    for (col, value) in [(0, 10.0), (1, 20.0), (2, 30.0), (3, 40.0)] {
        wb.set_number(id, 0, col, value).unwrap();
    }
    wb.delete_cols(id, 1, 2).unwrap();
    wb.undo().unwrap();
    for (col, value) in [(0, 10.0), (1, 20.0), (2, 30.0), (3, 40.0)] {
        assert_eq!(
            wb.get(id, 0, col).unwrap().unwrap().value,
            Value::Number(value)
        );
    }
}

#[test]
fn transact_try_rolls_back_partial_mutation() {
    let mut wb = Workbook::new();
    let id = wb.active_sheet();
    wb.set_number(id, 0, 0, 1.0).unwrap();
    let err = wb
        .transact_try(|wb| {
            wb.set_number(id, 0, 1, 2.0)?;
            Err::<(), _>(omacell_core::error::CoreError::new("test.fail", "boom"))
        })
        .unwrap_err();
    assert_eq!(err.code, "test.fail");
    assert_eq!(wb.get(id, 0, 0).unwrap().unwrap().value, Value::Number(1.0));
    assert!(wb.get(id, 0, 1).unwrap().is_none());
}

#[test]
fn side_tables_reject_out_of_grid_cells() {
    let mut wb = Workbook::new();
    let id = wb.active_sheet();
    assert!(
        wb.set_note(
            id,
            omacell_core::limits::MAX_ROWS,
            0,
            Some(omacell_core::sheet::Note {
                author: None,
                text: "invalid".into(),
            }),
        )
        .is_err()
    );
}

#[test]
fn define_name_and_table() {
    let mut wb = Workbook::new();
    let id = wb.active_sheet();
    wb.define_name(DefinedName {
        name: "TaxRate".into(),
        scope: NameScope::Workbook,
        referent: NameReferent::Constant(Value::Number(0.2)),
        comment: None,
    })
    .unwrap();
    assert!(wb.names().get(NameScope::Workbook, "taxrate").is_some());
    let tid = wb
        .add_table(omacell_core::tables::Table::new(
            omacell_core::tables::TableId::new(0),
            "Sales",
            id,
            0,
            0,
            4,
            1,
        ))
        .unwrap();
    assert_eq!(wb.tables().get(tid).unwrap().name, "Sales");
}

#[derive(Clone, Debug)]
enum Model {
    Empty,
    Slots(Vec<(u32, u16, f64)>),
}

fn capture(wb: &Workbook, id: SheetId) -> Model {
    let sheet = wb.sheet(id).unwrap();
    let mut cells: Vec<_> = sheet
        .store
        .iter()
        .map(|(r, c, s)| {
            let n = match s.value {
                Value::Number(n) => n,
                _ => 0.0,
            };
            (r, c, n)
        })
        .collect();
    cells.sort_by_key(|t| (t.0, t.1));
    if cells.is_empty() {
        Model::Empty
    } else {
        Model::Slots(cells)
    }
}

fn eq_model(a: &Model, b: &Model) -> bool {
    match (a, b) {
        (Model::Empty, Model::Empty) => true,
        (Model::Slots(x), Model::Slots(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y.iter())
                    .all(|(l, r)| l.0 == r.0 && l.1 == r.1 && l.2.to_bits() == r.2.to_bits())
        }
        _ => false,
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        cases: 128,
        ..ProptestConfig::default()
    })]

    #[test]
    fn undo_redo_property(ops in prop::collection::vec((0u32..8, 0u16..8, -50i32..50i32, 0u8..3), 1..16)) {
        let mut wb = Workbook::new();
        let id = wb.active_sheet();
        let start = capture(&wb, id);
        let mut checkpoints = vec![start];
        for (row, col, n, kind) in &ops {
            match kind {
                0 => {
                    wb.set_number(id, *row, *col, f64::from(*n)).unwrap();
                }
                1 => {
                    let _ = wb.clear_cell(id, *row, *col).unwrap();
                }
                _ => {
                    wb.transact(|wb| {
                        wb.set_number(id, *row, *col, f64::from(*n)).unwrap();
                        let _ = wb.set_number(id, (*row + 1) % 8, *col, f64::from(*n) + 1.0);
                    });
                }
            }
            checkpoints.push(capture(&wb, id));
        }
        let end = checkpoints.last().cloned().unwrap();
        for i in (0..checkpoints.len() - 1).rev() {
            wb.undo().unwrap();
            prop_assert!(eq_model(&capture(&wb, id), &checkpoints[i]));
        }
        for cp in checkpoints.iter().skip(1) {
            wb.redo().unwrap();
            prop_assert!(eq_model(&capture(&wb, id), cp));
        }
        prop_assert!(eq_model(&capture(&wb, id), &end));
    }
}

#[test]
fn geometry_hidden_custom() {
    let mut a = AxisGeometry::rows();
    a.set_size(0, 50).unwrap();
    a.set_hidden(1, true).unwrap();
    a.set_size(2, 10).unwrap();
    assert_eq!(a.index_to_pixel(3), 60);
    assert_eq!(a.pixel_to_index(50), 2);
}

#[test]
fn formula_id_is_source_only() {
    let mut wb = Workbook::new();
    let id = wb.active_sheet();
    let fid = wb.set_formula_text(id, 0, 0, "=A1+1").unwrap();
    assert_eq!(wb.intern().formulas.get(fid), Some("=A1+1"));
    let slot: CellSlot = *wb.get(id, 0, 0).unwrap().unwrap();
    assert_eq!(slot.formula, Some(fid));
}
