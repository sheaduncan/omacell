//! Shared TSV corpus runner for `tests/corpus/functions/<NAME>.tsv`.

use std::path::Path;

use omacell_core::eval::FnRegistry;
use omacell_core::recalc::{RecalcEngine, format_cell};
use omacell_core::workbook::Workbook;

use crate::register_probes;

/// One corpus row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorpusRow {
    /// Formula including the leading `=`.
    pub formula: String,
    /// Expected display text.
    pub expected: String,
    /// Citation / note.
    pub note: String,
}

/// Parse a functions TSV (columns: formula, expected, note). `#` comments skipped.
pub fn parse_tsv(text: &str) -> Vec<CorpusRow> {
    text.lines()
        .filter(|line| {
            let t = line.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .filter_map(|line| {
            let mut cols = line.split('\t');
            let formula = cols.next()?.trim().to_string();
            let expected = cols.next().unwrap_or("").trim().to_string();
            let note = cols.next().unwrap_or("").trim().to_string();
            Some(CorpusRow {
                formula,
                expected,
                note,
            })
        })
        .collect()
}

/// Evaluate each row in a one-cell workbook using probe registrations.
pub fn run_corpus_file(path: &Path) -> Result<Vec<(CorpusRow, String)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let rows = parse_tsv(&text);
    let mut out = Vec::new();
    for row in rows {
        let mut registry = FnRegistry::new();
        register_probes(&mut registry);
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        wb.set_formula_text(sheet, 0, 0, &row.formula)
            .map_err(|e| format!("{}: {e}", row.formula))?;
        let mut engine = RecalcEngine::new(registry);
        engine.set_clock(Some(45_000.5));
        engine.set_random_nonce(Some(0x1111_2222_3333_4444));
        engine.recalc_full(&mut wb);
        let got = format_cell(&wb, sheet, 0, 0);
        out.push((row, got));
    }
    Ok(out)
}

/// Assert every row in `path` matches its expected display.
pub fn assert_corpus_file(path: &Path) {
    let results = run_corpus_file(path).unwrap_or_else(|e| panic!("{e}"));
    for (row, got) in results {
        assert_eq!(
            got, row.expected,
            "{} ({}) got {got} expected {}",
            row.formula, row.note, row.expected
        );
    }
}
