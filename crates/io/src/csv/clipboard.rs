//! Clipboard helpers for WP-14 (TSV / CSV / Markdown / HTML tables).

use omacell_core::error::CoreError;
use omacell_core::limits::MAX_COLS;
use omacell_core::locale::LocaleId;
use serde::{Deserialize, Serialize};

use super::plan::{
    ColumnPlan, ColumnType, ImportPlan, LineEnding, MAX_CLIPBOARD_BYTES, MAX_CLIPBOARD_CELLS,
    MAX_CLIPBOARD_ROWS, MAX_FIELD_BYTES, TextEncoding,
};
use super::records::parse_records;
use crate::error;

/// Declared clipboard flavour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardFormat {
    /// Sniff CSV vs TSV vs Markdown vs HTML.
    #[default]
    Auto,
    /// RFC 4180 CSV.
    Csv,
    /// Tab-separated.
    Tsv,
    /// GitHub-style pipe table.
    Markdown,
    /// First HTML `<table>`.
    Html,
}

/// Parsed clipboard table plus a plan the importer can reuse.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipboardTable {
    /// Detected / constructed plan.
    pub plan: ImportPlan,
    /// Header row when the source had one (markdown / html `<th>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<Vec<String>>,
    /// Body rows (raw strings).
    pub rows: Vec<Vec<String>>,
}

/// Parse pasted text into an [`ImportPlan`] and raw rows.
pub fn parse_clipboard(text: &str, kind: ClipboardFormat) -> Result<ClipboardTable, CoreError> {
    if text.len() > MAX_CLIPBOARD_BYTES {
        return Err(error::limit(format!(
            "clipboard payload is {} bytes; maximum is {MAX_CLIPBOARD_BYTES}",
            text.len()
        )));
    }
    let kind = if kind == ClipboardFormat::Auto {
        detect(text)
    } else {
        kind
    };
    match kind {
        ClipboardFormat::Html => parse_html(text),
        ClipboardFormat::Markdown => parse_markdown(text),
        ClipboardFormat::Tsv => parse_delimited(text, '\t'),
        ClipboardFormat::Csv => parse_delimited(text, ','),
        ClipboardFormat::Auto => parse_delimited(text, ','),
    }
}

fn detect(text: &str) -> ClipboardFormat {
    let t = text.trim_start();
    let lower = t.to_ascii_lowercase();
    if lower.contains("<table") {
        return ClipboardFormat::Html;
    }
    if looks_like_markdown(text) {
        return ClipboardFormat::Markdown;
    }
    let first = text.lines().next().unwrap_or("");
    if first.contains('\t') {
        ClipboardFormat::Tsv
    } else {
        ClipboardFormat::Csv
    }
}

fn looks_like_markdown(text: &str) -> bool {
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(4)
        .collect();
    if lines.len() < 2 {
        return false;
    }
    lines.iter().all(|l| l.contains('|'))
        && lines.get(1).is_some_and(|l| {
            l.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t')) && l.contains('-')
        })
}

fn parse_delimited(text: &str, delimiter: char) -> Result<ClipboardTable, CoreError> {
    let plan = ImportPlan {
        delimiter,
        quote: '"',
        encoding: TextEncoding::Utf8,
        bom: false,
        has_header: false,
        skip_rows: 0,
        locale: LocaleId::EN_US,
        decimal: '.',
        thousands: Some(','),
        line_ending: LineEnding::Lf,
        date_system: omacell_core::date_system::DateSystem::Excel1900,
        columns: Vec::new(),
    };
    let rows = parse_records(text.as_bytes(), &plan)?;
    Ok(ClipboardTable {
        plan,
        header: None,
        rows,
    })
}

fn parse_markdown(text: &str) -> Result<ClipboardTable, CoreError> {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let Some(header_line) = lines.next() else {
        return Err(error::parse("markdown table needs a header and a body"));
    };
    let header = split_md_row(header_line)?;
    let mut found_separator = false;
    for line in lines.by_ref() {
        if is_md_sep(line) {
            found_separator = true;
            break;
        }
    }
    if !found_separator {
        return Err(error::parse("markdown table is missing a separator row"));
    }
    let mut rows = Vec::new();
    let mut cells = header.len();
    for line in lines {
        if line.contains('|') {
            let mut row = split_md_row(line)?;
            let materialized_width = header.len().max(row.len());
            add_table_shape(&mut cells, rows.len(), materialized_width)?;
            row.resize(header.len(), String::new());
            rows.push(row);
        }
    }
    let mut plan = ImportPlan {
        delimiter: '|',
        has_header: true,
        ..ImportPlan::default()
    };
    plan.columns = header
        .iter()
        .map(|name| ColumnPlan {
            name: Some(name.clone()),
            ty: ColumnType::Auto,
        })
        .collect();
    Ok(ClipboardTable {
        plan,
        header: Some(header),
        rows,
    })
}

