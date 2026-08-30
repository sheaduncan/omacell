//! `.xlsx` → `.omc` → `.xlsx` L1/L2; diff stability; changeset round-trip.

use std::path::PathBuf;

use omacell_core::changeset::{
    ChangeSummary, Changeset, ChangesetId, ChangesetStatus, CommandCall,
};
use omacell_core::command::{CommandId, Origin};
use omacell_core::intern::{ArrayPayload, RichTextRun};
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::print::{Orientation, PageSetup, PaperSize};
use omacell_core::sheet::{Comment, Hyperlink, Note, ProtectionState, SplitView};
use omacell_core::storage::{CellFlags, CellSlot};
use omacell_core::style::{Color, Font, StyleId};
use omacell_core::tables::{Table, TableColumn, TableId};
use omacell_core::value::{Array2D, Value};
use omacell_core::workbook::Workbook;
use omacell_io::omc::{
    ConversionReport, OmcDocument, changeset_from_omc, changeset_to_omc, empty_package, from_xlsx,
    open_str, to_string,
};
use omacell_io::xlsx::{FileWarnings, XlsxDocument, diff, open, save_bytes};

fn corpus_xlsx() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/xlsx")
}

fn xlsx_files() -> Vec<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(corpus_xlsx())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("xlsx"))
        .collect();
    files.sort();
    files
}

fn modeled(doc: &XlsxDocument) -> XlsxDocument {
    XlsxDocument {
        workbook: doc.workbook.clone(),
        warnings: FileWarnings::new(),
        package: empty_package(),
        extras: doc.extras.clone(),
    }
}

#[test]
fn appendix_e_sketch_parses() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/omc/sketch.omc");
    let text = std::fs::read_to_string(&path).unwrap();
    let doc = open_str(&text).unwrap();
    assert!(doc.workbook.resolve_sheet_name("Inputs").is_ok());
    assert!(doc.workbook.resolve_sheet_name("Model").is_ok());
}

#[test]
fn modeled_page_setup_round_trips_through_omc() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let setup = PageSetup {
        paper: PaperSize::A4,
        orientation: Orientation::Landscape,
        title_rows: 2,
        ..PageSetup::default()
    };
    wb.set_page_setup(sheet, setup.clone()).unwrap();
    let text = to_string(&OmcDocument::from_workbook(wb)).unwrap();
    let reopened = open_str(&text).unwrap();
    assert_eq!(
        reopened
            .workbook
            .sheet(reopened.workbook.active_sheet())
            .unwrap()
            .page_setup,
        setup
    );
}

#[test]
fn xlsx_omc_xlsx_l1_l2_for_corpus() {
    for path in xlsx_files() {
        let x1 = open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let (omc, report): (OmcDocument, ConversionReport) = from_xlsx(&x1);
        let text = to_string(&omc).unwrap();
        let omc2 = open_str(&text).unwrap_or_else(|e| panic!("reopen omc {}: {e}", path.display()));
        let bytes = save_bytes(&XlsxDocument {
            workbook: omc2.workbook,
            warnings: FileWarnings::new(),
            package: empty_package(),
            extras: omc2.extras,
        })
        .unwrap();
        let x2 = omacell_io::xlsx::open_bytes(&bytes).unwrap();
        let d = diff(&modeled(&x1), &x2);
        assert!(
            d.cells.is_empty()
                && d.views.is_empty()
                && d.names.is_empty()
                && d.tables.is_empty()
                && d.annotations.is_empty()
                && d.extras.is_empty()
                && d.styles.is_empty(),
            "{}: {d:?} dropped={:?}",
            path.file_name().unwrap().to_string_lossy(),
            report.dropped
        );
    }
}

#[test]
fn single_cell_edit_is_one_line_diff() {
    let mut wb = Workbook::new();
    wb.undo_log_mut().set_enabled(false);
    let id = wb.active_sheet();
    wb.set_number(id, 0, 0, 1.0).unwrap();
    let a = to_string(&OmcDocument::from_workbook(wb.clone())).unwrap();
    wb.set_number(id, 0, 1, 2.0).unwrap();
    let b = to_string(&OmcDocument::from_workbook(wb)).unwrap();
    let lines_a: Vec<_> = a.lines().collect();
    let lines_b: Vec<_> = b.lines().collect();
    let only_b: Vec<_> = lines_b.iter().filter(|l| !lines_a.contains(l)).collect();
    let only_a: Vec<_> = lines_a.iter().filter(|l| !lines_b.contains(l)).collect();
    assert!(only_a.is_empty(), "removed lines: {only_a:?}");
    assert_eq!(
        only_b.len(),
        1,
        "expected one new cell line, got {only_b:?}"
    );
    assert!(only_b[0].starts_with("cell\t"), "{:?}", only_b[0]);
}

