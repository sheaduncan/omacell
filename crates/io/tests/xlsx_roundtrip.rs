//! Open → save → open; L1/L2 diff empty. External loaders skip if absent.

use std::path::PathBuf;
use std::process::Command;

use calamine::Reader;
use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::intern::RichTextRun;
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::sheet::{Comment, Hyperlink, Note, ProtectionState};
use omacell_core::style::{Color, Fill, Font, GradientFill, GradientKind, GradientStop, Style};
use omacell_core::tables::{Table, TableId};
use omacell_core::workbook::Workbook;
use omacell_io::xlsx::{
    SaveOptions, diff, open, open_bytes, save, save_bytes, save_workbook_bytes,
};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/xlsx")
}

fn xlsx_files() -> Vec<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(corpus_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("xlsx"))
        .collect();
    files.sort();
    files
}

#[test]
fn roundtrip_diff_empty_for_corpus() {
    for path in xlsx_files() {
        let doc = open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let bytes = save_bytes(&doc).unwrap_or_else(|e| panic!("save {}: {e}", path.display()));
        let again = open_bytes(&bytes).unwrap_or_else(|e| panic!("reopen {}: {e}", path.display()));
        let report = diff(&doc, &again);
        assert!(
            report.empty,
            "{}: {report:?}",
            path.file_name().unwrap().to_string_lossy()
        );
    }
}

