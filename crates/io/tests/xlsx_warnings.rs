//! Unparsable formulas are kept as text with a warning.

use std::io::{Cursor, Write};

use omacell_core::value::Value;
use omacell_io::xlsx::open_bytes;
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
                r#"<?xml version="1.0"?><Types xmlns="{ns_pkg}/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#
            )
            .as_bytes(),
        )
        .unwrap();
        z.start_file("xl/_rels/workbook.xml.rels", opt).unwrap();
        z.write_all(
            format!(
                r#"<?xml version="1.0"?><Relationships xmlns="{ns_pkg}/relationships"><Relationship Id="rId1" Type="{od}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
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
        z.finish().unwrap();
    }
    buf.into_inner()
}
