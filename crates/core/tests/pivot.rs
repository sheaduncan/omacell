//! WP-24 pivot, Goal Seek, and statistics corpora.

use std::path::PathBuf;

use omacell_core::addr::{CellRef, RangeRef, SheetId};
use omacell_core::dates::{CivilDate, date_to_serial};
use omacell_core::eval::FnRegistry;
use omacell_core::graph::CellCoord;
use omacell_core::ops::{Shift, delete_cells, delete_cols, delete_rows, insert_cells, insert_rows};
use omacell_core::pivot::{
    DateGroup, PivotAgg, PivotCalcField, PivotDataField, PivotGroup, PivotLayout, PivotTable,
    PivotValue, ShowAs, materialize,
};
use omacell_core::recalc::RecalcEngine;
use omacell_core::stats::describe_range;
use omacell_core::storage::CellSlot;
use omacell_core::style::{Font, Style};
use omacell_core::value::Value;
use omacell_core::whatif::{DEFAULT_MAX_ITER, DEFAULT_TOL, goal_seek};
use omacell_core::workbook::{DateSystem, Workbook};
use serde::Deserialize;

fn corpus(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(rel)
}

fn range(r0: u32, c0: u16, r1: u32, c1: u16) -> RangeRef {
    RangeRef::from_corners(CellRef::new(r0, c0).unwrap(), CellRef::new(r1, c1).unwrap())
}

#[derive(Deserialize)]
struct CaseFile {
    name: String,
    headers: Vec<String>,
    rows: Vec<Vec<serde_json::Value>>,
    #[serde(default)]
    rows_fields: Vec<String>,
    #[serde(default)]
    cols_fields: Vec<String>,
    #[serde(default)]
    data: Vec<DataSpec>,
    #[serde(default)]
    groups: std::collections::BTreeMap<String, GroupSpec>,
    #[serde(default)]
    layout: Option<String>,
    #[serde(default)]
    calc_fields: Vec<CalcFieldSpec>,
    expect: Vec<ExpectCell>,
}

#[derive(Deserialize)]
struct CalcFieldSpec {
    name: String,
    formula: String,
}

#[derive(Deserialize)]
struct DataSpec {
    source: String,
    #[serde(default)]
    agg: String,
    #[serde(default)]
    show_as: String,
}

#[derive(Deserialize)]
struct GroupSpec {
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    numeric: Option<NumericSpec>,
}

#[derive(Deserialize)]
struct NumericSpec {
    start: f64,
    size: f64,
}

#[derive(Deserialize)]
struct ExpectCell {
    row: u32,
    col: u16,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    n: Option<f64>,
}

fn parse_iso_date(text: &str) -> Option<f64> {
    let mut parts = text.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    date_to_serial(
        CivilDate {
            year,
            month,
            day,
            lotus_leap: false,
        },
        DateSystem::Excel1900,
    )
    .map(|n| n as f64)
}

fn load_source(case: &CaseFile) -> Workbook {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    for (c, header) in case.headers.iter().enumerate() {
        wb.set_text(sheet, 0, c as u16, header).unwrap();
    }
    for (r, row) in case.rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            match cell {
                serde_json::Value::Number(n) => {
                    wb.set_number(sheet, r as u32 + 1, c as u16, n.as_f64().unwrap())
                        .unwrap();
                }
                serde_json::Value::String(s) => {
                    if let Some(serial) = parse_iso_date(s) {
                        wb.set_number(sheet, r as u32 + 1, c as u16, serial)
                            .unwrap();
                    } else {
                        wb.set_text(sheet, r as u32 + 1, c as u16, s).unwrap();
                    }
                }
                _ => {}
            }
        }
    }
    wb
}

fn definition(case: &CaseFile, dest_row: u32, dest_col: u16) -> PivotTable {
    let sheet = SheetId::new(0);
    let last_row = u32::try_from(case.rows.len()).unwrap();
    let last_col = u16::try_from(case.headers.len().saturating_sub(1)).unwrap_or(0);
    let mut table = PivotTable::new(
        case.name.clone(),
        sheet,
        range(0, 0, last_row, last_col),
        sheet,
        dest_row,
        dest_col,
    );
    table.rows = case.rows_fields.clone();
    table.cols = case.cols_fields.clone();
    table.data = case
        .data
        .iter()
        .map(|d| PivotDataField {
            source: d.source.clone(),
            agg: PivotAgg::parse(if d.agg.is_empty() { "sum" } else { &d.agg }).unwrap(),
            show_as: if d.show_as.is_empty() {
                ShowAs::Normal
            } else {
                ShowAs::parse(&d.show_as).unwrap()
            },
        })
        .collect();
    table.groups = case
        .groups
        .iter()
        .map(|(name, spec)| {
            let group = if let Some(grain) = &spec.date {
                PivotGroup::Date(DateGroup::parse(grain).unwrap())
            } else if let Some(num) = &spec.numeric {
                PivotGroup::Numeric {
                    start: num.start,
                    size: num.size,
                }
            } else {
                PivotGroup::None
            };
            (name.clone(), group)
        })
        .collect();
    if let Some(layout) = &case.layout {
        table.layout = PivotLayout::parse(layout).unwrap();
    }
    table.calc_fields = case
        .calc_fields
        .iter()
        .map(|field| PivotCalcField {
            name: field.name.clone(),
            formula: field.formula.clone(),
        })
        .collect();
    table
}

