//! Open → save → open; L1/L2 diff empty. External loaders skip if absent.

use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::process::Command;

use calamine::Reader;
use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::intern::RichTextRun;
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::sheet::{
    Comment, Hyperlink, Note, ProtectedRange, ProtectionAllow, ProtectionState, SplitView,
};
use omacell_core::style::{Color, Fill, Font, GradientFill, GradientKind, GradientStop, Style};
use omacell_core::tables::{Table, TableId};
use omacell_core::workbook::{Workbook, WorkbookProtectionState};
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
fn split_panes_convert_pixels_to_ooxml_twips_and_back() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let mut view = wb.sheet(sheet).unwrap().view.clone();
    view.split = Some(SplitView { x_px: 96, y_px: 40 });
    wb.set_sheet_view(sheet, view.clone()).unwrap();

    let bytes = save_workbook_bytes(&wb).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut worksheet_xml = String::new();
    archive
        .by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut worksheet_xml)
        .unwrap();
    assert!(worksheet_xml.contains(r#"xSplit="1440" ySplit="600" state="split""#));
    drop(archive);

    let loaded = open_bytes(&bytes).unwrap();
    assert_eq!(
        loaded
            .workbook
            .sheet(loaded.workbook.active_sheet())
            .unwrap()
            .view,
        view
    );
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
            allow: Default::default(),
            protected_ranges: Vec::new(),
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
            allow: Default::default(),
            protected_ranges: Vec::new(),
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
fn wp17_l2_records_roundtrip_together() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let protected =
        RangeRef::from_corners(CellRef::new(1, 1).unwrap(), CellRef::new(2, 2).unwrap());
    wb.set_comment(
        sheet,
        0,
        0,
        Some(Comment {
            author: "Ada".into(),
            text: "review".into(),
            replies: vec![Comment {
                author: "Lin".into(),
                text: "done".into(),
                replies: Vec::new(),
                resolved: false,
            }],
            resolved: true,
        }),
    )
    .unwrap();
    wb.set_hyperlink(
        sheet,
        0,
        1,
        Some(Hyperlink {
            target: "https://example.com".into(),
            tooltip: Some("Example".into()),
            display: None,
        }),
    )
    .unwrap();
    wb.set_sheet_protection(
        sheet,
        ProtectionState {
            enabled: true,
            password: Some(b"83AF".to_vec()),
            allow: ProtectionAllow {
                format_cells: true,
                sort: true,
                ..ProtectionAllow::default()
            },
            protected_ranges: vec![ProtectedRange {
                name: "Editable".into(),
                ranges: vec![protected],
                password: Some(b"ABCD".to_vec()),
            }],
        },
    )
    .unwrap();
    wb.set_workbook_protection(WorkbookProtectionState {
        enabled: true,
        lock_structure: true,
        lock_windows: false,
        password: Some(b"83AF".to_vec()),
    })
    .unwrap();
    wb.set_row_outline_level(sheet, 4, 2).unwrap();
    wb.set_row_collapsed(sheet, 4, true).unwrap();
    wb.set_col_outline_level(sheet, 3, 1).unwrap();
    wb.set_col_collapsed(sheet, 3, true).unwrap();
    wb.set_sheet_merges(
        sheet,
        vec![RangeRef::from_corners(
            CellRef::new(6, 0).unwrap(),
            CellRef::new(6, 2).unwrap(),
        )],
    )
    .unwrap();

    let bytes = save_workbook_bytes(&wb).unwrap();
    let doc = open_bytes(&bytes).unwrap();
    let loaded = &doc.workbook;
    let loaded_sheet = loaded.sheet(loaded.active_sheet()).unwrap();
    assert_eq!(
        loaded_sheet.comments.get(&(0, 0)),
        wb.sheet(sheet).unwrap().comments.get(&(0, 0))
    );
    assert_eq!(loaded_sheet.hyperlinks, wb.sheet(sheet).unwrap().hyperlinks);
    assert_eq!(loaded_sheet.protection, wb.sheet(sheet).unwrap().protection);
    assert_eq!(loaded.protection(), wb.protection());
    assert_eq!(loaded_sheet.geometry.rows.outline_level(4), 2);
    assert!(loaded_sheet.geometry.rows.is_collapsed(4));
    assert_eq!(loaded_sheet.geometry.cols.outline_level(3), 1);
    assert!(loaded_sheet.geometry.cols.is_collapsed(3));
    assert_eq!(loaded_sheet.merges, wb.sheet(sheet).unwrap().merges);
    assert!(doc.package.part("xl/persons/person.xml").is_some());
    assert!(
        doc.package
            .part("xl/threadedComments/threadedComment1.xml")
            .is_some()
    );
    assert!(diff(&doc, &open_bytes(&save_bytes(&doc).unwrap()).unwrap()).empty);
}

#[test]
fn non_finite_numbers_are_rejected_before_xml_generation() {
    let mut wb = Workbook::new();
    wb.set_number(wb.active_sheet(), 0, 0, f64::NAN).unwrap();
    assert!(save_workbook_bytes(&wb).is_err());
}

#[test]
fn wp18_modeled_filter_dv_cf_roundtrip() {
    use omacell_core::condfmt::{CfDxf, CfKind, CfOp, CondFormat};
    use omacell_core::filter::{AutoFilter, FilterColumn, FilterCriteria, NumOp, apply_filter};
    use omacell_core::validation::{DataValidation, DvOp, DvType};

    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_text(sheet, 0, 0, "n").unwrap();
    wb.set_number(sheet, 1, 0, 1.0).unwrap();
    wb.set_number(sheet, 2, 0, 10.0).unwrap();
    let range = RangeRef::from_corners(CellRef::new(0, 0).unwrap(), CellRef::new(2, 0).unwrap());
    apply_filter(
        &mut wb,
        sheet,
        &AutoFilter {
            range,
            columns: vec![FilterColumn {
                col_id: 0,
                criteria: FilterCriteria::Number {
                    op: NumOp::Greater,
                    value: 5.0,
                    value2: None,
                },
            }],
        },
    )
    .unwrap();
    wb.set_validations(
        sheet,
        vec![DataValidation {
            range,
            kind: DvType::Whole,
            op: DvOp::Between,
            formula1: Some("1".into()),
            formula2: Some("10".into()),
            ..DataValidation::default()
        }],
    )
    .unwrap();
    wb.set_cond_formats(
        sheet,
        vec![CondFormat {
            range,
            priority: 1,
            stop_if_true: true,
            kind: CfKind::CellIs {
                op: CfOp::Greater,
                formula1: "5".into(),
                formula2: None,
            },
            dxf: CfDxf {
                fill: Some(Color::Rgb { argb: 0xFFFF_0000 }),
                font: None,
            },
        }],
    )
    .unwrap();
    // Keep the worksheet AutoFilter and table on separate sheets. Some
    // LibreOffice versions discard a sheet-level AutoFilter when they also
    // rewrite a table on that same sheet.
    let table_sheet = wb.add_sheet("TableSheet").unwrap();
    wb.set_text(table_sheet, 0, 0, "Item").unwrap();
    wb.set_text(table_sheet, 0, 1, "Amount").unwrap();
    wb.set_text(table_sheet, 1, 0, "one").unwrap();
    wb.set_number(table_sheet, 1, 1, 1.0).unwrap();
    let table_id = wb
        .create_table(
            table_sheet,
            RangeRef::from_corners(CellRef::new(0, 0).unwrap(), CellRef::new(1, 1).unwrap()),
            "LoTable",
        )
        .unwrap();
    wb.set_table_totals(table_id, true, vec![None, Some("sum".into())])
        .unwrap();
    let bytes = save_workbook_bytes(&wb).unwrap();
    let doc = open_bytes(&bytes).unwrap();
    let workbook_xml = String::from_utf8_lossy(
        &doc.package
            .part("xl/workbook.xml")
            .expect("workbook part")
            .bytes,
    );
    assert!(workbook_xml.contains(
        r#"<definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">Sheet1!$A$1:$A$3</definedName>"#
    ));
    assert!(
        doc.workbook
            .names()
            .get(
                omacell_core::names::NameScope::Sheet(sheet),
                "_xlnm._FilterDatabase"
            )
            .is_none()
    );
    let sheet_xml = String::from_utf8_lossy(
        &doc.package
            .part("xl/worksheets/sheet1.xml")
            .expect("data worksheet part")
            .bytes,
    );
    assert!(sheet_xml.contains(r#"<sheetPr filterMode="1">"#));
    let loaded = doc.workbook.sheet(doc.workbook.active_sheet()).unwrap();
    let filter = loaded.autofilter.as_ref().expect("autofilter");
    assert_eq!(filter.columns.len(), 1);
    assert_eq!(loaded.validations.len(), 1);
    assert_eq!(loaded.validations[0].kind, DvType::Whole);
    assert_eq!(loaded.cond_formats.len(), 1);
    let table = doc.workbook.tables().get_by_name("LoTable").unwrap();
    assert!(table.has_totals);
    assert_eq!(table.columns[1].totals_fn.as_deref(), Some("sum"));
    assert!(matches!(
        loaded.cond_formats[0].kind,
        CfKind::CellIs {
            op: CfOp::Greater,
            ..
        }
    ));
    assert_eq!(
        loaded.cond_formats[0].dxf.fill,
        Some(Color::Rgb { argb: 0xFFFF_0000 })
    );

    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("omacell-wp18-lo-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    let input_dir = dir.join("input");
    let output_dir = dir.join("output");
    std::fs::create_dir_all(&input_dir).unwrap();
    std::fs::create_dir_all(&output_dir).unwrap();
    let tmp = input_dir.join("in.xlsx");
    std::fs::write(&tmp, &bytes).unwrap();
    let soffice = ["soffice", "libreoffice"]
        .iter()
        .find(|b| Command::new(b).arg("--version").output().is_ok());
    if let Some(bin) = soffice {
        let profile = dir.join("lo-profile");
        std::fs::create_dir_all(&profile).unwrap();
        let profile_uri = format!("file://{}", profile.display());
        let out = Command::new(bin)
            .args([
                "--headless",
                &format!("-env:UserInstallation={profile_uri}"),
                "--convert-to",
                "xlsx",
                "--outdir",
                output_dir.to_str().unwrap(),
                tmp.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "LibreOffice refused the modeled WP-18 workbook: {}",
            String::from_utf8_lossy(&out.stderr),
        );
        let converted = std::fs::read(output_dir.join("in.xlsx")).unwrap();
        let converted = open_bytes(&converted).unwrap();
        // LibreOffice may change the active tab while rewriting the workbook,
        // so resolve the sheet that owns these definitions explicitly.
        let sheet = converted.workbook.sheet_by_name("Sheet1").unwrap();
        let worksheet_xml = converted
            .package
            .parts
            .values()
            .filter(|part| part.name.starts_with("xl/worksheets/") && part.name.ends_with(".xml"))
            .map(|part| format!("{}: {}", part.name, String::from_utf8_lossy(&part.bytes)))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            sheet.autofilter.is_some(),
            "LibreOffice removed or rewrote the AutoFilter:\n{worksheet_xml}"
        );
        assert_eq!(sheet.validations.len(), 1);
        assert_eq!(sheet.cond_formats.len(), 1);
        let table = converted.workbook.tables().get_by_name("LoTable").unwrap();
        assert!(table.has_totals);
        assert_eq!(table.columns[1].totals_fn.as_deref(), Some("sum"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn wp18_all_modeled_data_definitions_roundtrip() {
    use omacell_core::condfmt::{CfDxf, CfKind, CfOp, CfTimePeriod, CondFormat};
    use omacell_core::filter::{AutoFilter, FilterColumn, FilterCriteria, NumOp, TextOp};
    use omacell_core::validation::{DataValidation, DvOp, DvType};

    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    for col in 0..8u16 {
        wb.set_text(sheet, 0, col, &format!("H{col}")).unwrap();
    }
    let filter_range =
        RangeRef::from_corners(CellRef::new(0, 0).unwrap(), CellRef::new(2, 7).unwrap());
    let filter = AutoFilter {
        range: filter_range,
        columns: vec![
            FilterColumn {
                col_id: 0,
                criteria: FilterCriteria::Values(vec!["x".into(), "y".into()]),
            },
            FilterColumn {
                col_id: 1,
                criteria: FilterCriteria::Text {
                    op: TextOp::Contains,
                    value: "a*b?~c".into(),
                },
            },
            FilterColumn {
                col_id: 2,
                criteria: FilterCriteria::Number {
                    op: NumOp::Between,
                    value: 1.0,
                    value2: Some(10.0),
                },
            },
            FilterColumn {
                col_id: 3,
                criteria: FilterCriteria::Number {
                    op: NumOp::NotEqual,
                    value: 5.0,
                    value2: None,
                },
            },
            FilterColumn {
                col_id: 4,
                criteria: FilterCriteria::TopN {
                    n: 25,
                    percent: true,
                    bottom: true,
                },
            },
            FilterColumn {
                col_id: 5,
                criteria: FilterCriteria::Average { below: true },
            },
            FilterColumn {
                col_id: 6,
                criteria: FilterCriteria::Color {
                    fill: false,
                    argb: 0xFF11_2233,
                },
            },
            FilterColumn {
                col_id: 7,
                criteria: FilterCriteria::Period {
                    year: Some(2026),
                    month: Some(8),
                },
            },
        ],
    };
    wb.set_autofilter(sheet, Some(filter.clone())).unwrap();

    let validations = vec![
        DataValidation {
            range: filter_range,
            kind: DvType::Whole,
            op: DvOp::Between,
            formula1: Some("1".into()),
            formula2: Some("10".into()),
            error_title: Some("Whole".into()),
            input_message: Some("Enter 1–10".into()),
            ..DataValidation::default()
        },
        DataValidation {
            range: RangeRef::from_corners(CellRef::new(1, 1).unwrap(), CellRef::new(2, 1).unwrap()),
            kind: DvType::List,
            formula1: Some("\"red,blue\"".into()),
            ..DataValidation::default()
        },
        DataValidation {
            range: RangeRef::from_corners(CellRef::new(1, 2).unwrap(), CellRef::new(2, 2).unwrap()),
            kind: DvType::Custom,
            formula1: Some("=C2>0".into()),
            ..DataValidation::default()
        },
        DataValidation {
            range: RangeRef::from_corners(CellRef::new(1, 3).unwrap(), CellRef::new(2, 3).unwrap()),
            kind: DvType::Time,
            op: DvOp::LessEq,
            formula1: Some("0.5".into()),
            ..DataValidation::default()
        },
    ];
    wb.set_validations(sheet, validations.clone()).unwrap();

    let cf_range = RangeRef::from_corners(CellRef::new(1, 0).unwrap(), CellRef::new(2, 0).unwrap());
    let red = Color::Rgb { argb: 0xFFFF_0000 };
    let yellow = Color::Rgb { argb: 0xFFFF_FF00 };
    let green = Color::Rgb { argb: 0xFF00_FF00 };
    let kinds = vec![
        CfKind::CellIs {
            op: CfOp::Between,
            formula1: "1".into(),
            formula2: Some("10".into()),
        },
        CfKind::Formula("=A2>0".into()),
        CfKind::ContainsText("x\"y".into()),
        CfKind::Blanks,
        CfKind::Errors,
        CfKind::Duplicate,
        CfKind::Unique,
        CfKind::TopN {
            n: 50,
            percent: true,
            bottom: true,
        },
        CfKind::Average { below: true },
        CfKind::TimePeriod(CfTimePeriod::Last7Days),
        CfKind::ColorScale {
            colors: vec![red, yellow, green],
        },
        CfKind::DataBar {
            color: Color::Rgb { argb: 0xFF63_8EC6 },
            gradient: false,
        },
        CfKind::IconSet { icons: 5 },
    ];
    let rules: Vec<_> = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| CondFormat {
            range: cf_range,
            priority: u32::try_from(index + 1).unwrap(),
            stop_if_true: index == 0,
            kind,
            dxf: if index == 0 {
                CfDxf {
                    fill: Some(red),
                    font: None,
                }
            } else {
                CfDxf::default()
            },
        })
        .collect();
    wb.set_cond_formats(sheet, rules.clone()).unwrap();

    wb.set_text(sheet, 0, 9, "Item").unwrap();
    wb.set_text(sheet, 0, 10, "Amount").unwrap();
    wb.set_text(sheet, 1, 9, "a").unwrap();
    wb.set_number(sheet, 1, 10, 2.0).unwrap();
    let table_id = wb
        .create_table(
            sheet,
            RangeRef::from_corners(CellRef::new(0, 9).unwrap(), CellRef::new(1, 10).unwrap()),
            "Sales",
        )
        .unwrap();
    wb.set_table_totals(table_id, true, vec![None, Some("sum".into())])
        .unwrap();

    let loaded = open_bytes(&save_workbook_bytes(&wb).unwrap())
        .unwrap()
        .workbook;
    let loaded_sheet = loaded.sheet(loaded.active_sheet()).unwrap();
    assert_eq!(loaded_sheet.autofilter.as_ref(), Some(&filter));
    assert_eq!(loaded_sheet.validations, validations);
    assert_eq!(loaded_sheet.cond_formats, rules);
    let table = loaded.tables().get_by_name("Sales").unwrap();
    assert!(table.has_totals);
    assert_eq!(table.columns[1].totals_fn.as_deref(), Some("sum"));
}

#[test]
fn wp18_edits_override_imported_definition_fragments() {
    use omacell_core::condfmt::{CfDxf, CfKind, CondFormat};
    use omacell_core::filter::{AutoFilter, FilterColumn, FilterCriteria};
    use omacell_core::validation::{DataValidation, DvOp, DvType};

    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let range = RangeRef::from_corners(CellRef::new(0, 0).unwrap(), CellRef::new(2, 0).unwrap());
    wb.set_autofilter(
        sheet,
        Some(AutoFilter {
            range,
            columns: vec![FilterColumn {
                col_id: 0,
                criteria: FilterCriteria::Values(vec!["old".into()]),
            }],
        }),
    )
    .unwrap();
    wb.set_validations(
        sheet,
        vec![DataValidation {
            range,
            kind: DvType::Whole,
            op: DvOp::Greater,
            formula1: Some("1".into()),
            ..DataValidation::default()
        }],
    )
    .unwrap();
    wb.set_cond_formats(
        sheet,
        vec![CondFormat {
            range,
            priority: 1,
            stop_if_true: false,
            kind: CfKind::Blanks,
            dxf: CfDxf::default(),
        }],
    )
    .unwrap();

    let mut imported = open_bytes(&save_workbook_bytes(&wb).unwrap()).unwrap();
    let sheet = imported.workbook.active_sheet();
    let changed_filter = AutoFilter {
        range,
        columns: vec![FilterColumn {
            col_id: 0,
            criteria: FilterCriteria::Average { below: true },
        }],
    };
    let changed_validation = DataValidation {
        range,
        kind: DvType::TextLength,
        op: DvOp::LessEq,
        formula1: Some("8".into()),
        ..DataValidation::default()
    };
    let changed_cf = CondFormat {
        range,
        priority: 1,
        stop_if_true: true,
        kind: CfKind::IconSet { icons: 4 },
        dxf: CfDxf::default(),
    };
    imported
        .workbook
        .set_autofilter(sheet, Some(changed_filter.clone()))
        .unwrap();
    imported
        .workbook
        .set_validations(sheet, vec![changed_validation.clone()])
        .unwrap();
    imported
        .workbook
        .set_cond_formats(sheet, vec![changed_cf.clone()])
        .unwrap();

    let mut reopened = open_bytes(&save_bytes(&imported).unwrap()).unwrap();
    let sheet = reopened.workbook.active_sheet();
    let loaded_sheet = reopened.workbook.sheet(sheet).unwrap();
    assert_eq!(loaded_sheet.autofilter.as_ref(), Some(&changed_filter));
    assert_eq!(loaded_sheet.validations, vec![changed_validation]);
    assert_eq!(loaded_sheet.cond_formats, vec![changed_cf]);

    reopened.workbook.set_autofilter(sheet, None).unwrap();
    reopened
        .workbook
        .set_validations(sheet, Vec::new())
        .unwrap();
    reopened
        .workbook
        .set_cond_formats(sheet, Vec::new())
        .unwrap();
    let cleared = open_bytes(&save_bytes(&reopened).unwrap()).unwrap();
    let sheet = cleared
        .workbook
        .sheet(cleared.workbook.active_sheet())
        .unwrap();
    assert!(sheet.autofilter.is_none());
    assert!(sheet.validations.is_empty());
    assert!(sheet.cond_formats.is_empty());
}
