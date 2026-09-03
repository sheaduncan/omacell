//! Lazy-branch, hidden-row, random, whole-column, criteria, and fuzz smoke.

use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::coerce::Scalar;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, FnRegistry, RuntimeArray, RuntimeValue};
use omacell_core::filter::{AutoFilter, FilterColumn, FilterCriteria, NumOp, apply_filter};
use omacell_core::graph::{CellCoord, Precedent};
use omacell_core::recalc::{RecalcEngine, format_cell};
use omacell_core::spill::SpillTable;
use omacell_core::workbook::Workbook;
use omacell_fn::{all_specs, register_all};

fn display(wb: &Workbook, row: u32, col: u16) -> String {
    format_cell(wb, wb.active_sheet(), row, col)
}

fn engine() -> RecalcEngine {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    RecalcEngine::new(registry)
}

#[test]
fn lazy_if_family_skips_unselected_error_volatile_and_async() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=IF(TRUE,9,1/0)").unwrap();
    wb.set_formula_text(s, 0, 1, "=IFS(FALSE,1/0,TRUE,4)")
        .unwrap();
    wb.set_formula_text(s, 0, 2, "=SWITCH(1,2,1/0,1,8)")
        .unwrap();
    wb.set_formula_text(s, 0, 3, "=IFERROR(1,1/0)").unwrap();
    wb.set_formula_text(s, 0, 4, "=IFNA(1,1/0)").unwrap();
    let mut eng = engine();
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 0), "9");
    assert_eq!(display(&wb, 0, 1), "4");
    assert_eq!(display(&wb, 0, 2), "8");
    assert_eq!(display(&wb, 0, 3), "1");
    assert_eq!(display(&wb, 0, 4), "1");
}

#[test]
fn array_if_selects_and_broadcasts_cells_without_evaluating_unused_branches() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    for (row, value) in [1.0, 2.0, 3.0].into_iter().enumerate() {
        wb.set_number(s, row as u32, 0, value).unwrap();
    }
    for (row, formula) in [
        "=SUM(IF(A1:A3>1,A1:A3,0))",
        "=SUM(IF(A1:A3>1,{10;20;30},1))",
        "=SUM(IF(A1:A3>9,1/0,7))",
        "=SUM(IF(A1:A3>0,7,1/0))",
    ]
    .into_iter()
    .enumerate()
    {
        wb.set_formula_text(s, row as u32, 2, formula).unwrap();
    }
    wb.set_formula_text(s, 4, 2, "=IF({TRUE,FALSE,TRUE},{1,2,3},10)")
        .unwrap();
    wb.set_formula_text(s, 5, 2, "=IF({TRUE,#N/A,FALSE},1,2)")
        .unwrap();

    let mut eng = engine();
    eng.recalc_full(&mut wb);

    assert_eq!(display(&wb, 0, 2), "5");
    assert_eq!(display(&wb, 1, 2), "51");
    assert_eq!(display(&wb, 2, 2), "21");
    assert_eq!(display(&wb, 3, 2), "21");
    assert_eq!(display(&wb, 4, 2), "1");
    assert_eq!(display(&wb, 4, 3), "10");
    assert_eq!(display(&wb, 4, 4), "3");
    assert_eq!(display(&wb, 5, 2), "1");
    assert_eq!(display(&wb, 5, 3), "#N/A");
    assert_eq!(display(&wb, 5, 4), "2");
}

#[test]
fn array_error_handlers_replace_cells_and_preserve_unhandled_errors() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_number(sheet, 0, 0, 1.0).unwrap();
    wb.set_formula_text(sheet, 1, 0, "=NA()").unwrap();
    wb.set_formula_text(sheet, 2, 0, "=1/0").unwrap();
    wb.set_formula_text(sheet, 0, 2, "=IFERROR(A1:A3,0)")
        .unwrap();
    wb.set_formula_text(sheet, 0, 3, "=IFNA(A1:A3,0)").unwrap();

    let mut engine = engine();
    engine.recalc_full(&mut wb);

    assert_eq!(display(&wb, 0, 2), "1");
    assert_eq!(display(&wb, 1, 2), "0");
    assert_eq!(display(&wb, 2, 2), "0");
    assert_eq!(display(&wb, 0, 3), "1");
    assert_eq!(display(&wb, 1, 3), "0");
    assert_eq!(display(&wb, 2, 3), "#DIV/0!");
}

