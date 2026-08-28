//! Probe registrations, corpus runner, and schema snapshot.

use std::path::PathBuf;

use omacell_core::eval::FnRegistry;
use omacell_core::recalc::{RecalcEngine, format_cell};
use omacell_core::workbook::Workbook;
use omacell_fn::{
    FunctionsEnvelope, SCHEMA, all_specs, functions_json, register_all, run_corpus_file,
};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/functions")
}

#[test]
fn probe_corpus_files() {
    for name in ["ABS", "SUM", "IF", "SEQUENCE"] {
        let path = corpus_dir().join(format!("{name}.tsv"));
        let results = run_corpus_file(&path).unwrap();
        for (row, got) in results {
            assert_eq!(got, row.expected, "{} ({})", row.formula, row.note);
        }
    }
}

#[test]
fn text_and_date_corpus_files() {
    let mut files: Vec<_> = std::fs::read_dir(corpus_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("tsv"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 60,
        "expected WP-05b corpora, found {}",
        files.len()
    );
    for path in files {
        let results = run_corpus_file(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            results.len() >= 10,
            "{} has {} rows",
            path.display(),
            results.len()
        );
        for (row, got) in results {
            assert_eq!(
                got,
                row.expected,
                "{}: {} ({})",
                path.file_name().unwrap().to_string_lossy(),
                row.formula,
                row.note
            );
        }
    }
}

#[test]
fn functions_json_is_sorted_and_matches_schema_version() {
    let json = functions_json().unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let envelope: FunctionsEnvelope = serde_json::from_value(value.clone()).unwrap();
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
    validate_schema(&value, &schema, "$").unwrap();
    let specs = all_specs();
    assert_eq!(envelope.functions.len(), specs.len());
    assert!(names.contains(&"SUMIFS".to_string()));
    assert!(names.contains(&"ISOMITTED".to_string()));
    assert!(names.contains(&"TEXT".to_string()));
    assert!(names.contains(&"NOW".to_string()));
    assert!(names.contains(&"XLOOKUP".to_string()));
    assert!(names.contains(&"SEQUENCE".to_string()));
    assert!(names.contains(&"LET".to_string()));
    for spec in &specs {
        assert!(
            spec.min_args <= spec.max_args,
            "bad arity for {}",
            spec.name
        );
        assert_eq!(spec.strategy(), spec.to_json().strategy);
    }
}

fn validate_schema(
    value: &serde_json::Value,
    schema: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    if let Some(expected) = schema.get("const")
        && value != expected
    {
        return Err(format!("{path}: expected const {expected}, got {value}"));
    }
    if let Some(choices) = schema.get("enum").and_then(serde_json::Value::as_array)
        && !choices.contains(value)
    {
        return Err(format!("{path}: {value} is not in enum"));
    }
    match schema.get("type").and_then(serde_json::Value::as_str) {
        Some("object") => {
            let object = value
                .as_object()
                .ok_or_else(|| format!("{path}: expected object"))?;
            let properties = schema
                .get("properties")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| format!("{path}: schema has no properties"))?;
            if let Some(required) = schema.get("required").and_then(serde_json::Value::as_array) {
                for key in required.iter().filter_map(serde_json::Value::as_str) {
                    if !object.contains_key(key) {
                        return Err(format!("{path}: missing {key}"));
                    }
                }
            }
            if schema.get("additionalProperties") == Some(&serde_json::Value::Bool(false)) {
                for key in object.keys() {
                    if !properties.contains_key(key) {
                        return Err(format!("{path}: unexpected property {key}"));
                    }
                }
            }
            for (key, child) in object {
                if let Some(child_schema) = properties.get(key) {
                    validate_schema(child, child_schema, &format!("{path}.{key}"))?;
                }
            }
        }
        Some("array") => {
            let array = value
                .as_array()
                .ok_or_else(|| format!("{path}: expected array"))?;
            if let Some(items) = schema.get("items") {
                for (index, item) in array.iter().enumerate() {
                    validate_schema(item, items, &format!("{path}[{index}]"))?;
                }
            }
        }
        Some("string") => {
            let string = value
                .as_str()
                .ok_or_else(|| format!("{path}: expected string"))?;
            if let Some(minimum) = schema.get("minLength").and_then(serde_json::Value::as_u64)
                && string.chars().count() < minimum as usize
            {
                return Err(format!("{path}: string is too short"));
            }
        }
        Some("integer") => {
            let number = value
                .as_u64()
                .ok_or_else(|| format!("{path}: expected non-negative integer"))?;
            if let Some(minimum) = schema.get("minimum").and_then(serde_json::Value::as_u64)
                && number < minimum
            {
                return Err(format!("{path}: below minimum"));
            }
            if let Some(maximum) = schema.get("maximum").and_then(serde_json::Value::as_u64)
                && number > maximum
            {
                return Err(format!("{path}: above maximum"));
            }
        }
        Some("boolean") => {
            if !value.is_boolean() {
                return Err(format!("{path}: expected boolean"));
            }
        }
        Some(other) => return Err(format!("{path}: unsupported schema type {other}")),
        None => {}
    }
    Ok(())
}

#[test]
fn lazy_if_probe_skips_unselected_error() {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
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
    register_all(&mut registry);
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
