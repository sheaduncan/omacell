//! Zip bomb, path traversal, DTD, and deep XML are rejected.

use std::io::{Cursor, Write};

use omacell_io::error::codes;
use omacell_io::xlsx::{MAX_XML_DEPTH, open_bytes, open_package, sanitize_path};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn zip_one(name: &str, data: &[u8]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut buf);
        z.start_file(name, SimpleFileOptions::default()).unwrap();
        z.write_all(data).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

#[test]
fn path_traversal_rejected() {
    assert!(sanitize_path("../xl/workbook.xml").is_err());
    assert!(sanitize_path("/xl/workbook.xml").is_err());
    assert!(sanitize_path("\\xl\\workbook.xml").is_err());
    assert!(sanitize_path("C:\\xl\\workbook.xml").is_err());
    let bytes = zip_one("../xl/workbook.xml", b"not-xml");
    let err = open_bytes(&bytes).unwrap_err();
    assert_eq!(err.code, codes::XLSX_PATH);
}

#[test]
fn absolute_zip_entry_rejected() {
    let bytes = zip_one("/xl/workbook.xml", b"not-xml");
    let err = open_bytes(&bytes).unwrap_err();
    assert_eq!(err.code, codes::XLSX_PATH);
}

#[test]
fn declared_uncompressed_size_mismatch_rejected() {
    let mut bytes = zip_one("part.bin", &[7; 4096]);
    let central = bytes
        .windows(4)
        .rposition(|window| window == [0x50, 0x4b, 0x01, 0x02])
        .expect("central directory header");
    bytes[central + 24..central + 28].copy_from_slice(&1u32.to_le_bytes());
    let err = open_package(&bytes).unwrap_err();
    assert_eq!(err.code, codes::XLSX_ZIP);
}

#[test]
fn duplicate_part_names_rejected() {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut buf);
        let opt = SimpleFileOptions::default();
        z.start_file("xl/workbook.xml", opt).unwrap();
        z.write_all(b"first").unwrap();
        z.start_file("XL/WORKBOOK.XML", opt).unwrap();
        z.write_all(b"second").unwrap();
        z.finish().unwrap();
    }
    let err = open_package(&buf.into_inner()).unwrap_err();
    assert_eq!(err.code, codes::XLSX_FORMAT);
}

#[test]
fn doctype_rejected() {
    let xml = br#"<?xml version="1.0"?><!DOCTYPE foo [<!ENTITY xxe SYSTEM "file:///etc/passwd">]><a>&xxe;</a>"#;
    let bytes = minimal_package_with_workbook(xml);
    let err = open_bytes(&bytes).unwrap_err();
    assert_eq!(err.code, codes::XLSX_XML);
}

#[test]
fn deep_xml_rejected() {
    let mut inner = String::from("<a/>");
    for _ in 0..(MAX_XML_DEPTH + 2) {
        inner = format!("<e>{inner}</e>");
    }
    let xml = format!(r#"<?xml version="1.0"?>{inner}"#);
    let bytes = minimal_package_with_workbook(xml.as_bytes());
    let err = open_bytes(&bytes).unwrap_err();
    assert_eq!(err.code, codes::XLSX_LIMIT);
}

#[test]
fn compression_ratio_rejected() {
    let zeros = vec![0u8; 1_000_000];
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut buf);
        z.start_file(
            "xl/workbook.xml",
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        z.write_all(&zeros).unwrap();
        z.finish().unwrap();
    }
    let bytes = buf.into_inner();
    let err = open_bytes(&bytes).unwrap_err();
    assert_eq!(err.code, codes::XLSX_LIMIT);
}

#[test]
fn oversized_column_range_rejected_without_iteration() {
    let sheet = br#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="1" max="4294967295" hidden="1"/></cols><sheetData/></worksheet>"#;
    let bytes = minimal_package_with_sheet(sheet);
    let err = open_bytes(&bytes).unwrap_err();
    assert_eq!(err.code, codes::XLSX_LIMIT);
}

fn minimal_package_with_workbook(workbook_xml: &[u8]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut buf);
        let opt = SimpleFileOptions::default();
        z.start_file("_rels/.rels", opt).unwrap();
        z.write_all(
            br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        )
        .unwrap();
        z.start_file("[Content_Types].xml", opt).unwrap();
        z.write_all(
            br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#,
        )
        .unwrap();
        z.start_file("xl/workbook.xml", opt).unwrap();
        z.write_all(workbook_xml).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn minimal_package_with_sheet(sheet_xml: &[u8]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut buf);
        let opt = SimpleFileOptions::default();
        z.start_file("_rels/.rels", opt).unwrap();
        z.write_all(
            br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        )
        .unwrap();
        z.start_file("[Content_Types].xml", opt).unwrap();
        z.write_all(
            br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
        )
        .unwrap();
        z.start_file("xl/_rels/workbook.xml.rels", opt).unwrap();
        z.write_all(
            br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        )
        .unwrap();
        z.start_file("xl/workbook.xml", opt).unwrap();
        z.write_all(
            br#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        )
        .unwrap();
        z.start_file("xl/worksheets/sheet1.xml", opt).unwrap();
        z.write_all(sheet_xml).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}