#[test]
fn let_bindings_keep_error_values_for_error_handlers() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_formula_text(sheet, 0, 0, "=LET(x,1/0,IFERROR(x,5))")
        .unwrap();
    wb.set_formula_text(sheet, 1, 0, "=LET(x,1/0,x+1)").unwrap();

    let mut engine = engine();
    engine.recalc_full(&mut wb);

    assert_eq!(display(&wb, 0, 0), "5");
    assert_eq!(display(&wb, 1, 0), "#DIV/0!");
}

#[test]
fn and_or_do_not_short_circuit() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=AND(FALSE,1/0)").unwrap();
    wb.set_formula_text(s, 0, 1, "=OR(TRUE,1/0)").unwrap();
    let mut eng = engine();
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 0), "#DIV/0!");
    assert_eq!(display(&wb, 0, 1), "#DIV/0!");
}

#[test]
fn logical_aggregates_ignore_text_and_empty_cells_inside_references() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_formula_text(s, 0, 0, "=TRUE()").unwrap();
    wb.set_text(s, 1, 0, "ignored").unwrap();
    wb.set_formula_text(s, 3, 0, "=FALSE()").unwrap();
    for (row, formula) in [
        "=AND(A1:A3)",
        "=OR(A2:A4)",
        "=XOR(A1:A4)",
        "=AND(A2:A3)",
        "=AND(TRUE,\"text\")",
    ]
    .into_iter()
    .enumerate()
    {
        wb.set_formula_text(s, row as u32, 1, formula).unwrap();
    }

    let mut eng = engine();
    eng.recalc_full(&mut wb);

    assert_eq!(display(&wb, 0, 1), "TRUE");
    assert_eq!(display(&wb, 1, 1), "FALSE");
    assert_eq!(display(&wb, 2, 1), "TRUE");
    assert_eq!(display(&wb, 3, 1), "#VALUE!");
    assert_eq!(display(&wb, 4, 1), "#VALUE!");
}

#[test]
fn hidden_row_subtotal_skips_101_family() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_number(s, 0, 0, 10.0).unwrap();
    wb.set_number(s, 1, 0, 20.0).unwrap();
    wb.set_number(s, 2, 0, 30.0).unwrap();
    wb.set_row_hidden(s, 1, true).unwrap();
    wb.set_formula_text(s, 0, 1, "=SUBTOTAL(9,A1:A3)").unwrap();
    wb.set_formula_text(s, 1, 1, "=SUBTOTAL(109,A1:A3)")
        .unwrap();
    wb.set_formula_text(s, 2, 1, "=AGGREGATE(9,5,A1:A3)")
        .unwrap();
    wb.set_formula_text(s, 3, 1, "=AGGREGATE(9,1,A1:A3)")
        .unwrap();
    wb.set_formula_text(s, 4, 1, "=AGGREGATE(9,4,A1:A3)")
        .unwrap();
    let mut eng = engine();
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 1), "60");
    assert_eq!(display(&wb, 1, 1), "40");
    assert_eq!(display(&wb, 2, 1), "40");
    assert_eq!(display(&wb, 3, 1), "40");
    assert_eq!(display(&wb, 4, 1), "60");
}

