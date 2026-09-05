//! Sort, AutoFilter, DV, and CF corpora (WP-18).

use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::coerce::Scalar;
use omacell_core::condfmt::{
    CfDxf, CfKind, CfOp, CfOverlay, CfTimePeriod, CfVisual, CondFormat, OverlaySource, overlay_at,
    overlay_at_with_registry, resolve_overlay,
};
use omacell_core::eval::{ArgVal, ArrayLift, EvalCtx, FnDef, FnRegistry, RuntimeValue};
use omacell_core::filter::{
    AutoFilter, FilterColumn, FilterCriteria, NumOp, apply_filter, clear_filter, restore_filter,
};
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::sheet::{Comment, Hyperlink, Note};
use omacell_core::sort::{SortBy, SortKey, SortSpec, sort_range};
use omacell_core::style::Color;
use omacell_core::validation::{
    DataValidation, DvOp, DvType, invalid_cells, validate_cell, validate_cell_with_registry,
    validation_list_values,
};
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;

fn range(r0: u32, c0: u16, r1: u32, c1: u16) -> RangeRef {
    RangeRef::from_corners(CellRef::new(r0, c0).unwrap(), CellRef::new(r1, c1).unwrap())
}

fn num(wb: &Workbook, row: u32, col: u16) -> f64 {
    match wb.get(wb.active_sheet(), row, col).unwrap().unwrap().value {
        Value::Number(n) => n,
        _ => panic!("expected number"),
    }
}

fn cell_display(wb: &Workbook, row: u32, col: u16) -> String {
    let Some(slot) = wb.get(wb.active_sheet(), row, col).unwrap() else {
        return "_".into();
    };
    match slot.value {
        Value::Empty => "_".into(),
        Value::Number(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Text(id) => wb.intern().strings.get(id).unwrap_or_default().into(),
        Value::Error(error) => error.as_str().into(),
        Value::Array(_) => "array".into(),
    }
}

fn set_corpus_value(wb: &mut Workbook, row: u32, token: &str) {
    let sheet = wb.active_sheet();
    if token == "_" {
        return;
    }
    if let Ok(number) = token.parse::<f64>() {
        wb.set_number(sheet, row, 0, number).unwrap();
    } else {
        wb.set_text(sheet, row, 0, token).unwrap();
    }
}

fn allow_fn(_ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    RuntimeValue::Scalar(Scalar::Bool(true))
}

#[test]
fn sort_corpus() {
    let corpus = include_str!("../../../tests/corpus/data-tools/sort.tsv");
    for line in corpus.lines().skip(1).filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 6, "bad sort corpus row: {line}");
        let mut wb = Workbook::new();
        let values: Vec<_> = fields[1].split(',').collect();
        for (row, value) in values.iter().enumerate() {
            set_corpus_value(&mut wb, u32::try_from(row).unwrap(), value);
        }
        let custom_list = if fields[3] == "-" {
            Vec::new()
        } else {
            fields[3].split(',').map(str::to_string).collect()
        };
        let sheet = wb.active_sheet();
        sort_range(
            &mut wb,
            sheet,
            range(0, 0, u32::try_from(values.len() - 1).unwrap(), 0),
            &SortSpec {
                keys: vec![SortKey {
                    offset: 0,
                    descending: fields[2] == "true",
                    by: SortBy::Value,
                    custom_list,
                }],
                ..SortSpec::default()
            },
        )
        .unwrap();
        let actual = (0..values.len())
            .map(|row| cell_display(&wb, u32::try_from(row).unwrap(), 0))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(actual, fields[4], "{} ({})", fields[0], fields[5]);
    }
}

#[test]
fn filter_corpus() {
    let corpus = include_str!("../../../tests/corpus/data-tools/filter.tsv");
    for line in corpus.lines().skip(1).filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 6, "bad filter corpus row: {line}");
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        wb.set_text(sheet, 0, 0, "n").unwrap();
        let values: Vec<f64> = fields[1]
            .split(',')
            .map(|value| value.parse().unwrap())
            .collect();
        for (index, value) in values.iter().enumerate() {
            wb.set_number(sheet, u32::try_from(index + 1).unwrap(), 0, *value)
                .unwrap();
        }
        let argument = fields[3].parse::<f64>().unwrap_or(0.0);
        let criteria = match fields[2] {
            "greater" => FilterCriteria::Number {
                op: NumOp::Greater,
                value: argument,
                value2: None,
            },
            "not_equal" => FilterCriteria::Number {
                op: NumOp::NotEqual,
                value: argument,
                value2: None,
            },
            "top_percent" => FilterCriteria::TopN {
                n: argument as u32,
                percent: true,
                bottom: false,
            },
            "below_average" => FilterCriteria::Average { below: true },
            other => panic!("unknown filter corpus criterion {other}"),
        };
        apply_filter(
            &mut wb,
            sheet,
            &AutoFilter {
                range: range(0, 0, u32::try_from(values.len()).unwrap(), 0),
                columns: vec![FilterColumn {
                    col_id: 0,
                    criteria,
                }],
            },
        )
        .unwrap();
        let actual = values
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                !wb.sheet(sheet)
                    .unwrap()
                    .geometry
                    .rows
                    .is_hidden(u32::try_from(index + 1).unwrap())
                    .unwrap()
            })
            .map(|(_, value)| value.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(actual, fields[4], "{} ({})", fields[0], fields[5]);
    }
}