fn cell_text(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> String {
    match wb.get(sheet, row, col).ok().flatten().map(|s| s.value) {
        Some(Value::Text(id)) => wb.intern().strings.get(id).unwrap_or("").to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn cell_num(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> Option<f64> {
    match wb.get(sheet, row, col).ok().flatten().map(|s| s.value) {
        Some(Value::Number(n)) => Some(n),
        _ => None,
    }
}

fn rendered_text(cells: &[omacell_core::pivot::PivotCell], row: u32, col: u16) -> Option<&str> {
    cells
        .iter()
        .find(|cell| cell.row == row && cell.col == col)
        .and_then(|cell| match &cell.value {
            PivotValue::Text(text) => Some(text.as_str()),
            PivotValue::Number(_) | PivotValue::Empty => None,
        })
}

fn assert_one_rendered_cell(cells: &[omacell_core::pivot::PivotCell], row: u32, col: u16) {
    assert_eq!(
        cells
            .iter()
            .filter(|cell| cell.row == row && cell.col == col)
            .count(),
        1,
        "rendered coordinate ({row}, {col}) must not contain colliding headers"
    );
}

fn assert_expect(wb: &Workbook, dest_row: u32, dest_col: u16, expect: &[ExpectCell], name: &str) {
    let sheet = wb.active_sheet();
    for cell in expect {
        let row = dest_row + cell.row;
        let col = dest_col + cell.col;
        if let Some(text) = &cell.text {
            assert_eq!(
                cell_text(wb, sheet, row, col),
                *text,
                "{name} {}{}",
                omacell_core::addr::col_to_letters(col).unwrap(),
                row + 1
            );
        }
        if let Some(n) = cell.n {
            let got = cell_num(wb, sheet, row, col).unwrap_or(f64::NAN);
            assert!(
                (got - n).abs() < 1e-9,
                "{name} {}{}: got {got} want {n}",
                omacell_core::addr::col_to_letters(col).unwrap(),
                row + 1
            );
        }
    }
}

#[test]
fn pivot_corpus_definitions_match_expected_tables() {
    let cases: Vec<CaseFile> =
        serde_json::from_str(&std::fs::read_to_string(corpus("pivot/cases.json")).unwrap())
            .unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let mut wb = load_source(&case);
        let table = definition(&case, 0, 4);
        let cells = materialize(&wb, &table).unwrap();
        let mut stored = table;
        omacell_core::pivot::write_output(&mut wb, &mut stored, &cells).unwrap();
        assert_expect(&wb, 0, 4, &case.expect, &case.name);
    }
}

#[test]
fn pivot_refresh_after_source_change() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Region").unwrap();
    wb.set_text(s, 0, 1, "Amount").unwrap();
    wb.set_text(s, 1, 0, "East").unwrap();
    wb.set_number(s, 1, 1, 10.0).unwrap();
    wb.set_text(s, 2, 0, "West").unwrap();
    wb.set_number(s, 2, 1, 70.0).unwrap();
    let mut table = PivotTable::new("Sales", s, range(0, 0, 2, 1), s, 0, 4);
    table.rows = vec!["Region".into()];
    table.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    let id = wb.add_pivot(table).unwrap();
    assert_eq!(cell_num(&wb, s, 1, 5), Some(10.0));
    wb.set_number(s, 1, 1, 15.0).unwrap();
    wb.refresh_pivot(id).unwrap();
    assert_eq!(cell_num(&wb, s, 1, 5), Some(15.0));
    assert_eq!(cell_num(&wb, s, 3, 5), Some(85.0));
}

#[test]
fn multiple_value_fields_get_a_dedicated_caption_row_after_all_column_fields() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    for (col, header) in ["Region", "Quarter", "Channel", "Sales", "Units"]
        .iter()
        .enumerate()
    {
        wb.set_text(sheet, 0, col as u16, header).unwrap();
    }
    wb.set_text(sheet, 1, 0, "East").unwrap();
    wb.set_text(sheet, 1, 1, "Q1").unwrap();
    wb.set_text(sheet, 1, 2, "Retail").unwrap();
    wb.set_number(sheet, 1, 3, 10.0).unwrap();
    wb.set_number(sheet, 1, 4, 2.0).unwrap();

    let mut one_column = PivotTable::new("One", sheet, range(0, 0, 1, 4), sheet, 10, 0);
    one_column.rows = vec!["Region".into()];
    one_column.cols = vec!["Quarter".into()];
    one_column.data = vec![
        PivotDataField::new("Sales", PivotAgg::Sum),
        PivotDataField::new("Units", PivotAgg::Sum),
    ];
    let cells = materialize(&wb, &one_column).unwrap();
    assert_eq!(rendered_text(&cells, 0, 1), Some("Q1"));
    assert_eq!(rendered_text(&cells, 1, 1), Some("Sum of Sales"));
    assert_eq!(rendered_text(&cells, 1, 2), Some("Sum of Units"));
    assert_eq!(rendered_text(&cells, 2, 0), Some("East"));
    assert_eq!(rendered_text(&cells, 0, 3), Some("Grand Total"));
    assert_eq!(rendered_text(&cells, 1, 3), Some("Sum of Sales"));
    assert_eq!(rendered_text(&cells, 1, 4), Some("Sum of Units"));
    assert_one_rendered_cell(&cells, 0, 1);

    let mut two_columns = one_column.clone();
    two_columns.name = "Two".into();
    two_columns.cols.push("Channel".into());
    let cells = materialize(&wb, &two_columns).unwrap();
    assert_eq!(rendered_text(&cells, 0, 1), Some("Q1"));
    assert_eq!(rendered_text(&cells, 1, 1), Some("Retail"));
    assert_eq!(rendered_text(&cells, 2, 1), Some("Sum of Sales"));
    assert_eq!(rendered_text(&cells, 2, 2), Some("Sum of Units"));
    assert_eq!(rendered_text(&cells, 3, 0), Some("East"));
    assert_one_rendered_cell(&cells, 1, 1);
}

#[test]
fn pivot_items_use_spreadsheet_display_and_put_blanks_last() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_text(sheet, 0, 0, "Key").unwrap();
    wb.set_text(sheet, 0, 1, "Amount").unwrap();
    wb.set_number(sheet, 1, 1, 1.0).unwrap();
    for (row, text) in [(2, "Zulu"), (3, "alpha"), (4, "Beta")] {
        wb.set_text(sheet, row, 0, text).unwrap();
        wb.set_number(sheet, row, 1, 1.0).unwrap();
    }
    wb.set_number(sheet, 5, 0, 0.30000000000000004).unwrap();
    wb.set_number(sheet, 5, 1, 1.0).unwrap();
    let serial = date_to_serial(
        CivilDate {
            year: 2026,
            month: 1,
            day: 5,
            lotus_leap: false,
        },
        DateSystem::Excel1900,
    )
    .unwrap() as f64;
    wb.set_number(sheet, 6, 0, serial).unwrap();
    wb.set_number(sheet, 6, 1, 1.0).unwrap();
    let date_fmt = wb.intern_num_fmt("m/d/yyyy").unwrap();
    wb.set_cell_style(
        sheet,
        6,
        0,
        Style {
            num_fmt: date_fmt,
            ..Style::default()
        },
    )
    .unwrap();

    let mut table = PivotTable::new("Labels", sheet, range(0, 0, 6, 1), sheet, 10, 0);
    table.rows = vec!["Key".into()];
    table.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    let cells = materialize(&wb, &table).unwrap();
    let labels = (1..=6)
        .map(|row| rendered_text(&cells, row, 0).unwrap_or(""))
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        ["0.3", "1/5/2026", "alpha", "Beta", "Zulu", "(blank)"]
    );
}

