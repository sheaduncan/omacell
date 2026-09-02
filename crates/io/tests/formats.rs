//! WP-27 corpora: ODS, JSON, HTML/Markdown, native `.xls`, locks.

use omacell_core::addr::CellRef;
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::ops;
use omacell_core::style::{Font, Style};
use omacell_core::workbook::Workbook;
use omacell_io::csv::ClipboardFormat;
use omacell_io::error::codes;
use omacell_io::xlsx::{lock_path, peer_lock_blocks};
use omacell_io::{bridge, html, json, ods};
use std::io::{Cursor, Write};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

#[path = "../../../tests/support/libreoffice.rs"]
mod libreoffice;

fn cell_text(wb: &Workbook, row: u32, col: u16) -> String {
    let sheet = wb.active_sheet();
    match wb.get(sheet, row, col).ok().flatten() {
        Some(slot) => match slot.value {
            omacell_core::value::Value::Number(n) => n.to_string(),
            omacell_core::value::Value::Text(id) => {
                wb.intern().strings.get(id).unwrap_or("").to_string()
            }
            omacell_core::value::Value::Bool(true) => "TRUE".into(),
            omacell_core::value::Value::Bool(false) => "FALSE".into(),
            _ => String::new(),
        },
        None => String::new(),
    }
}

#[test]
fn ods_round_trips_values_formula_merge_name_and_bold() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.rename_sheet(sheet, "O'Brien").unwrap();
    wb.set_number(sheet, 0, 0, 1.5).unwrap();
    wb.set_text(sheet, 0, 1, "Ada").unwrap();
    wb.set_formula_text(sheet, 1, 0, "=A1+1").unwrap();
    wb.set_formula_text(sheet, 1, 1, "=LOG10(100)").unwrap();
    ops::merge(
        &mut wb,
        sheet,
        omacell_core::addr::RangeRef::from_corners(
            CellRef::new(2, 0).unwrap(),
            CellRef::new(2, 1).unwrap(),
        ),
    )
    .unwrap();
    wb.set_text(sheet, 2, 0, "merged").unwrap();
    wb.define_name(DefinedName {
        name: "Tax".into(),
        scope: NameScope::Workbook,
        referent: NameReferent::Range(omacell_core::addr::RangeRef::from_corners(
            CellRef::new(0, 0).unwrap(),
            CellRef::new(0, 0).unwrap(),
        )),
        comment: None,
    })
    .unwrap();
    wb.set_cell_style(
        sheet,
        0,
        1,
        Style {
            font: Font {
                bold: true,
                ..Font::default()
            },
            ..Style::default()
        },
    )
    .unwrap();

    let bytes = ods::save_bytes(&wb).unwrap();
    let again = ods::open_bytes(&bytes).unwrap();
    assert_eq!(cell_text(&again, 0, 0), "1.5");
    assert_eq!(cell_text(&again, 0, 1), "Ada");
    let slot = again.get(again.active_sheet(), 1, 0).unwrap().unwrap();
    let src = slot
        .formula
        .and_then(|id| again.intern().formulas.get(id).map(str::to_string))
        .unwrap();
    assert!(src.contains("A1"), "{src}");
    let log_slot = again.get(again.active_sheet(), 1, 1).unwrap().unwrap();
    assert_eq!(
        log_slot
            .formula
            .and_then(|id| again.intern().formulas.get(id)),
        Some("=LOG10(100)")
    );
    assert_eq!(again.sheet(again.active_sheet()).unwrap().merges.len(), 1);
    assert!(
        again
            .names()
            .iter()
            .any(|n| n.name.eq_ignore_ascii_case("Tax"))
    );
    let style = again
        .intern()
        .styles
        .get(
            again
                .get(again.active_sheet(), 0, 1)
                .unwrap()
                .unwrap()
                .style,
        )
        .unwrap();
    assert!(style.font.bold);
}

#[test]
fn ods_written_file_reopens_in_libreoffice_if_present() {
    let Some(soffice) = libreoffice::find_calc() else {
        return;
    };
    let dir = tempfile_dir("ods-lo");
    let ods_path = dir.join("book.ods");
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_number(sheet, 0, 0, 1.0).unwrap();
    wb.set_number(sheet, 1, 0, 2.0).unwrap();
    wb.set_formula_text(sheet, 2, 0, "=SUM(A1:A2)").unwrap();
    ods::save(&wb, &ods_path).unwrap();
    let status = std::process::Command::new(&soffice)
        .arg(format!(
            "-env:UserInstallation=file://{}",
            dir.join("profile").display()
        ))
        .env("HOME", &dir)
        .env("SAL_USE_VCLPLUGIN", "svp")
        .args(["--headless", "--convert-to", "xlsx", "--outdir"])
        .arg(&dir)
        .arg(&ods_path)
        .status()
        .unwrap();
    assert!(status.success());
    let reopened = omacell_io::xlsx::open(&dir.join("book.xlsx")).unwrap();
    assert!(
        reopened
            .workbook
            .get(reopened.workbook.active_sheet(), 2, 0)
            .unwrap()
            .unwrap()
            .formula
            .is_some()
    );
}

