//! Fuzz worksheet XML wrapped in a minimal OPC package.
#![no_main]

use std::io::{Cursor, Write};

use libfuzzer_sys::fuzz_target;
use omacell_io::xlsx::open_bytes;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fuzz_target!(|data: &[u8]| {
    if data.len() > 8 * 1024 {
        return;
    }
    let Ok(body) = std::str::from_utf8(data) else {
        return;
    };
    if body.contains("<!DOCTYPE") || body.contains("<!ENTITY") {
        return;
    }
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{body}</sheetData></worksheet>"#
    );
    let ns_pkg = "http://schemas.openxmlformats.org/package/2006";
    let od = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let ns = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    let mut cur = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut cur);
        let opt = SimpleFileOptions::default();
        let _ = z.start_file("_rels/.rels", opt);
        let _ = z.write_all(format!(r#"<?xml version="1.0"?><Relationships xmlns="{ns_pkg}/relationships"><Relationship Id="rId1" Type="{od}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#).as_bytes());
        let _ = z.start_file("[Content_Types].xml", opt);
        let _ = z.write_all(format!(r#"<?xml version="1.0"?><Types xmlns="{ns_pkg}/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#).as_bytes());
        let _ = z.start_file("xl/_rels/workbook.xml.rels", opt);
        let _ = z.write_all(format!(r#"<?xml version="1.0"?><Relationships xmlns="{ns_pkg}/relationships"><Relationship Id="rId1" Type="{od}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#).as_bytes());
        let _ = z.start_file("xl/workbook.xml", opt);
        let _ = z.write_all(format!(r#"<?xml version="1.0"?><workbook xmlns="{ns}" xmlns:r="{od}"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#).as_bytes());
        let _ = z.start_file("xl/worksheets/sheet1.xml", opt);
        let _ = z.write_all(sheet.as_bytes());
        let _ = z.finish();
    }
    let _ = open_bytes(&cur.into_inner());
});