#[test]
fn pivot_create_is_atomic_for_undo_and_blocks_sheet_removal() {
    let mut wb = Workbook::new();
    let source = wb.active_sheet();
    let output = wb.add_sheet("Output").unwrap();
    wb.set_text(source, 0, 0, "Region").unwrap();
    wb.set_text(source, 0, 1, "Amount").unwrap();
    wb.set_text(source, 1, 0, "East").unwrap();
    wb.set_number(source, 1, 1, 10.0).unwrap();
    let mut table = PivotTable::new("Sales", source, range(0, 0, 1, 1), output, 0, 0);
    table.rows = vec!["Region".into()];
    table.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    wb.add_pivot(table).unwrap();

    assert_eq!(wb.remove_sheet(source).unwrap_err().code, "pivot.sheet");
    assert_eq!(wb.remove_sheet(output).unwrap_err().code, "pivot.sheet");
    wb.undo().unwrap();
    assert!(wb.pivots().is_empty());
    assert!(wb.get(output, 0, 0).unwrap().is_none());
    assert!(wb.get(output, 1, 1).unwrap().is_none());
    wb.redo().unwrap();
    assert_eq!(wb.pivots().len(), 1);
    assert_eq!(cell_num(&wb, output, 1, 1), Some(10.0));
}

#[test]
fn pivot_output_is_read_only() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Region").unwrap();
    wb.set_text(s, 0, 1, "Amount").unwrap();
    wb.set_text(s, 1, 0, "East").unwrap();
    wb.set_number(s, 1, 1, 10.0).unwrap();
    let mut table = PivotTable::new("Sales", s, range(0, 0, 1, 1), s, 0, 4);
    table.rows = vec!["Region".into()];
    table.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    wb.add_pivot(table).unwrap();
    let err = wb.set_number(s, 1, 5, 99.0).unwrap_err();
    assert_eq!(err.code, "pivot.readonly");
    let err = wb.clear_cell(s, 1, 4).unwrap_err();
    assert_eq!(err.code, "pivot.readonly");
    let err = wb.set_cell_contents(s, 0, 5, "x").unwrap_err();
    assert_eq!(err.code, "pivot.readonly");
    let err = wb.set_slot(s, 1, 5, CellSlot::number(99.0)).unwrap_err();
    assert_eq!(err.code, "pivot.readonly");
    let err = wb
        .set_cell_style(
            s,
            1,
            5,
            Style {
                font: Font {
                    bold: true,
                    ..Font::default()
                },
                ..Style::default()
            },
        )
        .unwrap_err();
    assert_eq!(err.code, "pivot.readonly");
}