fn is_md_sep(line: &str) -> bool {
    let t = line.trim().trim_matches('|').trim();
    !t.is_empty() && t.chars().all(|c| matches!(c, '-' | ':' | '|' | ' ' | '\t')) && t.contains('-')
}

fn split_md_row(line: &str) -> Result<Vec<String>, CoreError> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if t.ends_with('|') && !is_escaped(t, t.len() - 1) {
        &t[..t.len() - 1]
    } else {
        t
    };
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = t.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '|' => push_md_cell(&mut cells, &mut cell)?,
            '\\' => match chars.next() {
                Some(escaped @ ('|' | '\\')) => cell.push(escaped),
                Some(other) => {
                    cell.push('\\');
                    cell.push(other);
                }
                None => cell.push('\\'),
            },
            other => cell.push(other),
        }
        if cell.len() > MAX_FIELD_BYTES {
            return Err(error::limit(format!(
                "clipboard field is more than {MAX_FIELD_BYTES} bytes"
            )));
        }
    }
    push_md_cell(&mut cells, &mut cell)?;
    Ok(cells)
}

fn is_escaped(text: &str, byte_index: usize) -> bool {
    text.as_bytes()[..byte_index]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

fn push_md_cell(cells: &mut Vec<String>, cell: &mut String) -> Result<(), CoreError> {
    if cells.len() >= usize::from(MAX_COLS) {
        return Err(error::limit(format!(
            "clipboard row has more than {MAX_COLS} columns"
        )));
    }
    let trimmed = cell.trim();
    if trimmed.len() > MAX_FIELD_BYTES {
        return Err(error::limit(format!(
            "clipboard field is {} bytes; maximum is {MAX_FIELD_BYTES}",
            trimmed.len()
        )));
    }
    cells.push(trimmed.to_string());
    cell.clear();
    Ok(())
}

fn parse_html(text: &str) -> Result<ClipboardTable, CoreError> {
    let lower = text.to_ascii_lowercase();
    let start = lower
        .find("<table")
        .ok_or_else(|| error::parse("no HTML table"))?;
    let rest = &text[start..];
    let end_rel = rest
        .to_ascii_lowercase()
        .find("</table>")
        .map(|i| i + 8)
        .unwrap_or(rest.len());
    let table = &rest[..end_rel];
    let mut rows = Vec::new();
    let mut header = None;
    let mut cells = 0usize;
    let mut pos = 0;
    let table_l = table.to_ascii_lowercase();
    while let Some(tr) = table_l[pos..].find("<tr") {
        let tr_at = pos + tr;
        let after = table_l[tr_at..]
            .find('>')
            .map(|i| tr_at + i + 1)
            .unwrap_or(table.len());
        let close = table_l[after..]
            .find("</tr>")
            .map(|i| after + i)
            .unwrap_or(table.len());
        let row_html = &table[after..close];
        let (row, is_th) = parse_html_cells(row_html)?;
        if is_th && header.is_none() {
            add_cells(&mut cells, row.len())?;
            header = Some(row);
        } else if !row.is_empty() {
            add_table_shape(&mut cells, rows.len(), row.len())?;
            rows.push(row);
        }
        pos = close.saturating_add(5);
        if pos >= table.len() {
            break;
        }
    }
    if header.is_none() && rows.is_empty() {
        return Err(error::parse("HTML table has no rows"));
    }
    let mut plan = ImportPlan {
        has_header: header.is_some(),
        ..ImportPlan::default()
    };
    if let Some(h) = &header {
        plan.columns = h
            .iter()
            .map(|name| ColumnPlan {
                name: Some(name.clone()),
                ty: ColumnType::Auto,
            })
            .collect();
    }
    Ok(ClipboardTable { plan, header, rows })
}

fn parse_html_cells(row_html: &str) -> Result<(Vec<String>, bool), CoreError> {
    let lower = row_html.to_ascii_lowercase();
    let mut cells = Vec::new();
    let mut is_th = false;
    let mut pos = 0;
    loop {
        let td = lower[pos..].find("<td");
        let th = lower[pos..].find("<th");
        let (kind, rel) = match (td, th) {
            (None, None) => break,
            (Some(a), Some(b)) if a <= b => ("td", a),
            (Some(a), None) => ("td", a),
            (None, Some(b)) | (Some(_), Some(b)) => {
                is_th = true;
                ("th", b)
            }
        };
        let at = pos + rel;
        let after = lower[at..]
            .find('>')
            .map(|i| at + i + 1)
            .unwrap_or(row_html.len());
        let close_tag = format!("</{kind}>");
        let close = lower[after..]
            .find(&close_tag)
            .map(|i| after + i)
            .unwrap_or(row_html.len());
        let inner = &row_html[after..close];
        if cells.len() >= usize::from(MAX_COLS) {
            return Err(error::limit(format!(
                "clipboard row has more than {MAX_COLS} columns"
            )));
        }
        let text = html_text(inner);
        if text.len() > MAX_FIELD_BYTES {
            return Err(error::limit(format!(
                "clipboard field is {} bytes; maximum is {MAX_FIELD_BYTES}",
                text.len()
            )));
        }
        cells.push(text);
        pos = close.saturating_add(close_tag.len());
        if pos >= row_html.len() {
            break;
        }
    }
    Ok((cells, is_th))
}

fn add_table_shape(cells: &mut usize, current_rows: usize, width: usize) -> Result<(), CoreError> {
    if current_rows >= MAX_CLIPBOARD_ROWS {
        return Err(error::limit(format!(
            "clipboard table has more than {MAX_CLIPBOARD_ROWS} body rows"
        )));
    }
    add_cells(cells, width)
}

fn add_cells(cells: &mut usize, count: usize) -> Result<(), CoreError> {
    *cells = cells
        .checked_add(count)
        .ok_or_else(|| error::limit("clipboard table cell count overflow"))?;
    if *cells > MAX_CLIPBOARD_CELLS {
        return Err(error::limit(format!(
            "clipboard table has more than {MAX_CLIPBOARD_CELLS} cells"
        )));
    }
    Ok(())
}

fn html_text(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('<')
            && let Some(end) = stripped.find('>')
        {
            rest = &stripped[end + 1..];
            continue;
        }
        if let Some(stripped) = rest.strip_prefix('&')
            && let Some((ent, tail)) = split_entity(stripped)
        {
            out.push_str(&ent);
            rest = tail;
            continue;
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    collapse_ws(&out)
}

fn split_entity(rest: &str) -> Option<(String, &str)> {
    const MAX_HTML_ENTITY_NAME_BYTES: usize = 64;
    let end = rest
        .as_bytes()
        .iter()
        .take(MAX_HTML_ENTITY_NAME_BYTES)
        .position(|byte| *byte == b';')?;
    let body = &rest[..end];
    let tail = &rest[end + 1..];
    let decoded = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
        let n = u32::from_str_radix(hex, 16).ok()?;
        char::from_u32(n)?.to_string()
    } else if let Some(dec) = body.strip_prefix('#') {
        let n: u32 = dec.parse().ok()?;
        char::from_u32(n)?.to_string()
    } else {
        quick_xml::escape::resolve_html5_entity(body)
            .or_else(|| resolve_html5_supplemental_entity(body))?
            .to_owned()
    };
    Some((decoded, tail))
}

// quick-xml 0.41's `escape-html` table contains all single-code-point HTML5
// names except these two, but omits the standard's multi-code-point entries.
// This supplement completes the 2,125 semicolon-terminated named references.
fn resolve_html5_supplemental_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "NotEqualTilde" => "\u{2242}\u{338}",
        "NotGreaterFullEqual" => "\u{2267}\u{338}",
        "NotGreaterGreater" => "\u{226B}\u{338}",
        "NotGreaterSlantEqual" => "\u{2A7E}\u{338}",
        "NotHumpDownHump" => "\u{224E}\u{338}",
        "NotHumpEqual" => "\u{224F}\u{338}",
        "NotLeftTriangleBar" => "\u{29CF}\u{338}",
        "NotLessLess" => "\u{226A}\u{338}",
        "NotLessSlantEqual" => "\u{2A7D}\u{338}",
        "NotNestedGreaterGreater" => "\u{2AA2}\u{338}",
        "NotNestedLessLess" => "\u{2AA1}\u{338}",
        "NotPrecedesEqual" => "\u{2AAF}\u{338}",
        "NotRightTriangleBar" => "\u{29D0}\u{338}",
        "NotSquareSubset" => "\u{228F}\u{338}",
        "NotSquareSuperset" => "\u{2290}\u{338}",
        "NotSubset" => "\u{2282}\u{20D2}",
        "NotSucceedsEqual" => "\u{2AB0}\u{338}",
        "NotSucceedsTilde" => "\u{227F}\u{338}",
        "NotSuperset" => "\u{2283}\u{20D2}",
        "ThickSpace" => "\u{205F}\u{200A}",
        "acE" => "\u{223E}\u{333}",
        "bne" => "=\u{20E5}",
        "bnequiv" => "\u{2261}\u{20E5}",
        "bsolhsub" => "\u{27C8}",
        "caps" => "\u{2229}\u{FE00}",
        "cups" => "\u{222A}\u{FE00}",
        "fjlig" => "fj",
        "gesl" => "\u{22DB}\u{FE00}",
        "gvertneqq" | "gvnE" => "\u{2269}\u{FE00}",
        "lates" => "\u{2AAD}\u{FE00}",
        "lesg" => "\u{22DA}\u{FE00}",
        "lvertneqq" | "lvnE" => "\u{2268}\u{FE00}",
        "nGg" => "\u{22D9}\u{338}",
        "nGt" => "\u{226B}\u{20D2}",
        "nGtv" => "\u{226B}\u{338}",
        "nLl" => "\u{22D8}\u{338}",
        "nLt" => "\u{226A}\u{20D2}",
        "nLtv" => "\u{226A}\u{338}",
        "nang" => "\u{2220}\u{20D2}",
        "napE" => "\u{2A70}\u{338}",
        "napid" => "\u{224B}\u{338}",
        "nbump" => "\u{224E}\u{338}",
        "nbumpe" => "\u{224F}\u{338}",
        "ncongdot" => "\u{2A6D}\u{338}",
        "nedot" => "\u{2250}\u{338}",
        "nesim" => "\u{2242}\u{338}",
        "ngE" | "ngeqq" => "\u{2267}\u{338}",
        "ngeqslant" | "nges" => "\u{2A7E}\u{338}",
        "nlE" | "nleqq" => "\u{2266}\u{338}",
        "nleqslant" | "nles" => "\u{2A7D}\u{338}",
        "notinE" => "\u{22F9}\u{338}",
        "notindot" => "\u{22F5}\u{338}",
        "nparsl" => "\u{2AFD}\u{20E5}",
        "npart" => "\u{2202}\u{338}",
        "npre" | "npreceq" => "\u{2AAF}\u{338}",
        "nrarrc" => "\u{2933}\u{338}",
        "nrarrw" => "\u{219D}\u{338}",
        "nsce" => "\u{2AB0}\u{338}",
        "nsubE" | "nsubseteqq" => "\u{2AC5}\u{338}",
        "nsubset" | "vnsub" => "\u{2282}\u{20D2}",
        "nsucceq" => "\u{2AB0}\u{338}",
        "nsupE" | "nsupseteqq" => "\u{2AC6}\u{338}",
        "nsupset" | "vnsup" => "\u{2283}\u{20D2}",
        "nvap" => "\u{224D}\u{20D2}",
        "nvge" => "\u{2265}\u{20D2}",
        "nvgt" => ">\u{20D2}",
        "nvle" => "\u{2264}\u{20D2}",
        "nvlt" => "<\u{20D2}",
        "nvltrie" => "\u{22B4}\u{20D2}",
        "nvrtrie" => "\u{22B5}\u{20D2}",
        "nvsim" => "\u{223C}\u{20D2}",
        "smtes" => "\u{2AAC}\u{FE00}",
        "sqcaps" => "\u{2293}\u{FE00}",
        "sqcups" => "\u{2294}\u{FE00}",
        "suphsol" => "\u{27C9}",
        "varsubsetneq" | "vsubne" => "\u{228A}\u{FE00}",
        "varsubsetneqq" | "vsubnE" => "\u{2ACB}\u{FE00}",
        "varsupsetneq" | "vsupne" => "\u{228B}\u{FE00}",
        "varsupsetneqq" | "vsupnE" => "\u{2ACC}\u{FE00}",
        _ => return None,
    })
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::new();
    let mut prev_space = true;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}
