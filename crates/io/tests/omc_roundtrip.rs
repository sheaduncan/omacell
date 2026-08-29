//! `.xlsx` → `.omc` → `.xlsx` L1/L2; diff stability; changeset round-trip.

use std::path::PathBuf;

use omacell_core::changeset::{
    ChangeSummary, Changeset, ChangesetId, ChangesetStatus, CommandCall,
};
use omacell_core::command::{CommandId, Origin};
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
        origin: Origin::User,
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
            text: "set A1".into(),
            ..ChangeSummary::default()
        },
    };
    let text = changeset_to_omc(&cs).unwrap();
    let again = changeset_from_omc(&text).unwrap();
    assert_eq!(again.id.as_str(), "cs-1");
    assert_eq!(again.forward, cs.forward);
    assert_eq!(again.inverse, cs.inverse);
    assert_eq!(again.status, ChangesetStatus::Applied);
    assert_eq!(again.origin, Origin::User);
}
