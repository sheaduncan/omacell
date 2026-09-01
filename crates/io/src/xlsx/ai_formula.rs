//! Cached-value XLSX bridge for Omacell AI formulas.

use omacell_core::storage::CellSlot;
use omacell_core::workbook::Workbook;
use serde::{Deserialize, Serialize};

use super::warnings::FileWarnings;
use crate::error;

pub(crate) const PART: &str = "xl/omacell/ai-formulas.json";
const VERSION: u32 = 1;
const MAX_FORMULAS: usize = 100_000;
const MAX_PART_BYTES: usize = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FormulaPart {
    version: u32,
    formulas: Vec<FormulaRecord>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FormulaRecord {
    sheet: String,
    row: u32,
    col: u16,
    formula: String,
}

pub(crate) fn is_ai_formula(source: &str) -> bool {
    let upper = source.trim_start_matches('=').trim().to_ascii_uppercase();
    upper.starts_with("AI(") || upper.starts_with("AI.")
}

pub(crate) fn encode(
    workbook: &Workbook,
) -> Result<Option<Vec<u8>>, omacell_core::error::CoreError> {
    let mut formulas = Vec::new();
    for sheet in workbook.sheets() {
        for (row, col, slot) in sheet.store.iter() {
            let Some(source) = slot
                .formula
                .and_then(|id| workbook.intern().formulas.get(id))
            else {
                continue;
            };
            if is_ai_formula(source) {
                if formulas.len() >= MAX_FORMULAS {
                    return Err(error::xlsx_write(format!(
                        "AI formula bridge exceeds {MAX_FORMULAS} cells"
                    )));
                }
                formulas.push(FormulaRecord {
                    sheet: sheet.name.clone(),
                    row,
                    col,
                    formula: source.to_string(),
                });
            }
        }
    }
    if formulas.is_empty() {
        return Ok(None);
    }
    let bytes = serde_json::to_vec(&FormulaPart {
        version: VERSION,
        formulas,
    })
    .map_err(|err| error::xlsx_write(format!("cannot encode AI formula bridge: {err}")))?;
    if bytes.len() > MAX_PART_BYTES {
        return Err(error::xlsx_write(format!(
            "AI formula bridge is {} bytes; maximum is {MAX_PART_BYTES}",
            bytes.len()
        )));
    }
    Ok(Some(bytes))
}

pub(crate) fn restore(workbook: &mut Workbook, warnings: &mut FileWarnings) {
    let Some(bytes) = workbook.custom_parts.get(PART).cloned() else {
        return;
    };
    if bytes.len() > MAX_PART_BYTES {
        warnings.push(
            "xlsx.ai_formula",
            format!("ignored AI formula bridge larger than {MAX_PART_BYTES} bytes"),
            Some(PART.into()),
        );
        return;
    }
    let part: FormulaPart = match serde_json::from_slice(&bytes) {
        Ok(part) => part,
        Err(err) => {
            warnings.push(
                "xlsx.ai_formula",
                format!("ignored invalid AI formula bridge: {err}"),
                Some(PART.into()),
            );
            return;
        }
    };
    if part.version != VERSION || part.formulas.len() > MAX_FORMULAS {
        warnings.push(
            "xlsx.ai_formula",
            "ignored unsupported or oversized AI formula bridge",
            Some(PART.into()),
        );
        return;
    }
    for record in part.formulas {
        let Some(sheet) = workbook.sheet_by_name(&record.sheet).map(|sheet| sheet.id) else {
            warnings.push(
                "xlsx.ai_formula",
                format!("AI formula bridge names missing sheet {:?}", record.sheet),
                Some(PART.into()),
            );
            continue;
        };
        if !is_ai_formula(&record.formula) {
            warnings.push(
                "xlsx.ai_formula",
                "AI formula bridge contains a non-AI formula",
                Some(PART.into()),
            );
            continue;
        }
        let mut slot = match workbook.get(sheet, record.row, record.col) {
            Ok(Some(slot)) => *slot,
            Ok(None) => CellSlot::empty(),
            Err(err) => {
                warnings.push("xlsx.ai_formula", err.message, Some(PART.into()));
                continue;
            }
        };
        let formula = match workbook.intern_formula(&record.formula) {
            Ok(formula) => formula,
            Err(err) => {
                warnings.push("xlsx.ai_formula", err.message, Some(PART.into()));
                continue;
            }
        };
        slot.formula = Some(formula);
        if let Err(err) = workbook.set_slot(sheet, record.row, record.col, slot) {
            warnings.push("xlsx.ai_formula", err.message, Some(PART.into()));
        }
        workbook.release_formula(formula);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_bridge_is_rejected_before_json_parsing() {
        let mut workbook = Workbook::new();
        workbook
            .custom_parts
            .insert(PART.into(), vec![b' '; MAX_PART_BYTES + 1]);
        let mut warnings = FileWarnings::new();

        restore(&mut workbook, &mut warnings);

        assert_eq!(warnings.items.len(), 1);
        assert!(warnings.items[0].message.contains("larger than"));
    }
}