/// Excel type order: numbers, then text, then logicals, errors, blanks last.
#[test]
fn sort_mixed_types_numbers_before_text() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "b").unwrap();
    wb.set_number(s, 1, 0, 2.0).unwrap();
    wb.set_text(s, 2, 0, "a").unwrap();
    wb.set_number(s, 3, 0, 1.0).unwrap();
    sort_range(
        &mut wb,
        s,
        range(0, 0, 3, 0),
        &SortSpec {
            keys: vec![SortKey {
                offset: 0,
                descending: false,
                by: SortBy::Value,
                custom_list: Vec::new(),
            }],
            ..SortSpec::default()
        },
    )
    .unwrap();
    assert_eq!(num(&wb, 0, 0), 1.0);
    assert_eq!(num(&wb, 1, 0), 2.0);
}

#[test]
fn sort_is_stable_on_ties() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_text(s, 0, 1, "x").unwrap();
    wb.set_number(s, 1, 0, 1.0).unwrap();
    wb.set_text(s, 1, 1, "y").unwrap();
    sort_range(
        &mut wb,
        s,
        range(0, 0, 1, 1),
        &SortSpec {
            keys: vec![SortKey {
                offset: 0,
                descending: false,
                by: SortBy::Value,
                custom_list: Vec::new(),
            }],
            ..SortSpec::default()
        },
    )
    .unwrap();
    match wb.get(s, 0, 1).unwrap().unwrap().value {
        Value::Text(id) => assert_eq!(wb.intern().strings.get(id), Some("x")),
        _ => panic!("stable order lost"),
    }
}

#[test]
fn sort_retains_unique_text_when_undo_is_disabled() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_text(s, 0, 0, "bravo").unwrap();
    wb.set_text(s, 1, 0, "alpha").unwrap();

    sort_range(
        &mut wb,
        s,
        range(0, 0, 1, 0),
        &SortSpec {
            keys: vec![SortKey {
                offset: 0,
                descending: false,
                by: SortBy::Value,
                custom_list: Vec::new(),
            }],
            ..SortSpec::default()
        },
    )
    .unwrap();

    assert_eq!(cell_display(&wb, 0, 0), "alpha");
    assert_eq!(cell_display(&wb, 1, 0), "bravo");
}

#[test]
fn sort_moves_annotations_and_hyperlinks_with_their_record() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let note = Note {
        author: Some("Ada".into()),
        text: "record two".into(),
    };
    let comment = Comment {
        author: "Ada".into(),
        text: "review two".into(),
        replies: Vec::new(),
        resolved: false,
    };
    let hyperlink = Hyperlink {
        target: "https://example.com/two".into(),
        tooltip: None,
        display: None,
    };
    wb.set_number(sheet, 0, 0, 2.0).unwrap();
    wb.set_number(sheet, 1, 0, 1.0).unwrap();
    wb.set_note(sheet, 0, 0, Some(note.clone())).unwrap();
    wb.set_comment(sheet, 0, 0, Some(comment.clone())).unwrap();
    wb.set_hyperlink(sheet, 0, 0, Some(hyperlink.clone()))
        .unwrap();

    sort_range(
        &mut wb,
        sheet,
        range(0, 0, 1, 0),
        &SortSpec {
            keys: vec![SortKey {
                offset: 0,
                descending: false,
                by: SortBy::Value,
                custom_list: Vec::new(),
            }],
            ..SortSpec::default()
        },
    )
    .unwrap();

    let sorted = wb.sheet(sheet).unwrap();
    assert_eq!(sorted.notes.get(&(1, 0)), Some(&note));
    assert_eq!(sorted.comments.get(&(1, 0)), Some(&comment));
    assert_eq!(sorted.hyperlinks.get(&(1, 0)), Some(&hyperlink));
    assert!(!sorted.hyperlinks.contains_key(&(0, 0)));

    wb.undo().unwrap();
    let restored = wb.sheet(sheet).unwrap();
    assert_eq!(restored.notes.get(&(0, 0)), Some(&note));
    assert_eq!(restored.comments.get(&(0, 0)), Some(&comment));
    assert_eq!(restored.hyperlinks.get(&(0, 0)), Some(&hyperlink));
}

#[test]
fn sort_skips_hidden_rows() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 3.0).unwrap();
    wb.set_number(s, 1, 0, 1.0).unwrap();
    wb.set_number(s, 2, 0, 2.0).unwrap();
    wb.set_row_hidden(s, 1, true).unwrap();
    sort_range(
        &mut wb,
        s,
        range(0, 0, 2, 0),
        &SortSpec {
            keys: vec![SortKey {
                offset: 0,
                descending: false,
                by: SortBy::Value,
                custom_list: Vec::new(),
            }],
            ..SortSpec::default()
        },
    )
    .unwrap();
    assert_eq!(num(&wb, 1, 0), 1.0);
    assert!(wb.sheet(s).unwrap().geometry.rows.is_hidden(1).unwrap());
}