#[test]
fn json_flattens_nested_objects_and_respects_pointer() {
    let src = br#"{
        "skip": 1,
        "items": [
            {"user": {"name": "Ada"}, "n": 1},
            {"user": {"name": "Bob"}, "n": 2}
        ]
    }"#;
    let err = json::open_bytes(src).unwrap_err();
    assert_eq!(err.code, codes::JSON_FORMAT);
    let wb = json::open_bytes_with_pointer(src, Some(".items")).unwrap();
    assert_eq!(cell_text(&wb, 0, 0), "n");
    assert_eq!(cell_text(&wb, 0, 1), "user.name");
    assert_eq!(cell_text(&wb, 1, 1), "Ada");
    assert_eq!(cell_text(&wb, 2, 0), "2");
    let exported = json::export(&wb).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&exported).unwrap();
    assert!(value.is_array());
}

#[test]
fn json_rejects_non_objects_flattening_collisions_and_duplicate_headers() {
    let err = json::open_bytes(br#"[{"ok":1}, 2]"#).unwrap_err();
    assert_eq!(err.code, codes::JSON_FORMAT);

    let err = json::open_bytes(br#"[{"a.b":1,"a":{"b":2}}]"#).unwrap_err();
    assert_eq!(err.code, codes::JSON_FORMAT);

    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_text(sheet, 0, 0, "same").unwrap();
    wb.set_text(sheet, 0, 1, "same").unwrap();
    wb.set_number(sheet, 1, 0, 1.0).unwrap();
    let err = json::export(&wb).unwrap_err();
    assert_eq!(err.code, codes::JSON_FORMAT);
}

#[test]
fn ods_rejects_invalid_values_and_expands_repeated_cells() {
    let invalid = ods_package(
        r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"><office:body><office:spreadsheet><table:table table:name="Sheet1"><table:table-row><table:table-cell office:value-type="float" office:value="not-a-number"/></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#,
    );
    assert_eq!(
        ods::open_bytes(&invalid).unwrap_err().code,
        codes::ODS_FORMAT
    );

    let repeated = ods_package(
        r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="Sheet1"><table:table-row><table:table-cell table:number-columns-repeated="2" office:value-type="string"><text:p>x</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#,
    );
    let wb = ods::open_bytes(&repeated).unwrap();
    assert_eq!(cell_text(&wb, 0, 0), "x");
    assert_eq!(cell_text(&wb, 0, 1), "x");

    let mut unsupported = Workbook::new();
    let second = unsupported.add_sheet("Second").unwrap();
    unsupported.set_number(second, 0, 0, 1.0).unwrap();
    let first = unsupported.active_sheet();
    unsupported
        .set_formula_text(first, 0, 0, "=Second!A1")
        .unwrap();
    assert_eq!(
        ods::save_bytes(&unsupported).unwrap_err().code,
        codes::ODS_FORMAT
    );
}

#[test]
fn ods_reads_number_format_styles() {
    let formatted = ods_package(
        r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0"><office:automatic-styles><number:percentage-style style:name="N1"><number:number number:decimal-places="1" number:min-integer-digits="1"/><number:text>%</number:text></number:percentage-style><style:style style:name="ce1" style:family="table-cell" style:data-style-name="N1"/></office:automatic-styles><office:body><office:spreadsheet><table:table table:name="Sheet1"><table:table-row><table:table-cell table:style-name="ce1" office:value-type="percentage" office:value="0.125"/></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#,
    );
    let wb = ods::open_bytes(&formatted).unwrap();
    let slot = wb.get(wb.active_sheet(), 0, 0).unwrap().copied().unwrap();
    let style = wb.intern().styles.get(slot.style).unwrap();
    assert_eq!(wb.num_fmt_code(style.num_fmt).as_deref(), Some("0.0%"));
}

#[test]
fn ods_zip_ratio_and_save_lock_fail_closed() {
    let compressed = ods_package(&"x".repeat(1_048_576));
    assert_eq!(
        ods::open_bytes(&compressed).unwrap_err().code,
        codes::XLSX_LIMIT
    );

    let dir = tempfile_dir("save-lock");
    let path = dir.join("book.ods");
    std::fs::write(&path, b"original").unwrap();
    std::fs::write(lock_path(&path), "foreign,lock,file:///x,file:///y,now;").unwrap();
    let err = ods::save(&Workbook::new(), &path).unwrap_err();
    assert_eq!(err.code, codes::XLSX_LOCK);
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
}

#[test]
fn corpus_json_html_markdown_files() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    let json = json::open_with_pointer(&root.join("json/items.json"), Some(".items")).unwrap();
    assert_eq!(cell_text(&json, 1, 1), "Ada");
    let html = html::open_html(&root.join("html/table.html")).unwrap();
    assert_eq!(cell_text(&html, 1, 0), "Ada");
    let md = html::open_markdown(&root.join("md/table.md")).unwrap();
    assert_eq!(cell_text(&md, 1, 1), "02115");
}

#[test]
fn html_and_markdown_files_import() {
    let html = b"<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>x</td></tr></table>";
    let wb = html::open_bytes(html, ClipboardFormat::Html).unwrap();
    assert_eq!(cell_text(&wb, 0, 0), "A");
    assert_eq!(cell_text(&wb, 1, 0), "1");
    let md = b"| name | zip |\n| --- | --- |\n| Ada | 02115 |\n";
    let wb = html::open_bytes(md, ClipboardFormat::Markdown).unwrap();
    assert_eq!(cell_text(&wb, 0, 0), "name");
    assert_eq!(cell_text(&wb, 1, 1), "02115");
    let markup = html::export_html(&wb).unwrap();
    assert!(String::from_utf8_lossy(&markup).contains("<table>"));
}

#[test]
fn calc_lock_blocks_ods_open() {
    let dir = tempfile_dir("lock");
    let path = dir.join("book.ods");
    let mut wb = Workbook::new();
    wb.set_number(wb.active_sheet(), 0, 0, 1.0).unwrap();
    std::fs::write(&path, ods::save_bytes(&wb).unwrap()).unwrap();
    let lock = lock_path(&path);
    std::fs::write(
        &lock,
        "user,host,file:///config,file:///book.ods,28.08.2026 12:00;",
    )
    .unwrap();
    let err = peer_lock_blocks(&path).unwrap_err();
    assert_eq!(err.code, codes::XLSX_LOCK);
    let err = ods::open(&path).unwrap_err();
    assert_eq!(err.code, codes::XLSX_LOCK);
}

#[test]
fn native_xls_import_preserves_core_content() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/xls");

    let values = bridge::open_xls(&root.join("l1_values.xls")).unwrap();
    assert_eq!(cell_text(&values, 0, 0), "1.5");
    assert_eq!(cell_text(&values, 0, 1), "hello");
    assert_eq!(cell_text(&values, 0, 2), "TRUE");
    let date = values
        .get(values.active_sheet(), 1, 0)
        .unwrap()
        .expect("A2 date");
    assert!(matches!(date.value, omacell_core::value::Value::Number(_)));
    assert_ne!(date.style, omacell_core::style::StyleId::DEFAULT);

    let formulas = bridge::open_xls(&root.join("l1_formulas.xls")).unwrap();
    let formula = formulas
        .get(formulas.active_sheet(), 0, 2)
        .unwrap()
        .unwrap()
        .formula
        .expect("C1 formula");
    assert_eq!(formulas.intern().formulas.get(formula), Some("=A1+B1"));

    let merged = bridge::open_xls(&root.join("l2_merges_freeze.xls")).unwrap();
    let sheet = merged.sheet(merged.active_sheet()).unwrap();
    assert_eq!(sheet.merges.len(), 1);
    assert_eq!(sheet.merges[0].to_a1(), "A1:B1");

    let named = bridge::open_xls(&root.join("l2_names.xls")).unwrap();
    assert!(named.names().iter().any(|name| name.name == "Rate"));

    let hidden = bridge::open_xls(&root.join("l2_hidden_sheet.xls")).unwrap();
    assert_eq!(
        hidden.sheet_by_name("Visible").unwrap().visibility,
        omacell_core::sheet::SheetVisibility::Visible
    );
    assert_eq!(
        hidden.sheet_by_name("Hidden").unwrap().visibility,
        omacell_core::sheet::SheetVisibility::Hidden
    );
}

#[test]
fn native_xls_import_is_bounded_and_reports_parse_errors() {
    let dir = tempfile_dir("xls-limits");
    let invalid = dir.join("invalid.xls");
    std::fs::write(&invalid, b"not a BIFF workbook").unwrap();
    assert_eq!(
        bridge::open_xls(&invalid).unwrap_err().code,
        codes::XLS_BRIDGE
    );

    let oversized = dir.join("oversized.xls");
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(bridge::MAX_XLS_BYTES + 1)
        .unwrap();
    assert_eq!(
        bridge::open_xls(&oversized).unwrap_err().code,
        codes::XLSX_LIMIT
    );
}

fn ods_package(content: &str) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("content.xml", options).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

fn tempfile_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("omacell-wp27-{tag}-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

use std::path::Path;