#[test]
fn pivot_rejects_unknown_fields_and_unsafe_destinations() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Region").unwrap();
    wb.set_text(s, 0, 1, "Amount").unwrap();
    wb.set_text(s, 1, 0, "East").unwrap();
    wb.set_number(s, 1, 1, 10.0).unwrap();

    let mut unknown = PivotTable::new("Unknown", s, range(0, 0, 1, 1), s, 0, 4);
    unknown.rows = vec!["Missing".into()];
    unknown.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    let err = wb.add_pivot(unknown).unwrap_err();
    assert_eq!(err.code, "pivot.field");

    let mut overlap = PivotTable::new("Overlap", s, range(0, 0, 1, 1), s, 1, 1);
    overlap.rows = vec!["Region".into()];
    overlap.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    let err = wb.add_pivot(overlap).unwrap_err();
    assert_eq!(err.code, "pivot.output");

    let mut overflow = PivotTable::new("Overflow", s, range(0, 0, 1, 1), s, 0, 16_383);
    overflow.rows = vec!["Region".into()];
    overflow.data = vec![
        PivotDataField::new("Amount", PivotAgg::Sum),
        PivotDataField::new("Amount", PivotAgg::Count),
    ];
    let err = wb.add_pivot(overflow).unwrap_err();
    assert_eq!(err.code, "pivot.output");
}

#[test]
fn percent_show_as_uses_fractional_values_and_percent_style() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Region").unwrap();
    wb.set_text(s, 0, 1, "Amount").unwrap();
    wb.set_text(s, 1, 0, "East").unwrap();
    wb.set_number(s, 1, 1, 30.0).unwrap();
    wb.set_text(s, 2, 0, "West").unwrap();
    wb.set_number(s, 2, 1, 70.0).unwrap();
    let mut table = PivotTable::new("Percent", s, range(0, 0, 2, 1), s, 0, 4);
    table.rows = vec!["Region".into()];
    table.data = vec![PivotDataField {
        source: "Amount".into(),
        agg: PivotAgg::Sum,
        show_as: ShowAs::PctOfTotal,
    }];
    wb.add_pivot(table).unwrap();

    assert_eq!(cell_num(&wb, s, 1, 5), Some(0.3));
    assert_eq!(cell_num(&wb, s, 2, 5), Some(0.7));
    assert_eq!(cell_num(&wb, s, 3, 5), Some(1.0));
    let slot = wb.get(s, 1, 5).unwrap().unwrap();
    let style = wb.intern().styles.get(slot.style).unwrap();
    assert_eq!(wb.num_fmt_code(style.num_fmt).as_deref(), Some("0.00%"));
}

#[test]
fn pivot_layouts_and_aggs() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Region").unwrap();
    wb.set_text(s, 0, 1, "Product").unwrap();
    wb.set_text(s, 0, 2, "Amount").unwrap();
    wb.set_text(s, 1, 0, "East").unwrap();
    wb.set_text(s, 1, 1, "A").unwrap();
    wb.set_number(s, 1, 2, 10.0).unwrap();
    wb.set_text(s, 2, 0, "East").unwrap();
    wb.set_text(s, 2, 1, "B").unwrap();
    wb.set_number(s, 2, 2, 20.0).unwrap();
    wb.set_text(s, 3, 0, "West").unwrap();
    wb.set_text(s, 3, 1, "A").unwrap();
    wb.set_number(s, 3, 2, 40.0).unwrap();
    let mut table = PivotTable::new("Sales", s, range(0, 0, 3, 2), s, 0, 5);
    table.rows = vec!["Region".into(), "Product".into()];
    table.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    table.layout = PivotLayout::Tabular;
    let cells = materialize(&wb, &table).unwrap();
    omacell_core::pivot::write_output(&mut wb, &mut table, &cells).unwrap();
    assert_eq!(cell_text(&wb, s, 1, 5), "East");
    assert_eq!(cell_text(&wb, s, 1, 6), "A");
    assert!(
        cell_text(&wb, s, 3, 5).contains("East"),
        "subtotal {}",
        cell_text(&wb, s, 3, 5)
    );
    let mut count = PivotTable::new("Count", s, range(0, 0, 3, 2), s, 20, 0);
    count.rows = vec!["Region".into()];
    count.data = vec![PivotDataField::new("Amount", PivotAgg::Count)];
    let cells = materialize(&wb, &count).unwrap();
    omacell_core::pivot::write_output(&mut wb, &mut count, &cells).unwrap();
    assert_eq!(cell_num(&wb, s, 21, 1), Some(2.0));
    let mut avg = PivotTable::new("Avg", s, range(0, 0, 3, 2), s, 30, 0);
    avg.rows = vec!["Region".into()];
    avg.data = vec![PivotDataField::new("Amount", PivotAgg::Average)];
    let cells = materialize(&wb, &avg).unwrap();
    omacell_core::pivot::write_output(&mut wb, &mut avg, &cells).unwrap();
    assert_eq!(cell_num(&wb, s, 31, 1), Some(15.0));
}

