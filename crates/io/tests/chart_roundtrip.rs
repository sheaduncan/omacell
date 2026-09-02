//! Chart xlsx round-trip and SVG/PNG export.

use omacell_core::addr::{CellRef, RangeRef, SheetId};
use omacell_core::chart::{ChartKind, ChartTheme, Sparkline, SparklineKind, chart_from_range};
use omacell_core::workbook::Workbook;
use omacell_io::xlsx::{diff, open_bytes, save_bytes, save_workbook_bytes};

#[path = "../../../tests/support/libreoffice.rs"]
mod libreoffice;

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
    assert_eq!(charts[0].kind, ChartKind::Column);
    assert_eq!(charts[0].title.as_deref(), Some("Sales"));
    assert_eq!(charts[0].series.len(), 1);
    assert_eq!(charts[0].series[0].name, "East");
    assert_eq!(charts[0].anchor.from_row, 1);
    assert_eq!(charts[0].anchor.to_col, 12);
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
fn every_modeled_chart_kind_keeps_its_kind() {
    let kinds = [
        ChartKind::Line,
        ChartKind::Column,
        ChartKind::Bar,
        ChartKind::ColumnStacked,
        ChartKind::BarStacked,
        ChartKind::ColumnPct,
        ChartKind::BarPct,
        ChartKind::Area,
        ChartKind::Pie,
        ChartKind::Donut,
        ChartKind::Scatter,
        ChartKind::Bubble,
        ChartKind::Combo,
        ChartKind::Histogram,
    ];
    for kind in kinds {
        let mut wb = seed();
        let sheet = wb.active_sheet();
        let existing = wb.sheet(sheet).unwrap().charts[0].id;
        wb.remove_chart(sheet, existing).unwrap();
        let chart = chart_from_range(
            &wb,
            sheet,
            RangeRef::from_corners(
                CellRef::new(0, 0).unwrap().on_sheet(sheet),
                CellRef::new(2, 1).unwrap().on_sheet(sheet),
            ),
            kind,
            Some(kind.as_str().into()),
        )
        .unwrap();
        wb.add_chart(chart).unwrap();
        let opened = open_bytes(&save_workbook_bytes(&wb).unwrap()).unwrap();
        assert_eq!(
            opened
                .workbook
                .sheet(opened.workbook.active_sheet())
                .unwrap()
                .charts[0]
                .kind,
            kind,
            "{kind:?}"
        );
    }
}

#[test]
fn sparkline_added_to_opened_workbook_is_written_and_read() {
    let original = save_workbook_bytes(&seed()).unwrap();
    let mut opened = open_bytes(&original).unwrap();
    let sheet = opened.workbook.active_sheet();
    opened
        .workbook
        .add_sparkline(Sparkline {
            kind: SparklineKind::WinLoss,
            data: RangeRef::from_corners(
                CellRef::new(1, 1).unwrap().on_sheet(sheet),
                CellRef::new(2, 1).unwrap().on_sheet(sheet),
            ),
            row: 1,
            col: 3,
            sheet,
        })
        .unwrap();
    let again = open_bytes(&save_bytes(&opened).unwrap()).unwrap();
    let sparklines = &again
        .workbook
        .sheet(again.workbook.active_sheet())
        .unwrap()
        .sparklines;
    assert_eq!(sparklines.len(), 1);
    assert_eq!(sparklines[0].kind, SparklineKind::WinLoss);
    assert_eq!((sparklines[0].row, sparklines[0].col), (1, 3));
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
fn svg_escapes_workbook_control_and_attribute_text() {
    let wb = seed();
    let sheet = wb.active_sheet();
    let mut chart = wb.sheet(sheet).unwrap().charts[0].clone();
    chart.title = Some("<bad>\0title".into());
    chart.series[0].color = Some("#fff\" onload=\"alert(1)".into());
    let svg =
        omacell_io::chart_export::chart_svg(&wb, &chart, &ChartTheme::neutral(), 320.0, 200.0)
            .unwrap();
    assert!(svg.contains("&lt;bad&gt;�title"));
    assert!(!svg.contains(" onload="));
}

#[test]
fn png_dimensions_are_bounded() {
    let wb = seed();
    let chart = wb.sheet(wb.active_sheet()).unwrap().charts[0].clone();
    let error =
        omacell_io::chart_export::chart_png(&wb, &chart, &ChartTheme::neutral(), 100_000, 100_000)
            .unwrap_err();
    assert_eq!(error.code, "chart.export");
}

#[test]
fn libreoffice_opens_modeled_chart_if_present() {
    let Some(soffice) = libreoffice::find_calc() else {
        return;
    };
    let wb = seed();
    let bytes = save_workbook_bytes(&wb).unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-tmp")
        .join(format!("omacell-chart-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let dir = dir.canonicalize().unwrap();
    let path = dir.join("chart.xlsx");
    let profile = dir.join("libreoffice-profile");
    std::fs::write(&path, bytes).unwrap();
    let out = std::process::Command::new(&soffice)
        .arg(format!(
            "-env:UserInstallation=file://{}",
            profile.display()
        ))
        .env("HOME", &dir)
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("SAL_USE_VCLPLUGIN", "svp")
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
    let failure = format!(
        "LibreOffice failed with {}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{failure}");
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
    let report = diff(&doc, &again);
    assert!(
        report.empty,
        "corpus without modeled charts must stay L3-identical: {report:?}"
    );
}
