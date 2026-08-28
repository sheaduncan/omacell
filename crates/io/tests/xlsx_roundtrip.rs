//! Open → save → open; L1/L2 diff empty. External loaders skip if absent.

use std::path::PathBuf;
use std::process::Command;

use calamine::Reader;
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
