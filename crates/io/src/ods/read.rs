//! ODS content.xml / styles.xml reader.

use std::collections::BTreeMap;

use omacell_core::addr::{CellRef, RangeRef, SheetId};
use omacell_core::date_system::DateSystem;
use omacell_core::dates::{CivilDate, date_to_serial};
use omacell_core::error::CoreError;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::style::{Color, Fill, Font, Style};
use omacell_core::workbook::Workbook;

use super::zip;
use crate::error;
use crate::xlsx::xml::{XmlEvent, XmlReader, attr};

const MAX_REPEATED: u32 = 1_048_576;

/// Open ODS bytes.
pub fn open_bytes(bytes: &[u8]) -> Result<Workbook, CoreError> {
    let parts = zip::read_parts(bytes)?;
    let content = parts
        .get("content.xml")
        .ok_or_else(|| error::ods_format("missing content.xml"))?;
    let styles = parts.get("styles.xml").map(Vec::as_slice).unwrap_or(b"");
    let mut style_map = parse_styles(styles)?;
    style_map.extend(parse_styles(content)?);
    parse_content(content, &style_map)
}

struct CellStyle {
    style: Style,
}

fn parse_styles(bytes: &[u8]) -> Result<BTreeMap<String, CellStyle>, CoreError> {
    if bytes.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut reader = XmlReader::new(bytes);
    let mut out = BTreeMap::new();
    let mut name = String::new();
    let mut current = Style::default();
    let mut in_cell = false;
    while let Some(ev) = reader.next()? {
        match ev {
            XmlEvent::Start { name: tag, attrs } if tag == "style" => {
                let family = attr(&attrs, "family").unwrap_or("");
                name = attr(&attrs, "name").unwrap_or("").to_string();
                in_cell = family == "table-cell" || family.is_empty();
                current = Style::default();
            }
            XmlEvent::Empty { name: tag, attrs } if tag == "style" => {
                let family = attr(&attrs, "family").unwrap_or("");
                name = attr(&attrs, "name").unwrap_or("").to_string();
                in_cell = family == "table-cell" || family.is_empty();
                current = Style::default();
                if in_cell && !name.is_empty() {
                    out.insert(
                        name.clone(),
                        CellStyle {
                            style: current.clone(),
                        },
                    );
                }
            }
            XmlEvent::Empty { name: tag, attrs } | XmlEvent::Start { name: tag, attrs }
                if in_cell && tag == "text-properties" =>
            {
                apply_text_props(&mut current.font, &attrs);
            }
            XmlEvent::Empty { name: tag, attrs } | XmlEvent::Start { name: tag, attrs }
                if in_cell && tag == "table-cell-properties" =>
            {
                if let Some(bg) = attr(&attrs, "background-color").and_then(parse_color) {
                    current.fill = Fill::Solid { fg: bg };
                }
            }
            XmlEvent::End { name: tag } if tag == "style" => {
                if in_cell && !name.is_empty() {
                    out.insert(
                        name.clone(),
                        CellStyle {
                            style: current.clone(),
                        },
                    );
                }
                in_cell = false;
            }
            _ => {}
        }
    }
    let _ = bytes;
    Ok(out)
}

fn apply_text_props(font: &mut Font, attrs: &[(String, String)]) {
    if attr(attrs, "font-weight")
        .is_some_and(|w| w == "bold" || w.parse::<u32>().unwrap_or(0) >= 700)
    {
        font.bold = true;
    }
    if attr(attrs, "font-style") == Some("italic") {
        font.italic = true;
    }
    if let Some(color) = attr(attrs, "color").and_then(parse_color) {
        font.color = color;
    }
    if let Some(name) = attr(attrs, "font-name") {
        font.name = name.to_string();
    }
}

fn parse_color(spec: &str) -> Option<Color> {
    let hex = spec.strip_prefix('#')?;
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    let argb = if hex.len() == 6 {
        0xFF00_0000 | rgb
    } else {
        rgb
    };
    Some(Color::Rgb { argb })
}

