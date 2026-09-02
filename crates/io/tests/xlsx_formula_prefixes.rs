use std::io::{Cursor, Read};

use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::condfmt::{CfDxf, CfKind, CondFormat};
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::validation::{DataValidation, DvType};
use omacell_core::workbook::Workbook;
use omacell_io::xlsx::{open_bytes, save_bytes, save_workbook_bytes};

fn formula_at(workbook: &Workbook, row: u32) -> String {
    workbook
        .formula_text_at(workbook.active_sheet(), row, 0)
        .expect("formula cell")
}

fn part_xml(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("xlsx package");
    let mut xml = String::new();
    archive
        .by_name(name)
        .expect("XML part")
        .read_to_string(&mut xml)
        .expect("part XML");
    xml
}

fn worksheet_xml(bytes: &[u8]) -> String {
    part_xml(bytes, "xl/worksheets/sheet1.xml")
}

fn name_formula(workbook: &Workbook, name: &str) -> String {
    let defined = workbook
        .names()
        .get(NameScope::Workbook, name)
        .expect("defined name");
    let NameReferent::Formula(formula) = &defined.referent else {
        panic!("defined name is not a formula");
    };
    formula.clone()
}

#[test]
fn import_normalizes_excel_future_function_and_parameter_prefixes() {
    let formulas = [
        "=_xlfn.XLOOKUP(2,A1:A2,B1:B2)",
        "=_xlfn._xlws.FILTER(A1:B2,A1:A2>0)",
        "=_xlfn._xlws.SORT(A1:A2)",
        "=_xlfn.LET(_xlpm.x,2,_xlfn.LAMBDA(_xlpm.y,_xlpm.x+_xlpm.y)(3))",
        "=_xlfn.SINGLE(A1:A2)",
        "=_xlfn.NOT_A_FUNCTION(1)",
    ];
    let expected = [
        "=XLOOKUP(2,A1:A2,B1:B2)",
        "=FILTER(A1:B2,A1:A2>0)",
        "=SORT(A1:A2)",
        "=LET(x,2,LAMBDA(y,x+y)(3))",
        "=@A1:A2",
        "=_xlfn.NOT_A_FUNCTION(1)",
    ];
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    for (row, formula) in formulas.iter().enumerate() {
        workbook
            .set_formula_text(sheet, row as u32, 0, formula)
            .unwrap();
    }
    workbook
        .define_name(DefinedName {
            name: "FutureName".into(),
            scope: NameScope::Workbook,
            referent: NameReferent::Formula("_xlfn.XLOOKUP(2,A1:A2,B1:B2)".into()),
            comment: None,
        })
        .unwrap();

    let bytes = save_workbook_bytes(&workbook).unwrap();
    let loaded = open_bytes(&bytes).unwrap();
    for (row, expected) in expected.iter().enumerate() {
        assert_eq!(formula_at(&loaded.workbook, row as u32), *expected);
    }
    assert_eq!(
        name_formula(&loaded.workbook, "FutureName"),
        "XLOOKUP(2,A1:A2,B1:B2)"
    );
}

#[test]
fn export_uses_excel_prefixes_without_rewriting_formula_like_text() {
    let formulas = [
        "=XLOOKUP(2,A1:A2,B1:B2)",
        "=FILTER(A1:B2,A1:A2>0)",
        "=SORT(A1:A2)",
        "=LET(x,2,LAMBDA(y,x+y)(3))",
        "=@A1:A2",
        "=SUM(A1:A2)",
        "=\"_xlfn.XLOOKUP(\"&A1",
    ];
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    for (row, formula) in formulas.iter().enumerate() {
        workbook
            .set_formula_text(sheet, row as u32, 0, formula)
            .unwrap();
    }
    workbook
        .define_name(DefinedName {
            name: "FutureName".into(),
            scope: NameScope::Workbook,
            referent: NameReferent::Formula("XLOOKUP(2,A1:A2,B1:B2)".into()),
            comment: None,
        })
        .unwrap();

    let bytes = save_workbook_bytes(&workbook).unwrap();
    let xml = worksheet_xml(&bytes);
    assert!(xml.contains("<f>_XLFN.XLOOKUP(2,A1:A2,B1:B2)</f>"));
    assert!(xml.contains("<f>_XLFN._XLWS.FILTER(A1:B2,A1:A2&gt;0)</f>"));
    assert!(xml.contains("<f>_XLFN._XLWS.SORT(A1:A2)</f>"));
    assert!(xml.contains("<f>_XLFN.LET(_xlpm.x,2,_XLFN.LAMBDA(_xlpm.y,_xlpm.x+_xlpm.y)(3))</f>"));
    assert!(xml.contains("<f>_XLFN.SINGLE(A1:A2)</f>"));
    assert!(xml.contains("<f>SUM(A1:A2)</f>"));
    assert!(xml.contains("<f>&quot;_xlfn.XLOOKUP(&quot;&amp;A1</f>"));
    let workbook_xml = part_xml(&bytes, "xl/workbook.xml");
    assert!(
        workbook_xml.contains(
            "<definedName name=\"FutureName\">_XLFN.XLOOKUP(2,A1:A2,B1:B2)</definedName>"
        )
    );

    let loaded = open_bytes(&bytes).unwrap();
    for (row, formula) in formulas.iter().enumerate() {
        assert_eq!(formula_at(&loaded.workbook, row as u32), *formula);
    }
    assert_eq!(
        name_formula(&loaded.workbook, "FutureName"),
        "XLOOKUP(2,A1:A2,B1:B2)"
    );

    let resaved = save_bytes(&loaded).unwrap();
    assert_eq!(worksheet_xml(&resaved), xml);
    assert_eq!(part_xml(&resaved, "xl/workbook.xml"), workbook_xml);
}

#[test]
fn validation_and_conditional_format_formulas_use_the_same_boundary() {
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    let range = RangeRef::from_corners(CellRef::new(0, 0).unwrap(), CellRef::new(0, 0).unwrap());
    let validations = vec![DataValidation {
        range,
        kind: DvType::Custom,
        formula1: Some("XLOOKUP(A1,B1:B2,C1:C2)>0".into()),
        ..DataValidation::default()
    }];
    let formats = vec![CondFormat {
        range,
        priority: 1,
        stop_if_true: false,
        kind: CfKind::Formula("FILTER(A1:A2,A1:A2>0)".into()),
        dxf: CfDxf::default(),
    }];
    workbook
        .set_validations(sheet, validations.clone())
        .unwrap();
    workbook.set_cond_formats(sheet, formats.clone()).unwrap();

    let bytes = save_workbook_bytes(&workbook).unwrap();
    let xml = worksheet_xml(&bytes);
    assert!(xml.contains("<formula1>_XLFN.XLOOKUP(A1,B1:B2,C1:C2)&gt;0</formula1>"));
    assert!(xml.contains("<formula>_XLFN._XLWS.FILTER(A1:A2,A1:A2&gt;0)</formula>"));

    let loaded = open_bytes(&bytes).unwrap();
    let sheet = loaded
        .workbook
        .sheet(loaded.workbook.active_sheet())
        .unwrap();
    assert_eq!(sheet.validations, validations);
    assert_eq!(sheet.cond_formats, formats);
}
