//! Golden PDF text/geometry and pagination parity (spec F-11.2).

use omacell_core::print::{PageSetup, paginate};
use omacell_core::workbook::Workbook;
use omacell_io::pdf::{
    PdfOptions, pdf_extract_text, pdf_has_fontfile2, pdf_media_box, pdf_page_count, write_pdf,
};
use omacell_io::xlsx::{open_bytes, save_bytes, save_workbook_bytes};

fn seed() -> Workbook {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Hello").unwrap();
    wb.set_number(s, 0, 1, 42.0).unwrap();
    for r in 1..50 {
        wb.set_number(s, r, 0, f64::from(r)).unwrap();
    }
    wb
}

#[test]
fn pdf_page_count_matches_paginator() {
    let wb = seed();
    let pages = paginate(wb.sheet(wb.active_sheet()).unwrap(), &PageSetup::default()).unwrap();
    let bytes = write_pdf(&wb, &PdfOptions::default()).unwrap();
    assert_eq!(pdf_page_count(&bytes), pages.len());
    assert!(bytes.starts_with(b"%PDF-"));
}

#[test]
fn pdf_extracted_text_includes_a1_value() {
    // A1 is the literal "Hello"; pdftotext-equivalent scan of `(...)` strings.
    let wb = seed();
    let bytes = write_pdf(&wb, &PdfOptions::default()).unwrap();
    let text = pdf_extract_text(&bytes);
    assert!(
        text.contains("Hello"),
        "expected A1 text in PDF content, got {text:?}"
    );
}

#[test]
fn pdf_media_box_is_letter_portrait() {
    let wb = seed();
    let bytes = write_pdf(&wb, &PdfOptions::default()).unwrap();
    let box_pt = pdf_media_box(&bytes).expect("MediaBox");
    assert!((box_pt.0 - 612.0).abs() < 0.5, "{box_pt:?}");
    assert!((box_pt.1 - 792.0).abs() < 0.5, "{box_pt:?}");
}

#[test]
fn helvetica_path_has_no_fontfile2() {
    let wb = seed();
    let bytes = write_pdf(&wb, &PdfOptions::default()).unwrap();
    assert!(!pdf_has_fontfile2(&bytes));
    assert!(String::from_utf8_lossy(&bytes).contains("/Helvetica"));
}

#[test]
fn ttf_embed_writes_fontfile2_when_a_face_is_present() {
    let Some(font) = system_ttf() else {
        return;
    };
    let wb = seed();
    let bytes = write_pdf(
        &wb,
        &PdfOptions {
            font_path: Some(font),
            ..PdfOptions::default()
        },
    )
    .unwrap();
    assert!(
        pdf_has_fontfile2(&bytes),
        "embedded TTF must set /FontFile2"
    );
}

#[test]
fn modeled_page_setup_round_trips_through_xlsx() {
    let mut wb = seed();
    let setup = PageSetup {
        paper: omacell_core::print::PaperSize::A4,
        orientation: omacell_core::print::Orientation::Landscape,
        gridlines: true,
        header: Some("&C&A".into()),
        row_breaks: vec![9],
        ..PageSetup::default()
    };
    wb.set_page_setup(wb.active_sheet(), setup.clone()).unwrap();
    let bytes = save_workbook_bytes(&wb).unwrap();
    let doc = open_bytes(&bytes).unwrap();
    let got = &doc
        .workbook
        .sheet(doc.workbook.active_sheet())
        .unwrap()
        .page_setup;
    assert_eq!(got.paper, setup.paper);
    assert_eq!(got.orientation, setup.orientation);
    assert!(got.gridlines);
    assert_eq!(got.header.as_deref(), Some("&C&A"));
    assert_eq!(got.row_breaks, vec![9]);
}

#[test]
fn changed_page_setup_replaces_stale_xlsx_fragments_and_names() {
    let mut wb = seed();
    let original = PageSetup {
        paper: omacell_core::print::PaperSize::A4,
        title_rows: 1,
        ..PageSetup::default()
    };
    wb.set_page_setup(wb.active_sheet(), original).unwrap();
    let original_bytes = save_workbook_bytes(&wb).unwrap();
    let mut doc = open_bytes(&original_bytes).unwrap();
    let sheet = doc.workbook.active_sheet();
    let changed = PageSetup {
        paper: omacell_core::print::PaperSize::Legal,
        title_rows: 2,
        margins: omacell_core::print::Margins {
            left: 0.25,
            ..Default::default()
        },
        ..PageSetup::default()
    };
    doc.workbook.set_page_setup(sheet, changed.clone()).unwrap();

    let saved = save_bytes(&doc).unwrap();
    let reopened = open_bytes(&saved).unwrap();
    let got = &reopened
        .workbook
        .sheet(reopened.workbook.active_sheet())
        .unwrap()
        .page_setup;
    assert_eq!(got.paper, changed.paper);
    assert_eq!(got.title_rows, 2);
    assert_eq!(got.margins.left, 0.25);
}

#[test]
fn title_columns_are_repeated_in_pdf_content() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_text(sheet, 0, 0, "REPEATED_TITLE").unwrap();
    for col in 1..30 {
        wb.set_number(sheet, 0, col, f64::from(col)).unwrap();
    }
    let setup = PageSetup {
        title_cols: 1,
        ..PageSetup::default()
    };
    wb.set_page_setup(sheet, setup.clone()).unwrap();
    let pages = paginate(wb.sheet(sheet).unwrap(), &setup).unwrap();
    assert!(pages.len() > 1);

    let bytes = write_pdf(&wb, &PdfOptions::default()).unwrap();
    assert_eq!(
        pdf_extract_text(&bytes).matches("REPEATED_TITLE").count(),
        pages.len()
    );
}

#[test]
fn oversized_font_is_rejected_before_reading_it() {
    let dir = test_scratch("oversized-font");
    let path = dir.join("oversized.ttf");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(64 * 1024 * 1024 + 1).unwrap();
    drop(file);

    let error = write_pdf(
        &seed(),
        &PdfOptions {
            font_path: Some(path),
            ..PdfOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "pdf.limit");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pdftotext_and_mutool_open_without_warnings_if_present() {
    let wb = seed();
    let bytes = write_pdf(&wb, &PdfOptions::default()).unwrap();
    let dir = test_scratch("external-tools");
    let path = dir.join("out.pdf");
    std::fs::write(&path, &bytes).unwrap();
    if which("pdftotext") {
        let out = std::process::Command::new("pdftotext")
            .args(["-layout", path.to_str().unwrap(), "-"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "pdftotext: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("Hello"), "{text}");
        assert!(out.stderr.is_empty() || String::from_utf8_lossy(&out.stderr).trim().is_empty());
    }
    if which("mutool") {
        let out = std::process::Command::new("mutool")
            .args(["info", path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "mutool: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

fn test_scratch(label: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-scratch")
        .join(format!("omacell-pdf-{}-{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .unwrap_or_default()
        .to_string_lossy()
        .split(':')
        .any(|dir| std::path::Path::new(dir).join(bin).exists())
}

fn system_ttf() -> Option<std::path::PathBuf> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
    ];
    CANDIDATES
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_file())
}
