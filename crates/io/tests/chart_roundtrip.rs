//! Chart xlsx round-trip and SVG/PNG export.

use omacell_core::addr::{CellRef, RangeRef, SheetId};
use omacell_core::chart::{ChartKind, ChartTheme, chart_from_range};
use omacell_core::workbook::Workbook;
use omacell_io::xlsx::{diff, open_bytes, save_workbook_bytes};

fn seed() -> Workbook {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Cat").unwrap();
    wb.set_text(s, 0, 1, "East").unwrap();
    wb.set_number(s, 1, 0, 1.0).unwrap();
    wb.set_number(s, 1, 1, 10.0).unwrap();
    wb.set_number(s, 2, 0, 2.0).unwrap();
    wb.set_number(s, 2, 1, 20.0).unwrap();
    let chart = chart_from_range(
        &wb,
        s,
        RangeRef::from_corners(
            CellRef::new(0, 0).unwrap().on_sheet(SheetId::new(0)),
            CellRef::new(2, 1).unwrap().on_sheet(SheetId::new(0)),
        ),
        ChartKind::Column,
        Some("Sales".into()),
    )
    .unwrap();
    wb.add_chart(chart).unwrap();
    wb
}

#[test]
fn modeled_chart_round_trips_xlsx() {
    let wb = seed();
    let bytes = save_workbook_bytes(&wb).unwrap();
    let doc = open_bytes(&bytes).unwrap();
    assert!(
        !doc.workbook
            .sheet(doc.workbook.active_sheet())
            .unwrap()
            .charts
            .is_empty(),
        "chart missing after open"
    );
    let charts = &doc
        .workbook
        .sheet(doc.workbook.active_sheet())
        .unwrap()
        .charts;
    assert_eq!(charts.len(), 1);
    assert_eq!(charts[0].title.as_deref(), Some("Sales"));
    assert!(!charts[0].series.is_empty());
    let again = save_workbook_bytes(&doc.workbook).unwrap();
    let doc2 = open_bytes(&again).unwrap();
    assert_eq!(
        doc2.workbook
            .sheet(doc2.workbook.active_sheet())
            .unwrap()
            .charts
            .len(),
        1
    );
}

#[test]
fn chart_svg_and_png_export() {
    let wb = seed();
    let chart = wb.sheet(wb.active_sheet()).unwrap().charts[0].clone();
    let theme = ChartTheme::neutral();
    let svg = omacell_io::chart_export::chart_svg(&wb, &chart, &theme, 320.0, 200.0).unwrap();
    assert!(svg.contains("<svg"));
    let png = omacell_io::chart_export::chart_png(&wb, &chart, &theme, 320, 200).unwrap();
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
}

#[test]
fn libreoffice_opens_modeled_chart_if_present() {
    let Some(soffice) = ["soffice", "libreoffice"]
        .into_iter()
        .find(|bin| which(bin))
    else {
        return;
    };
    let wb = seed();
    let bytes = save_workbook_bytes(&wb).unwrap();
    let dir = std::env::temp_dir().join(format!("omacell-chart-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("chart.xlsx");
    std::fs::write(&path, bytes).unwrap();
    let out = std::process::Command::new(soffice)
        .args([
            "--headless",
            "--convert-to",
            "pdf",
            "--outdir",
            dir.to_str().unwrap(),
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "LibreOffice failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .unwrap_or_default()
        .to_string_lossy()
        .split(':')
        .any(|dir| std::path::Path::new(dir).join(bin).exists())
}

#[test]
fn unsupported_drawing_parts_stay_on_package() {
    let doc = omacell_io::xlsx::open(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/xlsx/l2_print.xlsx"
    )))
    .unwrap();
    let bytes = omacell_io::xlsx::save_bytes(&doc).unwrap();
    let again = omacell_io::xlsx::open_bytes(&bytes).unwrap();
    assert!(
        diff(&doc, &again).empty,
        "corpus without modeled charts must stay L3-identical"
    );
}
