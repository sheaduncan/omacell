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
    num_fmt: Option<String>,
}

fn parse_styles(bytes: &[u8]) -> Result<BTreeMap<String, CellStyle>, CoreError> {
    if bytes.is_empty() {
        return Ok(BTreeMap::new());
    }
    let number_formats = parse_number_formats(bytes)?;
    let mut reader = XmlReader::new(bytes);
    let mut out = BTreeMap::new();
    let mut name = String::new();
    let mut current = Style::default();
    let mut current_num_fmt = None;
    let mut in_cell = false;
    while let Some(ev) = reader.next()? {
        match ev {
            XmlEvent::Start { name: tag, attrs } if tag == "style" => {
                let family = attr(&attrs, "family").unwrap_or("");
                name = attr(&attrs, "name").unwrap_or("").to_string();
                in_cell = family == "table-cell" || family.is_empty();
                current = Style::default();
                current_num_fmt = attr(&attrs, "data-style-name")
                    .and_then(|style_name| number_formats.get(style_name))
                    .cloned();
            }
            XmlEvent::Empty { name: tag, attrs } if tag == "style" => {
                let family = attr(&attrs, "family").unwrap_or("");
                name = attr(&attrs, "name").unwrap_or("").to_string();
                in_cell = family == "table-cell" || family.is_empty();
                current = Style::default();
                current_num_fmt = attr(&attrs, "data-style-name")
                    .and_then(|style_name| number_formats.get(style_name))
                    .cloned();
                if in_cell && !name.is_empty() {
                    out.insert(
                        name.clone(),
                        CellStyle {
                            style: current.clone(),
                            num_fmt: current_num_fmt.clone(),
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
                            num_fmt: current_num_fmt.clone(),
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

#[derive(Clone, Copy)]
enum NumberStyleKind {
    Number,
    Percentage,
    Currency,
    Date,
    Time,
}

struct NumberStyle {
    name: String,
    kind: NumberStyleKind,
    code: String,
    text: Option<(String, bool)>,
}

fn parse_number_formats(bytes: &[u8]) -> Result<BTreeMap<String, String>, CoreError> {
    let mut reader = XmlReader::new(bytes);
    let mut out = BTreeMap::new();
    let mut current: Option<NumberStyle> = None;
    while let Some(event) = reader.next()? {
        match event {
            XmlEvent::Start { name, attrs } => {
                if let Some(kind) = number_style_kind(&name) {
                    current = Some(NumberStyle {
                        name: attr(&attrs, "name").unwrap_or("").to_string(),
                        kind,
                        code: String::new(),
                        text: None,
                    });
                } else if let Some(style) = current.as_mut() {
                    if name == "text" || name == "currency-symbol" {
                        style.text = Some((String::new(), name == "currency-symbol"));
                    } else {
                        append_number_component(style, &name, &attrs)?;
                    }
                }
            }
            XmlEvent::Empty { name, attrs } => {
                if let Some(kind) = number_style_kind(&name) {
                    let style_name = attr(&attrs, "name").unwrap_or("");
                    if !style_name.is_empty() {
                        out.insert(
                            style_name.to_string(),
                            default_number_code(kind).to_string(),
                        );
                    }
                } else if let Some(style) = current.as_mut() {
                    append_number_component(style, &name, &attrs)?;
                }
            }
            XmlEvent::Text(text) => {
                if let Some((collected, _)) = current.as_mut().and_then(|style| style.text.as_mut())
                {
                    collected.push_str(&text);
                }
            }
            XmlEvent::End { name } => {
                if name == "text" || name == "currency-symbol" {
                    if let Some(style) = current.as_mut()
                        && let Some((text, currency)) = style.text.take()
                    {
                        if currency {
                            style.code.insert_str(0, &excel_literal(&text));
                        } else {
                            style.code.push_str(&excel_literal(&text));
                        }
                    }
                } else if number_style_kind(&name).is_some()
                    && let Some(mut style) = current.take()
                    && !style.name.is_empty()
                {
                    finish_number_code(&mut style);
                    out.insert(style.name, style.code);
                }
            }
        }
    }
    Ok(out)
}

fn number_style_kind(name: &str) -> Option<NumberStyleKind> {
    match name {
        "number-style" => Some(NumberStyleKind::Number),
        "percentage-style" => Some(NumberStyleKind::Percentage),
        "currency-style" => Some(NumberStyleKind::Currency),
        "date-style" => Some(NumberStyleKind::Date),
        "time-style" => Some(NumberStyleKind::Time),
        _ => None,
    }
}

fn append_number_component(
    style: &mut NumberStyle,
    name: &str,
    attrs: &[(String, String)],
) -> Result<(), CoreError> {
    match name {
        "number" => style.code.push_str(&decimal_pattern(attrs)?),
        "scientific-number" => {
            style.code.push_str(&decimal_pattern(attrs)?);
            style.code.push_str("E+00");
        }
        "fraction" => style.code.push_str("# ?/?"),
        "day" => style
            .code
            .push_str(if attr(attrs, "style") == Some("long") {
                "dd"
            } else {
                "d"
            }),
        "month" => {
            let long = attr(attrs, "style") == Some("long");
            let textual = attr(attrs, "textual") == Some("true");
            style.code.push_str(match (textual, long) {
                (true, true) => "mmmm",
                (true, false) => "mmm",
                (false, true) => "mm",
                (false, false) => "m",
            });
        }
        "year" => style
            .code
            .push_str(if attr(attrs, "style") == Some("long") {
                "yyyy"
            } else {
                "yy"
            }),
        "hours" => style
            .code
            .push_str(if attr(attrs, "style") == Some("long") {
                "hh"
            } else {
                "h"
            }),
        "minutes" => style
            .code
            .push_str(if attr(attrs, "style") == Some("long") {
                "mm"
            } else {
                "m"
            }),
        "seconds" => {
            style
                .code
                .push_str(if attr(attrs, "style") == Some("long") {
                    "ss"
                } else {
                    "s"
                });
            let decimals = bounded_count(attrs, "decimal-places", 15, 0)?;
            if decimals > 0 {
                style.code.push('.');
                style.code.extend(std::iter::repeat_n('0', decimals));
            }
        }
        "am-pm" => style.code.push_str(" AM/PM"),
        _ => {}
    }
    Ok(())
}

fn decimal_pattern(attrs: &[(String, String)]) -> Result<String, CoreError> {
    let integers = bounded_count(attrs, "min-integer-digits", 30, 1)?.max(1);
    let decimals = bounded_count(attrs, "decimal-places", 30, 0)?;
    let grouping = attr(attrs, "grouping") == Some("true");
    let mut code = if grouping {
        format!("#,##{}", "0".repeat(integers))
    } else {
        "0".repeat(integers)
    };
    if decimals > 0 {
        code.push('.');
        code.extend(std::iter::repeat_n('0', decimals));
    }
    Ok(code)
}

fn bounded_count(
    attrs: &[(String, String)],
    name: &str,
    maximum: usize,
    default: usize,
) -> Result<usize, CoreError> {
    let Some(raw) = attr(attrs, name) else {
        return Ok(default);
    };
    raw.parse::<usize>()
        .ok()
        .filter(|value| *value <= maximum)
        .ok_or_else(|| error::ods_format(format!("invalid number-format {name} value {raw:?}")))
}

fn finish_number_code(style: &mut NumberStyle) {
    if style.code.is_empty() {
        style.code.push_str(default_number_code(style.kind));
    }
    match style.kind {
        NumberStyleKind::Percentage if !style.code.contains('%') => style.code.push('%'),
        NumberStyleKind::Currency
            if !style
                .code
                .chars()
                .any(|ch| matches!(ch, '$' | '\u{00a3}' | '\u{20ac}' | '\u{00a5}')) =>
        {
            style.code.insert(0, '$');
        }
        _ => {}
    }
}

fn default_number_code(kind: NumberStyleKind) -> &'static str {
    match kind {
        NumberStyleKind::Number => "0.00",
        NumberStyleKind::Percentage => "0.00%",
        NumberStyleKind::Currency => "$#,##0.00",
        NumberStyleKind::Date => "m/d/yyyy",
        NumberStyleKind::Time => "h:mm:ss",
    }
}

fn excel_literal(text: &str) -> String {
    if text
        .chars()
        .all(|ch| matches!(ch, '/' | '-' | ':' | ' ' | ',' | '.' | '%' | '$'))
    {
        text.to_string()
    } else {
        format!("\"{}\"", text.replace('"', "\"\""))
    }
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
    if !matches!(hex.len(), 6 | 8) {
        return None;
    }
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
                        wb.rename_sheet(id, &table_name)?;
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
    if sheet_index == 0 {
        return Err(error::ods_format(
            "content.xml contains no spreadsheet table",
        ));
    }
    for (name, addr) in names {
        if let Some(range) = parse_ods_range(wb, &addr) {
            wb.define_name(DefinedName {
                name,
                scope: NameScope::Workbook,
                referent: NameReferent::Range(range),
                comment: None,
            })?;
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
                let repeat = repeated(&attrs, "number-rows-repeated", MAX_ROWS)?;
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
                    .checked_add(repeated(&attrs, "number-rows-repeated", MAX_ROWS)?)
                    .filter(|r| *r <= MAX_ROWS)
                    .ok_or_else(|| error::ods_format("row repeat exceeds the grid"))?;
            }
            XmlEvent::End { name } if name == "table" => return Ok(()),
            _ => {}
        }
    }
    Err(error::ods_format(
        "unexpected end of content.xml inside table",
    ))
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
                    .checked_add(repeated(
                        &attrs,
                        "number-columns-repeated",
                        u32::from(MAX_COLS),
                    )?)
                    .filter(|c| *c <= u32::from(MAX_COLS))
                    .ok_or_else(|| error::ods_format("covered cells exceed the grid"))?;
                skip_until(reader, "covered-table-cell")?;
            }
            XmlEvent::Empty { name, attrs } if name == "covered-table-cell" => {
                col = col
                    .checked_add(repeated(
                        &attrs,
                        "number-columns-repeated",
                        u32::from(MAX_COLS),
                    )?)
                    .filter(|c| *c <= u32::from(MAX_COLS))
                    .ok_or_else(|| error::ods_format("covered cells exceed the grid"))?;
            }
            XmlEvent::End { name } if name == "table-row" => return Ok(()),
            _ => {}
        }
    }
    Err(error::ods_format(
        "unexpected end of content.xml inside table row",
    ))
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
    let repeat = repeated(&attrs, "number-columns-repeated", u32::from(MAX_COLS))?;
    let span_cols = positive_count(&attrs, "number-columns-spanned", u32::from(MAX_COLS))?;
    let span_rows = positive_count(&attrs, "number-rows-spanned", MAX_ROWS)?;
    let advance = repeat
        .checked_mul(span_cols)
        .and_then(|count| col.checked_add(count))
        .filter(|end| *end <= u32::from(MAX_COLS))
        .ok_or_else(|| error::ods_format("repeated cell exceeds the column grid"))?;
    let end_row = row
        .checked_add(span_rows - 1)
        .filter(|end| *end < MAX_ROWS)
        .ok_or_else(|| error::ods_format("cell span exceeds the row grid"))?;
    for index in 0..repeat {
        let start_col = col + index * span_cols;
        let end_col = start_col + span_cols - 1;
        write_cell(wb, sheet, row, start_col as u16, &attrs, text, styles)?;
        if span_cols > 1 || span_rows > 1 {
            let start = CellRef::new(row, start_col as u16)?;
            let end = CellRef::new(end_row, end_col as u16)?;
            omacell_core::ops::merge(wb, sheet, RangeRef::from_corners(start, end))?;
        }
    }
    Ok(advance)
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
                    return Ok(text.trim_end_matches('\n').to_string());
                }
                if name == "p" {
                    text.push('\n');
                }
            }
            _ => {}
        }
    }
    Err(error::ods_format(
        "unexpected end of content.xml inside table cell",
    ))
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
    Err(error::ods_format(format!(
        "unexpected end of content.xml inside {end}"
    )))
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
    let value_type = attr(attrs, "value-type").unwrap_or("string");
    if let Some(formula) = attr(attrs, "formula") {
        let src = ods_formula_to_excel(formula);
        wb.set_formula_text(sheet, row, col, &src)?;
    } else {
        match value_type {
            "float" | "percentage" | "currency" => {
                let raw = attr(attrs, "value")
                    .filter(|value| !value.is_empty())
                    .or_else(|| (!text.is_empty()).then_some(text))
                    .ok_or_else(|| error::ods_format("numeric cell is missing office:value"))?;
                let n = raw
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| error::ods_format(format!("invalid numeric value {raw:?}")))?;
                wb.set_number(sheet, row, col, n)?;
            }
            "boolean" => {
                let v = attr(attrs, "boolean-value").unwrap_or(text);
                let b = match v {
                    "true" | "1" => true,
                    "false" | "0" => false,
                    _ => {
                        return Err(error::ods_format(format!("invalid boolean value {v:?}")));
                    }
                };
                wb.set_cell_contents(sheet, row, col, if b { "TRUE" } else { "FALSE" })?;
            }
            "date" => {
                let raw = attr(attrs, "date-value")
                    .ok_or_else(|| error::ods_format("date cell is missing office:date-value"))?;
                let serial = ods_date_serial(raw)
                    .ok_or_else(|| error::ods_format(format!("invalid ODS date {raw:?}")))?;
                wb.set_number(sheet, row, col, serial as f64)?;
            }
            _ => {
                if !text.is_empty() {
                    wb.set_text(sheet, row, col, text)?;
                }
            }
        }
    }
    let cell_style = attr(attrs, "style-name").and_then(|style_name| styles.get(style_name));
    let fallback_num_fmt = match value_type {
        "percentage" => Some("0.00%"),
        "currency" => Some("$#,##0.00"),
        "date" => Some("m/d/yyyy"),
        _ => None,
    };
    if cell_style.is_some() || fallback_num_fmt.is_some() {
        let mut style = cell_style.map(|cs| cs.style.clone()).unwrap_or_default();
        if let Some(code) = cell_style
            .and_then(|cs| cs.num_fmt.as_deref())
            .or(fallback_num_fmt)
        {
            style.num_fmt = wb.intern_num_fmt(code)?;
        }
        wb.set_cell_style(sheet, row, col, style)?;
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
                let Some(n) = chars.next() else {
                    break;
                };
                if n != '.' {
                    out.push(n);
                }
            }
            let _ = chars.next();
        } else if ch == ';' {
            out.push(',');
        } else {
            out.push(ch);
        }
    }
    out
}

