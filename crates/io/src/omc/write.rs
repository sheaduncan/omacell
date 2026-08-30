//! Encode a workbook / changeset as `.omc` text.

use indexmap::IndexMap;
use omacell_core::addr::{col_to_letters, quote_sheet_name};
use omacell_core::changeset::Changeset;
use omacell_core::error::CoreError;
use omacell_core::intern::{Interners, RichTextRun};
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::names::NameReferent;
use omacell_core::sheet::SheetVisibility;
use omacell_core::style::{Style, StyleId};
use omacell_core::value::Value;
use omacell_core::workbook::{CalcMode, DateSystem};
use serde::{Deserialize, Serialize};

use super::{ConversionReport, OmcDocument};
use crate::xlsx::XlsxDocument;

const MAX_VALUE_DEPTH: u8 = 16;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum WireValue {
    Empty,
    Number {
        value: f64,
    },
    Bool {
        value: bool,
    },
    Text {
        value: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rich: Vec<RichTextRun>,
    },
    Error {
        value: omacell_core::error::ErrorKind,
    },
    Array {
        rows: u32,
        cols: u32,
        values: Vec<WireValue>,
    },
}

/// Pivot wire record uses sheet names because `.omc` does not persist `SheetId`s.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct PivotWire {
    pub(super) source_sheet: String,
    pub(super) dest_sheet: String,
    pub(super) table: omacell_core::pivot::PivotTable,
}

pub(super) fn encode(doc: &OmcDocument) -> Result<String, CoreError> {
    let mut out = String::from("omc 1\n");
    encode_workbook(&mut out, doc)?;
    if let Some(cs) = &doc.changeset {
        encode_changeset_body(&mut out, cs)?;
    }
    Ok(out)
}

pub(super) fn encode_changeset(cs: &Changeset) -> Result<String, CoreError> {
    let mut out = String::from("omc 1\n");
    encode_changeset_body(&mut out, cs)?;
    Ok(out)
}

