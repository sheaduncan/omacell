//! No-silent-conversion suite (F-9.4).

use omacell_core::locale::LocaleId;
use omacell_io::csv::{ColumnType, ImportPlan, convert_cell};

mod common;

#[test]
fn auto_corpus_no_silent_conversion() {
    let rows = common::read_tsv(&common::corpus_file("auto.tsv"));
    assert!(!rows.is_empty(), "auto.tsv is empty");
    for (i, row) in rows.iter().enumerate() {
        assert!(
            row.len() >= 7,
            "line {}: expected 7 columns, got {row:?}",
            i + 1
        );
        let raw = &row[0];
        let locale = LocaleId::parse_tag(&row[1]).unwrap_or_else(|| panic!("locale {}", row[1]));
        let ty = ColumnType::from_corpus(&row[2]).unwrap_or_else(|| panic!("type {}", row[2]));
        let kind = row[3].as_str();
        let would = row[4].as_str();
        let changed = row[5] == "true";
        let note = &row[6];
        let plan = ImportPlan::with_locale(locale);
        let got = convert_cell(raw, &ty, &plan);
        assert_eq!(kind_str(&got), kind, "row {raw:?} ({note}): kind");
        assert_eq!(
            got.preview_text(&plan),
            would,
            "row {raw:?} ({note}): would_become"
        );
        assert_eq!(got.changed(), changed, "row {raw:?} ({note}): changed");
        if changed {
            assert!(
                !matches!(got, omacell_io::csv::Converted::Text(_)),
                "changed cell {raw:?} must not stay text ({note})"
            );
        }
    }
}

fn kind_str(c: &omacell_io::csv::Converted) -> &'static str {
    match c.kind() {
        omacell_io::csv::ConvertedKind::Empty => "empty",
        omacell_io::csv::ConvertedKind::Number => "number",
        omacell_io::csv::ConvertedKind::Bool => "bool",
        omacell_io::csv::ConvertedKind::Date => "date",
        omacell_io::csv::ConvertedKind::Text => "text",
    }
}

#[test]
fn preview_marks_every_changed_cell() {
    let plan = ImportPlan::with_locale(LocaleId::EN_US);
    let bytes = b"007,123,SEPT1,2020-01-02\n";
    let preview = omacell_io::csv::preview(bytes, &plan, 1).unwrap();
    let row = &preview.rows[0];
    assert!(!row[0].changed && row[0].kind == "text");
    assert!(row[1].changed && row[1].kind == "number");
    assert!(!row[2].changed && row[2].kind == "text");
    assert!(row[3].changed && row[3].kind == "date");
}