#[test]
fn sort_custom_list() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "medium").unwrap();
    wb.set_text(s, 1, 0, "low").unwrap();
    wb.set_text(s, 2, 0, "high").unwrap();
    sort_range(
        &mut wb,
        s,
        range(0, 0, 2, 0),
        &SortSpec {
            keys: vec![SortKey {
                offset: 0,
                descending: false,
                by: SortBy::Value,
                custom_list: vec!["low".into(), "medium".into(), "high".into()],
            }],
            ..SortSpec::default()
        },
    )
    .unwrap();
    match wb.get(s, 0, 0).unwrap().unwrap().value {
        Value::Text(id) => assert_eq!(wb.intern().strings.get(id), Some("low")),
        _ => panic!(),
    }
}

#[test]
fn sort_by_fill_color_and_left_to_right() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_text(sheet, 0, 0, "red").unwrap();
    wb.set_text(sheet, 0, 1, "blue").unwrap();
    wb.set_cell_style(
        sheet,
        0,
        0,
        omacell_core::style::Style {
            fill: omacell_core::style::Fill::Solid {
                fg: Color::Rgb { argb: 0xFFFF_0000 },
            },
            ..omacell_core::style::Style::default()
        },
    )
    .unwrap();
    wb.set_cell_style(
        sheet,
        0,
        1,
        omacell_core::style::Style {
            fill: omacell_core::style::Fill::Solid {
                fg: Color::Rgb { argb: 0xFF00_00FF },
            },
            ..omacell_core::style::Style::default()
        },
    )
    .unwrap();
    sort_range(
        &mut wb,
        sheet,
        range(0, 0, 0, 1),
        &SortSpec {
            keys: vec![SortKey {
                offset: 0,
                descending: false,
                by: SortBy::FillColor,
                custom_list: Vec::new(),
            }],
            left_to_right: true,
            ..SortSpec::default()
        },
    )
    .unwrap();
    assert_eq!(cell_display(&wb, 0, 0), "blue");
    assert_eq!(cell_display(&wb, 0, 1), "red");
}

#[test]
fn sort_descending_keeps_blanks_last() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_number(s, 2, 0, 2.0).unwrap();
    sort_range(
        &mut wb,
        s,
        range(0, 0, 2, 0),
        &SortSpec {
            keys: vec![SortKey {
                offset: 0,
                descending: true,
                by: SortBy::Value,
                custom_list: Vec::new(),
            }],
            ..SortSpec::default()
        },
    )
    .unwrap();
    assert_eq!(num(&wb, 0, 0), 2.0);
    assert_eq!(num(&wb, 1, 0), 1.0);
    assert!(wb.get(s, 2, 0).unwrap().is_none());
}

#[test]
fn sort_header_stays() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Name").unwrap();
    wb.set_text(s, 1, 0, "b").unwrap();
    wb.set_text(s, 2, 0, "a").unwrap();
    sort_range(
        &mut wb,
        s,
        range(0, 0, 2, 0),
        &SortSpec {
            keys: vec![SortKey {
                offset: 0,
                descending: false,
                by: SortBy::Value,
                custom_list: Vec::new(),
            }],
            header: true,
            ..SortSpec::default()
        },
    )
    .unwrap();
    match wb.get(s, 0, 0).unwrap().unwrap().value {
        Value::Text(id) => assert_eq!(wb.intern().strings.get(id), Some("Name")),
        _ => panic!(),
    }
}

#[test]
fn filter_greater_than_hides_rows() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "n").unwrap();
    wb.set_number(s, 1, 0, 1.0).unwrap();
    wb.set_number(s, 2, 0, 10.0).unwrap();
    apply_filter(
        &mut wb,
        s,
        &AutoFilter {
            range: range(0, 0, 2, 0),
            columns: vec![FilterColumn {
                col_id: 0,
                criteria: FilterCriteria::Number {
                    op: NumOp::Greater,
                    value: 5.0,
                    value2: None,
                },
            }],
        },
    )
    .unwrap();
    assert!(wb.sheet(s).unwrap().geometry.rows.is_hidden(1).unwrap());
    assert!(!wb.sheet(s).unwrap().geometry.rows.is_hidden(2).unwrap());
    clear_filter(&mut wb, s).unwrap();
    assert!(!wb.sheet(s).unwrap().geometry.rows.is_hidden(1).unwrap());
}

