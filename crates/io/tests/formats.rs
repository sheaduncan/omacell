//! WP-27 corpora: ODS, JSON, HTML/Markdown, .xls bridge, locks.

use omacell_core::addr::CellRef;
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::ops;
use omacell_core::style::{Font, Style};
use omacell_core::workbook::Workbook;
use omacell_io::csv::ClipboardFormat;
use omacell_io::error::codes;
use omacell_io::xlsx::{lock_path, peer_lock_blocks};
use omacell_io::{bridge, html, json, ods};

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
    wb.set_number(sheet, 0, 0, 1.5).unwrap();
    wb.set_text(sheet, 0, 1, "Ada").unwrap();
    wb.set_formula_text(sheet, 1, 0, "=A1+1").unwrap();
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
fn xls_bridge_skips_or_round_trips_when_soffice_present() {
    let has_lo = ["soffice", "libreoffice"].into_iter().any(|bin| {
        std::process::Command::new(bin)
            .arg("--version")
            .output()
            .is_ok()
    });
    if !has_lo {
        let err = bridge::open_xls(Path::new("/tmp/missing.xls"), true).unwrap_err();
        assert_eq!(err.code, codes::XLS_BRIDGE);
        return;
    }
    let dir = tempfile_dir("xls");
    let xlsx = dir.join("book.xlsx");
    let mut wb = Workbook::new();
    wb.set_number(wb.active_sheet(), 0, 0, 3.0).unwrap();
    omacell_io::xlsx::save_workbook(
        &wb,
        &xlsx,
        omacell_io::xlsx::SaveOptions {
            keep_backups: 0,
            lock: false,
        },
    )
    .unwrap();
    let status = std::process::Command::new("soffice")
        .arg(format!(
            "-env:UserInstallation=file://{}",
            dir.join("profile").display()
        ))
        .args(["--headless", "--convert-to", "xls", "--outdir"])
        .arg(&dir)
        .arg(&xlsx)
        .status();
    let Ok(status) = status else {
        return;
    };
    if !status.success() {
        return;
    }
    let xls = dir.join("book.xls");
    if !xls.exists() {
        return;
    }
    let opened = bridge::open_xls(&xls, true).unwrap();
    assert_eq!(
        opened
            .workbook
            .get(opened.workbook.active_sheet(), 0, 0)
            .unwrap()
            .unwrap()
            .value,
        omacell_core::value::Value::Number(3.0)
    );
}

fn tempfile_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("omacell-wp27-{tag}-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

use std::path::Path;
