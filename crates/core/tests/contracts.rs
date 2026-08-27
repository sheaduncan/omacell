//! WP-01 acceptance tests: addressing properties, serde round-trips, corpora.

use std::mem::size_of;
use std::path::PathBuf;

use omacell_core::addr::{
    CellRef, ParsedRef, RangeRef, RefKind, SheetId, SheetSpec, col_from_letters, col_to_letters,
    parse_a1, parse_a1_cell, parse_r1c1,
};
use omacell_core::changeset::{
    ChangeSummary, Changeset, ChangesetId, ChangesetStatus, CommandCall,
};
use omacell_core::command::{CommandDescriptor, CommandId, Origin, Outcome, UndoUnit, UndoUnitId};
use omacell_core::error::{CoreError, ErrorKind, codes};
use omacell_core::event::Event;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::locale::{LocaleId, LocaleSeparators};
use omacell_core::style::{
    Alignment, Border, BorderSide, BorderStyle, Color, Fill, Font, GradientFill, GradientKind,
    GradientStop, HorizontalAlign, NumFmtId, PatternType, Protection, Style, StyleId, Underline,
    VerticalAlign,
};
use omacell_core::value::{Array2D, ArrayId, StrId, Value};
use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, FileFailurePersistence};
use serde::Serialize;
use serde::de::DeserializeOwned;

fn corpus(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(rel)
}

fn read_tsv(path: &std::path::Path) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|line| {
            let t = line.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .map(|line| line.split('\t').map(ToOwned::to_owned).collect())
        .collect()
}

fn json_roundtrip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&json).unwrap();
    assert_eq!(*value, back, "json was {json}");
}

fn dummy_schema() -> schemars::Schema {
    serde_json::from_value(serde_json::json!({"type": "object"})).expect("schema")
}

#[test]
fn value_is_at_most_16_bytes() {
    assert!(
        size_of::<Value>() <= 16,
        "Value is {} bytes",
        size_of::<Value>()
    );
}

#[test]
fn a1_roundtrip_every_column() {
    for col in 0..MAX_COLS {
        for row in [0, 1, 25, MAX_ROWS - 1] {
            let cell = CellRef::new(row, col).unwrap();
            let text = cell.to_a1();
            let parsed = parse_a1_cell(&text).unwrap();
            assert_eq!(parsed.row, row, "{text}");
            assert_eq!(parsed.col, col, "{text}");
            assert!(!parsed.row_abs);
            assert!(!parsed.col_abs);
        }
        let abs = CellRef::with_abs(0, col, true, true).unwrap();
        assert_eq!(parse_a1_cell(&abs.to_a1()).unwrap(), abs);
    }
}

#[test]
fn a1_roundtrip_every_row_on_a_and_xfd() {
    for row in 0..MAX_ROWS {
        for col in [0, MAX_COLS - 1] {
            let cell = CellRef::new(row, col).unwrap();
            let text = cell.to_a1();
            let parsed = parse_a1_cell(&text).unwrap();
            assert_eq!(parsed, cell, "{text}");
        }
    }
}

#[test]
fn out_of_range_is_ref_class() {
    for input in ["XFE1", "A1048577", "AAAA1", "A0"] {
        let err = parse_a1(input).unwrap_err();
        assert_eq!(err.code, codes::ADDR_REF, "{input}: {err}");
        assert_eq!(err.excel_error(), Some(ErrorKind::Ref), "{input}");
    }
    assert_eq!(
        CellRef::new(MAX_ROWS, 0).unwrap_err().excel_error(),
        Some(ErrorKind::Ref)
    );
    assert_eq!(
        CellRef::new(0, MAX_COLS).unwrap_err().excel_error(),
        Some(ErrorKind::Ref)
    );
    assert_eq!(
        col_from_letters("XFE").unwrap_err().excel_error(),
        Some(ErrorKind::Ref)
    );
}

#[test]
fn a1_corpus() {
    let rows = read_tsv(&corpus("addr/a1.tsv"));
    assert!(rows.len() >= 20, "a1 corpus too small");
    for row in rows {
        let input = row[0].as_str();
        let print = row[1].as_str();
        let error = row[2].as_str();
        let note = row.get(3).map(String::as_str).unwrap_or("");
        match parse_a1(input) {
            Ok(parsed) => {
                assert!(
                    error.is_empty(),
                    "expected error {error} for {input} ({note})"
                );
                assert_eq!(parsed.to_a1(), print, "{input} ({note})");
            }
            Err(err) => {
                assert_eq!(err.code, error, "{input} ({note}): {err}");
            }
        }
    }
}

