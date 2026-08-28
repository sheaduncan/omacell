//! Shared corpus paths for WP-08 tests.

use std::path::{Path, PathBuf};

pub fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/csv")
}

pub fn corpus_file(name: &str) -> PathBuf {
    corpus_dir().join(name)
}

pub fn read_tsv(path: &Path) -> Vec<Vec<String>> {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    text.lines()
        .filter(|line| {
            let t = line.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .map(|line| line.split('\t').map(ToOwned::to_owned).collect())
        .collect()
}