#[test]
fn clearing_filter_preserves_rows_hidden_before_filtering() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "n").unwrap();
    wb.set_number(s, 1, 0, 1.0).unwrap();
    wb.set_number(s, 2, 0, 10.0).unwrap();
    wb.set_row_hidden(s, 2, true).unwrap();
    apply_filter(
        &mut wb,
        s,
        &AutoFilter {
            range: range(0, 0, 2, 0),
            columns: vec![FilterColumn {
                col_id: 0,
                criteria: FilterCriteria::Number {
                    op: NumOp::Greater,
                    value: 5.0,
                    value2: None,
                },
            }],
        },
    )
    .unwrap();
    clear_filter(&mut wb, s).unwrap();
    assert!(!wb.sheet(s).unwrap().geometry.rows.is_hidden(1).unwrap());
    assert!(wb.sheet(s).unwrap().geometry.rows.is_hidden(2).unwrap());
}

#[test]
fn imported_filter_tracks_failing_rows_but_preserves_manual_passing_rows() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "n").unwrap();
    wb.set_number(s, 1, 0, 1.0).unwrap();
    wb.set_number(s, 2, 0, 10.0).unwrap();
    wb.set_row_hidden(s, 1, true).unwrap();
    wb.set_row_hidden(s, 2, true).unwrap();
    restore_filter(
        &mut wb,
        s,
        &AutoFilter {
            range: range(0, 0, 2, 0),
            columns: vec![FilterColumn {
                col_id: 0,
                criteria: FilterCriteria::Number {
                    op: NumOp::Greater,
                    value: 5.0,
                    value2: None,
                },
            }],
        },
    )
    .unwrap();
    clear_filter(&mut wb, s).unwrap();
    assert!(!wb.sheet(s).unwrap().geometry.rows.is_hidden(1).unwrap());
    assert!(wb.sheet(s).unwrap().geometry.rows.is_hidden(2).unwrap());
}

#[test]
fn validation_whole_between() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_validations(
        s,
        vec![DataValidation {
            range: range(0, 0, 5, 0),
            kind: DvType::Whole,
            op: DvOp::Between,
            formula1: Some("1".into()),
            formula2: Some("10".into()),
            ..DataValidation::default()
        }],
    )
    .unwrap();
    wb.set_number(s, 0, 0, 5.0).unwrap();
    assert!(validate_cell(&wb, s, 0, 0).is_ok());
    wb.set_number(s, 1, 0, 99.0).unwrap();
    assert!(validate_cell(&wb, s, 1, 0).is_err());
    assert_eq!(invalid_cells(&wb, s), vec![(1, 0)]);
}

#[test]
fn validation_text_length_counts_unicode_scalars_and_custom_formula_uses_cell_origin() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_validations(
        s,
        vec![DataValidation {
            range: range(0, 0, 0, 0),
            kind: DvType::TextLength,
            op: DvOp::Equal,
            formula1: Some("1".into()),
            ..DataValidation::default()
        }],
    )
    .unwrap();
    wb.set_text(s, 0, 0, "é").unwrap();
    assert!(validate_cell(&wb, s, 0, 0).is_ok());

    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_number(s, 1, 0, -1.0).unwrap();
    wb.set_number(s, 0, 1, 9.0).unwrap();
    wb.set_number(s, 1, 1, 9.0).unwrap();
    wb.set_validations(
        s,
        vec![DataValidation {
            range: range(0, 1, 1, 1),
            kind: DvType::Custom,
            formula1: Some("=A1>0".into()),
            ..DataValidation::default()
        }],
    )
    .unwrap();
    assert!(validate_cell(&wb, s, 0, 1).is_ok());
    assert!(validate_cell(&wb, s, 1, 1).is_err());
}

#[test]
fn formula_rules_use_the_application_function_registry() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_number(sheet, 0, 0, 1.0).unwrap();
    wb.set_validations(
        sheet,
        vec![DataValidation {
            range: range(0, 0, 0, 0),
            kind: DvType::Custom,
            formula1: Some("=ALLOW()".into()),
            ..DataValidation::default()
        }],
    )
    .unwrap();
    wb.set_cond_formats(
        sheet,
        vec![CondFormat {
            range: range(0, 0, 0, 0),
            priority: 1,
            stop_if_true: false,
            kind: CfKind::Formula("=ALLOW()".into()),
            dxf: CfDxf {
                fill: Some(Color::Rgb { argb: 0xFFFF_0000 }),
                font: None,
            },
        }],
    )
    .unwrap();

    let mut registry = FnRegistry::new();
    registry.register(FnDef::eager(
        "ALLOW",
        0,
        0,
        false,
        false,
        ArrayLift::None,
        allow_fn,
    ));
    assert!(validate_cell(&wb, sheet, 0, 0).is_err());
    assert!(validate_cell_with_registry(&wb, sheet, 0, 0, &registry).is_ok());
    assert_eq!(overlay_at(&wb, sheet, 0, 0).fill, None);
    assert_eq!(
        overlay_at_with_registry(&wb, sheet, 0, 0, &registry).fill,
        Some(Color::Rgb { argb: 0xFFFF_0000 })
    );
}

