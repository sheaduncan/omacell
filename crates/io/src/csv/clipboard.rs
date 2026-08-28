//! Clipboard helpers for WP-14 (TSV / CSV / Markdown / HTML tables).

use omacell_core::error::CoreError;
use omacell_core::locale::LocaleId;
use serde::{Deserialize, Serialize};

use super::plan::{ColumnPlan, ColumnType, ImportPlan, LineEnding, TextEncoding};
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
    let mut lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() < 2 {
        return Err(error::parse("markdown table needs a header and a body"));
    }
    let header = split_md_row(lines[0]);
    let sep_idx = lines.iter().position(|l| is_md_sep(l)).unwrap_or(1);
    if sep_idx + 1 > lines.len() {
        return Err(error::parse("markdown table is missing a body"));
    }
    lines.drain(..=sep_idx);
    let mut rows = Vec::new();
    for line in lines {
        if line.contains('|') {
            rows.push(split_md_row(line));
        }
    }
    let width = header.len();
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
    for row in &mut rows {
        row.resize(width, String::new());
    }
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

fn split_md_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| c.trim().to_string()).collect()
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
        let (cells, is_th) = parse_html_cells(row_html);
        if is_th && header.is_none() {
            header = Some(cells);
        } else if !cells.is_empty() {
            rows.push(cells);
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

fn parse_html_cells(row_html: &str) -> (Vec<String>, bool) {
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
        cells.push(html_text(inner));
        pos = close.saturating_add(close_tag.len());
        if pos >= row_html.len() {
            break;
        }
    }
    (cells, is_th)
}

fn html_text(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('<') {
            if let Some(end) = stripped.find('>') {
                rest = &stripped[end + 1..];
                continue;
            }
        }
        if let Some(stripped) = rest.strip_prefix('&') {
            if let Some((ent, tail)) = split_entity(stripped) {
                out.push_str(&ent);
                rest = tail;
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    collapse_ws(&out)
}

fn split_entity(rest: &str) -> Option<(String, &str)> {
    let end = rest.find(';')?;
    let body = &rest[..end];
    let tail = &rest[end + 1..];
    let ch = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
        let n = u32::from_str_radix(hex, 16).ok()?;
        char::from_u32(n)?
    } else if let Some(dec) = body.strip_prefix('#') {
        let n: u32 = dec.parse().ok()?;
        char::from_u32(n)?
    } else {
        match body {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            "nbsp" => ' ',
            _ => return None,
        }
    };
    Some((ch.to_string(), tail))
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