fn parse_content(
    bytes: &[u8],
    styles: &BTreeMap<String, CellStyle>,
) -> Result<Workbook, CoreError> {
    let mut wb = Workbook::new();
    let undo = wb.undo_log().is_enabled();
    wb.undo_log_mut().set_enabled(false);
    let result = parse_content_inner(&mut wb, bytes, styles);
    wb.undo_log_mut().set_enabled(undo);
    result?;
    Ok(wb)
}

fn parse_content_inner(
    wb: &mut Workbook,
    bytes: &[u8],
    styles: &BTreeMap<String, CellStyle>,
) -> Result<(), CoreError> {
    let mut reader = XmlReader::new(bytes);
    let mut sheet_index = 0u32;
    let mut names: Vec<(String, String)> = Vec::new();
    while let Some(ev) = reader.next()? {
        match ev {
            XmlEvent::Start { name, attrs } if name == "table" => {
                let table_name = attr(&attrs, "name").unwrap_or("Sheet1").to_string();
                let sheet = if sheet_index == 0 {
                    let id = wb.active_sheet();
                    if table_name != "Sheet1" {
                        let _ = wb.rename_sheet(id, &table_name);
                    }
                    id
                } else {
                    wb.add_sheet(&table_name)?
                };
                parse_table(wb, &mut reader, sheet, styles)?;
                sheet_index += 1;
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if name == "named-range" =>
            {
                if let (Some(n), Some(addr)) =
                    (attr(&attrs, "name"), attr(&attrs, "cell-range-address"))
                {
                    names.push((n.to_string(), addr.to_string()));
                }
            }
            _ => {}
        }
    }
    for (name, addr) in names {
        if let Some(range) = parse_ods_range(wb, &addr) {
            let _ = wb.define_name(DefinedName {
                name,
                scope: NameScope::Workbook,
                referent: NameReferent::Range(range),
                comment: None,
            });
        }
    }
    Ok(())
}

fn parse_table(
    wb: &mut Workbook,
    reader: &mut XmlReader<'_>,
    sheet: SheetId,
    styles: &BTreeMap<String, CellStyle>,
) -> Result<(), CoreError> {
    let mut row: u32 = 0;
    while let Some(ev) = reader.next()? {
        match ev {
            XmlEvent::Start { name, attrs } if name == "table-row" => {
                let repeat = repeat_count(&attrs);
                let start_row = row;
                parse_row(wb, reader, sheet, start_row, styles)?;
                row = row
                    .checked_add(repeat)
                    .filter(|r| *r <= MAX_ROWS)
                    .ok_or_else(|| error::ods_format("row repeat exceeds the grid"))?;
                if repeat > 1 {
                    copy_row(wb, sheet, start_row, repeat)?;
                }
            }
            XmlEvent::Empty { name, attrs } if name == "table-row" => {
                row = row
                    .checked_add(repeat_count(&attrs))
                    .filter(|r| *r <= MAX_ROWS)
                    .ok_or_else(|| error::ods_format("row repeat exceeds the grid"))?;
            }
            XmlEvent::End { name } if name == "table" => return Ok(()),
            _ => {}
        }
    }
    Ok(())
}

fn parse_row(
    wb: &mut Workbook,
    reader: &mut XmlReader<'_>,
    sheet: SheetId,
    row: u32,
    styles: &BTreeMap<String, CellStyle>,
) -> Result<(), CoreError> {
    let mut col: u32 = 0;
    while let Some(ev) = reader.next()? {
        match ev {
            XmlEvent::Start { name, attrs } if name == "table-cell" => {
                let text = collect_cell_text(reader)?;
                col = emit_cell(wb, sheet, row, col, attrs, &text, styles)?;
            }
            XmlEvent::Empty { name, attrs } if name == "table-cell" => {
                col = emit_cell(wb, sheet, row, col, attrs, "", styles)?;
            }
            XmlEvent::Start { name, attrs } if name == "covered-table-cell" => {
                col = col
                    .checked_add(repeat_count(&attrs))
                    .filter(|c| *c <= u32::from(MAX_COLS))
                    .ok_or_else(|| error::ods_format("covered cells exceed the grid"))?;
                skip_until(reader, "covered-table-cell")?;
            }
            XmlEvent::Empty { name, attrs } if name == "covered-table-cell" => {
                col = col
                    .checked_add(repeat_count(&attrs))
                    .filter(|c| *c <= u32::from(MAX_COLS))
                    .ok_or_else(|| error::ods_format("covered cells exceed the grid"))?;
            }
            XmlEvent::End { name } if name == "table-row" => return Ok(()),
            _ => {}
        }
    }
    Ok(())
}

fn emit_cell(
    wb: &mut Workbook,
    sheet: SheetId,
    row: u32,
    col: u32,
    attrs: Vec<(String, String)>,
    text: &str,
    styles: &BTreeMap<String, CellStyle>,
) -> Result<u32, CoreError> {
    let repeat = repeat_count(&attrs);
    let span_cols = attr(&attrs, "number-columns-spanned")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);
    let span_rows = attr(&attrs, "number-rows-spanned")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);
    write_cell(wb, sheet, row, col as u16, &attrs, text, styles)?;
    if span_cols > 1 || span_rows > 1 {
        let end_row = row.saturating_add(span_rows - 1);
        let end_col = (col as u16).saturating_add((span_cols as u16).saturating_sub(1));
        if let (Ok(start), Ok(end)) = (
            CellRef::new(row, col as u16),
            CellRef::new(end_row, end_col),
        ) {
            let _ = omacell_core::ops::merge(wb, sheet, RangeRef::from_corners(start, end));
        }
    }
    col.checked_add(repeat.max(span_cols))
        .filter(|c| *c <= u32::from(MAX_COLS))
        .ok_or_else(|| error::ods_format("column repeat exceeds the grid"))
}

