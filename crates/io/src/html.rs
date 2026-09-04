//! HTML and Markdown table file import/export (F-9.5).

use std::path::Path;

use omacell_core::error::CoreError;
use omacell_core::workbook::Workbook;

use crate::csv::{ClipboardFormat, ClipboardTable, parse_clipboard};
use crate::error;
use crate::xlsx::xml::{escape, is_xml10_char};
use crate::xlsx::{SaveOptions, atomic_write_bytes, peer_lock_blocks};

const MAX_TABLE_FILE_BYTES: usize = 8 * 1_048_576;
const MAX_TABLE_EXPORT_CELLS: u64 = 1_000_000;

/// Open an HTML file (first `<table>`).
pub fn open_html(path: &Path) -> Result<Workbook, CoreError> {
    open_kind(path, ClipboardFormat::Html)
}

/// Open a Markdown table file.
pub fn open_markdown(path: &Path) -> Result<Workbook, CoreError> {
    open_kind(path, ClipboardFormat::Markdown)
}

/// Open HTML or Markdown bytes.
pub fn open_bytes(bytes: &[u8], kind: ClipboardFormat) -> Result<Workbook, CoreError> {
    if bytes.len() > MAX_TABLE_FILE_BYTES {
        return Err(error::xlsx_limit(format!(
            "table file is {} bytes; maximum is {MAX_TABLE_FILE_BYTES}",
            bytes.len()
        )));
    }
    let text = std::str::from_utf8(bytes).map_err(|e| error::html_format(e.to_string()))?;
    table_to_workbook(parse_clipboard(text, kind).map_err(|err| {
        if err.code != error::codes::CSV_PARSE {
            return err;
        }
        let mut mapped = CoreError::new(error::codes::HTML_FORMAT, err.message);
        if let Some(hint) = err.hint {
            mapped = mapped.with_hint(hint);
        }
        mapped
    })?)
}

fn open_kind(path: &Path, kind: ClipboardFormat) -> Result<Workbook, CoreError> {
    peer_lock_blocks(path)?;
    let len = std::fs::metadata(path)
        .map_err(|e| error::html_format(e.to_string()))?
        .len();
    if len > MAX_TABLE_FILE_BYTES as u64 {
        return Err(error::xlsx_limit(format!(
            "table file is {len} bytes; maximum is {MAX_TABLE_FILE_BYTES}",
        )));
    }
    let bytes = std::fs::read(path).map_err(|e| error::html_format(e.to_string()))?;
    open_bytes(&bytes, kind)
}

fn table_to_workbook(table: ClipboardTable) -> Result<Workbook, CoreError> {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let undo = wb.undo_log().is_enabled();
    wb.undo_log_mut().set_enabled(false);
    let mut row = 0u32;
    if let Some(header) = &table.header {
        for (c, cell) in header.iter().enumerate() {
            wb.set_text(sheet, 0, c as u16, cell)?;
        }
        row = 1;
    }
    for rec in table.rows {
        for (c, cell) in rec.iter().enumerate() {
            if !has_significant_leading_zero(cell)
                && let Ok(n) = cell.parse::<f64>()
                && n.is_finite()
            {
                wb.set_number(sheet, row, c as u16, n)?;
                continue;
            }
            if !cell.is_empty() {
                wb.set_text(sheet, row, c as u16, cell)?;
            }
        }
        row += 1;
    }
    wb.undo_log_mut().set_enabled(undo);
    Ok(wb)
}

/// Export the active sheet used range as an HTML table.
pub fn export_html(wb: &Workbook) -> Result<Vec<u8>, CoreError> {
    Ok(export_markup(wb, Markup::Html)?.into_bytes())
}

/// Export the active sheet used range as a GitHub-style Markdown table.
pub fn export_markdown(wb: &Workbook) -> Result<Vec<u8>, CoreError> {
    Ok(export_markup(wb, Markup::Markdown)?.into_bytes())
}

