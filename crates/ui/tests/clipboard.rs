//! Clipboard round-trips.

use omacell_io::csv::ClipboardFormat;
use omacell_ui::ClipboardPayload;
use serde_json::json;

#[test]
fn tsv_csv_html_markdown_round_trip() {
    let rows = vec![
        vec!["a\tb".into(), "line\nbreak".into()],
        vec!["1".into(), "2,3 \"quoted\" | pipe <script>".into()],
    ];
    let payload = ClipboardPayload::from_rows(&rows).unwrap();
    assert!(payload.tsv.contains('\t'));
    assert!(
        payload
            .csv
            .contains("\"2,3 \"\"quoted\"\" | pipe <script>\"")
    );
    assert!(payload.html.contains("<td>"));
    assert!(payload.markdown.contains("\\|"));
    assert!(payload.markdown.contains("&lt;script&gt;"));
    let table = ClipboardPayload::decode(&payload.tsv, ClipboardFormat::Tsv).unwrap();
    assert_eq!(table.rows, rows);
    let table = ClipboardPayload::decode(&payload.csv, ClipboardFormat::Csv).unwrap();
    assert_eq!(table.rows, rows);
}

#[test]
fn bus_clipboard_keeps_rich_internal_payload_and_exports_display_text() {
    let bus_result = json!({
        "payload": {
            "cut": false,
            "sheet": 0,
            "row": 0,
            "col": 0,
            "cells": [[
                {"input": "=1+1", "value": {"kind": "number", "value": 2.0}},
                {"input": "hello", "value": {"kind": "text", "value": "hello"}},
                {"input": "", "value": {"kind": "error", "value": "#DIV/0!"}}
            ]],
            "extras": {"rows": 1, "cols": 3, "notes": [], "comments": [], "hyperlinks": [], "merges": []}
        }
    });

    let clipboard = ClipboardPayload::from_bus_result(&bus_result).unwrap();

    assert_eq!(clipboard.tsv, "2\thello\t#DIV/0!");
    assert_eq!(clipboard.internal_json().unwrap(), bus_result["payload"]);
}

#[test]
fn pasted_text_becomes_one_bounded_range_set_call() {
    let cursor = omacell_core::addr::CellRef::new(4, 2).unwrap();

    let args = ClipboardPayload::text_paste_args("1\t2\n3\t=SUM(A1:A2)", cursor).unwrap();

    assert_eq!(args["range"], "C5:D6");
    assert_eq!(args["values"], json!([["1", "2"], ["3", "=SUM(A1:A2)"]]));
}