#[test]
fn validation_corpus() {
    let corpus = include_str!("../../../tests/corpus/data-tools/validation.tsv");
    for line in corpus.lines().skip(1).filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 8, "bad validation corpus row: {line}");
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        let kind = match fields[1] {
            "whole" => DvType::Whole,
            "text_length" => DvType::TextLength,
            "time" => DvType::Time,
            "date" => DvType::Date,
            other => panic!("unknown validation corpus kind {other}"),
        };
        let op = match fields[2] {
            "between" => DvOp::Between,
            "equal" => DvOp::Equal,
            "less_eq" => DvOp::LessEq,
            "greater_eq" => DvOp::GreaterEq,
            other => panic!("unknown validation corpus operator {other}"),
        };
        wb.set_validations(
            sheet,
            vec![DataValidation {
                range: range(0, 0, 0, 0),
                kind,
                op,
                formula1: (fields[3] != "-").then(|| fields[3].to_string()),
                formula2: (fields[4] != "-").then(|| fields[4].to_string()),
                ..DataValidation::default()
            }],
        )
        .unwrap();
        if let Ok(number) = fields[5].parse::<f64>() {
            wb.set_number(sheet, 0, 0, number).unwrap();
        } else {
            wb.set_text(sheet, 0, 0, fields[5]).unwrap();
        }
        assert_eq!(
            validate_cell(&wb, sheet, 0, 0).is_ok(),
            fields[6] == "true",
            "{} ({})",
            fields[0],
            fields[7]
        );
    }
}

#[test]
fn cf_stop_if_true_beats_later_rule() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 10.0).unwrap();
    let red = Color::Rgb { argb: 0xFFFF_0000 };
    let blue = Color::Rgb { argb: 0xFF00_00FF };
    wb.set_cond_formats(
        s,
        vec![
            CondFormat {
                range: range(0, 0, 0, 0),
                priority: 1,
                stop_if_true: true,
                kind: CfKind::CellIs {
                    op: CfOp::Greater,
                    formula1: "5".into(),
                    formula2: None,
                },
                dxf: CfDxf {
                    fill: Some(red),
                    font: None,
                },
            },
            CondFormat {
                range: range(0, 0, 0, 0),
                priority: 2,
                stop_if_true: false,
                kind: CfKind::CellIs {
                    op: CfOp::Greater,
                    formula1: "1".into(),
                    formula2: None,
                },
                dxf: CfDxf {
                    fill: Some(blue),
                    font: None,
                },
            },
        ],
    )
    .unwrap();
    let overlay = overlay_at(&wb, s, 0, 0);
    assert_eq!(overlay.fill, Some(red));
    assert_eq!(
        overlay.source,
        OverlaySource::Rule {
            priority: 1,
            stop: true
        }
    );
    let _ = CfOverlay {
        fill: None,
        font: None,
        visual: None,
        source: OverlaySource::File,
    };
}

#[test]
fn cf_higher_priority_fill_wins_without_stop_if_true() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 10.0).unwrap();
    let red = Color::Rgb { argb: 0xFFFF_0000 };
    let blue = Color::Rgb { argb: 0xFF00_00FF };
    wb.set_cond_formats(
        s,
        vec![
            CondFormat {
                range: range(0, 0, 0, 0),
                priority: 1,
                stop_if_true: false,
                kind: CfKind::CellIs {
                    op: CfOp::Greater,
                    formula1: "5".into(),
                    formula2: None,
                },
                dxf: CfDxf {
                    fill: Some(red),
                    font: None,
                },
            },
            CondFormat {
                range: range(0, 0, 0, 0),
                priority: 2,
                stop_if_true: false,
                kind: CfKind::CellIs {
                    op: CfOp::Greater,
                    formula1: "1".into(),
                    formula2: None,
                },
                dxf: CfDxf {
                    fill: Some(blue),
                    font: None,
                },
            },
        ],
    )
    .unwrap();
    let overlay = overlay_at(&wb, s, 0, 0);
    assert_eq!(overlay.fill, Some(red));
    assert_eq!(
        overlay.source,
        OverlaySource::Rule {
            priority: 1,
            stop: false,
        }
    );
}