fn encode_workbook(out: &mut String, doc: &OmcDocument) -> Result<(), CoreError> {
    let wb = &doc.workbook;
    if !wb.settings().iteration.max_change.is_finite() || wb.settings().iteration.max_change < 0.0 {
        return Err(crate::error::omc_format(
            "iteration max_change must be finite and non-negative",
        ));
    }
    for name in doc.extras.keys() {
        let sheet = wb.sheet_by_name(name).ok_or_else(|| {
            crate::error::omc_format(format!("extras reference unknown sheet {name:?}"))
        })?;
        if sheet.name != *name {
            return Err(crate::error::omc_format(format!(
                "extras sheet name {name:?} does not match workbook casing {:?}",
                sheet.name
            )));
        }
    }
    for pivot in wb.pivots().iter() {
        if wb.sheet(pivot.source_sheet).is_none() || wb.sheet(pivot.dest_sheet).is_none() {
            return Err(crate::error::omc_format(format!(
                "pivot {:?} references an unknown sheet",
                pivot.name
            )));
        }
        if pivot.dest_row >= MAX_ROWS
            || u32::from(pivot.dest_col) >= u32::from(MAX_COLS)
            || pivot.out_end_row >= MAX_ROWS
            || u32::from(pivot.out_end_col) >= u32::from(MAX_COLS)
        {
            return Err(crate::error::omc_format(format!(
                "pivot {:?} output is outside the worksheet grid",
                pivot.name
            )));
        }
    }
    let intern = wb.intern();
    let sheets: Vec<_> = wb.sheets().collect();
    let date = match wb.settings().date_system {
        DateSystem::Excel1904 => "1904",
        DateSystem::Excel1900 => "1900",
    };
    let calc = match wb.settings().calc_mode {
        CalcMode::Manual => "manual",
        CalcMode::AutomaticExceptTables => "autoNoTable",
        CalcMode::Automatic => "automatic",
    };
    let active = wb
        .sheet(wb.active_sheet())
        .map(|s| s.name.as_str())
        .ok_or_else(|| crate::error::omc_format("active sheet id is not present"))?;
    out.push_str("book");
    push_kv(out, "date_system", date);
    push_kv(out, "calc", calc);
    push_kv(out, "active", active);
    push_json_kv(out, "settings", wb.settings())?;
    if wb.protection() != &omacell_core::workbook::WorkbookProtectionState::default() {
        push_json_kv(out, "protection", wb.protection())?;
    }
    push_json_kv(out, "meta", wb.meta())?;
    out.push('\n');

    let mut style_ids: IndexMap<Style, u32> = IndexMap::new();
    for sheet in &sheets {
        for (_, _, slot) in sheet.store.iter() {
            if slot.style != StyleId::DEFAULT
                && let Some(style) = intern.styles.get(slot.style)
                && !style_ids.contains_key(style)
            {
                let i = style_ids.len() as u32 + 1;
                style_ids.insert(style.clone(), i);
            }
        }
    }
    let mut styles: Vec<(&Style, u32)> = style_ids.iter().map(|(s, i)| (s, *i)).collect();
    styles.sort_by_key(|(_, i)| *i);
    let mut formats = std::collections::BTreeMap::new();
    for (style, _) in &styles {
        let nid = style.num_fmt.index();
        if nid >= 164 {
            let code = wb.num_fmt_code(style.num_fmt).ok_or_else(|| {
                crate::error::omc_format(format!("custom number format {nid} has no code"))
            })?;
            formats.insert(nid, code.into_owned());
        }
    }
    for (nid, code) in formats {
        out.push_str("numfmt\t");
        out.push_str(&nid.to_string());
        out.push('\t');
        push_field(out, &code);
        out.push('\n');
    }
    for (style, id) in styles {
        let json =
            serde_json::to_string(style).map_err(|e| crate::error::omc_parse(e.to_string()))?;
        out.push_str("style\t");
        out.push_str(&id.to_string());
        out.push('\t');
        push_field(out, &json);
        out.push('\n');
    }

    for sheet in &sheets {
        validate_sheet_for_write(wb, sheet)?;
        out.push_str("sheet\t");
        push_field(out, &sheet.name);
        match sheet.visibility {
            SheetVisibility::Hidden => out.push_str("\thidden"),
            SheetVisibility::VeryHidden => out.push_str("\tveryHidden"),
            SheetVisibility::Visible => {}
        }
        push_json_kv(out, "view", &sheet.view)?;
        push_json_kv(out, "protection", &sheet.protection)?;
        push_json_kv(out, "tab_color", &sheet.tab_color)?;
        push_json_kv(out, "page_setup", &sheet.page_setup)?;
        let row_sizes: Vec<_> = sheet.geometry.rows.iter_custom().collect();
        let row_hidden: Vec<_> = sheet.geometry.rows.iter_hidden().collect();
        let col_sizes: Vec<_> = sheet.geometry.cols.iter_custom().collect();
        let col_hidden: Vec<_> = sheet.geometry.cols.iter_hidden().collect();
        if !row_sizes.is_empty() {
            push_json_kv(out, "row_sizes", &row_sizes)?;
        }
        if !row_hidden.is_empty() {
            push_json_kv(out, "row_hidden", &row_hidden)?;
        }
        if !col_sizes.is_empty() {
            push_json_kv(out, "col_sizes", &col_sizes)?;
        }
        if !col_hidden.is_empty() {
            push_json_kv(out, "col_hidden", &col_hidden)?;
        }
        out.push('\n');

        for (row, col, slot) in sheet.store.iter() {
            let addr = cell_addr(&sheet.name, row, col)?;
            out.push_str("cell\t");
            push_field(out, &addr);
            out.push('\t');
            if let Some(fid) = slot.formula {
                let src = intern.formulas.get(fid).ok_or_else(|| {
                    crate::error::omc_format(format!("{addr} has an unknown formula id"))
                })?;
                if src.starts_with('=') {
                    push_field(out, src);
                } else {
                    push_field(out, &format!("={src}"));
                }
                push_kv(out, "type", "formula");
                if !matches!(slot.value, Value::Empty) {
                    push_cached_value(out, intern, slot.value)?;
                }
            } else {
                push_literal(out, intern, slot.value)?;
            }
            if slot.style != StyleId::DEFAULT {
                let style = intern.styles.get(slot.style).ok_or_else(|| {
                    crate::error::omc_format(format!("{addr} has an unknown style id"))
                })?;
                let id = style_ids.get(style).ok_or_else(|| {
                    crate::error::omc_format(format!("{addr} style was not emitted"))
                })?;
                push_kv(out, "s", &id.to_string());
            }
            out.push('\n');
        }
        for m in &sheet.merges {
            validate_local_range(sheet.id, *m, "merge")?;
            out.push_str("merge\t");
            push_field(
                out,
                &format!("{}!{}", quote_sheet_name(&sheet.name), m.to_a1()),
            );
            out.push('\n');
        }
        let mut notes: Vec<_> = sheet.notes.iter().collect();
        notes.sort_by_key(|((r, c), _)| (*r, *c));
        for ((row, col), n) in notes {
            let addr = cell_addr(&sheet.name, *row, *col)?;
            out.push_str("comment\t");
            push_field(out, &addr);
            if let Some(a) = &n.author {
                push_kv(out, "author", a);
            }
            out.push('\t');
            push_field(out, &n.text);
            out.push('\n');
        }
        let mut comments: Vec<_> = sheet.comments.iter().collect();
        comments.sort_by_key(|((r, c), _)| (*r, *c));
        for ((row, col), comment) in comments {
            out.push_str("threaded_comment\t");
            push_field(out, &cell_addr(&sheet.name, *row, *col)?);
            out.push('\t');
            push_json_field(out, comment)?;
            out.push('\n');
        }
        let mut hrefs: Vec<_> = sheet.hyperlinks.iter().collect();
        hrefs.sort_by_key(|((r, c), _)| (*r, *c));
        for ((row, col), h) in hrefs {
            let addr = cell_addr(&sheet.name, *row, *col)?;
            out.push_str("hyperlink\t");
            push_field(out, &addr);
            out.push('\t');
            push_field(out, &h.target);
            if let Some(d) = &h.display {
                push_kv(out, "display", d);
            }
            if let Some(t) = &h.tooltip {
                push_kv(out, "tooltip", t);
            }
            out.push('\n');
        }
        for table in wb.tables().iter().filter(|t| t.sheet == sheet.id) {
            if table.start_row > table.end_row || table.start_col > table.end_col {
                return Err(crate::error::omc_format(format!(
                    "table {:?} has a reversed range",
                    table.name
                )));
            }
            let expected_columns = usize::from(table.end_col - table.start_col) + 1;
            if table.columns.len() != expected_columns {
                return Err(crate::error::omc_format(format!(
                    "table {:?} has {} columns but its range has {expected_columns}",
                    table.name,
                    table.columns.len()
                )));
            }
            let start = format!(
                "{}{}",
                col_to_letters(table.start_col)
                    .map_err(|e| crate::error::omc_format(e.to_string()))?,
                table.start_row + 1
            );
            let end = format!(
                "{}{}",
                col_to_letters(table.end_col)
                    .map_err(|e| crate::error::omc_format(e.to_string()))?,
                table.end_row + 1
            );
            out.push_str("table\t");
            push_field(out, &table.name);
            out.push('\t');
            push_field(
                out,
                &format!("{}!{start}:{end}", quote_sheet_name(&sheet.name)),
            );
            push_kv(out, "header", &u8::from(table.has_header).to_string());
            push_kv(out, "totals", &u8::from(table.has_totals).to_string());
            push_kv(out, "banded_rows", &u8::from(table.banded_rows).to_string());
            push_kv(out, "banded_cols", &u8::from(table.banded_cols).to_string());
            push_kv(out, "auto_expand", &u8::from(table.auto_expand).to_string());
            if !table.columns.is_empty() {
                push_json_kv(out, "columns", &table.columns)?;
            }
            out.push('\n');
        }
        for pivot in wb
            .pivots()
            .iter()
            .filter(|pivot| pivot.dest_sheet == sheet.id)
        {
            out.push_str("pivot\t");
            push_json_field(
                out,
                &PivotWire {
                    source_sheet: wb
                        .sheet(pivot.source_sheet)
                        .map(|source| source.name.clone())
                        .ok_or_else(|| {
                            crate::error::omc_format(format!(
                                "pivot {:?} references an unknown source sheet",
                                pivot.name
                            ))
                        })?,
                    dest_sheet: sheet.name.clone(),
                    table: pivot.clone(),
                },
            )?;
            out.push('\n');
        }
        if let Some(filter) = &sheet.autofilter {
            out.push_str("extra\t");
            push_field(out, &sheet.name);
            out.push_str("\tautofilter_model\t");
            push_json_field(out, filter)?;
            out.push('\n');
        }
        if !sheet.validations.is_empty() {
            out.push_str("extra\t");
            push_field(out, &sheet.name);
            out.push_str("\tvalidation_model\t");
            push_json_field(out, &sheet.validations)?;
            out.push('\n');
        }
        if !sheet.cond_formats.is_empty() {
            out.push_str("extra\t");
            push_field(out, &sheet.name);
            out.push_str("\tcondfmt_model\t");
            push_json_field(out, &sheet.cond_formats)?;
            out.push('\n');
        }
        if let Some(ex) = doc.extras.get(&sheet.name) {
            if let Some(af) = &ex.autofilter {
                let json = serde_json::to_string(af)
                    .map_err(|e| crate::error::omc_parse(e.to_string()))?;
                out.push_str("extra\t");
                push_field(out, &sheet.name);
                out.push_str("\tautofilter\t");
                push_field(out, &json);
                out.push('\n');
            }
            if !ex.autofilter_xml.is_empty() {
                let text = std::str::from_utf8(&ex.autofilter_xml).map_err(|_| {
                    crate::error::omc_format(format!(
                        "{} autofilter extra is not valid UTF-8",
                        sheet.name
                    ))
                })?;
                let json = serde_json::to_string(text)
                    .map_err(|e| crate::error::omc_parse(e.to_string()))?;
                out.push_str("extra\t");
                push_field(out, &sheet.name);
                out.push_str("\tautofilter_xml\t");
                push_field(out, &json);
                out.push('\n');
            }
            for (kind, blobs) in [
                ("cf", &ex.conditional_formatting_xml),
                ("dv", &ex.data_validations_xml),
                ("print", &ex.print_xml),
                ("sparkline", &ex.sparkline_xml),
            ] {
                for blob in blobs {
                    let text = std::str::from_utf8(blob).map_err(|_| {
                        crate::error::omc_format(format!(
                            "{} {kind} extra is not valid UTF-8",
                            sheet.name
                        ))
                    })?;
                    let json = serde_json::to_string(text)
                        .map_err(|e| crate::error::omc_parse(e.to_string()))?;
                    out.push_str("extra\t");
                    push_field(out, &sheet.name);
                    out.push('\t');
                    out.push_str(kind);
                    out.push('\t');
                    push_field(out, &json);
                    out.push('\n');
                }
            }
        }
    }
    let mut names: Vec<_> = wb.names().iter().collect();
    names.sort_by(|a, b| a.name.cmp(&b.name));
    for n in names {
        out.push_str("name\t");
        push_field(out, &n.name);
        out.push('\t');
        match &n.referent {
            NameReferent::Range(r) => push_field(out, &range_addr(wb, *r)?),
            NameReferent::Formula(f) => {
                if f.starts_with('=') {
                    push_field(out, f);
                } else {
                    push_field(out, &format!("={f}"));
                }
                push_kv(out, "type", "formula");
            }
            NameReferent::Constant(Value::Text(id)) if intern.strings.get_rich(*id).is_some() => {
                return Err(crate::error::omc_format(format!(
                    "defined name {:?} has a rich-text constant, which .omc cannot intern safely",
                    n.name
                )));
            }
            NameReferent::Constant(v) => push_literal(out, intern, *v)?,
        }
        if let omacell_core::names::NameScope::Sheet(id) = n.scope {
            let sh = wb.sheet(id).ok_or_else(|| {
                crate::error::omc_format(format!(
                    "defined name {:?} has an unknown scope sheet",
                    n.name
                ))
            })?;
            push_kv(out, "scope", &sh.name);
        }
        if let Some(comment) = &n.comment {
            push_kv(out, "comment", comment);
        }
        out.push('\n');
    }

    let mut custom_names = std::collections::BTreeSet::new();
    for (name, bytes) in &wb.custom_parts {
        validate_custom_part_name(name)?;
        if !custom_names.insert(name.to_ascii_lowercase()) {
            return Err(crate::error::omc_format(format!(
                "duplicate custom part name {name:?} ignoring case"
            )));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| {
            crate::error::omc_format(format!("custom part {name:?} is not valid UTF-8"))
        })?;
        out.push_str("custom\t");
        push_field(out, name);
        out.push('\t');
        push_field(out, text);
        out.push('\n');
    }
    Ok(())
}