#[test]
fn goal_seek_corpus_converges_within_tolerance() {
    let text = std::fs::read_to_string(corpus("whatif/goalseek.tsv")).unwrap();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with("name\t") {
            continue;
        }
        let c: Vec<&str> = t.split('\t').collect();
        let name = c[0];
        let goal: f64 = c[2].parse().unwrap();
        let start: f64 = c[4].parse().unwrap();
        let formula = c[5];
        let want_ok = c[6] == "true";
        let mut wb = Workbook::new();
        let s = wb.active_sheet();
        wb.set_number(s, 0, 0, start).unwrap();
        wb.set_cell_contents(s, 0, 1, formula).unwrap();
        let mut engine = RecalcEngine::new(FnRegistry::new());
        engine.recalc_rebuild(&mut wb);
        let result = goal_seek(
            &mut wb,
            &mut engine,
            CellCoord::new(s, 0, 1),
            goal,
            CellCoord::new(s, 0, 0),
            DEFAULT_MAX_ITER,
            DEFAULT_TOL,
        )
        .unwrap();
        assert_eq!(result.converged, want_ok, "{name}");
        if want_ok {
            let expect: f64 = c[7].parse().unwrap();
            assert!(
                (result.input - expect).abs() < 1e-4,
                "{name} input {} want {expect}",
                result.input
            );
            assert!((result.output - goal).abs() <= DEFAULT_TOL * 10.0);
        } else {
            assert!(result.input.is_finite(), "{name} last trial must be finite");
        }
    }
}

#[test]
fn goal_seek_is_one_undo_unit_and_honors_iteration_cap() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_cell_style(
        s,
        0,
        0,
        Style {
            font: Font {
                bold: true,
                ..Font::default()
            },
            ..Style::default()
        },
    )
    .unwrap();
    wb.set_cell_contents(s, 0, 1, "=A1*2").unwrap();
    let mut engine = RecalcEngine::new(FnRegistry::new());
    engine.recalc_rebuild(&mut wb);

    wb.transact_try(|workbook| {
        goal_seek(
            workbook,
            &mut engine,
            CellCoord::new(s, 0, 1),
            10.0,
            CellCoord::new(s, 0, 0),
            DEFAULT_MAX_ITER,
            DEFAULT_TOL,
        )
        .map(|_| ())
    })
    .unwrap();
    assert_eq!(cell_num(&wb, s, 0, 0), Some(5.0));
    let style = wb
        .intern()
        .styles
        .get(wb.get(s, 0, 0).unwrap().unwrap().style)
        .unwrap();
    assert!(style.font.bold);
    wb.undo().unwrap();
    assert_eq!(cell_num(&wb, s, 0, 0), Some(1.0));
    let style = wb
        .intern()
        .styles
        .get(wb.get(s, 0, 0).unwrap().unwrap().style)
        .unwrap();
    assert!(style.font.bold);
    assert!(wb.get(s, 0, 1).unwrap().unwrap().formula.is_some());

    let result = goal_seek(
        &mut wb,
        &mut engine,
        CellCoord::new(s, 0, 1),
        10.0,
        CellCoord::new(s, 0, 0),
        1,
        DEFAULT_TOL,
    )
    .unwrap();
    assert_eq!(result.iterations, 1);
}

#[test]
fn goal_seek_rejects_a_fixed_array_formula_follower_as_input() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_array_formula_text(sheet, range(0, 0, 0, 1), "={1,2}")
        .unwrap();
    wb.set_cell_contents(sheet, 1, 0, "=B1*2").unwrap();
    let mut engine = RecalcEngine::new(FnRegistry::new());
    engine.recalc_rebuild(&mut wb);

    let error = goal_seek(
        &mut wb,
        &mut engine,
        CellCoord::new(sheet, 1, 0),
        10.0,
        CellCoord::new(sheet, 0, 1),
        DEFAULT_MAX_ITER,
        DEFAULT_TOL,
    )
    .unwrap_err();

    assert_eq!(error.code, "formula.array");
    assert_eq!(cell_num(&wb, sheet, 0, 1), Some(2.0));
}