#[test]
fn changeset_forward_inverse_roundtrip() {
    let cs = Changeset {
        id: ChangesetId::new("cs-1").unwrap(),
        origin: Origin::ExternalAgent,
        status: ChangesetStatus::Applied,
        forward: vec![CommandCall {
            id: CommandId::new("cell.set").unwrap(),
            args: serde_json::json!({"ref": "A1", "input": "1"}),
        }],
        inverse: vec![CommandCall {
            id: CommandId::new("cell.restore").unwrap(),
            args: serde_json::json!({"ref": "A1"}),
        }],
        summary: ChangeSummary {
            cells: 1,
            rows: 2,
            columns: 3,
            sheets: 4,
            styles: 5,
            text: "set A1\twith metadata=inside".into(),
        },
    };
    let text = changeset_to_omc(&cs).unwrap();
    let again = changeset_from_omc(&text).unwrap();
    assert_eq!(again, cs);
}

#[test]
fn omc_roundtrips_ambiguous_text_and_l2_metadata() {
    let mut wb = Workbook::new();
    wb.undo_log_mut().set_enabled(false);
    let sheet = wb.active_sheet();
    wb.rename_sheet(sheet, "Data Set").unwrap();
    wb.settings_mut().iteration.enabled = true;
    wb.settings_mut().iteration.max_iterations = 17;
    wb.settings_mut().iteration.max_change = 0.000_01;
    wb.settings_mut().precision_as_displayed = true;
    wb.meta_mut().title = Some("Quarterly\tmodel".into());
    wb.meta_mut().author = Some("Ada \"A\"".into());
    wb.meta_mut().custom.insert("review".into(), "yes".into());

    for (col, value) in [
        (0, ""),
        (1, "TRUE"),
        (2, "1.25"),
        (3, "#N/A"),
        (4, "=SUM(A1:A2)"),
        (5, "line one\nline two"),
    ] {
        wb.set_text(sheet, 0, col, value).unwrap();
    }
    wb.set_number(sheet, 0, 6, -0.0).unwrap();
    wb.set_rich_text(
        sheet,
        1,
        0,
        "rich",
        vec![RichTextRun {
            start: 0,
            len: 4,
            font: Font {
                bold: true,
                ..Font::default()
            },
        }],
    )
    .unwrap();
    let cached = wb
        .set_rich_text(
            sheet,
            1,
            1,
            "TRUE",
            vec![RichTextRun {
                start: 0,
                len: 4,
                font: Font {
                    italic: true,
                    ..Font::default()
                },
            }],
        )
        .unwrap();
    let formula = wb.intern_formula("=A1").unwrap();
    wb.set_slot(
        sheet,
        1,
        1,
        CellSlot {
            value: Value::Text(cached),
            formula: Some(formula),
            style: StyleId::DEFAULT,
            flags: CellFlags::DEFAULT,
        },
    )
    .unwrap();
    wb.release_formula(formula);

    let array_text = wb.intern_text("FALSE");
    let payload = ArrayPayload::new(
        Array2D::new(1, 2).unwrap(),
        vec![Value::Text(array_text), Value::Number(2.0)],
    )
    .unwrap();
    let array = wb.intern_array(payload);
    wb.set_slot(
        sheet,
        2,
        0,
        CellSlot {
            value: Value::Array(array),
            formula: None,
            style: StyleId::DEFAULT,
            flags: CellFlags::DEFAULT,
        },
    )
    .unwrap();
    wb.release_array(array);

    let mut view = wb.sheet(sheet).unwrap().view.clone();
    view.zoom = 1.37;
    view.scroll_row = 40;
    view.scroll_col = 3;
    view.gridlines = false;
    view.show_formulas = true;
    view.split = Some(SplitView { x_px: 7, y_px: 9 });
    wb.set_sheet_view(sheet, view).unwrap();
    wb.set_tab_color(sheet, Some(Color::Rgb { argb: 0xFF12_3456 }))
        .unwrap();
    wb.set_sheet_protection(
        sheet,
        ProtectionState {
            enabled: true,
            password: Some(vec![0, 1, 2, 255]),
            allow: Default::default(),
            protected_ranges: Vec::new(),
        },
    )
    .unwrap();
    wb.set_row_height(sheet, 4, 31).unwrap();
    wb.set_row_hidden(sheet, 5, true).unwrap();
    wb.set_col_width(sheet, 2, 88).unwrap();
    wb.set_col_hidden(sheet, 3, true).unwrap();
    wb.set_note(
        sheet,
        0,
        0,
        Some(Note {
            author: Some("A=B\tC".into()),
            text: "note\nbody".into(),
        }),
    )
    .unwrap();
    wb.set_comment(
        sheet,
        0,
        1,
        Some(Comment {
            author: "Ada".into(),
            text: "thread".into(),
            replies: vec![Comment {
                author: "Lin".into(),
                text: "reply".into(),
                replies: vec![],
                resolved: false,
            }],
            resolved: false,
        }),
    )
    .unwrap();
    wb.set_hyperlink(
        sheet,
        0,
        2,
        Some(Hyperlink {
            target: "https://example.com/?a=1&b=2".into(),
            tooltip: Some("tip\ttext".into()),
            display: Some("A=B".into()),
        }),
    )
    .unwrap();

    let mut table = Table::new(TableId::new(0), "Sales", sheet, 4, 0, 5, 1);
    table.has_totals = true;
    table.banded_rows = false;
    table.banded_cols = true;
    table.auto_expand = false;
    table.columns = vec![
        TableColumn {
            name: "Last, First".into(),
        },
        TableColumn {
            name: "Amount\tUSD".into(),
        },
    ];
    wb.add_table(table).unwrap();

    let mut start = omacell_core::addr::CellRef::new(0, 0).unwrap();
    start.sheet = Some(sheet);
    wb.define_name(DefinedName {
        name: "StartCell".into(),
        scope: NameScope::Workbook,
        referent: NameReferent::Range(omacell_core::addr::RangeRef::from_corners(start, start)),
        comment: Some("points to data".into()),
    })
    .unwrap();
    let text_id = wb.intern_text("TRUE");
    wb.define_name(DefinedName {
        name: "TextFlag".into(),
        scope: NameScope::Sheet(sheet),
        referent: NameReferent::Constant(Value::Text(text_id)),
        comment: Some("not a bool".into()),
    })
    .unwrap();
    wb.custom_parts.insert(
        "xl/omacell/meta.json".into(),
        br#"{"review":"yes"}"#.to_vec(),
    );

    let original = OmcDocument::from_workbook(wb);
    let text = to_string(&original).unwrap();
    let reopened = open_str(&text).unwrap();
    let report = diff(
        &XlsxDocument {
            workbook: original.workbook.clone(),
            warnings: FileWarnings::new(),
            package: empty_package(),
            extras: original.extras.clone(),
        },
        &XlsxDocument {
            workbook: reopened.workbook.clone(),
            warnings: FileWarnings::new(),
            package: empty_package(),
            extras: reopened.extras.clone(),
        },
    );
    assert!(report.empty, "{report:?}\n{text}");
    assert_eq!(reopened.workbook.meta(), original.workbook.meta());
}