fn encode_changeset_body(out: &mut String, cs: &Changeset) -> Result<(), CoreError> {
    cs.validate()?;
    let origin_value =
        serde_json::to_value(cs.origin).map_err(|e| crate::error::omc_parse(e.to_string()))?;
    let origin = origin_value
        .as_str()
        .ok_or_else(|| crate::error::omc_format("changeset origin did not serialize as text"))?;
    let status = match cs.status {
        omacell_core::changeset::ChangesetStatus::Proposed => "proposed",
        omacell_core::changeset::ChangesetStatus::Applied => "applied",
        omacell_core::changeset::ChangesetStatus::Reverted => "reverted",
    };
    out.push_str("changeset");
    push_kv(out, "id", cs.id.as_str());
    push_kv(out, "status", status);
    push_kv(out, "origin", origin);
    push_kv(out, "cells", &cs.summary.cells.to_string());
    push_kv(out, "rows", &cs.summary.rows.to_string());
    push_kv(out, "columns", &cs.summary.columns.to_string());
    push_kv(out, "sheets", &cs.summary.sheets.to_string());
    push_kv(out, "styles", &cs.summary.styles.to_string());
    push_kv(out, "text", &cs.summary.text);
    out.push('\n');
    for call in &cs.forward {
        let json = serde_json::to_string(&call.args)
            .map_err(|e| crate::error::omc_parse(e.to_string()))?;
        out.push_str("change\tforward\t");
        push_field(out, call.id.as_str());
        out.push('\t');
        push_field(out, &json);
        out.push('\n');
    }
    for call in &cs.inverse {
        let json = serde_json::to_string(&call.args)
            .map_err(|e| crate::error::omc_parse(e.to_string()))?;
        out.push_str("change\tinverse\t");
        push_field(out, call.id.as_str());
        out.push('\t');
        push_field(out, &json);
        out.push('\n');
    }
    Ok(())
}