#[test]
fn stats_describe_known_range() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    for (i, n) in [1.0, 2.0, 3.0, 4.0, 5.0].into_iter().enumerate() {
        wb.set_number(s, i as u32, 0, n).unwrap();
    }
    wb.set_text(s, 5, 0, "x").unwrap();
    let summary = describe_range(&wb, s, range(0, 0, 5, 0)).unwrap();
    assert_eq!(summary.count, 5);
    assert_eq!(summary.count_a, 6);
    assert_eq!(summary.sum, 15.0);
    assert_eq!(summary.mean, Some(3.0));
    assert_eq!(summary.min, Some(1.0));
    assert_eq!(summary.max, Some(5.0));
    assert_eq!(summary.median, Some(3.0));
    let var = summary.var.unwrap();
    assert!((var - 2.5).abs() < 1e-12);
    assert!((summary.stdev.unwrap() - var.sqrt()).abs() < 1e-12);
    assert!(!summary.histogram.is_empty());
    assert!(summary.histogram.iter().map(|b| b.count).sum::<u32>() == 5);
}

#[test]
fn stats_rejects_an_unknown_sheet() {
    let wb = Workbook::new();
    let err = describe_range(&wb, SheetId::new(999), range(0, 0, 0, 0)).unwrap_err();
    assert_eq!(err.code, omacell_core::error::codes::SHEET_ID);
}

fn sales_source() -> (Workbook, SheetId) {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Region").unwrap();
    wb.set_text(s, 0, 1, "Amount").unwrap();
    wb.set_text(s, 1, 0, "East").unwrap();
    wb.set_number(s, 1, 1, 10.0).unwrap();
    wb.set_text(s, 2, 0, "West").unwrap();
    wb.set_number(s, 2, 1, 20.0).unwrap();
    (wb, s)
}

#[test]
fn pivot_structural_edits_rewrite_source_and_output_as_one_undo() {
    let (mut wb, s) = sales_source();
    let mut table = PivotTable::new("Sales", s, range(0, 0, 2, 1), s, 0, 4);
    table.rows = vec!["Region".into()];
    table.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    let id = wb.add_pivot(table).unwrap();
    let dest_col = wb.pivots().get(id).unwrap().dest_col;

    wb.transact_try(|workbook| insert_rows(workbook, s, 0, 1))
        .unwrap();
    let pivot = wb.pivots().get(id).unwrap();
    assert_eq!(pivot.source.start.row, 1);
    assert_eq!(pivot.source.end.row, 3);
    assert_eq!(pivot.dest_row, 1);
    assert_eq!(cell_text(&wb, s, 2, 0), "East");
    assert_eq!(cell_num(&wb, s, 2, dest_col + 1), Some(10.0));
    wb.undo().unwrap();
    let pivot = wb.pivots().get(id).unwrap();
    assert_eq!(pivot.source.start.row, 0);
    assert_eq!(pivot.dest_row, 0);
    assert_eq!(cell_text(&wb, s, 1, 0), "East");

    wb.insert_rows(s, 2, 1).unwrap();
    let pivot = wb.pivots().get(id).unwrap();
    assert_eq!(pivot.source.start.row, 0);
    assert_eq!(pivot.source.end.row, 3);

    wb.insert_rows(s, 10, 1).unwrap();
    let pivot = wb.pivots().get(id).unwrap();
    assert_eq!(pivot.source.end.row, 3);

    assert_eq!(wb.delete_rows(s, 0, 1).unwrap_err().code, "pivot.struct");
}

#[test]
fn pivot_deletion_before_ranges_rewrites_them_and_undoes() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 5, 0, "Region").unwrap();
    wb.set_text(s, 5, 1, "Amount").unwrap();
    wb.set_text(s, 6, 0, "East").unwrap();
    wb.set_number(s, 6, 1, 10.0).unwrap();
    wb.set_text(s, 7, 0, "West").unwrap();
    wb.set_number(s, 7, 1, 20.0).unwrap();
    let mut table = PivotTable::new("Sales", s, range(5, 0, 7, 1), s, 10, 4);
    table.rows = vec!["Region".into()];
    table.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    let id = wb.add_pivot(table).unwrap();

    wb.transact_try(|workbook| delete_rows(workbook, s, 0, 1))
        .unwrap();
    let pivot = wb.pivots().get(id).unwrap();
    assert_eq!((pivot.source.start.row, pivot.source.end.row), (4, 6));
    assert_eq!((pivot.dest_row, pivot.out_end_row), (9, 12));
    assert_eq!(cell_text(&wb, s, 5, 0), "East");
    assert_eq!(cell_num(&wb, s, 10, 5), Some(10.0));

    wb.undo().unwrap();
    let pivot = wb.pivots().get(id).unwrap();
    assert_eq!((pivot.source.start.row, pivot.source.end.row), (5, 7));
    assert_eq!((pivot.dest_row, pivot.out_end_row), (10, 13));
    assert_eq!(cell_text(&wb, s, 6, 0), "East");
    assert_eq!(cell_num(&wb, s, 11, 5), Some(10.0));
}