fn collect_cell_text(reader: &mut XmlReader<'_>) -> Result<String, CoreError> {
    let mut text = String::new();
    let mut depth = 1u32;
    while let Some(ev) = reader.next()? {
        match ev {
            XmlEvent::Start { .. } => depth += 1,
            XmlEvent::Text(t) => text.push_str(&t),
            XmlEvent::End { name } => {
                depth = depth.saturating_sub(1);
                if depth == 0 && name == "table-cell" {
                    break;
                }
                if name == "p" {
                    text.push('\n');
                }
            }
            _ => {}
        }
    }
    Ok(text.trim_end_matches('\n').to_string())
}

fn skip_until(reader: &mut XmlReader<'_>, end: &str) -> Result<(), CoreError> {
    let mut depth = 1u32;
    while let Some(ev) = reader.next()? {
        match ev {
            XmlEvent::Start { .. } => depth += 1,
            XmlEvent::End { name } => {
                depth = depth.saturating_sub(1);
                if depth == 0 && name == end {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn write_cell(
    wb: &mut Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    attrs: &[(String, String)],
    text: &str,
    styles: &BTreeMap<String, CellStyle>,
) -> Result<(), CoreError> {
    if let Some(formula) = attr(attrs, "formula") {
        let src = ods_formula_to_excel(formula);
        wb.set_formula_text(sheet, row, col, &src)?;
    } else {
        match attr(attrs, "value-type").unwrap_or("string") {
            "float" | "percentage" | "currency" => {
                let n = attr(attrs, "value")
                    .or(attr(attrs, "currency"))
                    .and_then(|s| s.parse::<f64>().ok())
                    .or_else(|| text.parse().ok())
                    .unwrap_or(0.0);
                wb.set_number(sheet, row, col, n)?;
            }
            "boolean" => {
                let v = attr(attrs, "boolean-value").unwrap_or(text);
                let b = v.eq_ignore_ascii_case("true");
                wb.set_cell_contents(sheet, row, col, if b { "TRUE" } else { "FALSE" })?;
            }
            "date" => {
                if let Some(serial) = attr(attrs, "date-value").and_then(ods_date_serial) {
                    wb.set_number(sheet, row, col, serial as f64)?;
                } else if !text.is_empty() {
                    wb.set_text(sheet, row, col, text)?;
                }
            }
            _ => {
                if !text.is_empty() {
                    wb.set_text(sheet, row, col, text)?;
                }
            }
        }
    }
    if let Some(style_name) = attr(attrs, "style-name")
        && let Some(cs) = styles.get(style_name)
    {
        wb.set_cell_style(sheet, row, col, cs.style.clone())?;
    }
    Ok(())
}

fn ods_date_serial(iso: &str) -> Option<i64> {
    let date = iso.split('T').next()?;
    let mut parts = date.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    date_to_serial(
        CivilDate {
            year,
            month,
            day,
            lotus_leap: false,
        },
        DateSystem::Excel1900,
    )
}

fn ods_formula_to_excel(src: &str) -> String {
    let rest = src.strip_prefix("of:").unwrap_or(src);
    let rest = rest.trim();
    let mut out = String::new();
    if !rest.starts_with('=') {
        out.push('=');
    }
    let mut chars = rest.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '[' {
            while chars.peek().is_some_and(|c| *c != ']') {
                let n = chars.next().unwrap();
                if n != '.' {
                    out.push(n);
                }
            }
            let _ = chars.next();
        } else {
            out.push(ch);
        }
    }
    out
}

fn repeat_count(attrs: &[(String, String)]) -> u32 {
    attr(attrs, "number-columns-repeated")
        .or_else(|| attr(attrs, "number-rows-repeated"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
        .clamp(1, MAX_REPEATED)
}

fn copy_row(wb: &mut Workbook, sheet: SheetId, src: u32, repeat: u32) -> Result<(), CoreError> {
    let Some(used) = wb.used_range(sheet)? else {
        return Ok(());
    };
    for i in 1..repeat {
        let dest = src + i;
        for col in used.min_col..=used.max_col {
            if let Some(slot) = wb.get(sheet, src, col)? {
                let input = if let Some(fid) = slot.formula {
                    wb.intern().formulas.get(fid).unwrap_or("").to_string()
                } else {
                    match slot.value {
                        omacell_core::value::Value::Number(n) => n.to_string(),
                        omacell_core::value::Value::Bool(true) => "TRUE".into(),
                        omacell_core::value::Value::Bool(false) => "FALSE".into(),
                        omacell_core::value::Value::Text(id) => {
                            wb.intern().strings.get(id).unwrap_or("").to_string()
                        }
                        _ => continue,
                    }
                };
                if !input.is_empty() {
                    wb.set_cell_contents(sheet, dest, col, &input)?;
                }
            }
        }
    }
    Ok(())
}

fn parse_ods_range(wb: &Workbook, addr: &str) -> Option<RangeRef> {
    let addr = addr.trim().trim_start_matches('$');
    let (sheet_name, body) = addr.split_once('.').unwrap_or(("", addr));
    let sheet_name = sheet_name.trim_start_matches('$');
    let sheet = if sheet_name.is_empty() {
        wb.active_sheet()
    } else {
        wb.sheet_by_name(sheet_name)?.id
    };
    let body = body.replace('$', "");
    let (start, end) = match body.split_once(':') {
        Some((a, b)) => (parse_a1_cell(a)?, parse_a1_cell(b)?),
        None => {
            let c = parse_a1_cell(&body)?;
            (c, c)
        }
    };
    let start = start.on_sheet(sheet);
    let end = end.on_sheet(sheet);
    Some(RangeRef::from_corners(start, end))
}

fn parse_a1_cell(text: &str) -> Option<CellRef> {
    omacell_core::addr::parse_a1_cell(text).ok()
}
