//! Golden workbook cards at every level.

use insta::assert_json_snapshot;
use omacell_ai::card::{CardLevel, CardRequest, build};
use omacell_core::workbook::Workbook;

fn fixture() -> Workbook {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, "Name").unwrap();
    wb.set_cell_contents(sheet, 0, 1, "Amount").unwrap();
    wb.set_cell_contents(sheet, 1, 0, "Ada").unwrap();
    wb.set_cell_contents(sheet, 1, 1, "10").unwrap();
    wb.set_cell_contents(sheet, 2, 1, "=B2+1").unwrap();
    wb
}

#[test]
fn golden_levels() {
    let wb = fixture();
    for (name, level) in [
        ("summary", CardLevel::Summary),
        ("columns", CardLevel::Columns),
        ("sample", CardLevel::Sample),
        ("full", CardLevel::Full),
    ] {
        let card = build(
            &wb,
            None,
            &CardRequest {
                level,
                file: Some("book.xlsx".into()),
                range: Some("Sheet1!A1:B3".into()),
                sample_rows: 5,
                token_budget: 2048,
                selection: None,
            },
        )
        .unwrap();
        assert_eq!(card["schema"], 1);
        assert_eq!(card["kind"], name);
        assert_json_snapshot!(name, card);
    }
}
