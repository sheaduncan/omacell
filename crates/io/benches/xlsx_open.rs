//! Open a synthetic ~50 MB workbook (spec §12.1).

use std::io::{Cursor, Write};
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use omacell_io::xlsx::open_bytes;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn big_xlsx() -> Vec<u8> {
    // 86k rows × 20 numeric cells is just over 50 MiB of worksheet XML.
    let mut sheet = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for r in 1..=86_000u32 {
        sheet.push_str("<row r=\"");
        sheet.push_str(&r.to_string());
        sheet.push_str("\">");
        for c in 0..20u16 {
            let addr = omacell_core::addr::col_to_letters(c).unwrap();
            sheet.push_str("<c r=\"");
            sheet.push_str(&addr);
            sheet.push_str(&r.to_string());
            sheet.push_str("\"><v>");
            sheet.push_str(&(r + u32::from(c)).to_string());
            sheet.push_str("</v></c>");
        }
        sheet.push_str("</row>");
    }
    sheet.push_str("</sheetData></worksheet>");
    assert!(sheet.len() >= 50 * 1024 * 1024);
    let ns_pkg = "http://schemas.openxmlformats.org/package/2006";
    let od = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let ns = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut buf);
        let opt = SimpleFileOptions::default();
        z.start_file("_rels/.rels", opt).unwrap();
        z.write_all(format!(r#"<?xml version="1.0"?><Relationships xmlns="{ns_pkg}/relationships"><Relationship Id="rId1" Type="{od}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#).as_bytes()).unwrap();
        z.start_file("[Content_Types].xml", opt).unwrap();
        z.write_all(format!(r#"<?xml version="1.0"?><Types xmlns="{ns_pkg}/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#).as_bytes()).unwrap();
        z.start_file("xl/_rels/workbook.xml.rels", opt).unwrap();
        z.write_all(format!(r#"<?xml version="1.0"?><Relationships xmlns="{ns_pkg}/relationships"><Relationship Id="rId1" Type="{od}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#).as_bytes()).unwrap();
        z.start_file("xl/workbook.xml", opt).unwrap();
        z.write_all(format!(r#"<?xml version="1.0"?><workbook xmlns="{ns}" xmlns:r="{od}"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#).as_bytes()).unwrap();
        z.start_file("xl/worksheets/sheet1.xml", opt).unwrap();
        z.write_all(sheet.as_bytes()).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn bench_open(c: &mut Criterion) {
    let bytes = big_xlsx();
    let mut group = c.benchmark_group("xlsx_open");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(8));
    group.bench_function("synthetic_sheet", |b| {
        b.iter(|| {
            let doc = open_bytes(&bytes).expect("open");
            std::hint::black_box(doc.workbook.used_range(doc.workbook.active_sheet()).ok());
        });
    });
    group.finish();
}

criterion_group!(benches, bench_open);
criterion_main!(benches);