#[test]
fn empty_min_max_family_and_aggregate_forms_return_zero() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    for (row, formula) in [
        "=MIN(A1:A2)",
        "=MAX(A1:A2)",
        "=MINA(A1:A2)",
        "=MAXA(A1:A2)",
        "=SUBTOTAL(4,A1:A2)",
        "=SUBTOTAL(5,A1:A2)",
        "=AGGREGATE(4,4,A1:A2)",
        "=AGGREGATE(5,4,A1:A2)",
    ]
    .into_iter()
    .enumerate()
    {
        wb.set_formula_text(s, row as u32, 1, formula).unwrap();
    }

    let mut eng = engine();
    eng.recalc_full(&mut wb);
    for row in 0..8 {
        assert_eq!(display(&wb, row, 1), "0");
    }
}

#[test]
fn explicit_omitted_numeric_argument_is_zero_but_empty_reference_is_ignored() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_formula_text(s, 0, 2, "=AVERAGE(A1:A2)").unwrap();
    wb.set_formula_text(s, 1, 2, "=AVERAGE(A1:A2,2,)").unwrap();

    let mut eng = engine();
    eng.recalc_full(&mut wb);

    assert_eq!(display(&wb, 0, 2), "#DIV/0!");
    assert_eq!(display(&wb, 1, 2), "1");
}

#[test]
fn subtotal_always_skips_filtered_rows_but_only_101_family_skips_manual_rows() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_text(s, 0, 0, "Value").unwrap();
    wb.set_number(s, 1, 0, 10.0).unwrap();
    wb.set_number(s, 2, 0, 20.0).unwrap();
    wb.set_number(s, 3, 0, 30.0).unwrap();
    apply_filter(
        &mut wb,
        s,
        &AutoFilter {
            range: RangeRef::from_corners(CellRef::new(0, 0).unwrap(), CellRef::new(3, 0).unwrap()),
            columns: vec![FilterColumn {
                col_id: 0,
                criteria: FilterCriteria::Number {
                    op: NumOp::Greater,
                    value: 10.0,
                    value2: None,
                },
            }],
        },
    )
    .unwrap();
    wb.set_row_hidden(s, 2, true).unwrap();
    wb.set_formula_text(s, 0, 1, "=SUBTOTAL(9,A2:A4)").unwrap();
    wb.set_formula_text(s, 1, 1, "=SUBTOTAL(109,A2:A4)")
        .unwrap();

    let mut eng = engine();
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 1), "50");
    assert_eq!(display(&wb, 1, 1), "30");
}

#[test]
fn nested_subtotal_is_ignored() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_number(s, 0, 0, 10.0).unwrap();
    wb.set_number(s, 1, 0, 20.0).unwrap();
    wb.set_formula_text(s, 2, 0, "=SUBTOTAL(9,A1:A2)").unwrap();
    wb.set_formula_text(s, 0, 1, "=SUBTOTAL(9,A1:A3)").unwrap();
    wb.set_formula_text(s, 1, 1, "=AGGREGATE(9,0,A1:A3)")
        .unwrap();
    wb.set_formula_text(s, 2, 1, "=AGGREGATE(9,4,A1:A3)")
        .unwrap();
    let mut eng = engine();
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 2, 0), "30");
    assert_eq!(display(&wb, 0, 1), "30");
    assert_eq!(display(&wb, 1, 1), "30");
    assert_eq!(display(&wb, 2, 1), "60");
}

#[test]
fn random_is_deterministic_across_thread_counts() {
    fn snapshot(threads: usize) -> Vec<String> {
        let mut wb = Workbook::new();
        let s = wb.active_sheet();
        wb.undo_log_mut().set_enabled(false);
        for i in 0..32u32 {
            wb.set_formula_text(s, i, 0, "=RAND()").unwrap();
            wb.set_formula_text(s, i, 1, "=RANDBETWEEN(1,100)").unwrap();
        }
        let mut eng = engine();
        eng.set_threads(threads);
        eng.set_random_nonce(Some(0xDEAD_BEEF_CAFE_BABE));
        eng.recalc_full(&mut wb);
        (0..32u32)
            .map(|i| format!("{}:{}", display(&wb, i, 0), display(&wb, i, 1)))
            .collect()
    }
    let one = snapshot(1);
    let eight = snapshot(8);
    assert_eq!(one, eight);
    assert_ne!(one[0], one[1]);
}

