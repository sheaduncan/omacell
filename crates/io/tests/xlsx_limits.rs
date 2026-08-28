//! Zip bomb, path traversal, DTD, and deep XML are rejected.

use std::io::{Cursor, Write};

use omacell_io::error::codes;
use omacell_io::xlsx::{MAX_XML_DEPTH, open_bytes, sanitize_path};
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
    let bytes = zip_one("../xl/workbook.xml", b"not-xml");
    let err = open_bytes(&bytes).unwrap_err();
    assert_eq!(err.code, codes::XLSX_PATH);
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
    let zeros = vec![0u8; 10_000];
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
    assert!(
        err.code == codes::XLSX_LIMIT || err.code == codes::XLSX_FORMAT,
        "{}",
        err.code
    );
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