pub(super) fn from_xlsx(doc: &XlsxDocument) -> (OmcDocument, ConversionReport) {
    let mut report = ConversionReport::default();
    for (name, _) in &doc.package.parts {
        if !is_modeled_part(name) {
            report.dropped.push(name.clone());
        }
    }
    for (name, bytes) in &doc.workbook.custom_parts {
        if std::str::from_utf8(bytes).is_err() {
            report.dropped.push(format!("{name} (non-UTF-8)"));
        }
    }
    let mut workbook = doc.workbook.clone();
    workbook
        .custom_parts
        .retain(|_, bytes| std::str::from_utf8(bytes).is_ok());
    (
        OmcDocument {
            workbook,
            extras: doc.extras.clone(),
            changeset: None,
        },
        report,
    )
}

fn push_literal(out: &mut String, intern: &Interners, v: Value) -> Result<(), CoreError> {
    match v {
        Value::Empty => {}
        Value::Number(n) => {
            if !n.is_finite() {
                return Err(crate::error::omc_format(
                    "non-finite numbers cannot be represented in .omc",
                ));
            }
            if n == 0.0 && n.is_sign_negative() {
                out.push_str("-0");
            } else if n.fract() == 0.0 && n.abs() < 1e15 {
                out.push_str(&format!("{}", n as i64));
            } else {
                out.push_str(&n.to_string());
            }
        }
        Value::Bool(true) => out.push_str("TRUE"),
        Value::Bool(false) => out.push_str("FALSE"),
        Value::Error(e) => out.push_str(e.as_str()),
        Value::Text(id) => {
            let t = intern.strings.get(id).ok_or_else(|| {
                crate::error::omc_format(format!("unknown string id {}", id.index()))
            })?;
            push_quoted_field(out, t);
            push_kv(out, "type", "text");
            if let Some(rich) = intern.strings.get_rich(id) {
                push_json_kv(out, "rich", rich)?;
            }
        }
        Value::Array(id) => {
            let wire = value_to_wire(intern, Value::Array(id), 0)?;
            push_json_kv(out, "array", &wire)?;
        }
    }
    Ok(())
}

