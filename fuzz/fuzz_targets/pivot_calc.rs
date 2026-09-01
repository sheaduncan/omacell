#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::pivot::{PivotCalcField, PivotTable, cache_table};
use omacell_core::workbook::Workbook;

fuzz_target!(|data: &[u8]| {
    if data.len() > 16_384 {
        return;
    }
    let Ok(formula) = std::str::from_utf8(data) else {
        return;
    };
    let (Some(start), Some(end)) = (CellRef::new(0, 0).ok(), CellRef::new(1, 0).ok()) else {
        return;
    };
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    if workbook.set_text(sheet, 0, 0, "Amount").is_err()
        || workbook.set_number(sheet, 1, 0, 1.0).is_err()
    {
        return;
    }
    let mut pivot = PivotTable::new(
        "Fuzz",
        sheet,
        RangeRef::from_corners(start, end),
        sheet,
        0,
        2,
    );
    pivot.calc_fields.push(PivotCalcField {
        name: "Result".into(),
        formula: formula.into(),
    });
    let _ = cache_table(&workbook, &pivot);
});
