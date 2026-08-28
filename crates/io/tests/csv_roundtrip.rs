//! Quoting round-trips via proptest.

use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use omacell_io::csv::{ColumnType, ExportPlan, ImportPlan, Quoting, convert_cell, export, load};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn quoting_round_trip(grid in grid_strategy()) {
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        for (r, row) in grid.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                wb.set_text(sheet, r as u32, c as u16, cell).unwrap();
            }
        }
        let plan = ExportPlan {
            quoting: Quoting::Necessary,
            ..ExportPlan::default()
        };
        let bytes = export(&wb, &plan).unwrap();
        let width = grid.iter().map(Vec::len).max().unwrap_or(0);
        let import = ImportPlan {
            has_header: false,
            columns: (0..width)
                .map(|_| omacell_io::csv::ColumnPlan {
                    name: None,
                    ty: ColumnType::Text,
                })
                .collect(),
            ..ImportPlan::default()
        };
        let (wb2, _) = load(&bytes, &import, Default::default()).unwrap();
        for (r, row) in grid.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let got = wb2.get(wb2.active_sheet(), r as u32, c as u16).unwrap();
                if cell.is_empty() {
                    prop_assert!(got.is_none());
                    continue;
                }
                let slot = got.expect("cell");
                match slot.value {
                    Value::Text(id) => {
                        prop_assert_eq!(wb2.intern().strings.get(id), Some(cell.as_str()));
                    }
                    other => prop_assert!(false, "expected text, got {other:?}"),
                }
            }
        }
    }
}

fn grid_strategy() -> impl Strategy<Value = Vec<Vec<String>>> {
    let cell = prop::string::string_regex("[a-zA-Z0-9 ,\"\\n\\t]{0,16}").unwrap();
    prop::collection::vec(prop::collection::vec(cell, 1..5), 1..5)
}

#[test]
fn convert_cell_is_deterministic() {
    let plan = ImportPlan::default();
    let a = convert_cell("007", &ColumnType::Auto, &plan);
    let b = convert_cell("007", &ColumnType::Auto, &plan);
    assert_eq!(a, b);
}