#[test]
fn omc_parser_rejects_ambiguous_or_unsafe_syntax() {
    for bad in [
        "omc 1\ncell\tSheet1!A1\t\"bad\\q\"\n",
        "omc 1\ncell\tSheet1!A1\tbad\"quote\n",
        "omc 1\ncell\tSheet1!A1\t\"text\"trailing\n",
        "omc 1\nbook\tcalc=automatic\tcalc=manual\n",
        "omc 1\ncell\tSheet1!A1\t1\ts=99\n",
        "omc 1\ncell\tSheet1!A1\ttext\ttype=bogus\n",
        "omc 1\ncell\tSheet1!A1\t1\ncell\tSheet1!A1\t2\n",
        "omc 1\ncell\tSheet1:Sheet1!A1\t1\n",
        "omc 1\ncustom\txl/omacell/../evil\tdata\n",
        "omc 1\nextra\tSheet1\tunknown\tdata\n",
        "omc 1\0\n",
    ] {
        assert!(open_str(bad).is_err(), "accepted {bad:?}");
    }

    let doc = open_str("omc 1\ncell\tSheet1!A1\t\"TRUE\"\n").unwrap();
    let slot = doc
        .workbook
        .get(doc.workbook.active_sheet(), 0, 0)
        .unwrap()
        .unwrap();
    let Value::Text(id) = slot.value else {
        panic!("quoted TRUE was not text");
    };
    assert_eq!(doc.workbook.intern().strings.get(id), Some("TRUE"));
}