#[test]
fn conditional_format_corpus() {
    let corpus = include_str!("../../../tests/corpus/data-tools/condfmt.tsv");
    for line in corpus.lines().skip(1).filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 5, "bad CF corpus row: {line}");
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        let values: Vec<f64> = fields[2]
            .split(',')
            .map(|value| value.parse().unwrap())
            .collect();
        for (row, value) in values.iter().enumerate() {
            wb.set_number(sheet, u32::try_from(row).unwrap(), 0, *value)
                .unwrap();
        }
        let kind = match fields[1] {
            "duplicate" => CfKind::Duplicate,
            "unique" => CfKind::Unique,
            "icon_set" => CfKind::IconSet { icons: 3 },
            "data_bar" => CfKind::DataBar {
                color: Color::Rgb { argb: 0xFF00_66CC },
                gradient: false,
            },
            other => panic!("unknown CF corpus kind {other}"),
        };
        wb.set_cond_formats(
            sheet,
            vec![CondFormat {
                range: range(0, 0, u32::try_from(values.len() - 1).unwrap(), 0),
                priority: 1,
                stop_if_true: false,
                kind,
                dxf: CfDxf {
                    fill: Some(Color::Rgb { argb: 0xFFFF_0000 }),
                    font: None,
                },
            }],
        )
        .unwrap();
        let actual = resolve_overlay(
            &wb,
            sheet,
            range(0, 0, u32::try_from(values.len() - 1).unwrap(), 0),
        )
        .unwrap();
        let actual = (0..values.len())
            .map(|row| {
                let overlay = actual
                    .get(u32::try_from(row).unwrap(), 0)
                    .expect("corpus coordinate");
                match overlay.visual {
                    Some(CfVisual::Icon { index, .. }) => format!("icon:{index}"),
                    Some(CfVisual::DataBar { fraction, .. }) => format!("bar:{fraction}"),
                    None if matches!(overlay.source, OverlaySource::Rule { .. }) => "match".into(),
                    None => "none".into(),
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(actual, fields[3], "{} ({})", fields[0], fields[4]);
    }
}

#[test]
fn duplicate_conditional_format_matches_text_case_insensitively() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Alpha").unwrap();
    wb.set_text(s, 1, 0, "alpha").unwrap();
    wb.set_cond_formats(
        s,
        vec![CondFormat {
            range: range(0, 0, 1, 0),
            priority: 1,
            stop_if_true: false,
            kind: CfKind::Duplicate,
            dxf: CfDxf {
                fill: Some(Color::Rgb { argb: 0xFFFF_0000 }),
                font: None,
            },
        }],
    )
    .unwrap();

    for row in 0..=1 {
        assert!(matches!(
            overlay_at(&wb, s, row, 0).source,
            OverlaySource::Rule { priority: 1, .. }
        ));
    }
}

#[test]
fn conditional_format_defaults_match_their_emitted_ooxml_thresholds() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    for (row, value) in [0.0, 1.0, 100.0].into_iter().enumerate() {
        wb.set_number(s, u32::try_from(row).unwrap(), 0, value)
            .unwrap();
    }
    let middle = Color::Rgb { argb: 0xFF80_8080 };
    wb.set_cond_formats(
        s,
        vec![
            CondFormat {
                range: range(0, 0, 2, 0),
                priority: 1,
                stop_if_true: false,
                kind: CfKind::ColorScale {
                    colors: vec![
                        Color::Rgb { argb: 0xFF00_0000 },
                        middle,
                        Color::Rgb { argb: 0xFFFF_FFFF },
                    ],
                },
                dxf: CfDxf::default(),
            },
            CondFormat {
                range: range(0, 0, 2, 0),
                priority: 2,
                stop_if_true: false,
                kind: CfKind::IconSet { icons: 3 },
                dxf: CfDxf::default(),
            },
        ],
    )
    .unwrap();

    let overlay = overlay_at(&wb, s, 1, 0);
    assert_eq!(overlay.fill, Some(middle));
    assert!(matches!(
        overlay.visual,
        Some(CfVisual::Icon { icons: 3, index: 0 })
    ));
}

#[test]
fn cf_visuals_are_resolved_and_icon_sort_uses_the_cached_bucket() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    for (row, value) in [100.0, 1.0, 50.0].into_iter().enumerate() {
        wb.set_number(s, u32::try_from(row).unwrap(), 0, value)
            .unwrap();
    }
    wb.set_cond_formats(
        s,
        vec![
            CondFormat {
                range: range(0, 0, 2, 0),
                priority: 1,
                stop_if_true: false,
                kind: CfKind::IconSet { icons: 3 },
                dxf: CfDxf::default(),
            },
            CondFormat {
                range: range(0, 1, 0, 1),
                priority: 2,
                stop_if_true: false,
                kind: CfKind::DataBar {
                    color: Color::Rgb { argb: 0xFF00_66CC },
                    gradient: false,
                },
                dxf: CfDxf::default(),
            },
        ],
    )
    .unwrap();
    wb.set_number(s, 0, 1, 5.0).unwrap();

    let resolved = resolve_overlay(&wb, s, range(0, 0, 2, 1)).unwrap();
    assert!(matches!(
        resolved.get(0, 0).unwrap().visual,
        Some(CfVisual::Icon { icons: 3, index: 2 })
    ));
    assert!(matches!(
        resolved.get(0, 1).unwrap().visual,
        Some(CfVisual::DataBar {
            gradient: false,
            fraction: 1.0,
            ..
        })
    ));

    sort_range(
        &mut wb,
        s,
        range(0, 0, 2, 0),
        &SortSpec {
            keys: vec![SortKey {
                offset: 0,
                descending: false,
                by: SortBy::Icon,
                custom_list: Vec::new(),
            }],
            ..SortSpec::default()
        },
    )
    .unwrap();
    assert_eq!(
        [num(&wb, 0, 0), num(&wb, 1, 0), num(&wb, 2, 0)],
        [1.0, 50.0, 100.0]
    );
}

