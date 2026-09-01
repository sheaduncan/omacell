//! Clipboard CSV / TSV / Markdown / HTML helpers.

use omacell_io::csv::{ClipboardFormat, MAX_CLIPBOARD_BYTES, parse_clipboard};
use omacell_io::error::codes;

#[test]
fn tsv_and_csv() {
    let tsv = parse_clipboard("a\tb\n1\t2\n", ClipboardFormat::Tsv).unwrap();
    assert_eq!(tsv.plan.delimiter, '\t');
    assert_eq!(tsv.rows[1][1], "2");
    let csv = parse_clipboard("a,b\n1,2\n", ClipboardFormat::Csv).unwrap();
    assert_eq!(csv.plan.delimiter, ',');
    assert_eq!(csv.rows[0][0], "a");
}

#[test]
fn markdown_table() {
    let md = "\
| name | zip |\n\
| --- | --- |\n\
| Ada | 02115 |\n";
    let table = parse_clipboard(md, ClipboardFormat::Markdown).unwrap();
    assert_eq!(table.header.as_ref().unwrap()[0], "name");
    assert_eq!(table.rows[0][1], "02115");
}

#[test]
fn html_table() {
    let html = "<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>caf&eacute; &NotEqualTilde;</td></tr></table>";
    let table = parse_clipboard(html, ClipboardFormat::Html).unwrap();
    assert_eq!(table.header.as_ref().unwrap()[0], "A");
    assert_eq!(table.rows[0][0], "1");
    assert_eq!(table.rows[0][1], "café ≂̸");
}

#[test]
fn html_nbsp_and_amp() {
    let html = "<table><tr><td>a&nbsp;b &amp; c</td></tr></table>";
    let table = parse_clipboard(html, ClipboardFormat::Auto).unwrap();
    assert_eq!(table.rows[0][0], "a b & c");
}

#[test]
fn auto_detects_markdown() {
    let md = "| a |\n| --- |\n| 1 |\n";
    let table = parse_clipboard(md, ClipboardFormat::Auto).unwrap();
    assert_eq!(table.plan.delimiter, '|');
}

#[test]
fn clipboard_payload_and_column_limits_fail_cleanly() {
    let oversized = "x".repeat(MAX_CLIPBOARD_BYTES + 1);
    let err = parse_clipboard(&oversized, ClipboardFormat::Csv).unwrap_err();
    assert_eq!(err.code, codes::CSV_LIMIT);

    let too_wide = ",".repeat(usize::from(omacell_core::limits::MAX_COLS));
    let err = parse_clipboard(&too_wide, ClipboardFormat::Csv).unwrap_err();
    assert_eq!(err.code, codes::CSV_LIMIT);
}