#[test]
fn saved_file_loads_in_calamine() {
    let path = corpus_dir().join("l1_values.xlsx");
    let doc = open(&path).unwrap();
    let bytes = save_bytes(&doc).unwrap();
    let tmp = std::env::temp_dir().join(format!("omacell-rt-calamine-{}.xlsx", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();
    let mut cal = calamine::open_workbook::<calamine::Xlsx<_>, _>(&tmp).unwrap();
    let range = cal.worksheet_range("Sheet1").unwrap();
    assert_eq!(range.get((0, 0)), Some(&calamine::Data::Float(1.5)));
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn saved_file_loads_in_openpyxl_if_present() {
    let path = corpus_dir().join("l1_values.xlsx");
    let doc = open(&path).unwrap();
    let bytes = save_bytes(&doc).unwrap();
    let tmp = std::env::temp_dir().join(format!("omacell-rt-py-{}.xlsx", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();
    let output = Command::new("python3")
        .args([
            "-c",
            &format!(
                "import openpyxl; openpyxl.load_workbook(r'{}')",
                tmp.display()
            ),
        ])
        .output();
    let _ = std::fs::remove_file(&tmp);
    match output {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            if err.contains("ModuleNotFoundError") || err.contains("ImportError") {
                return;
            }
            panic!("openpyxl load failed: {err}");
        }
        Err(_) => {}
    }
}

#[test]
fn saved_file_converts_in_libreoffice_if_present() {
    let path = corpus_dir().join("l1_values.xlsx");
    let doc = open(&path).unwrap();
    let bytes = save_bytes(&doc).unwrap();
    let dir = std::env::temp_dir().join(format!("omacell-rt-lo-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let tmp = dir.join("in.xlsx");
    std::fs::write(&tmp, &bytes).unwrap();
    let soffice = ["soffice", "libreoffice"]
        .iter()
        .find(|b| Command::new(b).arg("--version").output().is_ok());
    if soffice.is_none() {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let profile = dir.join("lo-profile");
    let _ = std::fs::create_dir_all(&profile);
    let profile_uri = format!("file://{}", profile.display());
    let out = Command::new(soffice.unwrap())
        .args([
            "--headless",
            &format!("-env:UserInstallation={profile_uri}"),
            "--convert-to",
            "csv",
            "--outdir",
            dir.to_str().unwrap(),
            tmp.to_str().unwrap(),
        ])
        .output();
    let csvs = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("csv"));
    if !csvs {
        let detail = out
            .ok()
            .map(|o| {
                format!(
                    "status={:?} stderr={}",
                    o.status.code(),
                    String::from_utf8_lossy(&o.stderr)
                )
            })
            .unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        panic!("LibreOffice conversion produced no CSV: {detail}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_to_path_roundtrip() {
    let path = corpus_dir().join("l1_formulas.xlsx");
    let doc = open(&path).unwrap();
    let tmp = std::env::temp_dir().join(format!("omacell-rt-save-{}.xlsx", std::process::id()));
    save(
        &doc,
        &tmp,
        SaveOptions {
            keep_backups: 0,
            lock: false,
        },
    )
    .unwrap();
    let again = open(&tmp).unwrap();
    assert!(diff(&doc, &again).empty);
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn new_workbook_save_bytes_reopens() {
    let mut wb = Workbook::new();
    let id = wb.active_sheet();
    wb.set_number(id, 0, 0, 42.0).unwrap();
    let bytes = save_workbook_bytes(&wb).unwrap();
    let doc = open_bytes(&bytes).unwrap();
    let slot = doc.workbook.get(doc.workbook.active_sheet(), 0, 0).unwrap();
    assert!(matches!(
        slot.unwrap().value,
        omacell_core::value::Value::Number(n) if n == 42.0
    ));
}

#[test]
fn new_workbook_preserves_modeled_l1_l2_fields() {
    let mut wb = Workbook::new();
    let first = wb.active_sheet();
    let second = wb.add_sheet("Summary").unwrap();
    wb.set_active_sheet(second).unwrap();
    wb.set_tab_color(first, Some(Color::Rgb { argb: 0xFF11_2233 }))
        .unwrap();
    wb.set_row_height(first, 4, 31).unwrap();
    wb.set_row_hidden(first, 5, true).unwrap();
    wb.set_col_width(first, 2, 123).unwrap();
    wb.set_col_hidden(first, 3, true).unwrap();

    let mut view = wb.sheet(first).unwrap().view.clone();
    view.zoom = 1.25;
    view.scroll_row = 7;
    view.scroll_col = 2;
    view.selection =
        RangeRef::from_corners(CellRef::new(2, 1).unwrap(), CellRef::new(3, 2).unwrap());
    wb.set_sheet_view(first, view.clone()).unwrap();
    wb.set_sheet_protection(
        first,
        ProtectionState {
            enabled: true,
            password: Some(b"ABCD".to_vec()),
        },
    )
    .unwrap();

    let rich_font = Font {
        name: "Fira Sans".into(),
        bold: true,
        color: Color::Rgb { argb: 0xFFAA_1100 },
        ..Font::default()
    };
    wb.set_rich_text(
        first,
        0,
        0,
        "Bold plain",
        vec![RichTextRun {
            start: 0,
            len: 4,
            font: rich_font.clone(),
        }],
    )
    .unwrap();
    wb.set_formula_text(first, 1, 0, r#"="cached""#).unwrap();
    let cached = wb.intern_text("cached");
    let mut formula_slot = *wb.get(first, 1, 0).unwrap().unwrap();
    formula_slot.value = omacell_core::value::Value::Text(cached);
    wb.set_slot(first, 1, 0, formula_slot).unwrap();
    wb.release_text(cached);
    wb.set_text(first, 6, 0, "control\u{1} literal_x0002_")
        .unwrap();

    let gradient = GradientFill {
        kind: GradientKind::Path,
        left: 0.1,
        right: 0.2,
        top: 0.3,
        bottom: 0.4,
        stops: vec![
            GradientStop {
                position: 0.0,
                color: Color::Theme {
                    theme: 2,
                    tint: 0.25,
                },
            },
            GradientStop {
                position: 1.0,
                color: Color::Rgb { argb: 0xFF00_00FF },
            },
        ]
        .into(),
        ..GradientFill::default()
    };
    wb.set_number(first, 2, 0, 1.0).unwrap();
    wb.set_cell_style(
        first,
        2,
        0,
        Style {
            fill: Fill::Gradient(gradient.clone()),
            ..Style::default()
        },
    )
    .unwrap();

    wb.set_hyperlink(
        first,
        3,
        0,
        Some(Hyperlink {
            target: "Summary!A1".into(),
            tooltip: Some("Jump to summary".into()),
            display: Some("Summary".into()),
        }),
    )
    .unwrap();
    wb.set_note(
        first,
        4,
        0,
        Some(Note {
            author: Some("Ada".into()),
            text: "Check this".into(),
        }),
    )
    .unwrap();
    wb.set_note(
        first,
        5,
        0,
        Some(Note {
            author: None,
            text: "Anonymous".into(),
        }),
    )
    .unwrap();
    wb.define_name(DefinedName {
        name: "TaxRate".into(),
        scope: NameScope::Workbook,
        referent: NameReferent::Constant(omacell_core::value::Value::Number(0.2)),
        comment: Some("Current tax rate".into()),
    })
    .unwrap();
    let mut table = Table::new(TableId::new(0), "DataTable", first, 10, 0, 12, 1);
    table.banded_rows = false;
    table.banded_cols = true;
    wb.add_table(table).unwrap();
    let table_two = Table::new(TableId::new(0), "SummaryTable", second, 0, 0, 1, 0);
    wb.add_table(table_two).unwrap();
    wb.custom_parts
        .insert("xl/omacell/state.json".into(), br#"{"version":1}"#.to_vec());

    let bytes = save_workbook_bytes(&wb).unwrap();
    let doc = open_bytes(&bytes).unwrap();
    let loaded = &doc.workbook;
    assert_eq!(loaded.sheet(loaded.active_sheet()).unwrap().name, "Summary");
    let sheet = loaded.sheet_by_name("Sheet1").unwrap();
    assert_eq!(sheet.tab_color, Some(Color::Rgb { argb: 0xFF11_2233 }));
    assert_eq!(sheet.view, view);
    assert_eq!(
        sheet.protection,
        ProtectionState {
            enabled: true,
            password: Some(b"ABCD".to_vec()),
        }
    );
    assert_eq!(sheet.geometry.rows.size(4).unwrap(), 31);
    assert!(sheet.geometry.rows.is_hidden(5).unwrap());
    assert_eq!(sheet.geometry.cols.size(2).unwrap(), 123);
    assert!(sheet.geometry.cols.is_hidden(3).unwrap());
    let rich_slot = loaded.get(sheet.id, 0, 0).unwrap().unwrap();
    let omacell_core::value::Value::Text(rich_id) = rich_slot.value else {
        panic!("rich text cell did not reopen as text");
    };
    let runs = loaded.intern().strings.get_rich(rich_id).unwrap();
    assert_eq!(runs[0].font, rich_font);
    let formula = loaded.get(sheet.id, 1, 0).unwrap().unwrap();
    assert!(formula.formula.is_some());
    let omacell_core::value::Value::Text(cached_id) = formula.value else {
        panic!("formula cache did not reopen as text");
    };
    assert_eq!(loaded.intern().strings.get(cached_id), Some("cached"));
    let escaped_text = loaded.get(sheet.id, 6, 0).unwrap().unwrap();
    let omacell_core::value::Value::Text(escaped_id) = escaped_text.value else {
        panic!("escaped text cell did not reopen as text");
    };
    assert_eq!(
        loaded.intern().strings.get(escaped_id),
        Some("control\u{1} literal_x0002_")
    );
    let gradient_slot = loaded.get(sheet.id, 2, 0).unwrap().unwrap();
    assert_eq!(
        loaded
            .intern()
            .styles
            .get(gradient_slot.style)
            .unwrap()
            .fill,
        Fill::Gradient(gradient)
    );
    assert_eq!(
        sheet.hyperlinks.get(&(3, 0)).unwrap(),
        &Hyperlink {
            target: "Summary!A1".into(),
            tooltip: Some("Jump to summary".into()),
            display: Some("Summary".into()),
        }
    );
    assert_eq!(
        sheet.notes.get(&(4, 0)).unwrap(),
        &Note {
            author: Some("Ada".into()),
            text: "Check this".into(),
        }
    );
    assert_eq!(sheet.notes.get(&(5, 0)).unwrap().author, None);
    assert!(doc.package.part("xl/drawings/vmlDrawing1.vml").is_some());
    assert!(
        doc.package
            .rels_for("xl/worksheets/sheet1.xml")
            .unwrap()
            .iter()
            .any(|rel| rel.rel_type.ends_with("/vmlDrawing"))
    );
    let name = loaded
        .names()
        .iter()
        .find(|name| name.name == "TaxRate")
        .unwrap();
    assert_eq!(name.comment.as_deref(), Some("Current tax rate"));
    assert!(matches!(
        name.referent,
        NameReferent::Constant(omacell_core::value::Value::Number(value)) if value == 0.2
    ));
    assert!(
        !loaded
            .tables()
            .get_by_name("DataTable")
            .unwrap()
            .banded_rows
    );
    assert!(
        loaded
            .tables()
            .get_by_name("DataTable")
            .unwrap()
            .banded_cols
    );
    assert!(doc.package.part("xl/tables/table1.xml").is_some());
    assert!(doc.package.part("xl/tables/table2.xml").is_some());
    assert_eq!(
        loaded.custom_parts.get("xl/omacell/state.json"),
        Some(&br#"{"version":1}"#.to_vec())
    );
    let again = open_bytes(&save_bytes(&doc).unwrap()).unwrap();
    assert!(diff(&doc, &again).empty, "{:?}", diff(&doc, &again));
}

#[test]
fn diff_is_symmetric_and_detects_added_content() {
    let original = open(&corpus_dir().join("l1_values.xlsx")).unwrap();
    let mut changed = original.clone();
    let sheet = changed.workbook.active_sheet();
    changed.workbook.set_number(sheet, 100, 4, 99.0).unwrap();
    changed
        .workbook
        .custom_parts
        .insert("xl/omacell/extra.json".into(), b"{}".to_vec());
    assert!(!diff(&original, &changed).empty);
    assert!(!diff(&changed, &original).empty);
}

#[test]
fn macro_workbook_content_type_is_preserved() {
    const MACRO_CT: &str = "application/vnd.ms-excel.sheet.macroEnabled.main+xml";
    let mut doc = open(&corpus_dir().join("l1_values.xlsx")).unwrap();
    doc.package
        .parts
        .get_mut("xl/workbook.xml")
        .unwrap()
        .content_type = Some(MACRO_CT.into());
    let bytes = save_bytes(&doc).unwrap();
    let reopened = open_bytes(&bytes).unwrap();
    assert_eq!(
        reopened
            .package
            .workbook_part()
            .unwrap()
            .content_type
            .as_deref(),
        Some(MACRO_CT)
    );
}

#[test]
fn custom_parts_are_confined_to_the_omacell_namespace() {
    let mut wb = Workbook::new();
    wb.custom_parts
        .insert("../xl/workbook.xml".into(), b"malicious".to_vec());
    assert!(save_workbook_bytes(&wb).is_err());
}

#[test]
fn injected_worksheet_fragments_are_rejected() {
    let mut doc = open(&corpus_dir().join("l1_values.xlsx")).unwrap();
    doc.extras
        .entry("Sheet1".into())
        .or_default()
        .conditional_formatting_xml
        .push(b"</worksheet><evil/>".to_vec());
    assert!(save_bytes(&doc).is_err());

    let mut declared = open(&corpus_dir().join("l1_values.xlsx")).unwrap();
    declared
        .extras
        .entry("Sheet1".into())
        .or_default()
        .conditional_formatting_xml
        .push(b"<?xml version=\"1.0\"?><conditionalFormatting/>".to_vec());
    assert!(save_bytes(&declared).is_err());
}

#[test]
fn threaded_comments_are_reported_and_not_silently_dropped() {
    let original = open(&corpus_dir().join("l1_values.xlsx")).unwrap();
    let mut changed = original.clone();
    changed
        .workbook
        .set_comment(
            changed.workbook.active_sheet(),
            0,
            0,
            Some(Comment {
                author: "Ada".into(),
                text: "review".into(),
                replies: Vec::new(),
            }),
        )
        .unwrap();
    assert!(!diff(&original, &changed).empty);
    assert!(!diff(&changed, &original).empty);
    assert!(save_bytes(&changed).is_err());
}

#[test]
fn non_finite_numbers_are_rejected_before_xml_generation() {
    let mut wb = Workbook::new();
    wb.set_number(wb.active_sheet(), 0, 0, f64::NAN).unwrap();
    assert!(save_workbook_bytes(&wb).is_err());
}