#[test]
fn pivot_cell_deletion_before_source_rewrites_a_covering_band() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 5, 0, "Region").unwrap();
    wb.set_text(s, 5, 1, "Amount").unwrap();
    wb.set_text(s, 6, 0, "East").unwrap();
    wb.set_number(s, 6, 1, 10.0).unwrap();
    let mut table = PivotTable::new("Sales", s, range(5, 0, 6, 1), s, 0, 4);
    table.rows = vec!["Region".into()];
    table.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    let id = wb.add_pivot(table).unwrap();

    delete_cells(&mut wb, s, range(0, 0, 0, 1), Shift::Down).unwrap();
    let pivot = wb.pivots().get(id).unwrap();
    assert_eq!((pivot.source.start.row, pivot.source.end.row), (4, 5));
    assert_eq!(pivot.dest_row, 0);
    assert_eq!(cell_text(&wb, s, 5, 0), "East");
}

#[test]
fn pivot_column_deletion_before_ranges_rewrites_them_and_undoes() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 4, "Region").unwrap();
    wb.set_text(s, 0, 5, "Amount").unwrap();
    wb.set_text(s, 1, 4, "East").unwrap();
    wb.set_number(s, 1, 5, 10.0).unwrap();
    let mut table = PivotTable::new("Sales", s, range(0, 4, 1, 5), s, 5, 10);
    table.rows = vec!["Region".into()];
    table.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    let id = wb.add_pivot(table).unwrap();

    wb.transact_try(|workbook| delete_cols(workbook, s, 0, 1))
        .unwrap();
    let pivot = wb.pivots().get(id).unwrap();
    assert_eq!((pivot.source.start.col, pivot.source.end.col), (3, 4));
    assert_eq!((pivot.dest_col, pivot.out_end_col), (9, 10));
    assert_eq!(cell_text(&wb, s, 1, 3), "East");
    assert_eq!(cell_num(&wb, s, 6, 10), Some(10.0));

    wb.undo().unwrap();
    let pivot = wb.pivots().get(id).unwrap();
    assert_eq!((pivot.source.start.col, pivot.source.end.col), (4, 5));
    assert_eq!((pivot.dest_col, pivot.out_end_col), (10, 11));
}

#[test]
fn pivot_cell_shift_refuses_to_split_and_rewrites_full_bands() {
    let (mut wb, s) = sales_source();
    let mut table = PivotTable::new("Sales", s, range(0, 0, 2, 1), s, 0, 4);
    table.rows = vec!["Region".into()];
    table.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    let id = wb.add_pivot(table).unwrap();
    assert_eq!(
        insert_cells(&mut wb, s, range(0, 0, 0, 0), Shift::Down)
            .unwrap_err()
            .code,
        "pivot.struct"
    );
    insert_cells(&mut wb, s, range(0, 8, 0, 8), Shift::Down).unwrap();
    let pivot = wb.pivots().get(id).unwrap();
    assert_eq!(pivot.source.start.row, 0);
    assert_eq!(pivot.dest_col, 4);
}

#[test]
fn pivot_calculated_field_is_aggregated() {
    let (mut wb, s) = sales_source();
    let mut table = PivotTable::new("Sales", s, range(0, 0, 2, 1), s, 0, 4);
    table.rows = vec!["Region".into()];
    table.calc_fields = vec![PivotCalcField {
        name: "Tax".into(),
        formula: "'Amount'*0.1".into(),
    }];
    table.data = vec![PivotDataField::new("Tax", PivotAgg::Sum)];
    wb.add_pivot(table).unwrap();
    assert_eq!(cell_num(&wb, s, 1, 5), Some(1.0));
    assert_eq!(cell_num(&wb, s, 2, 5), Some(2.0));
    assert_eq!(cell_num(&wb, s, 3, 5), Some(3.0));
}

#[test]
fn pivot_calculated_fields_handle_escaped_names_and_reject_duplicates() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Bob's Amount").unwrap();
    wb.set_number(s, 1, 0, 20.0).unwrap();
    let mut table = PivotTable::new("Escaped", s, range(0, 0, 1, 0), s, 0, 3);
    table.calc_fields = vec![PivotCalcField {
        name: "Tax".into(),
        formula: "'Bob''s Amount'*0.1".into(),
    }];
    table.data = vec![PivotDataField::new("Tax", PivotAgg::Sum)];
    let (headers, rows) = omacell_core::pivot::cache_table(&wb, &table).unwrap();
    assert_eq!(headers, ["Bob's Amount", "Tax"]);
    assert_eq!(rows[0][1], omacell_core::pivot::CacheValue::Number(2.0));
    wb.add_pivot(table).unwrap();

    let mut duplicate = PivotTable::new("Duplicate", s, range(0, 0, 1, 0), s, 5, 3);
    duplicate.calc_fields = vec![PivotCalcField {
        name: "Bob's Amount".into(),
        formula: "1".into(),
    }];
    duplicate.data = vec![PivotDataField::new("Bob's Amount", PivotAgg::Sum)];
    assert_eq!(wb.add_pivot(duplicate).unwrap_err().code, "pivot.field");

    let mut oversized = PivotTable::new("Oversized", s, range(0, 0, 1, 0), s, 5, 6);
    oversized.calc_fields = vec![PivotCalcField {
        name: "TooLong".into(),
        formula: "1".repeat(8_193),
    }];
    assert_eq!(
        omacell_core::pivot::cache_table(&wb, &oversized)
            .unwrap_err()
            .code,
        "pivot.field"
    );
}

