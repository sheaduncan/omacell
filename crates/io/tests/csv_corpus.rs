//! Encoding, delimiter, quoting, ragged, and locale sniff corpus.

use omacell_io::csv::{
    ImportPlan, LineEnding, TextEncoding, decode_all, encode_all, load, load_into, sniff,
    sniff_path,
};

mod common;

fn parse_delim(s: &str) -> char {
    match s {
        r"\t" => '\t',
        other if other.chars().count() == 1 => other.chars().next().unwrap(),
        other => panic!("bad delimiter {other:?}"),
    }
}

#[test]
fn sniff_corpus() {
    let rows = common::read_tsv(&common::corpus_file("sniff.tsv"));
    assert!(!rows.is_empty());
    for row in &rows {
        assert!(row.len() >= 10, "{row:?}");
        let file = &row[0];
        let delim = parse_delim(&row[1]);
        let quote = parse_delim(&row[2]);
        let enc = TextEncoding::from_tag(&row[3]).unwrap();
        let bom = row[4] == "true";
        let header = row[5] == "true";
        let decimal = row[6].chars().next().unwrap_or('.');
        let thousands = if row[7].is_empty() {
            None
        } else {
            row[7].chars().next()
        };
        let eol = LineEnding::from_tag(&row[8]).unwrap();
        let note = &row[9];
        let sniff =
            sniff_path(&common::corpus_file(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(sniff.plan.delimiter, delim, "{file} delimiter ({note})");
        assert_eq!(sniff.plan.quote, quote, "{file} quote ({note})");
        assert_eq!(sniff.plan.encoding, enc, "{file} encoding ({note})");
        assert_eq!(sniff.plan.bom, bom, "{file} bom ({note})");
        assert_eq!(sniff.plan.has_header, header, "{file} header ({note})");
        assert_eq!(sniff.plan.decimal, decimal, "{file} decimal ({note})");
        assert_eq!(sniff.plan.thousands, thousands, "{file} thousands ({note})");
        assert_eq!(sniff.plan.line_ending, eol, "{file} eol ({note})");
    }
}

#[test]
fn quoted_newline_round_trip() {
    let bytes = std::fs::read(common::corpus_file("quoted_newline.csv")).unwrap();
    let sniff = sniff(&bytes).unwrap();
    let (wb, result) = load(&bytes, &sniff.plan, Default::default()).unwrap();
    assert_eq!(result.rows_written, 2);
    let sheet = wb.active_sheet();
    let a1 = wb.get(sheet, 0, 1).unwrap().unwrap();
    let text = wb.intern().strings.get(match a1.value {
        omacell_core::value::Value::Text(id) => id,
        other => panic!("{other:?}"),
    });
    assert_eq!(text, Some("hello\nworld"));
}

#[test]
fn ragged_rows_load() {
    let bytes = std::fs::read(common::corpus_file("ragged.csv")).unwrap();
    let sniff = sniff(&bytes).unwrap();
    let (wb, result) = load(&bytes, &sniff.plan, Default::default()).unwrap();
    assert_eq!(result.cols, 4);
    let sheet = wb.active_sheet();
    assert!(wb.get(sheet, 0, 2).unwrap().is_none());
    assert_eq!(
        wb.get(sheet, 1, 3).unwrap().unwrap().value,
        omacell_core::value::Value::Number(6.0)
    );
}

#[test]
fn utf8_bom_round_trip() {
    let body = std::fs::read(common::corpus_file("simple.csv")).unwrap();
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(&body);
    let sniff = sniff(&bytes).unwrap();
    assert!(sniff.plan.bom);
    assert_eq!(sniff.plan.encoding, TextEncoding::Utf8);
    let (wb, _) = load(&bytes, &sniff.plan, Default::default()).unwrap();
    assert_eq!(
        wb.get(wb.active_sheet(), 0, 0).unwrap().unwrap().value,
        omacell_core::value::Value::Number(1.0)
    );
}

#[test]
fn utf16le_bom() {
    let text = std::fs::read_to_string(common::corpus_file("simple.csv")).unwrap();
    let bytes = encode_all(&text, TextEncoding::Utf16Le, true).unwrap();
    let sniff = sniff(&bytes).unwrap();
    assert_eq!(sniff.plan.encoding, TextEncoding::Utf16Le);
    assert!(sniff.plan.bom);
    let (wb, _) = load(&bytes, &sniff.plan, Default::default()).unwrap();
    assert_eq!(
        wb.get(wb.active_sheet(), 0, 0).unwrap().unwrap().value,
        omacell_core::value::Value::Number(1.0)
    );
}

#[test]
fn utf16be_bom() {
    let text = std::fs::read_to_string(common::corpus_file("simple.csv")).unwrap();
    let bytes = encode_all(&text, TextEncoding::Utf16Be, true).unwrap();
    let sniff = sniff(&bytes).unwrap();
    assert_eq!(sniff.plan.encoding, TextEncoding::Utf16Be);
    let (wb, _) = load(&bytes, &sniff.plan, Default::default()).unwrap();
    assert_eq!(
        wb.get(wb.active_sheet(), 1, 1).unwrap().unwrap().value,
        omacell_core::value::Value::Number(4.0)
    );
}

#[test]
fn latin1_cafe() {
    let mut bytes = b"caf".to_vec();
    bytes.push(0xE9);
    bytes.extend_from_slice(b",city\n");
    let sniff = sniff(&bytes).unwrap();
    assert_eq!(sniff.plan.encoding, TextEncoding::Latin1);
    let decoded = decode_all(&bytes, sniff.plan.encoding).unwrap();
    assert!(decoded.starts_with("café"), "{decoded:?}");
    let mut wb = omacell_core::workbook::Workbook::new();
    let mut plan = sniff.plan.clone();
    plan.columns.clear();
    plan.columns.push(omacell_io::csv::ColumnPlan {
        name: None,
        ty: omacell_io::csv::ColumnType::Text,
    });
    load_into(&mut wb, bytes.as_slice(), &plan, Default::default()).unwrap();
    let slot = wb.get(wb.active_sheet(), 0, 0).unwrap().unwrap();
    let omacell_core::value::Value::Text(id) = slot.value else {
        panic!("expected text");
    };
    assert_eq!(wb.intern().strings.get(id), Some("café"));
}

#[test]
fn header_keep_as_text_zip_and_id() {
    let sniff = sniff_path(&common::corpus_file("header.csv")).unwrap();
    assert!(sniff.plan.has_header);
    assert!(
        sniff
            .plan
            .columns
            .iter()
            .any(|c| c.name.as_deref() == Some("zip")
                && matches!(c.ty, omacell_io::csv::ColumnType::KeepAsText))
    );
    let bytes = std::fs::read(common::corpus_file("header.csv")).unwrap();
    let (wb, _) = load(&bytes, &sniff.plan, Default::default()).unwrap();
    let sheet = wb.active_sheet();
    let zip = wb.get(sheet, 1, 1).unwrap().unwrap();
    let omacell_core::value::Value::Text(id) = zip.value else {
        panic!("ZIP became {:?}", zip.value);
    };
    assert_eq!(wb.intern().strings.get(id), Some("02115"));
}

#[test]
fn import_plan_json_round_trip() {
    let plan = ImportPlan::default();
    let json = serde_json::to_string(&plan).unwrap();
    let back: ImportPlan = serde_json::from_str(&json).unwrap();
    assert_eq!(plan, back);
}

#[test]
fn semicolon_with_decimal_commas_is_not_split_on_decimal() {
    let bytes = b"value;amount\n1,5;2,6\n3,5;4,6\n";
    let sniffed = sniff(bytes).unwrap();
    assert_eq!(sniffed.plan.delimiter, ';');
    assert_eq!(sniffed.plan.decimal, ',');
    assert_eq!(sniffed.plan.thousands, None);
    assert_eq!(sniffed.sample_rows[1], ["1,5", "2,6"]);

    let (wb, result) = load(bytes, &sniffed.plan, Default::default()).unwrap();
    assert_eq!(result.cols, 2);
    assert_eq!(
        wb.get(wb.active_sheet(), 1, 1).unwrap().unwrap().value,
        omacell_core::value::Value::Number(2.6)
    );
}

#[test]
fn sniffs_single_quote_quoting() {
    let bytes = b"'a,b',c\n'd,e',f\n";
    let sniffed = sniff(bytes).unwrap();
    assert_eq!(sniffed.plan.delimiter, ',');
    assert_eq!(sniffed.plan.quote, '\'');
    assert_eq!(sniffed.sample_rows[0], ["a,b", "c"]);
}

#[test]
fn apostrophe_prefixed_values_do_not_select_single_quote_syntax() {
    let bytes = b"name,city\n'Twas John's book,Chicago\n'Alice's copy,Boston\n";
    let sniffed = sniff(bytes).unwrap();

    assert_eq!(sniffed.plan.delimiter, ',');
    assert_eq!(sniffed.plan.quote, '"');
    assert_eq!(sniffed.sample_rows[1], ["'Twas John's book", "Chicago"]);
}

#[test]
fn sniff_scoring_does_not_overflow_on_ragged_input() {
    let mut text = ",".repeat(9_999);
    text.push('\n');
    text.push_str(&"x\n".repeat(1_000));
    let sniffed = sniff(text.as_bytes()).unwrap();
    assert_eq!(sniffed.plan.delimiter, ',');
}

#[test]
fn sniff_accepts_utf8_codepoint_split_at_sample_boundary() {
    let mut bytes = vec![b'a'; omacell_io::csv::MAX_SNIFF_BYTES - 1];
    bytes.extend_from_slice("é".as_bytes());
    let sniffed = sniff(&bytes).unwrap();
    assert_eq!(sniffed.plan.encoding, TextEncoding::Utf8);
}

#[test]
fn load_checks_actual_bom_instead_of_trusting_plan_flag() {
    let expects_bom = ImportPlan {
        bom: true,
        columns: vec![omacell_io::csv::ColumnPlan {
            name: None,
            ty: omacell_io::csv::ColumnType::Text,
        }],
        ..ImportPlan::default()
    };
    let (wb, result) = load(b"abc\n", &expects_bom, Default::default()).unwrap();
    assert_eq!(result.rows_written, 1);
    let slot = wb.get(wb.active_sheet(), 0, 0).unwrap().unwrap();
    let omacell_core::value::Value::Text(id) = slot.value else {
        panic!("expected text");
    };
    assert_eq!(wb.intern().strings.get(id), Some("abc"));

    let mut actual_bom = vec![0xEF, 0xBB, 0xBF];
    actual_bom.extend_from_slice(b"xyz\n");
    let ignores_flag = ImportPlan {
        bom: false,
        columns: expects_bom.columns,
        ..ImportPlan::default()
    };
    let (wb, _) = load(&actual_bom, &ignores_flag, Default::default()).unwrap();
    let slot = wb.get(wb.active_sheet(), 0, 0).unwrap().unwrap();
    let omacell_core::value::Value::Text(id) = slot.value else {
        panic!("expected text");
    };
    assert_eq!(wb.intern().strings.get(id), Some("xyz"));
}
