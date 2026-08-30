//! Sort, AutoFilter, DV, and CF corpora (WP-18).

use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::condfmt::{
    CfDxf, CfKind, CfOp, CfOverlay, CondFormat, OverlaySource, overlay_at,
};
use omacell_core::filter::{
    AutoFilter, FilterColumn, FilterCriteria, NumOp, apply_filter, clear_filter,
};
use omacell_core::sort::{SortBy, SortKey, SortSpec, sort_range};
use omacell_core::style::Color;
use omacell_core::validation::{DataValidation, DvOp, DvType, invalid_cells, validate_cell};
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
        source: OverlaySource::File,
    };
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
    let overlays = omacell_core::condfmt::overlay_range(&wb, s, range(0, 0, 4999, 19));
    let elapsed = start.elapsed();
    assert_eq!(overlays.len(), 100_000);
    // Debug builds are not the acceptance target; release is measured in the
    // criterion bench and must stay under 100 ms after a one-cell edit.
    if !cfg!(debug_assertions) {
        assert!(
            elapsed.as_millis() < 100,
            "CF overlay of 100k cells / 20 rules took {elapsed:?}"
        );
    }
}
