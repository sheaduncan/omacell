//! Model-level L1/L2 comparison for round-trip tests and `omacell diff`.

use std::collections::{BTreeMap, BTreeSet};

use omacell_core::addr::col_to_letters;
use omacell_core::formula::{parse, print};
use omacell_core::intern::Interners;
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::storage::CellSlot;
use omacell_core::style::NumFmtId;
use omacell_core::tables::Table;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use serde::{Deserialize, Serialize};

use super::XlsxDocument;

/// JSON-serializable diff. Empty lists mean L1/L2 match.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffReport {
    /// True when every category is empty.
    pub empty: bool,
    /// Cell value / formula mismatches (`Sheet!A1: ...`).
    pub cells: Vec<String>,
    /// Workbook settings, active sheet, and sheet layout/view mismatches.
    pub views: Vec<String>,
    /// Defined names.
    pub names: Vec<String>,
    /// Tables.
    pub tables: Vec<String>,
    /// Hyperlinks and notes.
    pub annotations: Vec<String>,
    /// Exact extra-fragment and custom-payload mismatches.
    pub extras: Vec<String>,
    /// Style / number-format mismatches.
    pub styles: Vec<String>,
    /// L3 part bytes/content types that differ (non-rewritten parts).
    pub parts: Vec<String>,
}

