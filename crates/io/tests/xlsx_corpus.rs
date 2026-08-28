//! L1 calamine cross-check and L2 sidecar expectations.

use std::path::{Path, PathBuf};

use calamine::{Data, Reader};
use omacell_core::error::ErrorKind;
use omacell_core::sheet::SheetVisibility;
use omacell_core::value::Value;
use omacell_io::xlsx::open;
use serde_json::Value as Json;

fn corpus(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/xlsx")
        .join(name)
}

fn sidecar(xlsx: &Path) -> Json {
    let json = xlsx.with_extension("json");
    serde_json::from_str(&std::fs::read_to_string(json).unwrap()).unwrap()
}

fn xlsx_files() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/xlsx");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("xlsx"))
        .collect();
    files.sort();
    files
}

#[test]
fn every_corpus_xlsx_opens() {
    let files = xlsx_files();
    assert!(!files.is_empty(), "no xlsx corpus files");
    for path in &files {
        open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}

#[test]
fn l1_values_match_sidecar_and_calamine() {
    let path = corpus("l1_values.xlsx");
    let doc = open(&path).unwrap();
    let wb = &doc.workbook;
    let sheet = wb.active_sheet();
    let side = sidecar(&path);
    let cells = side["cells"].as_object().unwrap();
    for (addr, spec) in cells {
        let cell = omacell_core::addr::parse_a1_cell(addr).unwrap();
        let slot = wb.get(sheet, cell.row, cell.col).unwrap().unwrap();
        match spec["kind"].as_str().unwrap() {
            "number" => {
                let n = spec["n"].as_f64().unwrap();
                assert_eq!(slot.value, Value::Number(n), "{addr}");
            }
            "text" => {
                let t = spec["t"].as_str().unwrap();
                let Value::Text(id) = slot.value else {
                    panic!("{addr} {:?}", slot.value);
                };
                assert_eq!(wb.intern().strings.get(id), Some(t));
            }
            "bool" => {
                assert_eq!(slot.value, Value::Bool(spec["b"].as_bool().unwrap()));
            }
            "error" => {
                assert_eq!(
                    slot.value,
                    Value::Error(ErrorKind::from_display(spec["e"].as_str().unwrap()).unwrap())
                );
            }
            other => panic!("{other}"),
        }
    }

    let mut cal = calamine::open_workbook::<calamine::Xlsx<_>, _>(&path).unwrap();
    let range = cal.worksheet_range("Sheet1").unwrap();
    assert_eq!(range.get((0, 0)), Some(&Data::Float(1.5)));
    assert_eq!(range.get((0, 1)), Some(&Data::String("hello".into())));
    assert_eq!(range.get((0, 2)), Some(&Data::Bool(true)));
}

#[test]
fn l1_formulas_are_parsed_and_shared_formulas_shifted() {
    let path = corpus("l1_formulas.xlsx");
    let doc = open(&path).unwrap();
    let wb = &doc.workbook;
    let sheet = wb.active_sheet();
    let side = sidecar(&path);
    let formulas = side["formulas"].as_object().unwrap();
    for (addr, src) in formulas {
        let cell = omacell_core::addr::parse_a1_cell(addr).unwrap();
        let slot = wb.get(sheet, cell.row, cell.col).unwrap().unwrap();
        let fid = slot.formula.unwrap_or_else(|| {
            panic!("{addr} missing formula, value={:?} src={}", slot.value, src)
        });
        assert_eq!(wb.intern().formulas.get(fid), Some(src.as_str().unwrap()));
    }
}

#[test]
fn l2_merges_and_freeze() {
    let path = corpus("l2_merges_freeze.xlsx");
    let doc = open(&path).unwrap();
    let sheet = doc.workbook.sheet(doc.workbook.active_sheet()).unwrap();
    let expect = sidecar(&path);
    assert_eq!(sheet.merges.len(), 1);
    assert_eq!(
        sheet.merges[0].to_a1(),
        expect["merges"][0].as_str().unwrap()
    );
    assert_eq!(sheet.view.freeze.rows, 1);
    assert_eq!(sheet.view.freeze.cols, 1);
    assert!((sheet.view.zoom - 1.5).abs() < 1e-9);
}

#[test]
fn l2_defined_name() {
    let path = corpus("l2_names.xlsx");
    let doc = open(&path).unwrap();
    let names: Vec<_> = doc.workbook.names().iter().collect();
    assert!(names.iter().any(|n| n.name == "Rate"));
}

#[test]
fn l2_hyperlink() {
    let path = corpus("l2_hyperlinks.xlsx");
    let doc = open(&path).unwrap();
    let sheet = doc.workbook.sheet(doc.workbook.active_sheet()).unwrap();
    let h = sheet.hyperlinks.get(&(0, 0)).expect("hyperlink");
    assert_eq!(h.target, "https://example.com");
}

#[test]
fn l2_table() {
    let path = corpus("l2_table.xlsx");
    let doc = open(&path).unwrap();
    let tables: Vec<_> = doc.workbook.tables().iter().collect();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "Sales");
}

