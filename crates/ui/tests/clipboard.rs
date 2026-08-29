//! Clipboard round-trips.

use omacell_io::csv::ClipboardFormat;
use omacell_ui::ClipboardPayload;

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
