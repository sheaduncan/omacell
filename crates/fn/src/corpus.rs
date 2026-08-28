//! Shared TSV corpus runner for `tests/corpus/functions/<NAME>.tsv`.

use std::path::Path;

use omacell_core::eval::FnRegistry;
use omacell_core::recalc::{RecalcEngine, format_cell};
use omacell_core::workbook::Workbook;

use crate::register_all;

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

fn parse_tsv(text: &str) -> Result<Vec<CorpusRow>, String> {
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() != 3 {
            return Err(format!(
                "line {}: expected exactly formula, expected, and note columns",
                index + 1
            ));
        }
        let formula = cols[0].trim();
        let expected = cols[1].trim();
        let note = cols[2].trim();
        if !formula.starts_with('=') || note.is_empty() {
            return Err(format!(
                "line {}: formula must start with '=' and note must not be empty",
                index + 1
            ));
        }
        rows.push(CorpusRow {
            formula: formula.to_string(),
            expected: expected.to_string(),
            note: note.to_string(),
        });
    }
    if rows.is_empty() {
        return Err("corpus contains no data rows".to_string());
    }
    Ok(rows)
}

/// Evaluate each row in a one-cell workbook using the full function registry.
pub fn run_corpus_file(path: &Path) -> Result<Vec<(CorpusRow, String)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let rows = parse_tsv(&text)?;
    let mut out = Vec::new();
    for row in rows {
        let mut registry = FnRegistry::new();
        register_all(&mut registry);
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

#[cfg(test)]
mod tests {
    use super::parse_tsv;

    #[test]
    fn corpus_requires_three_columns_and_a_citation() {
        assert!(parse_tsv("=ABS(-1)\t1\tcitation").is_ok());
        assert!(parse_tsv("=ABS(-1)\t1").is_err());
        assert!(parse_tsv("=ABS(-1)\t1\t").is_err());
        assert!(parse_tsv("ABS(-1)\t1\tcitation").is_err());
    }
}
