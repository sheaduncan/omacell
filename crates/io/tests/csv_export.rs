//! Export controls: delimiter, quoting, encoding, range, formulas-or-values.

use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use omacell_io::csv::{
    ExportPlan, LineEnding, Quoting, TextEncoding, ValueMode, decode_all, export, load, sniff,
};

#[test]
fn values_and_formulas() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_number(sheet, 0, 0, 1.5).unwrap();
    wb.set_formula_text(sheet, 0, 1, "=A1*2").unwrap();
    let mut plan = ExportPlan::default();
    let values = export(&wb, &plan).unwrap();
    assert_eq!(String::from_utf8(values).unwrap(), "1.5,\n");
    plan.values = ValueMode::Formulas;
    let formulas = export(&wb, &plan).unwrap();
    let text = String::from_utf8(formulas).unwrap();
    assert!(text.contains("=A1*2"), "{text}");
}

#[test]
fn crlf_and_utf16() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_number(sheet, 0, 0, 1.0).unwrap();
    wb.set_number(sheet, 0, 1, 2.0).unwrap();
    let mut plan = ExportPlan {
        line_ending: LineEnding::CrLf,
        encoding: TextEncoding::Utf16Le,
        bom: true,
        ..ExportPlan::default()
    };
    let bytes = export(&wb, &plan).unwrap();
    assert!(bytes.starts_with(&[0xFF, 0xFE]));
    let text = decode_all(&bytes, TextEncoding::Utf16Le).unwrap();
    assert!(text.contains("\r\n"), "{text:?}");
    plan.encoding = TextEncoding::Utf8;
    plan.bom = false;
    let utf8 = export(&wb, &plan).unwrap();
    assert_eq!(utf8, b"1,2\r\n");
}

#[test]
fn always_quote() {
    let mut wb = Workbook::new();
    wb.set_number(wb.active_sheet(), 0, 0, 1.0).unwrap();
    let plan = ExportPlan {
        quoting: Quoting::Always,
        ..ExportPlan::default()
    };
    let out = export(&wb, &plan).unwrap();
    assert_eq!(out, b"\"1\"\n");
}

#[test]
fn range_and_sheet() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_number(sheet, 0, 0, 1.0).unwrap();
    wb.set_number(sheet, 0, 1, 2.0).unwrap();
    wb.set_number(sheet, 1, 0, 3.0).unwrap();
    let plan = ExportPlan {
        range: Some("A1:A2".into()),
        ..ExportPlan::default()
    };
    let out = String::from_utf8(export(&wb, &plan).unwrap()).unwrap();
    assert_eq!(out, "1\n3\n");
}

#[test]
fn export_import_simple() {
    let bytes = b"a,b\n1,2\n";
    let sniffed = sniff(bytes).unwrap();
    let (wb, _) = load(bytes, &sniffed.plan, Default::default()).unwrap();
    let out = export(&wb, &ExportPlan::default()).unwrap();
    let sniffed2 = sniff(&out).unwrap();
    let (wb2, _) = load(&out, &sniffed2.plan, Default::default()).unwrap();
    assert_eq!(
        wb2.get(wb2.active_sheet(), 1, 1).unwrap().unwrap().value,
        Value::Number(2.0)
    );
}
