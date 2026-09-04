//! Chart xlsx round-trip and SVG/PNG export.

use std::collections::HashSet;
use std::io::{Cursor, Read, Write};

use omacell_core::addr::{CellRef, RangeRef, SheetId};
use omacell_core::chart::{ChartKind, ChartTheme, Sparkline, SparklineKind, chart_from_range};
use omacell_core::print::{PageSetup, PaperSize};
use omacell_core::sheet::{Comment, Note};
use omacell_core::workbook::Workbook;
use omacell_io::xlsx::{diff, open_bytes, save_bytes, save_workbook_bytes};

#[path = "../../../tests/support/libreoffice.rs"]
mod libreoffice;

const REL_DRAWING: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing";
const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const REL_PRINTER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/printerSettings";
const REL_VML: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing";
const ROOT_RELATIONSHIP_BYTES: &[u8] = b"root-relationship-sentinel";

const PICTURE_ANCHOR: &str = r#"<xdr:oneCellAnchor><xdr:from><xdr:col>7</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>2</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="952500" cy="952500"/><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="900" name="Picture Sentinel"/><xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill><a:blip xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:embed="rIdImage"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic><xdr:clientData/></xdr:oneCellAnchor>"#;
const IMAGE_BYTES: &[u8] = b"png-image-sentinel";
const VML_BYTES: &[u8] =
    br#"<xml xmlns:v="urn:schemas-microsoft-com:vml"><v:shape id="vml-sentinel"/></xml>"#;
const DRAWING_COLLISION: &[u8] = br#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"><xdr:extLst><sentinel>drawing-two</sentinel></xdr:extLst></xdr:wsDr>"#;
const DRAWING_RELS_COLLISION: &[u8] = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="sentinel" Type="urn:omacell:test" Target="sentinel.bin"/></Relationships>"#;
const DRAWING_THREE_RELS_COLLISION: &[u8] = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="sentinel-three" Type="urn:omacell:test" Target="sentinel-three.bin"/></Relationships>"#;
const CHART_COLLISION: &[u8] = br#"<chart-sentinel>chart-two-one</chart-sentinel>"#;

fn rewrite_package(
    base: &[u8],
    mut rewrite: impl FnMut(&str, Vec<u8>) -> Vec<u8>,
    additions: &[(&str, &[u8])],
) -> Vec<u8> {
    let mut input = zip::ZipArchive::new(Cursor::new(base)).unwrap();
    let mut output = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut output);
        for index in 0..input.len() {
            let mut entry = input.by_index(index).unwrap();
            let name = entry.name().to_string();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            writer
                .start_file(&name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(&rewrite(&name, bytes)).unwrap();
        }
        for (name, bytes) in additions {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }
    output.into_inner()
}

fn drawing_and_print_fixture() -> Vec<u8> {
    let mut wb = seed();
    wb.set_page_setup(
        wb.active_sheet(),
        PageSetup {
            paper: PaperSize::A4,
            ..PageSetup::default()
        },
    )
    .unwrap();
    let base = save_workbook_bytes(&wb).unwrap();
    rewrite_package(
        &base,
        |name, bytes| {
            let Ok(xml) = String::from_utf8(bytes.clone()) else {
                return bytes;
            };
            match name {
                "[Content_Types].xml" => xml
                    .replace(
                        "</Types>",
                        r#"<Override PartName="/xl/media/image1.png" ContentType="image/png"/></Types>"#,
                    )
                    .into_bytes(),
                "xl/worksheets/sheet1.xml" => xml
                    .replacen(
                        "<pageSetup ",
                        r#"<pageSetup r:id="rIdPrinter" "#,
                        1,
                    )
                    .replace(
                        "</worksheet>",
                        r#"<legacyDrawing r:id="rIdVmlSource"/></worksheet>"#,
                    )
                    .into_bytes(),
                "xl/worksheets/_rels/sheet1.xml.rels" => xml
                    .replace(
                        "</Relationships>",
                        &format!(
                            r#"<Relationship Id="rIdPrinter" Type="{REL_PRINTER}" Target="../printerSettings/printerSettings1.bin"/><Relationship Id="rIdVmlSource" Type="{REL_VML}" Target="../drawings/vmlDrawing9.vml"/><Relationship Id="rIdRoot" Type="urn:omacell:test" Target="../../customXml/item1.xml"/></Relationships>"#,
                        ),
                    )
                    .into_bytes(),
                "xl/drawings/drawing1.xml" => xml
                    .replace("</xdr:wsDr>", &format!("{PICTURE_ANCHOR}</xdr:wsDr>"))
                    .into_bytes(),
                "xl/drawings/_rels/drawing1.xml.rels" => xml
                    .replace(
                        "</Relationships>",
                        &format!(
                            r#"<Relationship Id="rIdImage" Type="{REL_IMAGE}" Target="../media/image1.png"/></Relationships>"#,
                        ),
                    )
                    .into_bytes(),
                _ => bytes,
            }
        },
        &[
            (
                "xl/printerSettings/printerSettings1.bin",
                b"printer-settings-sentinel",
            ),
            ("xl/drawings/vmlDrawing9.vml", VML_BYTES),
            ("xl/media/image1.png", IMAGE_BYTES),
            ("customXml/item1.xml", ROOT_RELATIONSHIP_BYTES),
        ],
    )
}

