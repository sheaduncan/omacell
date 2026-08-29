//! Clipboard encode/decode (F-5.6) over WP-08 helpers.

use omacell_core::error::CoreError;
use omacell_io::csv::{ClipboardFormat, ClipboardTable, parse_clipboard};
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
    #[must_use]
    pub fn from_rows(rows: &[Vec<String>]) -> Self {
        let tsv = join_rows(rows, '\t');
        let csv = csv_escape(rows);
        let html = html_table(rows);
        let markdown = markdown_table(rows);
        let internal = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into());
        Self {
            tsv,
            csv,
            html,
            markdown,
            internal,
        }
    }

    /// Decode pasted text using WP-08 sniffing.
    pub fn decode(text: &str, kind: ClipboardFormat) -> Result<ClipboardTable, CoreError> {
        parse_clipboard(text, kind)
    }
}

fn join_rows(rows: &[Vec<String>], delim: char) -> String {
    rows.iter()
        .map(|r| r.join(&delim.to_string()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn csv_escape(rows: &[Vec<String>]) -> String {
    rows.iter()
        .map(|r| {
            r.iter()
                .map(|c| {
                    if c.contains([',', '"', '\n']) {
                        format!("\"{}\"", c.replace('"', "\"\""))
                    } else {
                        c.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
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
        lines.push(format!("|{}|", r.join("|")));
    }
    lines.join("\n")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
