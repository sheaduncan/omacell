//! Model-level L1/L2 comparison for round-trip tests and `omacell diff`.

use omacell_core::addr::col_to_letters;
use omacell_core::value::Value;
use serde::{Deserialize, Serialize};

use super::XlsxDocument;

/// JSON-serializable diff. Empty lists mean L1/L2 match.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffReport {
    /// True when every category is empty.
    pub empty: bool,
    /// Cell value / formula mismatches (`Sheet!A1: ...`).
    pub cells: Vec<String>,
    /// Merge / freeze / zoom / visibility.
    pub views: Vec<String>,
    /// Defined names.
    pub names: Vec<String>,
    /// Tables.
    pub tables: Vec<String>,
    /// Hyperlinks and notes.
    pub annotations: Vec<String>,
    /// Extra fragments (print / CF / DV) presence.
    pub extras: Vec<String>,
    /// Style / number-format mismatches.
    pub styles: Vec<String>,
    /// L3 part names whose bytes differ (non-rewritten parts).
    pub parts: Vec<String>,
}

/// Compare two opened documents.
#[must_use]
pub fn diff(a: &XlsxDocument, b: &XlsxDocument) -> DiffReport {
    let mut r = DiffReport::default();
    let intern_a = a.workbook.intern();
    let intern_b = b.workbook.intern();
    let sheets_a: Vec<_> = a.workbook.sheets().collect();
    let sheets_b: Vec<_> = b.workbook.sheets().collect();
    if sheets_a.len() != sheets_b.len() {
        r.views.push(format!(
            "sheet count {} vs {}",
            sheets_a.len(),
            sheets_b.len()
        ));
    }
    for sa in &sheets_a {
        let Some(sb) = b.workbook.sheet_by_name(&sa.name) else {
            r.views.push(format!("missing sheet {}", sa.name));
            continue;
        };
        if sa.visibility != sb.visibility {
            r.views.push(format!("{} visibility", sa.name));
        }
        if sa.view.freeze != sb.view.freeze {
            r.views.push(format!("{} freeze", sa.name));
        }
        if (sa.view.zoom - sb.view.zoom).abs() > 0.01 {
            r.views.push(format!("{} zoom", sa.name));
        }
        if sa.merges.len() != sb.merges.len() {
            r.views.push(format!("{} merges", sa.name));
        }
        let ha: Vec<u32> = sa.geometry.rows.iter_hidden().collect();
        let hb: Vec<u32> = sb.geometry.rows.iter_hidden().collect();
        if ha != hb {
            r.views.push(format!("{} hidden rows", sa.name));
        }
        let ca: Vec<u32> = sa.geometry.cols.iter_hidden().collect();
        let cb: Vec<u32> = sb.geometry.cols.iter_hidden().collect();
        if ca != cb {
            r.views.push(format!("{} hidden cols", sa.name));
        }
        for (row, col, slot_a) in sa.store.iter() {
            let slot_b = b.workbook.get(sb.id, row, col).ok().flatten();
            let addr = format!(
                "{}!{}{}",
                sa.name,
                col_to_letters(col).unwrap_or_else(|_| "?".into()),
                row + 1
            );
            let Some(slot_b) = slot_b else {
                r.cells.push(format!("{addr}: missing"));
                continue;
            };
            let fa = slot_a
                .formula
                .and_then(|id| intern_a.formulas.get(id).map(str::to_string));
            let fb = slot_b
                .formula
                .and_then(|id| intern_b.formulas.get(id).map(str::to_string));
            if fa != fb {
                r.cells.push(format!("{addr}: formula {fa:?} vs {fb:?}"));
            }
            if !values_eq(&a.workbook, intern_a, slot_a.value, intern_b, slot_b.value) {
                r.cells.push(format!("{addr}: value mismatch"));
            }
            let sty_a = intern_a.styles.get(slot_a.style);
            let sty_b = intern_b.styles.get(slot_b.style);
            if sty_a != sty_b {
                r.styles.push(format!("{addr}: style mismatch"));
            }
        }
        for ((row, col), ha) in &sa.hyperlinks {
            match sb.hyperlinks.get(&(*row, *col)) {
                Some(hb) if ha.target == hb.target => {}
                _ => r
                    .annotations
                    .push(format!("{} hyperlink r{row}c{col}", sa.name)),
            }
        }
        for ((row, col), na) in &sa.notes {
            match sb.notes.get(&(*row, *col)) {
                Some(nb) if na.text == nb.text => {}
                _ => r.annotations.push(format!("{} note r{row}c{col}", sa.name)),
            }
        }
    }
    let names_a: Vec<String> = a.workbook.names().iter().map(|n| n.name.clone()).collect();
    let names_b: Vec<String> = b.workbook.names().iter().map(|n| n.name.clone()).collect();
    for n in &names_a {
        if !names_b.contains(n) {
            r.names.push(format!("missing name {n}"));
        }
    }
    let tables_a: Vec<String> = a.workbook.tables().iter().map(|t| t.name.clone()).collect();
    let tables_b: Vec<String> = b.workbook.tables().iter().map(|t| t.name.clone()).collect();
    for n in &tables_a {
        if !tables_b.contains(n) {
            r.tables.push(format!("missing table {n}"));
        }
    }
    let mut extra_names: Vec<&String> = a.extras.keys().collect();
    extra_names.sort();
    for name in extra_names {
        let ea = &a.extras[name];
        if let Some(eb) = b.extras.get(name) {
            if ea.autofilter != eb.autofilter {
                r.extras.push(format!("{name} autofilter"));
            }
            if ea.print_xml != eb.print_xml {
                r.extras.push(format!("{name} print"));
            }
            if ea.conditional_formatting_xml != eb.conditional_formatting_xml {
                r.extras.push(format!("{name} cf"));
            }
            if ea.data_validations_xml != eb.data_validations_xml {
                r.extras.push(format!("{name} dv"));
            }
            if ea.sparkline_xml != eb.sparkline_xml {
                r.extras.push(format!("{name} sparkline"));
            }
        } else {
            r.extras.push(format!("{name} extras missing"));
        }
    }
    for (name, part) in &a.package.parts {
        let n = name.to_ascii_lowercase();
        if is_modeled_part(&n) {
            continue;
        }
        match b.package.part(name) {
            Some(p) if p.bytes == part.bytes => {}
            Some(_) => r.parts.push(format!("{name} bytes differ")),
            None => r.parts.push(format!("{name} missing after save")),
        }
    }
    r.empty = r.cells.is_empty()
        && r.views.is_empty()
        && r.names.is_empty()
        && r.tables.is_empty()
        && r.annotations.is_empty()
        && r.extras.is_empty()
        && r.styles.is_empty()
        && r.parts.is_empty();
    r
}

fn values_eq(
    _wa: &omacell_core::workbook::Workbook,
    ia: &omacell_core::intern::Interners,
    a: Value,
    ib: &omacell_core::intern::Interners,
    b: Value,
) -> bool {
    match (a, b) {
        (Value::Empty, Value::Empty) => true,
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Error(x), Value::Error(y)) => x == y,
        (Value::Text(x), Value::Text(y)) => ia.strings.get(x) == ib.strings.get(y),
        (Value::Array(_), Value::Array(_)) => true,
        _ => false,
    }
}

fn is_modeled_part(n: &str) -> bool {
    matches!(
        n,
        "[content_types].xml"
            | "_rels/.rels"
            | "xl/workbook.xml"
            | "xl/_rels/workbook.xml.rels"
            | "xl/sharedstrings.xml"
            | "xl/styles.xml"
            | "xl/calcchain.xml"
    ) || n.starts_with("xl/worksheets/")
        || n.starts_with("xl/tables/")
        || n.starts_with("xl/comments")
        || n.starts_with("xl/omacell/")
}
