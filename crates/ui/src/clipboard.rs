//! Clipboard encode/decode (F-5.6) over WP-08 helpers.

use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::error::CoreError;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::ops::ClipValue;
use omacell_io::csv::{
    ClipboardFormat, ClipboardTable, MAX_CLIPBOARD_BYTES, MAX_CLIPBOARD_CELLS, MAX_CLIPBOARD_ROWS,
    MAX_FIELD_BYTES, parse_clipboard,
};
use serde::{Deserialize, Serialize};

/// Internal MIME payload.
pub const INTERNAL_MIME: &str = "application/x-omacell-cells+json";

/// One clipboard snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipboardPayload {
    /// TSV (`text/plain`).
    pub tsv: String,
    /// CSV.
    pub csv: String,
    /// HTML table.
    pub html: String,
    /// Markdown table.
    pub markdown: String,
    /// Internal JSON (values + formulas).
    pub internal: String,
}

impl ClipboardPayload {
    /// Encode a row-major grid of display strings.
    pub fn from_rows(rows: &[Vec<String>]) -> Result<Self, CoreError> {
        let internal =
            serde_json::to_string(&rows).map_err(|err| crate::error::clipboard(err.to_string()))?;
        Self::from_rows_with_internal(rows, internal)
    }

    /// Capture the rich payload returned by `edit.copy` / `edit.cut` while also
    /// producing interoperable text formats from the cells' displayed values.
    pub fn from_bus_result(result: &serde_json::Value) -> Result<Self, CoreError> {
        let payload = result
            .get("payload")
            .ok_or_else(|| crate::error::clipboard("copy result is missing its payload"))?;
        let cells = payload
            .get("cells")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| crate::error::clipboard("copy payload is missing its cell rows"))?;
        let mut rows = Vec::with_capacity(cells.len());
        for cells in cells {
            let cells = cells
                .as_array()
                .ok_or_else(|| crate::error::clipboard("copy payload row is not an array"))?;
            let mut row = Vec::with_capacity(cells.len());
            for cell in cells {
                let value: ClipValue =
                    serde_json::from_value(cell.get("value").cloned().ok_or_else(|| {
                        crate::error::clipboard("copy cell is missing its value")
                    })?)
                    .map_err(|err| crate::error::clipboard(format!("invalid copy cell: {err}")))?;
                row.push(match value {
                    ClipValue::Empty => String::new(),
                    ClipValue::Number(value) => value.to_string(),
                    ClipValue::Bool(value) => if value { "TRUE" } else { "FALSE" }.to_string(),
                    ClipValue::Text(value) => value,
                    ClipValue::Error(error) => error.to_string(),
                });
            }
            rows.push(row);
        }
        let internal = serde_json::to_string(payload)
            .map_err(|err| crate::error::clipboard(err.to_string()))?;
        Self::from_rows_with_internal(&rows, internal)
    }

    /// Decode the rich internal JSON retained from a bus copy/cut result.
    pub fn internal_json(&self) -> Result<serde_json::Value, CoreError> {
        serde_json::from_str(&self.internal)
            .map_err(|err| crate::error::clipboard(format!("invalid internal clipboard: {err}")))
    }

    /// Convert external text paste into one bounded `range.set` argument object.
    pub fn text_paste_args(text: &str, cursor: CellRef) -> Result<serde_json::Value, CoreError> {
        let table = Self::decode(text, ClipboardFormat::Auto)?;
        let mut rows = table.rows;
        if let Some(header) = table.header {
            rows.insert(0, header);
        }
        validate_rows(&rows)?;
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        if rows.is_empty() || width == 0 {
            return Err(crate::error::clipboard("clipboard contains no cells"));
        }
        let height = u32::try_from(rows.len())
            .map_err(|_| crate::error::clipboard("clipboard is too tall"))?;
        let width =
            u16::try_from(width).map_err(|_| crate::error::clipboard("clipboard is too wide"))?;
        let end_row = cursor
            .row
            .checked_add(height - 1)
            .filter(|row| *row < MAX_ROWS)
            .ok_or_else(|| crate::error::clipboard("paste exceeds the worksheet row limit"))?;
        let end_col = cursor
            .col
            .checked_add(width - 1)
            .filter(|col| *col < MAX_COLS)
            .ok_or_else(|| crate::error::clipboard("paste exceeds the worksheet column limit"))?;
        let end = CellRef::new(end_row, end_col)?;
        let range = RangeRef::from_corners(cursor, end).to_a1();
        Ok(serde_json::json!({"range": range, "values": rows}))
    }

    fn from_rows_with_internal(rows: &[Vec<String>], internal: String) -> Result<Self, CoreError> {
        validate_rows(rows)?;
        let tsv = delimited(rows, '\t');
        let csv = delimited(rows, ',');
        let html = html_table(rows);
        let markdown = markdown_table(rows);
        for (format, payload) in [
            ("TSV", &tsv),
            ("CSV", &csv),
            ("HTML", &html),
            ("Markdown", &markdown),
            ("internal", &internal),
        ] {
            if payload.len() > MAX_CLIPBOARD_BYTES {
                return Err(crate::error::clipboard(format!(
                    "{format} clipboard payload exceeds {MAX_CLIPBOARD_BYTES} bytes"
                )));
            }
        }
        Ok(Self {
            tsv,
            csv,
            html,
            markdown,
            internal,
        })
    }

    /// Decode pasted text using WP-08 sniffing.
    pub fn decode(text: &str, kind: ClipboardFormat) -> Result<ClipboardTable, CoreError> {
        parse_clipboard(text, kind)
    }
}

