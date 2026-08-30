//! Pivot `.xlsx` round-trip and external-loader structure checks.

use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::pivot::{PivotAgg, PivotDataField, PivotTable};
use omacell_core::workbook::Workbook;
use omacell_io::xlsx::{open_bytes, save_workbook_bytes};

fn seed() -> Workbook {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Region").unwrap();
    wb.set_text(s, 0, 1, "Amount").unwrap();
    wb.set_text(s, 1, 0, "East").unwrap();
    wb.set_number(s, 1, 1, 10.0).unwrap();
    wb.set_text(s, 2, 0, "West").unwrap();
    wb.set_number(s, 2, 1, 70.0).unwrap();
    let mut table = PivotTable::new(
        "Sales",
        s,
        RangeRef::from_corners(CellRef::new(0, 0).unwrap(), CellRef::new(2, 1).unwrap()),
        s,
        0,
        4,
    );
    table.rows = vec!["Region".into()];
    table.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    table.refresh_on_load = true;
    wb.add_pivot(table).unwrap();
    wb
}

fn zip_names(bytes: &[u8]) -> Vec<String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn modeled_pivot_writes_cache_and_table_parts() {
    let bytes = save_workbook_bytes(&seed()).unwrap();
    let names = zip_names(&bytes);
    assert!(
        names.iter().any(|n| n.contains("pivotCacheDefinition")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.contains("pivotCacheRecords")),
        "{names:?}"
    );
    assert!(names.iter().any(|n| n.contains("pivotTable")), "{names:?}");
}

#[test]
fn modeled_pivot_round_trips() {
    let bytes = save_workbook_bytes(&seed()).unwrap();
    let doc = open_bytes(&bytes).unwrap();
    assert_eq!(doc.workbook.pivots().len(), 1);
    let pivot = doc.workbook.pivots().iter().next().unwrap();
    assert_eq!(pivot.name, "Sales");
    assert_eq!(pivot.rows, vec!["Region".to_string()]);
    assert_eq!(pivot.data[0].source, "Amount");
    assert_eq!(pivot.data[0].agg, PivotAgg::Sum);
    let again = save_workbook_bytes(&doc.workbook).unwrap();
    let doc2 = open_bytes(&again).unwrap();
    assert_eq!(doc2.workbook.pivots().len(), 1);
}

#[test]
fn openpyxl_sees_pivot_parts_if_present() {
    let bytes = save_workbook_bytes(&seed()).unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-tmp")
        .join(format!("omacell-pivot-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("pivot.xlsx");
    std::fs::write(&path, &bytes).unwrap();
    let output = std::process::Command::new("python3")
        .args([
            "-c",
            &format!(
                r#"
import zipfile, sys
z = zipfile.ZipFile(r'{path}')
names = z.namelist()
assert any('pivotCacheDefinition' in n for n in names), names
assert any('pivotTable' in n for n in names), names
try:
    import openpyxl
    wb = openpyxl.load_workbook(r'{path}')
    ws = wb.active
    pivots = getattr(ws, '_pivots', None) or []
    # openpyxl versions differ; the zip parts are the structure check.
    print('openpyxl pivots', len(pivots))
except ImportError:
    print('openpyxl missing')
"#,
                path = path.display()
            ),
        ])
        .output();
    match output {
        Ok(out) if out.status.success() => {}
        Ok(out) => panic!(
            "openpyxl/zip check failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(_) => {}
    }
}

#[test]
fn libreoffice_opens_modeled_pivot_if_present() {
    let Some(soffice) = ["soffice", "libreoffice"]
        .into_iter()
        .find(|bin| which(bin))
    else {
        return;
    };
    let bytes = save_workbook_bytes(&seed()).unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-tmp")
        .join(format!("omacell-pivot-lo-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let dir = dir.canonicalize().unwrap();
    let path = dir.join("pivot.xlsx");
    let profile = dir.join("libreoffice-profile");
    std::fs::write(&path, bytes).unwrap();
    let out = std::process::Command::new(soffice)
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
            "ods",
            "--outdir",
            dir.to_str().unwrap(),
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "libreoffice failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ods = dir.join("pivot.ods");
    assert!(ods.is_file(), "LibreOffice did not write {ods:?}");
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .and_then(|paths| std::env::split_paths(&paths).find(|p| p.join(bin).is_file()))
        .is_some()
}