#[test]
fn dynamic_references_order_targets_and_chains_in_the_same_pass() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_formula_text(sheet, 0, 0, "=INDIRECT(\"B1\")")
        .unwrap();
    wb.set_formula_text(sheet, 0, 1, "=C1*2").unwrap();
    wb.set_number(sheet, 0, 2, 3.0).unwrap();
    wb.set_formula_text(sheet, 0, 3, "=INDIRECT(\"A1\")+1")
        .unwrap();
    wb.set_formula_text(sheet, 0, 4, "=OFFSET(C1,0,-1)")
        .unwrap();
    wb.set_formula_text(sheet, 0, 5, "=ROWS(INDIRECT(\"F1:F3\"))")
        .unwrap();
    wb.set_formula_text(sheet, 0, 6, "=SUM(INDIRECT(\"C:C\"))")
        .unwrap();

    let mut engine = engine();
    let result = engine.recalc_full(&mut wb);

    assert!(result.circular.is_empty());
    assert_eq!(display(&wb, 0, 0), "6");
    assert_eq!(display(&wb, 0, 1), "6");
    assert_eq!(display(&wb, 0, 3), "7");
    assert_eq!(display(&wb, 0, 4), "6");
    assert_eq!(display(&wb, 0, 5), "3");
    assert_eq!(display(&wb, 0, 6), "3");
    assert!(
        engine
            .graph()
            .precedents(CellCoord::new(sheet, 0, 0))
            .contains(&Precedent::Cell(CellCoord::new(sheet, 0, 1)))
    );
    assert!(
        engine
            .graph()
            .precedents(CellCoord::new(sheet, 0, 5))
            .is_empty(),
        "shape-only dynamic references must not create value-ordering edges"
    );
    assert!(
        engine
            .graph()
            .precedents(CellCoord::new(sheet, 0, 6))
            .iter()
            .any(|precedent| matches!(
                precedent,
                Precedent::Range {
                    whole_col: true,
                    ..
                }
            ))
    );
}

#[test]
fn dynamically_resolved_cycles_use_the_circular_policy() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_formula_text(sheet, 0, 0, "=INDIRECT(\"B1\")+1")
        .unwrap();
    wb.set_formula_text(sheet, 0, 1, "=INDIRECT(\"A1\")+1")
        .unwrap();

    let mut engine = engine();
    let result = engine.recalc_full(&mut wb);

    assert_eq!(
        result.circular,
        vec![CellCoord::new(sheet, 0, 0), CellCoord::new(sheet, 0, 1)]
    );
    assert_eq!(display(&wb, 0, 0), "0");
    assert_eq!(display(&wb, 0, 1), "0");
}

#[test]
fn whole_column_sum_sumifs_subtotal() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    for i in 0..100u32 {
        wb.set_number(s, i, 0, 1.0).unwrap();
        wb.set_number(s, i, 1, if i % 2 == 0 { 1.0 } else { 0.0 })
            .unwrap();
    }
    wb.set_formula_text(s, 0, 2, "=SUM(A:A)").unwrap();
    wb.set_formula_text(s, 1, 2, "=SUMIFS(A:A,B:B,1)").unwrap();
    wb.set_formula_text(s, 2, 2, "=SUBTOTAL(9,A:A)").unwrap();
    let mut eng = engine();
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 2), "100");
    assert_eq!(display(&wb, 1, 2), "50");
    assert_eq!(display(&wb, 2, 2), "100");
}