/// Compare two opened documents.
#[must_use]
pub fn diff(a: &XlsxDocument, b: &XlsxDocument) -> DiffReport {
    let mut r = DiffReport::default();
    let intern_a = a.workbook.intern();
    let intern_b = b.workbook.intern();
    if a.workbook.settings() != b.workbook.settings() {
        r.views.push("workbook settings differ".into());
    }
    if active_sheet_name(&a.workbook) != active_sheet_name(&b.workbook) {
        r.views.push("active sheet differs".into());
    }
    let sheet_names: BTreeSet<String> = a
        .workbook
        .sheets()
        .chain(b.workbook.sheets())
        .map(|sheet| sheet.name.clone())
        .collect();
    for name in sheet_names {
        match (
            a.workbook.sheet_by_name(&name),
            b.workbook.sheet_by_name(&name),
        ) {
            (Some(sa), Some(sb)) => {
                compare_sheet(&mut r, &a.workbook, intern_a, sa, &b.workbook, intern_b, sb)
            }
            (Some(_), None) => r.views.push(format!("sheet {name} missing from right")),
            (None, Some(_)) => r.views.push(format!("sheet {name} missing from left")),
            (None, None) => {}
        }
    }
    compare_names(&mut r, &a.workbook, intern_a, &b.workbook, intern_b);
    compare_tables(&mut r, &a.workbook, &b.workbook);
    let extra_names: BTreeSet<&String> = a.extras.keys().chain(b.extras.keys()).collect();
    for name in extra_names {
        match (a.extras.get(name), b.extras.get(name)) {
            (Some(ea), Some(eb)) if ea == eb => {}
            (Some(_), Some(_)) => r.extras.push(format!("{name} extras differ")),
            (Some(_), None) => r.extras.push(format!("{name} extras missing from right")),
            (None, Some(_)) => r.extras.push(format!("{name} extras missing from left")),
            (None, None) => {}
        }
    }
    let custom_names: BTreeSet<&String> = a
        .workbook
        .custom_parts
        .keys()
        .chain(b.workbook.custom_parts.keys())
        .collect();
    for name in custom_names {
        if a.workbook.custom_parts.get(name) != b.workbook.custom_parts.get(name) {
            r.parts.push(format!("{name} custom payload differs"));
        }
    }
    let part_names: BTreeSet<&String> = a
        .package
        .parts
        .keys()
        .chain(b.package.parts.keys())
        .collect();
    for name in part_names {
        if is_modeled_part(&name.to_ascii_lowercase())
            || is_note_vml_part(a, name)
            || is_note_vml_part(b, name)
        {
            continue;
        }
        match (a.package.part(name), b.package.part(name)) {
            (Some(pa), Some(pb)) if pa.bytes == pb.bytes && pa.content_type == pb.content_type => {}
            (Some(pa), Some(pb)) if pa.bytes != pb.bytes => {
                r.parts.push(format!("{name} bytes differ"));
            }
            (Some(_), Some(_)) => r.parts.push(format!("{name} content type differs")),
            (Some(_), None) => r.parts.push(format!("{name} missing from right")),
            (None, Some(_)) => r.parts.push(format!("{name} missing from left")),
            (None, None) => {}
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

#[allow(clippy::too_many_arguments)]
fn compare_sheet(
    report: &mut DiffReport,
    wa: &Workbook,
    ia: &Interners,
    a: &omacell_core::sheet::Sheet,
    wb: &Workbook,
    ib: &Interners,
    b: &omacell_core::sheet::Sheet,
) {
    let name = &a.name;
    if a.visibility != b.visibility {
        report.views.push(format!("{name} visibility differs"));
    }
    if a.tab_color != b.tab_color {
        report.views.push(format!("{name} tab color differs"));
    }
    if a.view != b.view {
        report.views.push(format!("{name} view differs"));
    }
    if a.protection != b.protection {
        report.views.push(format!("{name} protection differs"));
    }
    if a.merges != b.merges {
        report.views.push(format!("{name} merges differ"));
    }
    if a.charts.len() != b.charts.len() {
        report.views.push(format!("{name} chart count differs"));
    } else {
        for (ca, cb) in a.charts.iter().zip(&b.charts) {
            if ca.kind != cb.kind || ca.title != cb.title || ca.series.len() != cb.series.len() {
                report
                    .views
                    .push(format!("{name} chart {} differs", ca.id.index()));
            }
        }
    }
    if a.sparklines.len() != b.sparklines.len() {
        report.views.push(format!("{name} sparkline count differs"));
    }
    if a.geometry.rows.iter_hidden().collect::<Vec<_>>()
        != b.geometry.rows.iter_hidden().collect::<Vec<_>>()
        || a.geometry.rows.iter_custom().collect::<Vec<_>>()
            != b.geometry.rows.iter_custom().collect::<Vec<_>>()
    {
        report.views.push(format!("{name} row geometry differs"));
    }
    if a.geometry.cols.iter_hidden().collect::<Vec<_>>()
        != b.geometry.cols.iter_hidden().collect::<Vec<_>>()
        || a.geometry.cols.iter_custom().collect::<Vec<_>>()
            != b.geometry.cols.iter_custom().collect::<Vec<_>>()
    {
        report.views.push(format!("{name} column geometry differs"));
    }
    let cells_a: BTreeMap<_, _> = a
        .store
        .iter()
        .map(|(row, col, slot)| ((row, col), slot))
        .collect();
    let cells_b: BTreeMap<_, _> = b
        .store
        .iter()
        .map(|(row, col, slot)| ((row, col), slot))
        .collect();
    let coords: BTreeSet<_> = cells_a.keys().chain(cells_b.keys()).copied().collect();
    for (row, col) in coords {
        let address = format!(
            "{name}!{}{}",
            col_to_letters(col).unwrap_or_else(|_| "?".into()),
            row + 1
        );
        match (cells_a.get(&(row, col)), cells_b.get(&(row, col))) {
            (Some(sa), Some(sb)) => compare_slot(report, wa, ia, sa, wb, ib, sb, &address),
            (Some(_), None) => report.cells.push(format!("{address}: missing from right")),
            (None, Some(_)) => report.cells.push(format!("{address}: missing from left")),
            (None, None) => {}
        }
    }
    compare_annotations(report, name, a, b);
}

#[allow(clippy::too_many_arguments)]
fn compare_slot(
    report: &mut DiffReport,
    wa: &Workbook,
    ia: &Interners,
    a: &CellSlot,
    wb: &Workbook,
    ib: &Interners,
    b: &CellSlot,
    address: &str,
) {
    let fa = a.formula.and_then(|id| ia.formulas.get(id));
    let fb = b.formula.and_then(|id| ib.formulas.get(id));
    if !formulas_eq(fa, fb) {
        report
            .cells
            .push(format!("{address}: formula {fa:?} vs {fb:?}"));
    }
    if !values_eq(ia, a.value, ib, b.value, 0) {
        report.cells.push(format!("{address}: value differs"));
    }
    if !styles_eq(wa, ia, a, wb, ib, b) {
        report.styles.push(format!("{address}: style differs"));
    }
}

fn formulas_eq(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => formula_sources_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn formula_sources_eq(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (Ok(a), Ok(b)) = (parse(a), parse(b)) else {
        return false;
    };
    print(&a) == print(&b)
}

fn styles_eq(
    wa: &Workbook,
    ia: &Interners,
    a: &CellSlot,
    wb: &Workbook,
    ib: &Interners,
    b: &CellSlot,
) -> bool {
    let (Some(style_a), Some(style_b)) = (ia.styles.get(a.style), ib.styles.get(b.style)) else {
        return false;
    };
    let code_a = wa.num_fmt_code(style_a.num_fmt);
    let code_b = wb.num_fmt_code(style_b.num_fmt);
    let mut normalized_a = style_a.clone();
    let mut normalized_b = style_b.clone();
    normalized_a.num_fmt = NumFmtId::GENERAL;
    normalized_b.num_fmt = NumFmtId::GENERAL;
    normalized_a == normalized_b && code_a == code_b
}

fn compare_annotations(
    report: &mut DiffReport,
    sheet_name: &str,
    a: &omacell_core::sheet::Sheet,
    b: &omacell_core::sheet::Sheet,
) {
    let hyperlink_cells: BTreeSet<_> = a
        .hyperlinks
        .keys()
        .chain(b.hyperlinks.keys())
        .copied()
        .collect();
    for (row, col) in hyperlink_cells {
        if a.hyperlinks.get(&(row, col)) != b.hyperlinks.get(&(row, col)) {
            report
                .annotations
                .push(format!("{sheet_name} hyperlink r{row}c{col} differs"));
        }
    }
    let note_cells: BTreeSet<_> = a.notes.keys().chain(b.notes.keys()).copied().collect();
    for (row, col) in note_cells {
        if a.notes.get(&(row, col)) != b.notes.get(&(row, col)) {
            report
                .annotations
                .push(format!("{sheet_name} note r{row}c{col} differs"));
        }
    }
    let comment_cells: BTreeSet<_> = a
        .comments
        .keys()
        .chain(b.comments.keys())
        .copied()
        .collect();
    for (row, col) in comment_cells {
        if a.comments.get(&(row, col)) != b.comments.get(&(row, col)) {
            report.annotations.push(format!(
                "{sheet_name} threaded comment r{row}c{col} differs"
            ));
        }
    }
}

fn compare_names(
    report: &mut DiffReport,
    wa: &Workbook,
    ia: &Interners,
    wb: &Workbook,
    ib: &Interners,
) {
    let names_a: BTreeMap<_, _> = wa
        .names()
        .iter()
        .map(|name| (name_key(wa, name), name))
        .collect();
    let names_b: BTreeMap<_, _> = wb
        .names()
        .iter()
        .map(|name| (name_key(wb, name), name))
        .collect();
    let keys: BTreeSet<_> = names_a.keys().chain(names_b.keys()).cloned().collect();
    for key in keys {
        match (names_a.get(&key), names_b.get(&key)) {
            (Some(a), Some(b)) if defined_names_eq(ia, a, ib, b) => {}
            (Some(_), Some(_)) => report.names.push(format!("name {key} differs")),
            (Some(_), None) => report.names.push(format!("name {key} missing from right")),
            (None, Some(_)) => report.names.push(format!("name {key} missing from left")),
            (None, None) => {}
        }
    }
}

fn name_key(workbook: &Workbook, name: &DefinedName) -> String {
    let scope = match name.scope {
        NameScope::Workbook => "workbook".into(),
        NameScope::Sheet(id) => workbook
            .sheet(id)
            .map(|sheet| sheet.name.clone())
            .unwrap_or_else(|| format!("sheet#{}", id.index())),
    };
    format!("{scope}:{}", name.name.to_lowercase())
}

fn defined_names_eq(ia: &Interners, a: &DefinedName, ib: &Interners, b: &DefinedName) -> bool {
    if a.name != b.name || a.comment != b.comment {
        return false;
    }
    match (&a.referent, &b.referent) {
        (NameReferent::Range(a), NameReferent::Range(b)) => a == b,
        (NameReferent::Formula(a), NameReferent::Formula(b)) => formula_sources_eq(a, b),
        (NameReferent::Constant(a), NameReferent::Constant(b)) => values_eq(ia, *a, ib, *b, 0),
        _ => false,
    }
}

fn compare_tables(report: &mut DiffReport, wa: &Workbook, wb: &Workbook) {
    let tables_a: BTreeMap<_, _> = wa
        .tables()
        .iter()
        .map(|table| (table.name.to_lowercase(), table))
        .collect();
    let tables_b: BTreeMap<_, _> = wb
        .tables()
        .iter()
        .map(|table| (table.name.to_lowercase(), table))
        .collect();
    let keys: BTreeSet<_> = tables_a.keys().chain(tables_b.keys()).cloned().collect();
    for key in keys {
        match (tables_a.get(&key), tables_b.get(&key)) {
            (Some(a), Some(b)) if tables_eq(wa, a, wb, b) => {}
            (Some(_), Some(_)) => report.tables.push(format!("table {key} differs")),
            (Some(_), None) => report
                .tables
                .push(format!("table {key} missing from right")),
            (None, Some(_)) => report.tables.push(format!("table {key} missing from left")),
            (None, None) => {}
        }
    }
}

fn tables_eq(wa: &Workbook, a: &Table, wb: &Workbook, b: &Table) -> bool {
    let sheet_a = wa.sheet(a.sheet).map(|sheet| &sheet.name);
    let sheet_b = wb.sheet(b.sheet).map(|sheet| &sheet.name);
    sheet_a == sheet_b
        && a.name == b.name
        && a.start_row == b.start_row
        && a.start_col == b.start_col
        && a.end_row == b.end_row
        && a.end_col == b.end_col
        && a.has_header == b.has_header
        && a.has_totals == b.has_totals
        && a.banded_rows == b.banded_rows
        && a.banded_cols == b.banded_cols
        && a.columns == b.columns
}

fn active_sheet_name(workbook: &Workbook) -> Option<&str> {
    workbook
        .sheet(workbook.active_sheet())
        .map(|sheet| sheet.name.as_str())
}

fn values_eq(ia: &Interners, a: Value, ib: &Interners, b: Value, depth: u8) -> bool {
    match (a, b) {
        (Value::Empty, Value::Empty) => true,
        (Value::Number(x), Value::Number(y)) => x.to_bits() == y.to_bits(),
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Error(x), Value::Error(y)) => x == y,
        (Value::Text(x), Value::Text(y)) => {
            ia.strings.get(x) == ib.strings.get(y)
                && ia.strings.get_rich(x) == ib.strings.get_rich(y)
        }
        (Value::Array(x), Value::Array(y)) if depth < 16 => {
            let (Some(x), Some(y)) = (ia.arrays.get(x), ib.arrays.get(y)) else {
                return false;
            };
            x.shape == y.shape
                && x.values.len() == y.values.len()
                && x.values
                    .iter()
                    .zip(y.values.iter())
                    .all(|(x, y)| values_eq(ia, *x, ib, *y, depth + 1))
        }
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
        || n.starts_with("xl/charts/")
        || n.starts_with("xl/drawings/")
        || n.starts_with("xl/omacell/")
}

fn is_note_vml_part(document: &XlsxDocument, name: &str) -> bool {
    if !name.to_ascii_lowercase().ends_with(".vml") {
        return false;
    }
    let Some(part) = document.package.part(name) else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(&part.bytes) else {
        return false;
    };
    let mut rest = text;
    let mut saw_note = false;
    while let Some(index) = rest.find("ObjectType=\"") {
        rest = &rest[index + "ObjectType=\"".len()..];
        let Some(end) = rest.find('"') else {
            return false;
        };
        if &rest[..end] != "Note" {
            return false;
        }
        saw_note = true;
        rest = &rest[end + 1..];
    }
    saw_note
}