fn repeated(attrs: &[(String, String)], name: &str, maximum: u32) -> Result<u32, CoreError> {
    positive_count(attrs, name, maximum.min(MAX_REPEATED))
}

fn positive_count(attrs: &[(String, String)], name: &str, maximum: u32) -> Result<u32, CoreError> {
    let Some(raw) = attr(attrs, name) else {
        return Ok(1);
    };
    raw.parse::<u32>()
        .ok()
        .filter(|value| (1..=maximum).contains(value))
        .ok_or_else(|| error::ods_format(format!("invalid {name} value {raw:?}")))
}

fn copy_row(wb: &mut Workbook, sheet: SheetId, src: u32, repeat: u32) -> Result<(), CoreError> {
    let Some(used) = wb.used_range(sheet)? else {
        return Ok(());
    };
    let cells = (used.min_col..=used.max_col)
        .filter_map(|col| {
            wb.get(sheet, src, col)
                .ok()
                .flatten()
                .copied()
                .map(|slot| (col, slot))
        })
        .collect::<Vec<_>>();
    for i in 1..repeat {
        let dest = src + i;
        for (col, slot) in &cells {
            wb.set_slot(sheet, dest, *col, *slot)?;
        }
    }
    Ok(())
}

fn parse_ods_range(wb: &Workbook, addr: &str) -> Option<RangeRef> {
    let addr = addr.trim().trim_start_matches('$');
    let (sheet_name, body) = split_ods_sheet(addr).unwrap_or_else(|| (String::new(), addr));
    let sheet = if sheet_name.is_empty() {
        wb.active_sheet()
    } else {
        wb.sheet_by_name(&sheet_name)?.id
    };
    let body = body.replace(['$', '.'], "");
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

fn split_ods_sheet(addr: &str) -> Option<(String, &str)> {
    if let Some(rest) = addr.strip_prefix('\'') {
        let bytes = rest.as_bytes();
        let mut index = 0usize;
        while index < bytes.len() {
            if bytes[index] == b'\'' {
                if bytes.get(index + 1) == Some(&b'\'') {
                    index += 2;
                    continue;
                }
                let tail = rest.get(index + 1..)?.strip_prefix('.')?;
                let quoted = rest.get(..index)?.replace("''", "'");
                return Some((quoted, tail));
            }
            index += 1;
        }
        None
    } else {
        addr.split_once('.')
            .map(|(sheet, tail)| (sheet.to_string(), tail))
    }
}

fn parse_a1_cell(text: &str) -> Option<CellRef> {
    omacell_core::addr::parse_a1_cell(text).ok()
}
