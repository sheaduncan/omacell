//! Encode a workbook / changeset as `.omc` text.

use indexmap::IndexMap;
use omacell_core::addr::col_to_letters;
use omacell_core::changeset::Changeset;
use omacell_core::error::CoreError;
use omacell_core::names::NameReferent;
use omacell_core::sheet::SheetVisibility;
use omacell_core::style::{Style, StyleId};
use omacell_core::value::Value;
use omacell_core::workbook::{CalcMode, DateSystem};

use super::{ConversionReport, OmcDocument};
use crate::xlsx::XlsxDocument;

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
        .unwrap_or("Sheet1");
    out.push_str("book\t");
    out.push_str(&format!("date_system={date}\tcalc={calc}\tactive="));
    push_field(out, active);
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
    let mut seen_fmt = std::collections::BTreeSet::new();
    for (style, _) in &styles {
        let nid = style.num_fmt.index();
        if nid >= 164
            && seen_fmt.insert(nid)
            && let Some(code) = wb.num_fmt_code(style.num_fmt)
        {
            out.push_str("numfmt\t");
            out.push_str(&nid.to_string());
            out.push('\t');
            push_field(out, code.as_ref());
            out.push('\n');
        }
    }
    for (style, id) in styles {
        let json =
            serde_json::to_string(style).map_err(|e| crate::error::omc_parse(e.to_string()))?;
        out.push_str("style\t");
        out.push_str(&id.to_string());
        out.push('\t');
        out.push_str(&json);
        out.push('\n');
    }

    for sheet in &sheets {
        out.push_str("sheet\t");
        push_field(out, &sheet.name);
        match sheet.visibility {
            SheetVisibility::Hidden => out.push_str("\thidden"),
            SheetVisibility::VeryHidden => out.push_str("\tveryHidden"),
            SheetVisibility::Visible => {}
        }
        if sheet.view.freeze.rows > 0 || sheet.view.freeze.cols > 0 {
            let addr = format!(
                "{}{}",
                col_to_letters(sheet.view.freeze.cols).unwrap_or_else(|_| "A".into()),
                sheet.view.freeze.rows + 1
            );
            out.push_str("\tfreeze=");
            out.push_str(&addr);
        }
        if (sheet.view.zoom - 1.0).abs() > f64::EPSILON {
            out.push_str(&format!("\tzoom={}", (sheet.view.zoom * 100.0).round()));
        }
        if let Some(split) = sheet.view.split {
            out.push_str(&format!("\tsplit={},{}", split.x_px, split.y_px));
        }
        if sheet.protection.enabled {
            out.push_str("\tprotect=1");
        }
        let custom: Vec<(u32, u32)> = sheet.geometry.cols.iter_custom().collect();
        if !custom.is_empty() {
            out.push_str("\tcols=");
            let mut first = true;
            for (i, px) in custom {
                if !first {
                    out.push(',');
                }
                first = false;
                let w = f64::from(px) * 8.43 / f64::from(omacell_core::geometry::DEFAULT_COL_PX);
                out.push_str(&format!(
                    "{}:{w}",
                    col_to_letters(i as u16).unwrap_or_else(|_| "A".into())
                ));
            }
        }
        out.push('\n');

        for (row, col, slot) in sheet.store.iter() {
            let addr = format!(
                "{}!{}{}",
                sheet.name,
                col_to_letters(col).unwrap_or_else(|_| "A".into()),
                row + 1
            );
            out.push_str("cell\t");
            push_field(out, &addr);
            out.push('\t');
            if let Some(fid) = slot.formula
                && let Some(src) = intern.formulas.get(fid)
            {
                if src.starts_with('=') {
                    push_field(out, src);
                } else {
                    push_field(out, &format!("={src}"));
                }
                if !matches!(slot.value, Value::Empty) {
                    out.push_str("\tv=");
                    // v= is a kv field; emit the literal after =
                    let mut tmp = String::new();
                    push_literal(&mut tmp, intern, slot.value);
                    out.push_str(&tmp);
                }
            } else {
                push_literal(out, intern, slot.value);
            }
            if slot.style != StyleId::DEFAULT
                && let Some(style) = intern.styles.get(slot.style)
                && let Some(id) = style_ids.get(style)
            {
                out.push_str(&format!("\ts={id}"));
            }
            out.push('\n');
        }
        for m in &sheet.merges {
            out.push_str("merge\t");
            push_field(out, &format!("{}!{}", sheet.name, m.to_a1()));
            out.push('\n');
        }
        let mut notes: Vec<_> = sheet.notes.iter().collect();
        notes.sort_by_key(|((r, c), _)| (*r, *c));
        for ((row, col), n) in notes {
            let addr = format!(
                "{}!{}{}",
                sheet.name,
                col_to_letters(*col).unwrap_or_else(|_| "A".into()),
                row + 1
            );
            out.push_str("comment\t");
            push_field(out, &addr);
            if let Some(a) = &n.author {
                out.push_str("\tauthor=");
                push_field(out, a);
            }
            out.push('\t');
            push_field(out, &n.text);
            out.push('\n');
        }
        let mut hrefs: Vec<_> = sheet.hyperlinks.iter().collect();
        hrefs.sort_by_key(|((r, c), _)| (*r, *c));
        for ((row, col), h) in hrefs {
            let addr = format!(
                "{}!{}{}",
                sheet.name,
                col_to_letters(*col).unwrap_or_else(|_| "A".into()),
                row + 1
            );
            out.push_str("hyperlink\t");
            push_field(out, &addr);
            out.push('\t');
            push_field(out, &h.target);
            if let Some(d) = &h.display {
                out.push_str("\tdisplay=");
                push_field(out, d);
            }
            out.push('\n');
        }
        for table in wb.tables().iter().filter(|t| t.sheet == sheet.id) {
            let start = format!(
                "{}{}",
                col_to_letters(table.start_col).unwrap_or_else(|_| "A".into()),
                table.start_row + 1
            );
            let end = format!(
                "{}{}",
                col_to_letters(table.end_col).unwrap_or_else(|_| "A".into()),
                table.end_row + 1
            );
            out.push_str("table\t");
            push_field(out, &table.name);
            out.push('\t');
            push_field(out, &format!("{}!{start}:{end}", sheet.name));
            out.push_str(&format!(
                "\theader={}\ttotals={}",
                u8::from(table.has_header),
                u8::from(table.has_totals)
            ));
            if !table.columns.is_empty() {
                let cols: Vec<_> = table.columns.iter().map(|c| c.name.as_str()).collect();
                out.push_str("\tcols=");
                out.push_str(&cols.join(","));
            }
            out.push('\n');
        }
        if let Some(ex) = doc.extras.get(&sheet.name) {
            if let Some(af) = &ex.autofilter {
                out.push_str("extra\t");
                push_field(out, &sheet.name);
                out.push_str("\tautofilter\t");
                push_field(out, af);
                out.push('\n');
            }
            for (kind, blobs) in [
                ("cf", &ex.conditional_formatting_xml),
                ("dv", &ex.data_validations_xml),
                ("print", &ex.print_xml),
                ("sparkline", &ex.sparkline_xml),
            ] {
                for blob in blobs {
                    let json = serde_json::to_string(&String::from_utf8_lossy(blob))
                        .map_err(|e| crate::error::omc_parse(e.to_string()))?;
                    out.push_str("extra\t");
                    push_field(out, &sheet.name);
                    out.push('\t');
                    out.push_str(kind);
                    out.push('\t');
                    out.push_str(&json);
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
            NameReferent::Range(r) => out.push_str(&r.to_a1()),
            NameReferent::Formula(f) => {
                if f.starts_with('=') {
                    out.push_str(f);
                } else {
                    out.push('=');
                    out.push_str(f);
                }
            }
            NameReferent::Constant(v) => push_literal(out, intern, *v),
        }
        if let omacell_core::names::NameScope::Sheet(id) = n.scope
            && let Some(sh) = wb.sheet(id)
        {
            out.push_str("\tscope=");
            push_field(out, &sh.name);
        }
        out.push('\n');
    }

    for (name, bytes) in &wb.custom_parts {
        if let Ok(text) = std::str::from_utf8(bytes) {
            out.push_str("custom\t");
            push_field(out, name);
            out.push('\t');
            push_field(out, text);
            out.push('\n');
        }
    }
    Ok(())
}

fn encode_changeset_body(out: &mut String, cs: &Changeset) -> Result<(), CoreError> {
    let origin = serde_json::to_value(cs.origin)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "user".into());
    let status = match cs.status {
        omacell_core::changeset::ChangesetStatus::Proposed => "proposed",
        omacell_core::changeset::ChangesetStatus::Applied => "applied",
        omacell_core::changeset::ChangesetStatus::Reverted => "reverted",
    };
    out.push_str("changeset\tid=");
    push_field(out, cs.id.as_str());
    out.push_str(&format!("\tstatus={status}\torigin="));
    push_field(out, &origin);
    if !cs.summary.text.is_empty() {
        out.push_str("\ttext=");
        push_field(out, &cs.summary.text);
    }
    out.push('\n');
    for call in &cs.forward {
        let json = serde_json::to_string(&call.args)
            .map_err(|e| crate::error::omc_parse(e.to_string()))?;
        out.push_str("change\tforward\t");
        push_field(out, call.id.as_str());
        out.push('\t');
        out.push_str(&json);
        out.push('\n');
    }
    for call in &cs.inverse {
        let json = serde_json::to_string(&call.args)
            .map_err(|e| crate::error::omc_parse(e.to_string()))?;
        out.push_str("change\tinverse\t");
        push_field(out, call.id.as_str());
        out.push('\t');
        out.push_str(&json);
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
    (
        OmcDocument {
            workbook: doc.workbook.clone(),
            extras: doc.extras.clone(),
            changeset: None,
        },
        report,
    )
}

fn push_literal(out: &mut String, intern: &omacell_core::intern::Interners, v: Value) {
    match v {
        Value::Empty => {}
        Value::Number(n) => {
            if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                out.push_str(&format!("{}", n as i64));
            } else {
                out.push_str(&n.to_string());
            }
        }
        Value::Bool(true) => out.push_str("TRUE"),
        Value::Bool(false) => out.push_str("FALSE"),
        Value::Error(e) => out.push_str(e.as_str()),
        Value::Text(id) => {
            if let Some(t) = intern.strings.get(id) {
                push_field(out, t);
            }
        }
        Value::Array(_) => {}
    }
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

fn needs_quote(s: &str) -> bool {
    s.is_empty()
        || s.contains(['\t', '\n', '\r', '"'])
        || s.starts_with(' ')
        || s.ends_with(' ')
        || s.starts_with('#')
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
    ) || n.starts_with("xl/worksheets/")
        || n.starts_with("xl/tables/")
        || n.starts_with("xl/comments")
        || n.starts_with("xl/omacell/")
}