fn collision_fixture() -> Vec<u8> {
    let base = save_workbook_bytes(&seed()).unwrap();
    rewrite_package(
        &base,
        |name, bytes| {
            if name != "[Content_Types].xml" {
                return bytes;
            }
            String::from_utf8(bytes)
                .unwrap()
                .replace(
                    "</Types>",
                    r#"<Override PartName="/xl/drawings/drawing2.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/><Override PartName="/xl/charts/chart2_1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/></Types>"#,
                )
                .into_bytes()
        },
        &[
            ("xl/drawings/drawing2.xml", DRAWING_COLLISION),
            (
                "xl/drawings/_rels/drawing2.xml.rels",
                DRAWING_RELS_COLLISION,
            ),
            (
                "xl/drawings/_rels/drawing3.xml.rels",
                DRAWING_THREE_RELS_COLLISION,
            ),
            ("xl/charts/chart2_1.xml", CHART_COLLISION),
        ],
    )
}

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
fn modeled_chart_keeps_picture_vml_and_printer_relationships() {
    let original = open_bytes(&drawing_and_print_fixture()).unwrap();
    let saved = open_bytes(&save_bytes(&original).unwrap()).unwrap();
    let sheet_rels = saved.package.rels_for("xl/worksheets/sheet1.xml").unwrap();
    let printer = sheet_rels
        .iter()
        .find(|rel| rel.rel_type == REL_PRINTER)
        .unwrap();
    assert_eq!(printer.id, "rIdPrinter");
    assert!(sheet_rels.iter().any(|relationship| {
        relationship.id == "rIdRoot"
            && relationship.target == "customXml/item1.xml"
            && relationship.rel_type == "urn:omacell:test"
    }));
    let sheet_xml = String::from_utf8(
        saved
            .package
            .part("xl/worksheets/sheet1.xml")
            .unwrap()
            .bytes
            .clone(),
    )
    .unwrap();
    assert!(sheet_xml.contains(r#"<pageSetup r:id="rIdPrinter" "#));
    assert!(sheet_xml.contains(r#"<legacyDrawing r:id="rIdVmlSource"/>"#));
    assert_eq!(
        saved
            .package
            .part("xl/drawings/vmlDrawing9.vml")
            .unwrap()
            .bytes,
        VML_BYTES
    );

    let drawing = sheet_rels
        .iter()
        .find(|rel| rel.rel_type == REL_DRAWING)
        .unwrap();
    let drawing_xml =
        String::from_utf8(saved.package.part(&drawing.target).unwrap().bytes.clone()).unwrap();
    assert!(drawing_xml.contains("Picture Sentinel"), "{drawing_xml}");
    let drawing_rels = saved.package.rels_for(&drawing.target).unwrap();
    assert!(
        drawing_rels
            .iter()
            .any(|rel| rel.id == "rIdImage" && rel.rel_type == REL_IMAGE)
    );
    assert_eq!(
        saved.package.part("xl/media/image1.png").unwrap().bytes,
        IMAGE_BYTES
    );
    assert_eq!(
        saved.package.part("customXml/item1.xml").unwrap().bytes,
        ROOT_RELATIONSHIP_BYTES
    );
}

#[test]
fn generated_chart_names_do_not_replace_preserved_parts() {
    let mut opened = open_bytes(&collision_fixture()).unwrap();
    let sheet = opened.workbook.add_sheet("Second").unwrap();
    opened.workbook.set_text(sheet, 0, 0, "Cat").unwrap();
    opened.workbook.set_text(sheet, 0, 1, "Value").unwrap();
    opened.workbook.set_number(sheet, 1, 0, 1.0).unwrap();
    opened.workbook.set_number(sheet, 1, 1, 2.0).unwrap();
    let chart = chart_from_range(
        &opened.workbook,
        sheet,
        RangeRef::from_corners(
            CellRef::new(0, 0).unwrap().on_sheet(sheet),
            CellRef::new(1, 1).unwrap().on_sheet(sheet),
        ),
        ChartKind::Line,
        Some("Second".into()),
    )
    .unwrap();
    opened.workbook.add_chart(chart).unwrap();
    let third = opened.workbook.add_sheet("Third").unwrap();
    opened.workbook.set_text(third, 0, 0, "Cat").unwrap();
    opened.workbook.set_text(third, 0, 1, "Value").unwrap();
    opened.workbook.set_number(third, 1, 0, 1.0).unwrap();
    opened.workbook.set_number(third, 1, 1, 3.0).unwrap();
    let chart = chart_from_range(
        &opened.workbook,
        third,
        RangeRef::from_corners(
            CellRef::new(0, 0).unwrap().on_sheet(third),
            CellRef::new(1, 1).unwrap().on_sheet(third),
        ),
        ChartKind::Column,
        Some("Third".into()),
    )
    .unwrap();
    opened.workbook.add_chart(chart).unwrap();

    let saved = open_bytes(&save_bytes(&opened).unwrap()).unwrap();
    assert_eq!(
        saved
            .package
            .part("xl/drawings/drawing2.xml")
            .unwrap()
            .bytes,
        DRAWING_COLLISION
    );
    assert_eq!(
        saved
            .package
            .part("xl/drawings/_rels/drawing2.xml.rels")
            .unwrap()
            .bytes,
        DRAWING_RELS_COLLISION
    );
    assert_eq!(
        saved
            .package
            .part("xl/drawings/_rels/drawing3.xml.rels")
            .unwrap()
            .bytes,
        DRAWING_THREE_RELS_COLLISION
    );
    assert_eq!(
        saved.package.part("xl/charts/chart2_1.xml").unwrap().bytes,
        CHART_COLLISION
    );
    let second_drawing = saved
        .package
        .rels_for("xl/worksheets/sheet2.xml")
        .unwrap()
        .into_iter()
        .find(|rel| rel.rel_type == REL_DRAWING)
        .unwrap();
    assert_ne!(second_drawing.target, "xl/drawings/drawing2.xml");
    assert!(saved.package.part(&second_drawing.target).is_some());
    let third_drawing = saved
        .package
        .rels_for("xl/worksheets/sheet3.xml")
        .unwrap()
        .into_iter()
        .find(|rel| rel.rel_type == REL_DRAWING)
        .unwrap();
    assert_ne!(third_drawing.target, "xl/drawings/drawing3.xml");
    assert!(saved.package.part(&third_drawing.target).is_some());
}

#[test]
fn generated_worksheet_relationship_ids_are_unique() {
    let mut wb = seed();
    let sheet = wb.active_sheet();
    wb.set_note(
        sheet,
        4,
        0,
        Some(Note {
            author: Some("Ada".into()),
            text: "legacy".into(),
        }),
    )
    .unwrap();
    wb.set_comment(
        sheet,
        5,
        0,
        Some(Comment {
            author: "Lin".into(),
            text: "threaded".into(),
            replies: Vec::new(),
            resolved: false,
        }),
    )
    .unwrap();
    let saved = open_bytes(&save_workbook_bytes(&wb).unwrap()).unwrap();
    let mut ids = HashSet::new();
    for rel in saved.package.rels_for("xl/worksheets/sheet1.xml").unwrap() {
        assert!(
            ids.insert(rel.id.clone()),
            "duplicate relationship {}",
            rel.id
        );
    }
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
    let saved = save_bytes(&opened).unwrap();
    let again = open_bytes(&saved).unwrap();
    let worksheet = String::from_utf8(
        again
            .package
            .part("xl/worksheets/sheet1.xml")
            .unwrap()
            .bytes
            .clone(),
    )
    .unwrap();
    assert!(worksheet.contains(
        r#"<extLst><ext uri="{05C60535-1F16-4FD2-B633-F4F36F0B64E0}"><x14:sparklineGroups"#
    ));
    assert!(worksheet.contains("</x14:sparklineGroups></ext></extLst></worksheet>"));
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
