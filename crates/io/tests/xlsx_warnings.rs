//! Unparsable formulas are kept as text with a warning.

use std::io::{Cursor, Read, Write};

use omacell_core::eval::FnRegistry;
use omacell_core::recalc::{RecalcEngine, format_cell};
use omacell_core::value::Value;
use omacell_io::xlsx::{open_bytes, save_workbook_bytes};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

#[test]
fn unparsable_formula_becomes_text_with_warning() {
    let sheet = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>
<row r="1"><c r="A1"><f>(((</f><v></v></c></row>
</sheetData></worksheet>"#;
    let bytes = package_with_sheet(sheet);
    let doc = open_bytes(&bytes).unwrap();
    let sheet_id = doc.workbook.active_sheet();
    let slot = doc.workbook.get(sheet_id, 0, 0).unwrap().unwrap();
    assert!(slot.formula.is_none());
    let Value::Text(id) = slot.value else {
        panic!("{:?}", slot.value);
    };
    let text = doc.workbook.intern().strings.get(id).unwrap();
    assert!(text.contains("((("), "{text}");
    assert!(
        doc.warnings.items.iter().any(|w| w.code == "xlsx.formula"),
        "{:?}",
        doc.warnings
    );
}

#[test]
fn formula_cached_values_keep_their_declared_types() {
    let sheet = br#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1">
<c r="A1" t="b"><f>1=1</f><v>1</v></c>
<c r="B1" t="str"><f>&quot;ok&quot;</f><v>ok</v></c>
<c r="C1" t="e"><f>1/0</f><v>#DIV/0!</v></c>
</row></sheetData></worksheet>"#;
    let doc = open_bytes(&package_with_sheet(sheet)).unwrap();
    let id = doc.workbook.active_sheet();
    let bool_slot = doc.workbook.get(id, 0, 0).unwrap().unwrap();
    assert!(bool_slot.formula.is_some());
    assert_eq!(bool_slot.value, Value::Bool(true));
    let text_slot = doc.workbook.get(id, 0, 1).unwrap().unwrap();
    assert!(text_slot.formula.is_some());
    let Value::Text(text_id) = text_slot.value else {
        panic!("{:?}", text_slot.value);
    };
    assert_eq!(doc.workbook.intern().strings.get(text_id), Some("ok"));
    let error_slot = doc.workbook.get(id, 0, 2).unwrap().unwrap();
    assert!(error_slot.formula.is_some());
    assert_eq!(
        error_slot.value,
        Value::Error(omacell_core::error::ErrorKind::Div0)
    );
}

#[test]
fn phonetic_runs_do_not_change_shared_string_text() {
    let sheet = br#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c></row></sheetData></worksheet>"#;
    let bytes = package_with_sheet(sheet);
    let doc = open_bytes(&bytes).unwrap();
    let slot = doc
        .workbook
        .get(doc.workbook.active_sheet(), 0, 0)
        .unwrap()
        .unwrap();
    let Value::Text(id) = slot.value else {
        panic!("{:?}", slot.value);
    };
    assert_eq!(doc.workbook.intern().strings.get(id), Some("漢字"));
}

#[test]
fn legacy_array_formula_range_recalculates_and_round_trips() {
    let sheet = br#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1"><f t="array" ref="A1:B2">{1,2,3}</f><v>1</v></c><c r="B1"><v>2</v></c></row>
<row r="2"><c r="A2" t="e"><v>#N/A</v></c><c r="B2" t="e"><v>#N/A</v></c></row>
</sheetData></worksheet>"#;
    let mut doc = open_bytes(&package_with_sheet(sheet)).unwrap();
    let sheet_id = doc.workbook.active_sheet();
    let cse = doc
        .workbook
        .sheet(sheet_id)
        .unwrap()
        .array_formula_at(1, 1)
        .unwrap();
    assert_eq!(cse.range.to_a1(), "A1:B2");

    let mut engine = RecalcEngine::new(FnRegistry::new());
    engine.recalc_full(&mut doc.workbook);
    assert_eq!(format_cell(&doc.workbook, sheet_id, 0, 1), "2");
    assert_eq!(format_cell(&doc.workbook, sheet_id, 1, 0), "#N/A");

    let saved = save_workbook_bytes(&doc.workbook).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(&saved)).unwrap();
    let mut worksheet = String::new();
    archive
        .by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut worksheet)
        .unwrap();
    assert!(
        worksheet.contains(r#"<f t="array" ref="A1:B2">{1,2,3}</f>"#),
        "{worksheet}"
    );
    drop(archive);

    let reopened = open_bytes(&saved).unwrap();
    assert!(
        reopened
            .workbook
            .sheet(reopened.workbook.active_sheet())
            .unwrap()
            .array_formula_at(1, 1)
            .is_some()
    );
}

fn package_with_sheet(sheet: &[u8]) -> Vec<u8> {
    let ns_pkg = "http://schemas.openxmlformats.org/package/2006";
    let od = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let ns = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut buf);
        let opt = SimpleFileOptions::default();
        z.start_file("_rels/.rels", opt).unwrap();
        z.write_all(
            format!(
                r#"<?xml version="1.0"?><Relationships xmlns="{ns_pkg}/relationships"><Relationship Id="rId1" Type="{od}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
            )
            .as_bytes(),
        )
        .unwrap();
        z.start_file("[Content_Types].xml", opt).unwrap();
        z.write_all(
            format!(
                r#"<?xml version="1.0"?><Types xmlns="{ns_pkg}/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>"#
            )
            .as_bytes(),
        )
        .unwrap();
        z.start_file("xl/_rels/workbook.xml.rels", opt).unwrap();
        z.write_all(
            format!(
                r#"<?xml version="1.0"?><Relationships xmlns="{ns_pkg}/relationships"><Relationship Id="rId1" Type="{od}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{od}/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#
            )
            .as_bytes(),
        )
        .unwrap();
        z.start_file("xl/workbook.xml", opt).unwrap();
        z.write_all(
            format!(
                r#"<?xml version="1.0"?><workbook xmlns="{ns}" xmlns:r="{od}"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#
            )
            .as_bytes(),
        )
        .unwrap();
        z.start_file("xl/worksheets/sheet1.xml", opt).unwrap();
        z.write_all(sheet).unwrap();
        z.start_file("xl/sharedStrings.xml", opt).unwrap();
        z.write_all(
            format!(
                r#"<?xml version="1.0"?><sst xmlns="{ns}"><si><t>漢字</t><rPh sb="0" eb="2"><t>かんじ</t></rPh></si></sst>"#
            )
            .as_bytes(),
        )
        .unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}