#[test]
fn criteria_wildcards_on_sheet_ranges() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_text(s, 0, 0, "apple").unwrap();
    wb.set_text(s, 1, 0, "apricot").unwrap();
    wb.set_text(s, 2, 0, "banana").unwrap();
    wb.set_text(s, 3, 0, "a*").unwrap();
    wb.set_number(s, 0, 1, 1.0).unwrap();
    wb.set_number(s, 1, 1, 2.0).unwrap();
    wb.set_number(s, 2, 1, 4.0).unwrap();
    wb.set_number(s, 3, 1, 8.0).unwrap();
    wb.set_formula_text(s, 0, 2, "=SUMIF(A1:A4,\"a*\",B1:B4)")
        .unwrap();
    wb.set_formula_text(s, 1, 2, "=COUNTIF(A1:A4,\"b*\")")
        .unwrap();
    wb.set_formula_text(s, 2, 2, "=SUMIF(A1:A4,\"a~*\",B1:B4)")
        .unwrap();
    let mut eng = engine();
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 2), "11");
    assert_eq!(display(&wb, 1, 2), "1");
    assert_eq!(display(&wb, 2, 2), "8");
}

#[test]
fn sumif_value_ranges_resize_from_top_left_and_track_implicit_cells() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);

    for (row, col, value) in [
        (0, 0, 1.0),
        (0, 1, 2.0),
        (1, 0, 3.0),
        (1, 1, 4.0),
        (0, 3, 10.0),
        (0, 4, 20.0),
        (1, 3, 30.0),
        (2, 3, 900.0),
        (3, 3, 900.0),
        (0, 5, 40.0),
    ] {
        wb.set_number(s, row, col, value).unwrap();
    }
    wb.set_formula_text(s, 1, 4, "=F1").unwrap();
    wb.set_formula_text(s, 0, 6, "=SUMIF(A1:B2,\">2\",D1:D4)")
        .unwrap();
    wb.set_formula_text(s, 1, 6, "=AVERAGEIF(A1:B2,\">2\",D1:D4)")
        .unwrap();

    let mut eng = engine();
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 6), "70");
    assert_eq!(display(&wb, 1, 6), "35");

    wb.set_number(s, 0, 5, 50.0).unwrap();
    eng.notify_edit(&wb, CellCoord::new(s, 0, 5));
    eng.recalc_incremental(&mut wb);
    assert_eq!(display(&wb, 0, 6), "80");
    assert_eq!(display(&wb, 1, 6), "40");
}

#[test]
fn sumif_resized_range_does_not_create_a_false_cycle_from_the_written_tail() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    for (row, col, value) in [
        (0, 0, 1.0),
        (0, 1, 2.0),
        (1, 0, 3.0),
        (1, 1, 4.0),
        (0, 3, 10.0),
        (0, 4, 20.0),
        (1, 3, 30.0),
        (1, 4, 40.0),
        (3, 3, 900.0),
    ] {
        wb.set_number(s, row, col, value).unwrap();
    }
    wb.set_formula_text(s, 2, 3, "=SUMIF(A1:B2,\">2\",D1:D4)")
        .unwrap();

    let mut eng = engine();
    let result = eng.recalc_full(&mut wb);
    assert!(result.circular.is_empty());
    assert_eq!(display(&wb, 2, 3), "70");
}

