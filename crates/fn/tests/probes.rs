//! Probe registrations, corpus runner, and schema snapshot.

use std::path::PathBuf;

use omacell_core::eval::FnRegistry;
use omacell_core::recalc::{RecalcEngine, format_cell};
use omacell_core::workbook::Workbook;
use omacell_fn::{SCHEMA, assert_corpus_file, functions_json, register_probes};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/functions")
}

#[test]
fn probe_corpus_files() {
    for name in ["ABS", "SUM", "IF", "SEQUENCE"] {
        let path = corpus_dir().join(format!("{name}.tsv"));
        assert_corpus_file(&path);
    }
}

#[test]
fn functions_json_is_sorted_and_matches_schema_version() {
    let json = functions_json();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["schema"], SCHEMA);
    let names: Vec<String> = value["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_string())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    assert!(names.contains(&"ABS".to_string()));
    assert!(names.contains(&"IF".to_string()));
    let schema_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/schemas/functions.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(schema_path).unwrap()).unwrap();
    assert_eq!(schema["properties"]["schema"]["const"], SCHEMA);
    for func in value["functions"].as_array().unwrap() {
        for key in [
            "name",
            "aliases",
            "tier",
            "category",
            "arg_kinds",
            "min_args",
            "max_args",
            "strategy",
            "volatile",
            "array",
            "async_node",
            "signature",
            "doc",
        ] {
            assert!(func.get(key).is_some(), "missing {key}");
        }
    }
}

#[test]
fn lazy_if_probe_skips_unselected_error() {
    let mut registry = FnRegistry::new();
    register_probes(&mut registry);
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=IF(TRUE,9,1/0)").unwrap();
    let mut eng = RecalcEngine::new(registry);
    eng.recalc_full(&mut wb);
    assert_eq!(format_cell(&wb, s, 0, 0), "9");
}

#[test]
fn clock_and_random_probes() {
    let mut registry = FnRegistry::new();
    register_probes(&mut registry);
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    for i in 0..32u32 {
        wb.set_formula_text(s, i, 0, "=NOW()").unwrap();
        wb.set_formula_text(s, i, 1, "=RAND()").unwrap();
    }
    let mut eng = RecalcEngine::new(registry);
    eng.set_clock(Some(12_345.5));
    eng.set_random_nonce(Some(7));
    eng.recalc_full(&mut wb);
    let now = format_cell(&wb, s, 0, 0);
    for i in 0..32u32 {
        assert_eq!(format_cell(&wb, s, i, 0), now);
    }
    let r0 = format_cell(&wb, s, 0, 1);
    let r1 = format_cell(&wb, s, 1, 1);
    assert_ne!(r0, r1);
}