fn push_cached_value(out: &mut String, intern: &Interners, value: Value) -> Result<(), CoreError> {
    match value {
        Value::Text(id) => {
            let text = intern.strings.get(id).ok_or_else(|| {
                crate::error::omc_format(format!("unknown string id {}", id.index()))
            })?;
            push_kv(out, "v_text", text);
            if let Some(rich) = intern.strings.get_rich(id) {
                push_json_kv(out, "v_rich", rich)?;
            }
        }
        Value::Array(_) => {
            let wire = value_to_wire(intern, value, 0)?;
            push_json_kv(out, "v_array", &wire)?;
        }
        _ => {
            let mut literal = String::new();
            push_literal(&mut literal, intern, value)?;
            push_kv(out, "v", &literal);
        }
    }
    Ok(())
}

fn value_to_wire(intern: &Interners, value: Value, depth: u8) -> Result<WireValue, CoreError> {
    if depth >= MAX_VALUE_DEPTH {
        return Err(crate::error::omc_limit("array nesting exceeds 16 levels"));
    }
    Ok(match value {
        Value::Empty => WireValue::Empty,
        Value::Number(value) if value.is_finite() => WireValue::Number { value },
        Value::Number(_) => {
            return Err(crate::error::omc_format(
                "non-finite numbers cannot be represented in .omc",
            ));
        }
        Value::Bool(value) => WireValue::Bool { value },
        Value::Text(id) => {
            if intern.strings.get_rich(id).is_some() && depth > 0 {
                return Err(crate::error::omc_format(
                    "rich text inside an array is not supported by .omc",
                ));
            }
            WireValue::Text {
                value: intern
                    .strings
                    .get(id)
                    .ok_or_else(|| {
                        crate::error::omc_format(format!("unknown string id {}", id.index()))
                    })?
                    .to_string(),
                rich: intern
                    .strings
                    .get_rich(id)
                    .map_or_else(Vec::new, ToOwned::to_owned),
            }
        }
        Value::Error(value) => WireValue::Error { value },
        Value::Array(id) => {
            let payload = intern.arrays.get(id).ok_or_else(|| {
                crate::error::omc_format(format!("unknown array id {}", id.index()))
            })?;
            let values = payload
                .values
                .iter()
                .map(|value| value_to_wire(intern, *value, depth + 1))
                .collect::<Result<_, _>>()?;
            WireValue::Array {
                rows: payload.shape.rows,
                cols: payload.shape.cols,
                values,
            }
        }
    })
}