fn delimited(rows: &[Vec<String>], delim: char) -> String {
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|cell| {
                    if cell
                        .chars()
                        .any(|ch| ch == delim || matches!(ch, '"' | '\r' | '\n'))
                    {
                        format!("\"{}\"", cell.replace('"', "\"\""))
                    } else {
                        cell.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(&delim.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn html_table(rows: &[Vec<String>]) -> String {
    let mut s = String::from("<table>");
    for row in rows {
        s.push_str("<tr>");
        for c in row {
            s.push_str("<td>");
            s.push_str(&escape(c));
            s.push_str("</td>");
        }
        s.push_str("</tr>");
    }
    s.push_str("</table>");
    s
}

fn markdown_table(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut lines = Vec::new();
    let header = vec![" ".to_string(); cols];
    lines.push(format!("|{}|", header.join("|")));
    lines.push(format!("|{}|", vec!["---"; cols].join("|")));
    for row in rows {
        let mut r = row.clone();
        r.resize(cols, String::new());
        lines.push(format!(
            "|{}|",
            r.iter()
                .map(|cell| escape_markdown(cell))
                .collect::<Vec<_>>()
                .join("|")
        ));
    }
    lines.join("\n")
}

fn escape_markdown(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace("\r\n", "<br>")
        .replace(['\r', '\n'], "<br>")
}

fn validate_rows(rows: &[Vec<String>]) -> Result<(), CoreError> {
    if rows.len() > MAX_CLIPBOARD_ROWS {
        return Err(crate::error::clipboard(format!(
            "clipboard has more than {MAX_CLIPBOARD_ROWS} rows"
        )));
    }
    let mut cells = 0_usize;
    let mut bytes = 0_usize;
    for row in rows {
        cells = cells
            .checked_add(row.len())
            .ok_or_else(|| crate::error::clipboard("clipboard cell count overflow"))?;
        if cells > MAX_CLIPBOARD_CELLS {
            return Err(crate::error::clipboard(format!(
                "clipboard has more than {MAX_CLIPBOARD_CELLS} cells"
            )));
        }
        for field in row {
            if field.len() > MAX_FIELD_BYTES {
                return Err(crate::error::clipboard(format!(
                    "clipboard field is {} bytes; maximum is {MAX_FIELD_BYTES}",
                    field.len()
                )));
            }
            bytes = bytes
                .checked_add(field.len().saturating_add(1))
                .ok_or_else(|| crate::error::clipboard("clipboard byte count overflow"))?;
            if bytes > MAX_CLIPBOARD_BYTES {
                return Err(crate::error::clipboard(format!(
                    "clipboard source exceeds {MAX_CLIPBOARD_BYTES} bytes"
                )));
            }
        }
    }
    Ok(())
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