#[test]
fn r1c1_corpus() {
    let rows = read_tsv(&corpus("addr/r1c1.tsv"));
    assert!(rows.len() >= 10, "r1c1 corpus too small");
    for row in rows {
        let input = row[0].as_str();
        let base = parse_a1_cell(&row[1]).unwrap();
        let print = row[2].as_str();
        let error = row[3].as_str();
        let note = row.get(4).map(String::as_str).unwrap_or("");
        match parse_r1c1(input, base.row, base.col) {
            Ok(parsed) => {
                assert!(
                    error.is_empty(),
                    "expected error {error} for {input} ({note})"
                );
                assert_eq!(
                    parsed.to_r1c1(base.row, base.col),
                    print,
                    "{input} ({note})"
                );
            }
            Err(err) => {
                assert_eq!(err.code, error, "{input} ({note}): {err}");
            }
        }
    }
}

#[test]
fn error_type_corpus() {
    let rows = read_tsv(&corpus("errors/error_type.tsv"));
    assert_eq!(rows.len(), ErrorKind::all().len());
    for row in rows {
        let display = &row[1];
        let kind = ErrorKind::from_display(display).expect(display);
        assert_eq!(kind.as_str(), display);
        if row[2].is_empty() {
            assert_eq!(kind.error_type(), None, "{display}");
        } else {
            let n: u8 = row[2].parse().unwrap();
            assert_eq!(kind.error_type(), Some(n), "{display}");
        }
    }
}

fn arb_cell_ref() -> impl Strategy<Value = CellRef> {
    (0u32..MAX_ROWS, 0u16..MAX_COLS, any::<bool>(), any::<bool>()).prop_map(
        |(row, col, row_abs, col_abs)| {
            CellRef::with_abs(row, col, row_abs, col_abs).expect("in range")
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    })]

    #[test]
    fn a1_parse_print_roundtrip(cell in arb_cell_ref()) {
        let text = cell.to_a1();
        let back = parse_a1_cell(&text).unwrap();
        prop_assert_eq!(back, cell);
        prop_assert_eq!(back.to_a1(), text);
    }

    #[test]
    fn r1c1_parse_print_roundtrip(
        cell in arb_cell_ref(),
        base_row in 0u32..MAX_ROWS,
        base_col in 0u16..MAX_COLS,
    ) {
        let text = cell.to_r1c1(base_row, base_col);
        let back = parse_r1c1(&text, base_row, base_col).unwrap();
        match back.kind {
            RefKind::Cell(got) => {
                prop_assert_eq!(got.row, cell.row);
                prop_assert_eq!(got.col, cell.col);
                prop_assert_eq!(got.row_abs, cell.row_abs);
                prop_assert_eq!(got.col_abs, cell.col_abs);
            }
            RefKind::Range(_) => prop_assert!(false, "expected cell, got range for {text}"),
        }
        prop_assert_eq!(back.to_r1c1(base_row, base_col), text);
    }

    #[test]
    fn letters_roundtrip(col in 0u16..MAX_COLS) {
        let letters = col_to_letters(col).unwrap();
        prop_assert_eq!(col_from_letters(&letters).unwrap(), col);
    }
}

fn sample_value() -> Value {
    Value::Number(1.5)
}

fn sample_changeset() -> Changeset {
    Changeset {
        id: ChangesetId::new("cs-1").unwrap(),
        origin: Origin::User,
        status: ChangesetStatus::Proposed,
        forward: vec![CommandCall {
            id: CommandId::new("cell.set").unwrap(),
            args: serde_json::json!({"ref": "A1"}),
        }],
        inverse: vec![],
        summary: ChangeSummary {
            cells: 1,
            rows: 0,
            columns: 0,
            sheets: 0,
            styles: 0,
            text: "set A1".into(),
        },
    }
}

fn sample_style() -> Style {
    Style {
        font: Font {
            name: "Carlito".into(),
            size_pt: 11.0,
            bold: true,
            italic: false,
            underline: Underline::Single,
            strike: false,
            color: Color::Rgb { argb: 0xFF00_0000 },
        },
        fill: Fill::Gradient({
            let mut g = GradientFill {
                kind: GradientKind::Linear,
                degree: 90.0,
                ..GradientFill::default()
            };
            g.stops.push(GradientStop {
                position: 0.0,
                color: Color::Auto,
            });
            g.stops.push(GradientStop {
                position: 1.0,
                color: Color::Theme {
                    theme: 1,
                    tint: -0.25,
                },
            });
            g
        }),
        border: Border {
            left: BorderSide {
                style: BorderStyle::Thin,
                color: Color::Indexed { index: 8 },
            },
            ..Border::default()
        },
        alignment: Alignment {
            horizontal: HorizontalAlign::Center,
            vertical: VerticalAlign::Center,
            wrap: true,
            ..Alignment::default()
        },
        protection: Protection::default(),
        num_fmt: NumFmtId::GENERAL,
    }
}

