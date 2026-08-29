//! Clipboard round-trips.

use omacell_io::csv::ClipboardFormat;
use omacell_ui::ClipboardPayload;

#[test]
fn tsv_csv_html_markdown_round_trip() {
    let rows = vec![vec!["a".into(), "b".into()], vec!["1".into(), "2,3".into()]];
    let payload = ClipboardPayload::from_rows(&rows);
    assert!(payload.tsv.contains('\t'));
    assert!(payload.csv.contains("\"2,3\""));
    assert!(payload.html.contains("<td>"));
    assert!(payload.markdown.contains('|'));
    let table = ClipboardPayload::decode(&payload.tsv, ClipboardFormat::Tsv).unwrap();
    assert_eq!(table.rows.len(), 2);
}
