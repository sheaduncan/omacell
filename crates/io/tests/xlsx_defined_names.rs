use std::io::{Cursor, Read};

use omacell_core::addr::{CellRef, RangeRef, SheetId};
use omacell_core::eval::FnRegistry;
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::recalc::{RecalcEngine, format_cell};
use omacell_core::workbook::Workbook;
use omacell_io::xlsx::{open_bytes, save_bytes, save_workbook_bytes};

fn cell_range(sheet: Option<SheetId>, row: u32, col: u16) -> RangeRef {
    let mut cell = CellRef::with_abs(row, col, true, true).unwrap();
    cell.sheet = sheet;
    RangeRef::from_corners(cell, cell)
}

fn range_referent(workbook: &Workbook, scope: NameScope, name: &str) -> RangeRef {
    let defined = workbook.names().get(scope, name).expect("defined name");
    let NameReferent::Range(range) = defined.referent else {
        panic!("defined name is not a range: {:?}", defined.referent);
    };
    range
}

fn workbook_xml(bytes: &[u8]) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("xlsx package");
    let mut xml = String::new();
    archive
        .by_name("xl/workbook.xml")
        .expect("workbook XML")
        .read_to_string(&mut xml)
        .expect("UTF-8 workbook XML");
    xml
}

#[test]
fn defined_name_ranges_preserve_sheet_qualifiers_and_evaluate_from_other_sheets() {
    let mut workbook = Workbook::new();
    let input = workbook.active_sheet();
    workbook.rename_sheet(input, "Input Data").unwrap();
    let calc = workbook.add_sheet("Calc").unwrap();
    let archive = workbook.add_sheet("Archive").unwrap();
    workbook.set_number(input, 0, 0, 4.0).unwrap();
    workbook.set_number(calc, 0, 0, 9.0).unwrap();
    workbook.set_formula_text(calc, 0, 1, "=Rate*2").unwrap();

    workbook
        .define_name(DefinedName {
            name: "Rate".into(),
            scope: NameScope::Workbook,
            referent: NameReferent::Range(cell_range(Some(input), 0, 0)),
            comment: None,
        })
        .unwrap();
    workbook
        .define_name(DefinedName {
            name: "LocalSource".into(),
            scope: NameScope::Sheet(calc),
            referent: NameReferent::Range(cell_range(Some(input), 3, 2)),
            comment: None,
        })
        .unwrap();
    workbook
        .define_name(DefinedName {
            name: "Relative".into(),
            scope: NameScope::Workbook,
            referent: NameReferent::Range(cell_range(None, 1, 1)),
            comment: None,
        })
        .unwrap();
    let mut span = RangeRef::from_corners(
        CellRef::with_abs(1, 1, true, true).unwrap().on_sheet(input),
        CellRef::with_abs(2, 2, true, true).unwrap().on_sheet(input),
    );
    span.sheet_end = Some(archive);
    workbook
        .define_name(DefinedName {
            name: "ThreeSheets".into(),
            scope: NameScope::Workbook,
            referent: NameReferent::Range(span),
            comment: None,
        })
        .unwrap();

    let bytes = save_workbook_bytes(&workbook).unwrap();
    let xml = workbook_xml(&bytes);
    assert!(
        xml.contains("<definedName name=\"Rate\">&apos;Input Data&apos;!$A$1:$A$1</definedName>")
    );
    assert!(xml.contains(
        "<definedName name=\"LocalSource\" localSheetId=\"1\">&apos;Input Data&apos;!$C$4:$C$4</definedName>"
    ));
    assert!(xml.contains("<definedName name=\"Relative\">$B$2:$B$2</definedName>"));
    assert!(xml.contains(
        "<definedName name=\"ThreeSheets\">&apos;Input Data:Archive&apos;!$B$2:$C$3</definedName>"
    ));

    let mut loaded = open_bytes(&bytes).unwrap();
    let loaded_input = loaded.workbook.sheet_by_name("Input Data").unwrap().id;
    let loaded_calc = loaded.workbook.sheet_by_name("Calc").unwrap().id;
    let loaded_archive = loaded.workbook.sheet_by_name("Archive").unwrap().id;
    assert_eq!(
        range_referent(&loaded.workbook, NameScope::Workbook, "Rate")
            .start
            .sheet,
        Some(loaded_input)
    );
    assert_eq!(
        range_referent(
            &loaded.workbook,
            NameScope::Sheet(loaded_calc),
            "LocalSource"
        )
        .start
        .sheet,
        Some(loaded_input)
    );
    assert_eq!(
        range_referent(&loaded.workbook, NameScope::Workbook, "Relative")
            .start
            .sheet,
        None
    );
    assert_eq!(
        range_referent(&loaded.workbook, NameScope::Workbook, "ThreeSheets").sheet_end,
        Some(loaded_archive)
    );

    let mut engine = RecalcEngine::new(FnRegistry::new());
    engine.recalc_full(&mut loaded.workbook);
    assert_eq!(format_cell(&loaded.workbook, loaded_calc, 0, 1), "8");

    let resaved = save_bytes(&loaded).unwrap();
    assert_eq!(workbook_xml(&resaved), xml);
}