fn push_field(out: &mut String, s: &str) {
    if needs_quote(s) {
        out.push('"');
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                _ => out.push(c),
            }
        }
        out.push('"');
    } else {
        out.push_str(s);
    }
}

fn push_quoted_field(out: &mut String, s: &str) {
    out.push('"');
    push_escaped(out, s);
    out.push('"');
}

fn push_escaped(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
}

fn push_kv(out: &mut String, key: &str, value: &str) {
    out.push('\t');
    push_field(out, &format!("{key}={value}"));
}

fn push_json_kv<T: Serialize + ?Sized>(
    out: &mut String,
    key: &str,
    value: &T,
) -> Result<(), CoreError> {
    let json = serde_json::to_string(value).map_err(|e| crate::error::omc_parse(e.to_string()))?;
    push_kv(out, key, &json);
    Ok(())
}

fn push_json_field<T: Serialize + ?Sized>(out: &mut String, value: &T) -> Result<(), CoreError> {
    let json = serde_json::to_string(value).map_err(|e| crate::error::omc_parse(e.to_string()))?;
    push_field(out, &json);
    Ok(())
}

fn needs_quote(s: &str) -> bool {
    s.is_empty()
        || s.contains(['\t', '\n', '\r', '"'])
        || s.starts_with(' ')
        || s.ends_with(' ')
        || s.starts_with('#')
}