#[test]
fn criteria_matching_keeps_blank_text_number_and_bool_distinct() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);

    // A1 and C1 are true blank cells. The remaining criteria-range values
    // deliberately distinguish formula empty text, numeric text, and booleans.
    wb.set_formula_text(s, 1, 0, "=\"\"").unwrap();
    wb.set_number(s, 2, 0, 0.0).unwrap();
    wb.set_formula_text(s, 3, 0, "=FALSE").unwrap();
    wb.set_number(s, 4, 0, 3.0).unwrap();
    wb.set_text(s, 5, 0, "3").unwrap();
    wb.set_text(s, 6, 0, "alpha").unwrap();
    wb.set_formula_text(s, 7, 0, "=TRUE").unwrap();
    wb.set_text(s, 8, 0, "0").unwrap();
    for row in 0..9 {
        wb.set_number(s, row, 1, f64::from((row + 1) * 10)).unwrap();
    }

    let cases = [
        ("=COUNTIF(A1:A9,\"<5\")", "4"),
        ("=COUNTIF(A1:A9,\"*\")", "4"),
        ("=COUNTIF(A1:A9,0)", "2"),
        ("=COUNTIF(A1:A9,FALSE)", "1"),
        ("=COUNTIF(A1:A9,\"FALSE\")", "1"),
        ("=COUNTIF(A1:A9,\"\")", "2"),
        ("=COUNTIF(A1:A9,C1)", "2"),
        ("=COUNTIF(A1:A9,\"=3\")", "2"),
        ("=SUMIF(A1:A9,\"<5\",B1:B9)", "230"),
        ("=AVERAGEIF(A1:A9,\"<5\",B1:B9)", "57.5"),
        ("=SUMIFS(B1:B9,A1:A9,\"<5\")", "230"),
        ("=COUNTIFS(A1:A9,\"<5\")", "4"),
        ("=AVERAGEIFS(B1:B9,A1:A9,\"<5\")", "57.5"),
        ("=MAXIFS(B1:B9,A1:A9,\"<5\")", "90"),
        ("=MINIFS(B1:B9,A1:A9,\"<5\")", "30"),
        ("=SUMIF(A1:A9,\"*\",B1:B9)", "240"),
        ("=AVERAGEIF(A1:A9,\"*\",B1:B9)", "60"),
        ("=SUMIF(A1:A9,FALSE,B1:B9)", "40"),
        ("=SUMIF(A1:A9,\"\",B1:B9)", "30"),
        ("=SUMIF(A1:A9,C1,B1:B9)", "120"),
    ];
    for (row, (formula, _)) in cases.iter().enumerate() {
        wb.set_formula_text(s, row as u32, 3, formula).unwrap();
    }

    let mut eng = engine();
    eng.recalc_full(&mut wb);
    for (row, (_, expected)) in cases.iter().enumerate() {
        assert_eq!(display(&wb, row as u32, 3), *expected);
    }
}

#[test]
fn if_family_requires_range_references() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    for (row, (criterion, value)) in [(1.0, 10.0), (2.0, 20.0), (3.0, 30.0)]
        .into_iter()
        .enumerate()
    {
        wb.set_number(s, row as u32, 0, criterion).unwrap();
        wb.set_number(s, row as u32, 1, value).unwrap();
    }
    let range_calls = [
        ("=SUMIF(A1:A3,\">1\",B1:B3)", "50"),
        ("=COUNTIF(A1:A3,\">1\")", "2"),
        ("=AVERAGEIF(A1:A3,\">1\",B1:B3)", "25"),
        ("=SUMIFS(B1:B3,A1:A3,\">1\")", "50"),
        ("=COUNTIFS(A1:A3,\">1\")", "2"),
        ("=AVERAGEIFS(B1:B3,A1:A3,\">1\")", "25"),
        ("=MAXIFS(B1:B3,A1:A3,\">1\")", "30"),
        ("=MINIFS(B1:B3,A1:A3,\">1\")", "20"),
    ];
    let array_calls = [
        "=SUMIF({1;2;3},\">1\",{10;20;30})",
        "=COUNTIF({1;2;3},\">1\")",
        "=AVERAGEIF({1;2;3},\">1\",{10;20;30})",
        "=SUMIFS({10;20;30},{1;2;3},\">1\")",
        "=COUNTIFS({1;2;3},\">1\")",
        "=AVERAGEIFS({10;20;30},{1;2;3},\">1\")",
        "=MAXIFS({10;20;30},{1;2;3},\">1\")",
        "=MINIFS({10;20;30},{1;2;3},\">1\")",
    ];
    for (row, (formula, _)) in range_calls.iter().enumerate() {
        wb.set_formula_text(s, row as u32, 2, formula).unwrap();
    }
    for (row, formula) in array_calls.iter().enumerate() {
        wb.set_formula_text(s, row as u32, 3, formula).unwrap();
    }
    let mut eng = engine();
    eng.recalc_full(&mut wb);
    for (row, (_, expected)) in range_calls.iter().enumerate() {
        assert_eq!(display(&wb, row as u32, 2), *expected);
        assert_eq!(display(&wb, row as u32, 3), "#VALUE!");
    }
}

