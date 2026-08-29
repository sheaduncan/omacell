//! Clipboard encode/decode (F-5.6) over WP-08 helpers.

use omacell_core::error::CoreError;
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
        validate_rows(rows)?;
        let tsv = delimited(rows, '\t');
        let csv = delimited(rows, ',');
        let html = html_table(rows);
        let markdown = markdown_table(rows);
        let internal =
            serde_json::to_string(&rows).map_err(|err| crate::error::clipboard(err.to_string()))?;
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