#[test]
fn l2_comment_note() {
    let path = corpus("l2_comments.xlsx");
    let doc = open(&path).unwrap();
    let sheet = doc.workbook.sheet(doc.workbook.active_sheet()).unwrap();
    let note = sheet.notes.get(&(0, 0)).expect("note");
    assert_eq!(note.author.as_deref(), Some("Ada"));
    assert_eq!(note.text, "check this");
}

#[test]
fn omacell_custom_part() {
    let path = corpus("omacell_part.xlsx");
    let doc = open(&path).unwrap();
    let bytes = doc
        .workbook
        .custom_parts
        .get("xl/omacell/meta.json")
        .expect("custom part");
    assert_eq!(bytes, b"{\"hello\":true}");
    assert!(doc.package.part("xl/omacell/meta.json").is_some());
}

#[test]
fn hidden_sheet() {
    let path = corpus("l2_hidden_sheet.xlsx");
    let doc = open(&path).unwrap();
    let vis = doc.workbook.sheet_by_name("Visible").unwrap();
    let hid = doc.workbook.sheet_by_name("Hidden").unwrap();
    assert_eq!(vis.visibility, SheetVisibility::Visible);
    assert_eq!(hid.visibility, SheetVisibility::Hidden);
}

#[test]
fn l1_matches_calamine_for_all_corpus_cells() {
    for path in xlsx_files() {
        let doc = open(&path).unwrap();
        let Ok(mut cal) = calamine::open_workbook::<calamine::Xlsx<_>, _>(&path) else {
            continue;
        };
        for sheet in doc.workbook.sheets() {
            let Ok(range) = cal.worksheet_range(&sheet.name) else {
                continue;
            };
            for (row, col, data) in range.used_cells() {
                let got = doc.workbook.get(sheet.id, row as u32, col as u16).unwrap();
                match data {
                    Data::Empty => {}
                    Data::Float(n) => {
                        assert_eq!(
                            got.map(|s| s.value),
                            Some(Value::Number(*n)),
                            "{} {} r{row}c{col}",
                            path.display(),
                            sheet.name
                        );
                    }
                    Data::Int(n) => {
                        assert_eq!(got.map(|s| s.value), Some(Value::Number(*n as f64)));
                    }
                    Data::String(s) => {
                        let Some(slot) = got else {
                            panic!("missing text");
                        };
                        let Value::Text(id) = slot.value else {
                            panic!("not text {:?}", slot.value);
                        };
                        assert_eq!(doc.workbook.intern().strings.get(id), Some(s.as_str()));
                    }
                    Data::Bool(b) => {
                        assert_eq!(got.map(|s| s.value), Some(Value::Bool(*b)));
                    }
                    Data::Error(_) => {
                        assert!(matches!(got.map(|s| s.value), Some(Value::Error(_))));
                    }
                    _ => {}
                }
            }
        }
    }
}