#[test]
fn serde_roundtrip_every_public_type() {
    json_roundtrip(&SheetId::new(3));
    json_roundtrip(&SheetSpec {
        start: "Data".into(),
        end: Some("Notes".into()),
    });
    json_roundtrip(&CellRef::with_abs(1, 2, true, false).unwrap());
    json_roundtrip(&RangeRef::from_corners(
        CellRef::new(0, 0).unwrap(),
        CellRef::new(4, 4).unwrap(),
    ));
    json_roundtrip(&parse_a1("Sheet1!A1:B2").unwrap());
    json_roundtrip(&RefKind::Cell(CellRef::new(0, 0).unwrap()));
    json_roundtrip(&Value::Empty);
    json_roundtrip(&sample_value());
    json_roundtrip(&Value::Number(0.0));
    json_roundtrip(&Value::Number(-1.25));
    json_roundtrip(&Value::Bool(true));
    json_roundtrip(&Value::Text(StrId::new(4)));
    json_roundtrip(&Value::Error(ErrorKind::Div0));
    json_roundtrip(&Value::Array(ArrayId::new(9)));
    json_roundtrip(&StrId::new(1));
    json_roundtrip(&ArrayId::new(2));
    json_roundtrip(&Array2D::new(2, 3).unwrap());
    json_roundtrip(&ErrorKind::Name);
    json_roundtrip(&CoreError::addr_ref("beyond XFD"));
    json_roundtrip(&LocaleId::EN_US);
    json_roundtrip(&LocaleSeparators::EN_US);
    json_roundtrip(&StyleId::DEFAULT);
    json_roundtrip(&NumFmtId::GENERAL);
    json_roundtrip(&Color::Auto);
    json_roundtrip(&Underline::DoubleAccounting);
    json_roundtrip(&Font::default());
    json_roundtrip(&PatternType::Gray125);
    json_roundtrip(&GradientKind::Path);
    json_roundtrip(&GradientStop {
        position: 0.5,
        color: Color::Auto,
    });
    json_roundtrip(&GradientFill::default());
    json_roundtrip(&Fill::Solid { fg: Color::Auto });
    json_roundtrip(&Fill::None);
    json_roundtrip(&BorderStyle::SlantDashDot);
    json_roundtrip(&BorderSide::default());
    json_roundtrip(&Border::default());
    json_roundtrip(&HorizontalAlign::CenterContinuous);
    json_roundtrip(&VerticalAlign::Distributed);
    json_roundtrip(&Alignment::default());
    json_roundtrip(&Protection::default());
    json_roundtrip(&sample_style());
    json_roundtrip(&CommandId::new("range.sort").unwrap());
    json_roundtrip(&CommandDescriptor {
        id: CommandId::new("cell.set").unwrap(),
        doc: "Set a cell".into(),
        arg_schema: dummy_schema(),
        mutating: true,
    });
    json_roundtrip(&Origin::ExternalAgent);
    json_roundtrip(&Outcome::success(serde_json::json!({"n": 1})));
    json_roundtrip(&Outcome::failure(CoreError::addr_parse("bad")));
    json_roundtrip(&UndoUnit);
    json_roundtrip(&UndoUnitId::new(8));
    json_roundtrip(&ChangesetId::new("cs-1").unwrap());
    json_roundtrip(&ChangesetStatus::Reverted);
    json_roundtrip(&CommandCall {
        id: CommandId::new("sheet.add").unwrap(),
        args: serde_json::json!({"name": "Data"}),
    });
    json_roundtrip(&ChangeSummary::default());
    json_roundtrip(&sample_changeset());
    json_roundtrip(&Event::CellChanged {
        sheet: SheetId::new(0),
        row: 0,
        col: 0,
    });
    json_roundtrip(&Event::ThemeChanged {
        name: "tokyo-night".into(),
    });
    json_roundtrip(&Event::RecalcDone {
        cells: 12,
        elapsed_ms: 3,
    });
    json_roundtrip(&Event::WorkbookOpened { path: None });
    json_roundtrip(&Event::BeforeSave {
        path: "a.omc".into(),
    });
    json_roundtrip(&Event::FileSaved {
        path: "a.omc".into(),
    });
    json_roundtrip(&Event::ChangesetProposed {
        id: ChangesetId::new("cs-1").unwrap(),
    });
    json_roundtrip(&Event::ChangesetApplied {
        id: ChangesetId::new("cs-1").unwrap(),
    });
    json_roundtrip(&Event::ChangesetReverted {
        id: ChangesetId::new("cs-1").unwrap(),
    });
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    })]

    #[test]
    fn serde_value_integer_number(n in any::<i32>()) {
        // JSON numbers are not bit-exact for every f64; integer values are.
        json_roundtrip(&Value::Number(f64::from(n)));
    }

    #[test]
    fn serde_cell_ref(cell in arb_cell_ref()) {
        json_roundtrip(&cell);
    }

    #[test]
    fn serde_error_kind(idx in 0usize..ErrorKind::all().len()) {
        json_roundtrip(&ErrorKind::all()[idx]);
    }
}

#[test]
fn parsed_ref_is_used() {
    let _ = ParsedRef {
        sheet: None,
        kind: RefKind::Cell(CellRef::new(0, 0).unwrap()),
    };
}

#[test]
fn xfd_letters() {
    assert_eq!(col_to_letters(MAX_COLS - 1).unwrap(), "XFD");
    assert_eq!(col_from_letters("XFD").unwrap(), MAX_COLS - 1);
}
