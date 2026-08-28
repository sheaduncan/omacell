//! WP-05a function corpus table tests.

use std::fs;
use std::path::PathBuf;

use omacell_fn::{FunctionSpec, all_specs, run_corpus_file};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/functions")
}

fn owned_specs() -> Vec<FunctionSpec> {
    all_specs()
        .into_iter()
        .filter(|s| s.name != "SEQUENCE")
        .collect()
}

#[test]
fn every_owned_function_has_at_least_ten_corpus_rows() {
    for spec in owned_specs() {
        let path = corpus_dir().join(format!("{}.tsv", spec.name));
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let n = text
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#')
            })
            .count();
        assert!(
            n >= 10,
            "{} has {n} corpus rows (need ≥10) at {}",
            spec.name,
            path.display()
        );
    }
}

#[test]
fn function_corpus_files_match_expected() {
    let mut failures = Vec::new();
    for spec in owned_specs() {
        let path = corpus_dir().join(format!("{}.tsv", spec.name));
        match run_corpus_file(&path) {
            Err(e) => failures.push(format!("{}: run error {e}", spec.name)),
            Ok(results) => {
                for (row, got) in results {
                    if got != row.expected {
                        failures.push(format!(
                            "{}: {} got {got:?} want {:?} ({})",
                            spec.name, row.formula, row.expected, row.note
                        ));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} corpus mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn probe_sequence_corpus_still_pass() {
    let path = corpus_dir().join("SEQUENCE.tsv");
    let results = run_corpus_file(&path).unwrap();
    for (row, got) in results {
        assert_eq!(got, row.expected, "{} ({})", row.formula, row.note);
    }
}
