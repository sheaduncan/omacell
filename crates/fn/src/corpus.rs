//! Shared TSV corpus runner for `tests/corpus/functions/<NAME>.tsv`.

use std::path::Path;

use omacell_core::eval::FnRegistry;
use omacell_core::graph::CellCoord;
use omacell_core::locale::LocaleId;
use omacell_core::recalc::{RecalcEngine, format_cell};
use omacell_core::spill::SpillRegion;
use omacell_core::workbook::{DateSystem, Workbook};

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
    /// Optional BCP-47 locale (`en-US`, `en-GB`, `de-DE`).
    pub locale: Option<String>,
    /// Optional `1900` or `1904`.
    pub date_system: Option<String>,
}

fn parse_tsv(text: &str) -> Result<Vec<CorpusRow>, String> {
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 3 || cols.len() > 5 {
            return Err(format!(
                "line {}: expected formula, expected, note, and optional locale/date_system",
                index + 1
            ));
        }
        let formula = cols[0].trim();
        // Expected text is significant, including leading/trailing spaces (`CHAR(32)`).
        let expected = cols[1];
        let note = cols[2].trim();
        if !formula.starts_with('=') || note.is_empty() {
            return Err(format!(
                "line {}: formula must start with '=' and note must not be empty",
                index + 1
            ));
        }
        let locale = cols
            .get(3)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let date_system = cols
            .get(4)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        rows.push(CorpusRow {
            formula: formula.to_string(),
            expected: expected.to_string(),
            note: note.to_string(),
            locale,
            date_system,
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
        if let Some(ds) = &row.date_system {
            wb.settings_mut().date_system = match ds.as_str() {
                "1900" => DateSystem::Excel1900,
                "1904" => DateSystem::Excel1904,
                other => {
                    return Err(format!("{}: unknown date_system {other}", row.formula));
                }
            };
        }
        let sheet = wb.active_sheet();
        wb.set_formula_text(sheet, 0, 0, &row.formula)
            .map_err(|e| format!("{}: {e}", row.formula))?;
        let mut engine = RecalcEngine::new(registry);
        engine.set_clock(Some(45_000.5));
        engine.set_random_nonce(Some(0x1111_2222_3333_4444));
        if let Some(tag) = &row.locale {
            let locale = LocaleId::parse_tag(tag)
                .ok_or_else(|| format!("{}: unknown locale {tag}", row.formula))?;
            engine.set_locale(locale);
        }
        engine.recalc_full(&mut wb);
        let got = format_result(&engine, &wb, sheet, 0, 0);
        out.push((row, got));
    }
    Ok(out)
}

/// Display the formula result. Spilled arrays are shown in `{a,b;c,d}` form
/// (the origin cell alone stores only the top-left scalar).
fn format_result(
    engine: &RecalcEngine,
    wb: &Workbook,
    sheet: omacell_core::addr::SheetId,
    row: u32,
    col: u16,
) -> String {
    let origin = CellCoord::new(sheet, row, col);
    if let Some(region) = engine.spill().get(origin)
        && region.blocked_by.is_none()
        && (region.rows > 1 || region.cols > 1)
    {
        return format_spill(wb, region);
    }
    format_cell(wb, sheet, row, col)
}

fn format_spill(wb: &Workbook, region: SpillRegion) -> String {
    let mut out = String::from("{");
    for dr in 0..region.rows {
        if dr > 0 {
            out.push(';');
        }
        for dc in 0..region.cols {
            if dc > 0 {
                out.push(',');
            }
            out.push_str(&format_cell(
                wb,
                region.origin.sheet,
                region.origin.row.saturating_add(dr),
                region.origin.col.saturating_add(dc as u16),
            ));
        }
    }
    out.push('}');
    out
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
        assert!(parse_tsv("=VALUE(\"1/2/2024\")\t45293\ten-US m/d\ten-US").is_ok());
        assert!(parse_tsv("=DATE(2024,1,1)\t43830\t1904 serial\t\t1904").is_ok());
    }
}