#[test]
fn cf_time_period_matches_today() {
    const UNIX_EPOCH_SERIAL: f64 = 25_569.0;
    let today = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| UNIX_EPOCH_SERIAL + (duration.as_secs() / 86_400) as f64)
        .unwrap_or(0.0);
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_number(sheet, 0, 0, today).unwrap();
    wb.set_number(sheet, 1, 0, today - 8.0).unwrap();
    wb.set_cond_formats(
        sheet,
        vec![CondFormat {
            range: range(0, 0, 1, 0),
            priority: 1,
            stop_if_true: false,
            kind: CfKind::TimePeriod(CfTimePeriod::Today),
            dxf: CfDxf {
                fill: Some(Color::Rgb { argb: 0xFFFF_0000 }),
                font: None,
            },
        }],
    )
    .unwrap();
    assert!(matches!(
        overlay_at(&wb, sheet, 0, 0).source,
        OverlaySource::Rule { .. }
    ));
    assert_eq!(overlay_at(&wb, sheet, 1, 0).source, OverlaySource::File);
}

#[test]
fn table_create_and_auto_expand() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Item").unwrap();
    wb.set_text(s, 1, 0, "a").unwrap();
    let id = wb.create_table(s, range(0, 0, 1, 0), "Sales").unwrap();
    wb.set_text(s, 2, 0, "b").unwrap();
    let t = wb.tables().get(id).unwrap();
    assert_eq!(t.end_row, 2);
    assert_eq!(t.columns[0].name, "Item");
}

#[test]
fn table_rename_updates_structured_references_and_totals_are_modeled() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Item").unwrap();
    wb.set_text(s, 0, 1, "Amount").unwrap();
    wb.set_text(s, 1, 0, "a").unwrap();
    wb.set_number(s, 1, 1, 2.0).unwrap();
    let id = wb.create_table(s, range(0, 0, 1, 1), "Sales").unwrap();
    wb.set_formula_text(s, 0, 3, "=SUM(Sales[Amount])").unwrap();

    wb.rename_table(id, "Orders").unwrap();
    let formula = wb
        .get(s, 0, 3)
        .unwrap()
        .unwrap()
        .formula
        .and_then(|id| wb.intern().formulas.get(id))
        .unwrap();
    assert_eq!(formula, "=SUM(Orders[Amount])");

    wb.set_table_totals(id, true, vec![None, Some("sum".into())])
        .unwrap();
    let table = wb.tables().get(id).unwrap();
    assert!(table.has_totals);
    assert_eq!(table.end_row, 2);
    assert_eq!(table.columns[1].totals_fn.as_deref(), Some("sum"));
}

#[test]
fn cf_file_style_is_distinguished_from_rule() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    let file_fill = Color::Rgb { argb: 0xFF00_FF00 };
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_cell_style(
        s,
        0,
        0,
        omacell_core::style::Style {
            fill: omacell_core::style::Fill::Solid { fg: file_fill },
            ..omacell_core::style::Style::default()
        },
    )
    .unwrap();
    let overlay = overlay_at(&wb, s, 0, 0);
    assert_eq!(overlay.fill, Some(file_fill));
    assert_eq!(overlay.source, OverlaySource::File);
}

#[test]
fn sort_rewrites_relative_formula_by_row_delta() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 2.0).unwrap();
    wb.set_cell_contents(s, 0, 1, "=A1+1").unwrap();
    wb.set_number(s, 1, 0, 1.0).unwrap();
    wb.set_cell_contents(s, 1, 1, "=A2+1").unwrap();
    sort_range(
        &mut wb,
        s,
        range(0, 0, 1, 1),
        &SortSpec {
            keys: vec![SortKey {
                offset: 0,
                descending: false,
                by: SortBy::Value,
                custom_list: Vec::new(),
            }],
            ..SortSpec::default()
        },
    )
    .unwrap();
    assert_eq!(num(&wb, 0, 0), 1.0);
    let slot = wb.get(s, 0, 1).unwrap().unwrap();
    let src = wb.intern().formulas.get(slot.formula.unwrap()).unwrap();
    assert_eq!(src, "=A1+1");
}

#[test]
fn filter_top_n_and_average() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "n").unwrap();
    for (i, v) in [1.0, 2.0, 3.0, 10.0].iter().enumerate() {
        wb.set_number(s, u32::try_from(i).unwrap() + 1, 0, *v)
            .unwrap();
    }
    apply_filter(
        &mut wb,
        s,
        &AutoFilter {
            range: range(0, 0, 4, 0),
            columns: vec![FilterColumn {
                col_id: 0,
                criteria: FilterCriteria::TopN {
                    n: 1,
                    percent: false,
                    bottom: false,
                },
            }],
        },
    )
    .unwrap();
    assert!(wb.sheet(s).unwrap().geometry.rows.is_hidden(1).unwrap());
    assert!(!wb.sheet(s).unwrap().geometry.rows.is_hidden(4).unwrap());
}