/// Save HTML/Markdown with a peer lock.
pub fn save(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    atomic_write_bytes(path, bytes, SaveOptions::default())
}

enum Markup {
    Html,
    Markdown,
}

fn export_markup(wb: &Workbook, kind: Markup) -> Result<String, CoreError> {
    let sheet = wb.active_sheet();
    let Some(used) = wb.used_range(sheet)? else {
        return Ok(match kind {
            Markup::Html => "<table></table>\n".into(),
            Markup::Markdown => String::new(),
        });
    };
    let row_count = u64::from(used.max_row - used.min_row + 1);
    let col_count = u64::from(used.max_col - used.min_col + 1);
    let cells = row_count.saturating_mul(col_count);
    if cells > MAX_TABLE_EXPORT_CELLS {
        return Err(error::xlsx_limit(format!(
            "table export would visit {cells} cells; maximum is {MAX_TABLE_EXPORT_CELLS}"
        )));
    }
    let mut rows = Vec::new();
    for row in used.min_row..=used.max_row {
        let mut rec = Vec::new();
        for col in used.min_col..=used.max_col {
            rec.push(display_cell(wb, sheet, row, col));
        }
        rows.push(rec);
    }
    Ok(match kind {
        Markup::Html => {
            let mut out = String::from("<table>\n");
            for (i, rec) in rows.iter().enumerate() {
                out.push_str("<tr>");
                let tag = if i == 0 { "th" } else { "td" };
                for cell in rec {
                    out.push_str(&format!("<{tag}>{}</{tag}>", html_escape(cell)?));
                }
                out.push_str("</tr>\n");
            }
            out.push_str("</table>\n");
            out
        }
        Markup::Markdown => {
            if rows.is_empty() {
                return Ok(String::new());
            }
            let mut out = String::new();
            out.push_str(&md_row(&rows[0]));
            out.push('\n');
            out.push('|');
            for _ in &rows[0] {
                out.push_str(" --- |");
            }
            out.push('\n');
            for rec in rows.iter().skip(1) {
                out.push_str(&md_row(rec));
                out.push('\n');
            }
            out
        }
    })
}

fn html_escape(value: &str) -> Result<String, CoreError> {
    if let Some(ch) = value.chars().find(|ch| !is_xml10_char(*ch)) {
        return Err(error::html_format(format!(
            "HTML text contains XML-forbidden character U+{:04X}",
            u32::from(ch)
        )));
    }
    Ok(escape(value))
}

fn md_row(rec: &[String]) -> String {
    let mut out = String::from("|");
    for cell in rec {
        out.push(' ');
        let escaped = cell
            .replace('\\', "\\\\")
            .replace('|', "\\|")
            .replace("\r\n", "<br>")
            .replace(['\r', '\n'], "<br>");
        out.push_str(&escaped);
        out.push_str(" |");
    }
    out
}

fn has_significant_leading_zero(value: &str) -> bool {
    let digits = value
        .strip_prefix(['+', '-'])
        .unwrap_or(value)
        .strip_prefix('0');
    digits.is_some_and(|rest| rest.as_bytes().first().is_some_and(u8::is_ascii_digit))
}

fn display_cell(wb: &Workbook, sheet: omacell_core::addr::SheetId, row: u32, col: u16) -> String {
    match wb.get(sheet, row, col).ok().flatten() {
        Some(slot) => match slot.value {
            omacell_core::value::Value::Number(n) => n.to_string(),
            omacell_core::value::Value::Bool(true) => "TRUE".into(),
            omacell_core::value::Value::Bool(false) => "FALSE".into(),
            omacell_core::value::Value::Text(id) => {
                wb.intern().strings.get(id).unwrap_or("").to_string()
            }
            omacell_core::value::Value::Error(kind) => kind.as_str().to_string(),
            _ => String::new(),
        },
        None => String::new(),
    }
}