#[test]
fn isformula_and_cell_on_references() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_number(s, 0, 0, 7.0).unwrap();
    wb.set_formula_text(s, 1, 0, "=A1+1").unwrap();
    wb.set_formula_text(s, 0, 1, "=ISFORMULA(A1)").unwrap();
    wb.set_formula_text(s, 1, 1, "=ISFORMULA(A2)").unwrap();
    wb.set_formula_text(s, 2, 1, "=ISREF(A1)").unwrap();
    wb.set_formula_text(s, 3, 1, "=CELL(\"contents\",A1)")
        .unwrap();
    wb.set_formula_text(s, 4, 1, "=CELL(\"row\",A2)").unwrap();
    let mut eng = engine();
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 1), "FALSE");
    assert_eq!(display(&wb, 1, 1), "TRUE");
    assert_eq!(display(&wb, 2, 1), "TRUE");
    assert_eq!(display(&wb, 3, 1), "7");
    assert_eq!(display(&wb, 4, 1), "2");
}

#[test]
fn cell_without_reference_uses_last_changed_session_cell() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_number(s, 2, 3, 42.0).unwrap();
    wb.set_formula_text(s, 0, 0, "=CELL(\"address\")").unwrap();
    wb.set_formula_text(s, 0, 1, "=CELL(\"contents\")").unwrap();

    let mut eng = engine();
    eng.notify_edit(&wb, CellCoord::new(s, 2, 3));
    eng.recalc_full(&mut wb);

    assert_eq!(display(&wb, 0, 0), "$D$3");
    assert_eq!(display(&wb, 0, 1), "42");
}

fn scalar_from_byte(byte: u8) -> Scalar {
    match byte % 6 {
        0 => Scalar::Empty,
        1 => Scalar::Number(f64::from(i8::from_le_bytes([byte]))),
        2 => Scalar::Bool(byte & 1 == 1),
        3 => Scalar::Text(std::sync::Arc::from("x")),
        4 => Scalar::Error(omacell_core::error::ErrorKind::Value),
        _ => Scalar::Number(0.0),
    }
}

#[test]
fn fuzz_smoke_eager_functions_do_not_panic() {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    let wb = Workbook::new();
    let spill = SpillTable::new();
    let cell = CellCoord::new(wb.active_sheet(), 0, 0);
    let mut ctx = EvalCtx::new(&wb, &registry, &spill, cell, 1);
    for spec in all_specs() {
        let FnBody::Eager(eval) = spec.body else {
            continue;
        };
        for seed in 0u8..16 {
            let limit = usize::from(spec.max_args).min(4);
            let minimum = usize::from(spec.min_args).min(limit);
            let width = limit.saturating_sub(minimum).saturating_add(1);
            let count = minimum + (seed as usize % width);
            let args: Vec<ArgVal> = (0..count)
                .map(|i| {
                    let scalar = scalar_from_byte(seed.wrapping_add(i as u8));
                    let value = match seed % 3 {
                        0 => RuntimeValue::Scalar(scalar),
                        1 => RuntimeValue::array(1, 1, vec![scalar]),
                        _ => RuntimeValue::Array(std::sync::Arc::new(RuntimeArray {
                            rows: 2,
                            cols: 2,
                            values: std::sync::Arc::from([
                                scalar.clone(),
                                Scalar::Empty,
                                Scalar::Empty,
                                Scalar::Number(1.0),
                            ]),
                        })),
                    };
                    ArgVal {
                        omitted: seed & 0x80 != 0,
                        value,
                    }
                })
                .collect();
            let _ = eval(&mut ctx, &args);
        }
    }
}
