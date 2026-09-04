//! Golden PDF text/geometry and pagination parity (spec F-11.2).

use omacell_core::print::{PageSetup, PrintTitleBand, paginate};
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
fn helvetica_declares_and_uses_win_ansi_for_supported_unicode() {
    let mut wb = Workbook::new();
    wb.set_text(wb.active_sheet(), 0, 0, "café € —").unwrap();
    let bytes = write_pdf(&wb, &PdfOptions::default()).unwrap();
    let source = String::from_utf8_lossy(&bytes);

    assert!(source.contains("/Encoding /WinAnsiEncoding"));
    assert!(source.contains("<636166E920802097>"), "{source}");
    if let Some(text) = pdftotext_output(&bytes, "helvetica-winansi") {
        assert!(text.contains("café € —"), "{text:?}");
    }
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
fn embedded_font_tounicode_covers_supported_non_ascii_glyphs() {
    let Some(font) = system_ttf() else {
        return;
    };
    let font_bytes = std::fs::read(&font).unwrap();
    let face = ttf_parser::Face::parse(&font_bytes, 0).unwrap();
    let mut wb = Workbook::new();
    wb.set_text(wb.active_sheet(), 0, 0, "café Ω").unwrap();
    let bytes = write_pdf(
        &wb,
        &PdfOptions {
            font_path: Some(font),
            ..PdfOptions::default()
        },
    )
    .unwrap();
    let source = String::from_utf8_lossy(&bytes);

    for ch in ['é', 'Ω'] {
        let Some(glyph) = face.glyph_index(ch) else {
            continue;
        };
        let mapping = format!("<{:04X}> <{:04X}>", glyph.0, u32::from(ch));
        assert!(
            source.contains(&mapping),
            "missing {ch:?} mapping {mapping}"
        );
    }
    if let Some(text) = pdftotext_output(&bytes, "embedded-tounicode") {
        assert!(text.contains("café Ω"), "{text:?}");
    }
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
    assert_eq!(
        got.row_title_band(0),
        Some(PrintTitleBand { start: 0, end: 1 })
    );
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
fn non_origin_title_rows_repeat_once_per_pdf_page_and_round_trip() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_text(sheet, 0, 0, "LEADING_DATA").unwrap();
    wb.set_text(sheet, 2, 0, "NON_ORIGIN_TITLE").unwrap();
    for row in 0..100 {
        wb.set_number(sheet, row, 1, f64::from(row)).unwrap();
    }
    let setup = PageSetup {
        title_row_band: Some(PrintTitleBand { start: 2, end: 2 }),
        ..PageSetup::default()
    };
    wb.set_page_setup(sheet, setup.clone()).unwrap();
    let pages = paginate(wb.sheet(sheet).unwrap(), &setup).unwrap();
    assert!(pages.len() > 1);
    let pdf = write_pdf(&wb, &PdfOptions::default()).unwrap();
    let text = pdf_extract_text(&pdf);
    assert!(text.contains("LEADING_DATA"));
    assert_eq!(text.matches("NON_ORIGIN_TITLE").count(), pages.len());

    let xlsx = save_workbook_bytes(&wb).unwrap();
    let reopened = open_bytes(&xlsx).unwrap();
    assert_eq!(
        reopened
            .workbook
            .sheet(reopened.workbook.active_sheet())
            .unwrap()
            .page_setup
            .title_row_band,
        setup.title_row_band
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

fn pdftotext_output(bytes: &[u8], label: &str) -> Option<String> {
    if !which("pdftotext") {
        return None;
    }
    let dir = test_scratch(label);
    let path = dir.join("out.pdf");
    std::fs::write(&path, bytes).unwrap();
    let output = std::process::Command::new("pdftotext")
        .args(["-layout", path.to_str().unwrap(), "-"])
        .output()
        .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    assert!(
        output.status.success(),
        "pdftotext: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty() || String::from_utf8_lossy(&output.stderr).trim().is_empty());
    Some(String::from_utf8(output.stdout).unwrap())
}

fn system_ttf() -> Option<std::path::PathBuf> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
    ];
    CANDIDATES
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_file())
}