fn validate_sheet_for_write(
    wb: &omacell_core::workbook::Workbook,
    sheet: &omacell_core::sheet::Sheet,
) -> Result<(), CoreError> {
    let view = &sheet.view;
    if !view.zoom.is_finite() || view.zoom <= 0.0 || view.zoom > 8.0 {
        return Err(crate::error::omc_format(format!(
            "sheet {:?} zoom must be finite and in (0, 8]",
            sheet.name
        )));
    }
    view.selection.start.validate()?;
    view.selection.end.validate()?;
    if view.freeze.rows > MAX_ROWS
        || view.freeze.cols > MAX_COLS
        || view.scroll_row >= MAX_ROWS
        || view.scroll_col >= MAX_COLS
    {
        return Err(crate::error::omc_format(format!(
            "sheet {:?} view is out of range",
            sheet.name
        )));
    }
    for id in [
        view.selection.start.sheet,
        view.selection.end.sheet,
        view.selection.sheet_end,
    ]
    .into_iter()
    .flatten()
    {
        if wb.sheet(id).is_none() {
            return Err(crate::error::omc_format(format!(
                "sheet {:?} view references unknown sheet id {}",
                sheet.name,
                id.index()
            )));
        }
    }
    Ok(())
}

fn validate_local_range(
    sheet: omacell_core::addr::SheetId,
    range: omacell_core::addr::RangeRef,
    what: &str,
) -> Result<(), CoreError> {
    range.start.validate()?;
    range.end.validate()?;
    if range.sheet_end.is_some()
        || range.start.sheet.is_some_and(|id| id != sheet)
        || range.end.sheet.is_some_and(|id| id != sheet)
    {
        return Err(crate::error::omc_format(format!(
            "{what} range references a different sheet"
        )));
    }
    Ok(())
}

fn cell_addr(sheet: &str, row: u32, col: u16) -> Result<String, CoreError> {
    let col = col_to_letters(col).map_err(|e| crate::error::omc_format(e.to_string()))?;
    Ok(format!("{}!{col}{}", quote_sheet_name(sheet), row + 1))
}

fn range_addr(
    wb: &omacell_core::workbook::Workbook,
    range: omacell_core::addr::RangeRef,
) -> Result<String, CoreError> {
    range.start.validate()?;
    range.end.validate()?;
    if range.start.sheet != range.end.sheet {
        return Err(crate::error::omc_format(
            "defined-name range endpoints have different sheets",
        ));
    }
    let Some(sheet_id) = range.start.sheet else {
        if range.sheet_end.is_some() {
            return Err(crate::error::omc_format(
                "defined-name 3-D range is missing its start sheet",
            ));
        }
        return Ok(range.to_a1());
    };
    let start = wb
        .sheet(sheet_id)
        .ok_or_else(|| crate::error::omc_format("defined name has an unknown sheet id"))?;
    let prefix = if let Some(end_id) = range.sheet_end {
        let end = wb
            .sheet(end_id)
            .ok_or_else(|| crate::error::omc_format("defined name has an unknown end sheet id"))?;
        omacell_core::addr::SheetSpec {
            start: start.name.clone(),
            end: Some(end.name.clone()),
        }
        .to_a1_prefix()
    } else {
        format!("{}!", quote_sheet_name(&start.name))
    };
    Ok(format!("{prefix}{}", range.to_a1()))
}

pub(super) fn validate_custom_part_name(name: &str) -> Result<(), CoreError> {
    if name.is_empty()
        || name.starts_with('/')
        || name.ends_with('/')
        || name.contains('\\')
        || !name.to_ascii_lowercase().starts_with("xl/omacell/")
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(crate::error::omc_format(format!(
            "custom part {name:?} must name a file below xl/omacell/"
        )));
    }
    Ok(())
}

fn is_modeled_part(name: &str) -> bool {
    let n = name.replace('\\', "/").to_ascii_lowercase();
    matches!(
        n.as_str(),
        "[content_types].xml"
            | "_rels/.rels"
            | "xl/workbook.xml"
            | "xl/_rels/workbook.xml.rels"
            | "xl/sharedstrings.xml"
            | "xl/styles.xml"
    ) || (n.starts_with("xl/worksheets/") && n.ends_with(".xml") && !n.contains("/_rels/"))
        || (n.starts_with("xl/tables/") && n.ends_with(".xml"))
        || (n.starts_with("xl/comments") && n.ends_with(".xml"))
        || n.starts_with("xl/omacell/")
}