#[test]
fn value_list_filter_matches_text_case_insensitively() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "name").unwrap();
    wb.set_text(s, 1, 0, "Alpha").unwrap();
    wb.set_text(s, 2, 0, "beta").unwrap();

    apply_filter(
        &mut wb,
        s,
        &AutoFilter {
            range: range(0, 0, 2, 0),
            columns: vec![FilterColumn {
                col_id: 0,
                criteria: FilterCriteria::Values(vec!["alpha".into()]),
            }],
        },
    )
    .unwrap();

    assert!(!wb.sheet(s).unwrap().geometry.rows.is_hidden(1).unwrap());
    assert!(wb.sheet(s).unwrap().geometry.rows.is_hidden(2).unwrap());
}

#[test]
fn validation_list_from_range_and_inline() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 1, "red").unwrap();
    wb.set_text(s, 1, 1, "blue").unwrap();
    wb.set_validations(
        s,
        vec![DataValidation {
            range: range(0, 0, 2, 0),
            kind: DvType::List,
            formula1: Some("B1:B2".into()),
            ..DataValidation::default()
        }],
    )
    .unwrap();
    wb.set_text(s, 0, 0, "red").unwrap();
    assert!(validate_cell(&wb, s, 0, 0).is_ok());
    wb.set_text(s, 1, 0, "green").unwrap();
    assert!(validate_cell(&wb, s, 1, 0).is_err());
    assert_eq!(
        validation_list_values(&wb, s, 0, 0).unwrap().unwrap(),
        ["red", "blue"]
    );
}

#[test]
fn validation_list_resolves_a_sheet_scoped_defined_name() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 1, "red").unwrap();
    wb.set_text(s, 1, 1, "blue").unwrap();
    wb.define_name(DefinedName {
        name: "Colors".into(),
        scope: NameScope::Sheet(s),
        referent: NameReferent::Range(RangeRef::from_corners(
            CellRef::new(0, 1).unwrap().on_sheet(s),
            CellRef::new(1, 1).unwrap().on_sheet(s),
        )),
        comment: None,
    })
    .unwrap();
    wb.set_validations(
        s,
        vec![DataValidation {
            range: range(0, 0, 1, 0),
            kind: DvType::List,
            formula1: Some("=Colors".into()),
            ..DataValidation::default()
        }],
    )
    .unwrap();
    wb.set_text(s, 0, 0, "red").unwrap();

    assert!(validate_cell(&wb, s, 0, 0).is_ok());
    assert_eq!(
        validation_list_values(&wb, s, 0, 0).unwrap().unwrap(),
        ["red", "blue"]
    );
}

#[test]
fn validation_list_rejects_defined_name_cycles() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    for (name, formula) in [("Colors", "=OtherColors"), ("OtherColors", "=Colors")] {
        wb.define_name(DefinedName {
            name: name.into(),
            scope: NameScope::Workbook,
            referent: NameReferent::Formula(formula.into()),
            comment: None,
        })
        .unwrap();
    }
    wb.set_validations(
        s,
        vec![DataValidation {
            range: range(0, 0, 0, 0),
            kind: DvType::List,
            formula1: Some("=Colors".into()),
            ..DataValidation::default()
        }],
    )
    .unwrap();

    let error = validation_list_values(&wb, s, 0, 0).unwrap_err();
    assert_eq!(error.code, "validation.list");
}

#[test]
fn flash_fill_first_word() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Ada Lovelace").unwrap();
    wb.set_text(s, 1, 0, "Grace Hopper").unwrap();
    wb.set_text(s, 2, 0, "Alan Turing").unwrap();
    wb.set_text(s, 0, 1, "Ada").unwrap();
    omacell_core::flashfill::flash_fill(&mut wb, s, range(0, 1, 2, 1)).unwrap();
    match wb.get(s, 1, 1).unwrap().unwrap().value {
        Value::Text(id) => assert_eq!(wb.intern().strings.get(id), Some("Grace")),
        _ => panic!("expected flash fill"),
    }
}

#[test]
#[ignore = "nightly wall-clock performance gate"]
fn cf_eval_100k_cells_20_rules_is_under_100ms() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    for i in 0..100_000u32 {
        let row = i / 20;
        let col = (i % 20) as u16;
        wb.set_number(s, row, col, f64::from(i)).unwrap();
    }
    let mut rules = Vec::new();
    for p in 1..=20 {
        rules.push(CondFormat {
            range: range(0, 0, 4999, 19),
            priority: p,
            stop_if_true: false,
            kind: CfKind::CellIs {
                op: CfOp::Greater,
                formula1: format!("{}", p * 1000),
                formula2: None,
            },
            dxf: CfDxf {
                fill: Some(Color::Rgb {
                    argb: 0xFF00_0000 | p,
                }),
                font: None,
            },
        });
    }
    wb.set_cond_formats(s, rules).unwrap();
    wb.set_number(s, 0, 0, 50_000.0).unwrap();
    let start = std::time::Instant::now();
    let overlays = resolve_overlay(&wb, s, range(0, 0, 4999, 19)).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(overlays.len(), 100_000);
    // The required PR lane runs debug correctness only. Nightly invokes this
    // ignored test in release mode for the product budget.
    if !cfg!(debug_assertions) {
        assert!(
            elapsed.as_millis() < 100,
            "CF overlay of 100k cells / 20 rules took {elapsed:?}"
        );
    }
}