#[test]
fn pivot_header_only_source_still_registers_calculated_fields() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Amount").unwrap();
    let mut table = PivotTable::new("Empty", s, range(0, 0, 0, 0), s, 0, 3);
    table.calc_fields = vec![PivotCalcField {
        name: "Tax".into(),
        formula: "'Amount'*0.1".into(),
    }];
    table.data = vec![PivotDataField::new("Tax", PivotAgg::Sum)];
    wb.add_pivot(table).unwrap();
}

#[test]
fn pivot_compact_layout_indents_nested_row_fields() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Region").unwrap();
    wb.set_text(s, 0, 1, "Product").unwrap();
    wb.set_text(s, 0, 2, "Amount").unwrap();
    wb.set_text(s, 1, 0, "East").unwrap();
    wb.set_text(s, 1, 1, "A").unwrap();
    wb.set_number(s, 1, 2, 10.0).unwrap();
    wb.set_text(s, 2, 0, "East").unwrap();
    wb.set_text(s, 2, 1, "B").unwrap();
    wb.set_number(s, 2, 2, 20.0).unwrap();
    wb.set_text(s, 3, 0, "West").unwrap();
    wb.set_text(s, 3, 1, "A").unwrap();
    wb.set_number(s, 3, 2, 40.0).unwrap();
    let mut table = PivotTable::new("Sales", s, range(0, 0, 3, 2), s, 0, 5);
    table.rows = vec!["Region".into(), "Product".into()];
    table.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    table.layout = PivotLayout::Compact;
    table.subtotals = false;
    let cells = materialize(&wb, &table).unwrap();
    omacell_core::pivot::write_output(&mut wb, &mut table, &cells).unwrap();
    assert_eq!(cell_text(&wb, s, 1, 5), "East");
    assert_eq!(cell_text(&wb, s, 2, 5), "  A");
    assert_eq!(cell_num(&wb, s, 2, 6), Some(10.0));
    assert_eq!(cell_text(&wb, s, 3, 5), "  B");
    assert_eq!(cell_num(&wb, s, 3, 6), Some(20.0));
    assert_eq!(cell_text(&wb, s, 4, 5), "West");
    assert_eq!(cell_text(&wb, s, 5, 5), "  A");
    assert_eq!(cell_num(&wb, s, 5, 6), Some(40.0));
}

#[test]
fn pivot_hierarchy_restarts_child_labels_when_parent_changes() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    for (col, header) in ["Region", "Category", "Product", "Amount"]
        .into_iter()
        .enumerate()
    {
        wb.set_text(s, 0, col as u16, header).unwrap();
    }
    for (row, region, product, amount) in [(1, "East", "X", 10.0), (2, "West", "Y", 20.0)] {
        wb.set_text(s, row, 0, region).unwrap();
        wb.set_text(s, row, 1, "A").unwrap();
        wb.set_text(s, row, 2, product).unwrap();
        wb.set_number(s, row, 3, amount).unwrap();
    }

    let mut compact = PivotTable::new("Compact", s, range(0, 0, 2, 3), s, 0, 6);
    compact.rows = vec!["Region".into(), "Category".into(), "Product".into()];
    compact.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    compact.subtotals = false;
    let compact_cells = materialize(&wb, &compact).unwrap();
    omacell_core::pivot::write_output(&mut wb, &mut compact, &compact_cells).unwrap();
    assert_eq!(cell_text(&wb, s, 4, 6), "West");
    assert_eq!(cell_text(&wb, s, 5, 6), "  A");
    assert_eq!(cell_text(&wb, s, 6, 6), "    Y");

    let mut outline = PivotTable::new("Outline", s, range(0, 0, 2, 3), s, 10, 6);
    outline.rows = vec!["Region".into(), "Category".into(), "Product".into()];
    outline.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    outline.layout = PivotLayout::Outline;
    outline.subtotals = false;
    let outline_cells = materialize(&wb, &outline).unwrap();
    omacell_core::pivot::write_output(&mut wb, &mut outline, &outline_cells).unwrap();
    assert_eq!(cell_text(&wb, s, 12, 6), "West");
    assert_eq!(cell_text(&wb, s, 12, 7), "A");
    assert_eq!(cell_text(&wb, s, 12, 8), "Y");
}
