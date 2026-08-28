//! WP-05c corpora, shape-limit, solver, and fuzz-smoke tests.

use std::path::PathBuf;
use std::sync::Arc;

use omacell_core::coerce::Scalar;
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, FnRegistry, RuntimeArray, RuntimeValue};
use omacell_core::graph::CellCoord;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::recalc::{RecalcEngine, format_cell};
use omacell_core::spill::SpillTable;
use omacell_core::workbook::Workbook;
use omacell_fn::{all_specs, register_all, run_corpus_file};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/functions")
}

const WP05C: &[&str] = &[
    "XLOOKUP",
    "XMATCH",
    "INDEX",
    "MATCH",
    "VLOOKUP",
    "HLOOKUP",
    "LOOKUP",
    "CHOOSE",
    "OFFSET",
    "INDIRECT",
    "ROW",
    "ROWS",
    "COLUMN",
    "COLUMNS",
    "ADDRESS",
    "AREAS",
    "TRANSPOSE",
    "FILTER",
    "SORT",
    "SORTBY",
    "UNIQUE",
    "SEQUENCE",
    "RANDARRAY",
    "TAKE",
    "DROP",
    "CHOOSEROWS",
    "CHOOSECOLS",
    "VSTACK",
    "HSTACK",
    "TOCOL",
    "TOROW",
    "WRAPROWS",
    "WRAPCOLS",
    "EXPAND",
    "MAP",
    "REDUCE",
    "SCAN",
    "BYROW",
    "BYCOL",
    "MAKEARRAY",
    "LET",
    "LAMBDA",
    "ISOMITTED",
    "PMT",
    "IPMT",
    "PPMT",
    "NPV",
    "XNPV",
    "IRR",
    "XIRR",
    "MIRR",
    "FV",
    "PV",
    "RATE",
    "NPER",
    "SLN",
    "DB",
    "DDB",
    "SYD",
    "EFFECT",
    "NOMINAL",
    "CUMIPMT",
    "CUMPRINC",
    "CONVERT",
    "DEC2BIN",
    "DEC2OCT",
    "DEC2HEX",
    "BIN2DEC",
    "OCT2DEC",
    "HEX2DEC",
    "BITAND",
    "BITOR",
    "BITXOR",
    "BITLSHIFT",
    "BITRSHIFT",
    "DELTA",
    "GESTEP",
];

#[test]
fn wp05c_corpus_files_have_at_least_ten_rows_and_pass() {
    let mut failures = Vec::new();
    for name in WP05C {
        let path = corpus_dir().join(format!("{name}.tsv"));
        let results = run_corpus_file(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            results.len() >= 10,
            "{name} has {} corpus rows; need ≥ 10",
            results.len()
        );
        for (row, got) in results {
            if got != row.expected {
                failures.push(format!(
                    "{name}: {} => got {got:?} expected {:?} ({})",
                    row.formula, row.expected, row.note
                ));
            }
        }
    }
    if !failures.is_empty() {
        panic!(
            "{} corpus mismatches:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}

fn eval_formula(formula: &str) -> String {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_formula_text(sheet, 0, 0, formula).unwrap();
    let mut engine = RecalcEngine::new(registry);
    engine.set_clock(Some(45_000.5));
    engine.set_random_nonce(Some(0x1111_2222_3333_4444));
    engine.recalc_full(&mut wb);
    format_cell(&wb, sheet, 0, 0)
}

#[test]
fn sequence_randarray_makearray_reject_invalid_shapes() {
    assert_eq!(eval_formula("=SEQUENCE(0)"), "#NUM!");
    assert_eq!(eval_formula("=SEQUENCE(1048577)"), "#NUM!");
    assert_eq!(eval_formula("=SEQUENCE(1,16385)"), "#NUM!");
    assert_eq!(eval_formula("=RANDARRAY(0)"), "#NUM!");
    assert_eq!(eval_formula("=RANDARRAY(1,16385)"), "#NUM!");
    assert_eq!(eval_formula("=MAKEARRAY(0,1,LAMBDA(r,c,1))"), "#NUM!");
    assert_eq!(eval_formula("=MAKEARRAY(1048577,1,LAMBDA(r,c,1))"), "#NUM!");
    assert_eq!(eval_formula("=WRAPROWS({1,2},0)"), "#NUM!");
    assert_eq!(eval_formula("=WRAPCOLS({1,2},1048577)"), "#NUM!");
    assert_eq!(eval_formula("=EXPAND({1},1048577,1,0)"), "#NUM!");
}

#[test]
fn language_constructs_are_not_registered() {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    assert!(registry.lookup("LET").is_none());
    assert!(registry.lookup("LAMBDA").is_none());
    assert!(registry.lookup("ISOMITTED").is_none());
    assert!(registry.lookup("MAP").is_some());
    assert!(registry.lookup("SEQUENCE").is_some());
}

#[test]
fn lambda_helpers_use_evaluator_call_cap() {
    // Nested MAKEARRAY calls increment `EvalCtx` depth via `lambda::apply`.
    let formula = "=MAKEARRAY(1,1,LAMBDA(r,c,MAKEARRAY(1,1,LAMBDA(x,y,x+y))))";
    assert_eq!(eval_formula(formula), "2");
}

#[test]
fn randarray_is_deterministic_across_thread_counts() {
    fn run(threads: usize) -> String {
        let mut registry = FnRegistry::new();
        register_all(&mut registry);
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        wb.set_formula_text(sheet, 0, 0, "=RANDARRAY(1,1,0,1)")
            .unwrap();
        let mut engine = RecalcEngine::new(registry);
        engine.set_threads(threads);
        engine.set_random_nonce(Some(0xDEAD_BEEF_CAFE_BABE));
        engine.recalc_full(&mut wb);
        format_cell(&wb, sheet, 0, 0)
    }
    let a = run(1);
    let b = run(8);
    assert_eq!(a, b);
    assert_ne!(a, "");
}

#[test]
fn financial_solvers_converge_and_fail_closed() {
    assert_eq!(eval_formula("=IRR({-100,110})"), "0.1");
    assert_eq!(eval_formula("=RATE(1,-110,100)"), "0.1");
    assert_eq!(eval_formula("=XIRR({-100,110},{0,365})"), "0.1");
    assert_eq!(eval_formula("=IRR({10,20})"), "#NUM!");
    assert_eq!(eval_formula("=RATE(1,0,0,1)"), "#NUM!");
}

#[test]
fn offset_indirect_record_without_allocating_out_of_grid() {
    assert_eq!(eval_formula("=OFFSET(C2,1048576,0)"), "#REF!");
    assert_eq!(eval_formula("=INDIRECT(\"not a ref\")"), "#REF!");
    assert_eq!(eval_formula("=ROWS(B:B)"), "1048576");
    assert_eq!(eval_formula("=COLUMNS(2:2)"), "16384");
}

#[test]
fn eager_functions_do_not_panic_on_garbage_args() {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    let wb = Workbook::new();
    let spill = SpillTable::new();
    let cell = CellCoord::new(wb.active_sheet(), 0, 0);
    let mut ctx = EvalCtx::new(&wb, &registry, &spill, cell, 1);
    let junk = [
        ArgVal {
            omitted: false,
            value: RuntimeValue::Scalar(Scalar::Empty),
        },
        ArgVal {
            omitted: false,
            value: RuntimeValue::Scalar(Scalar::Text(Arc::from("nope"))),
        },
        ArgVal {
            omitted: false,
            value: RuntimeValue::Scalar(Scalar::Error(ErrorKind::Value)),
        },
        ArgVal {
            omitted: true,
            value: RuntimeValue::Scalar(Scalar::Empty),
        },
        ArgVal {
            omitted: false,
            value: RuntimeValue::array(
                2,
                2,
                vec![
                    Scalar::Number(1.0),
                    Scalar::Empty,
                    Scalar::Bool(true),
                    Scalar::Error(ErrorKind::Na),
                ],
            ),
        },
        ArgVal {
            omitted: false,
            value: RuntimeValue::Array(Arc::new(RuntimeArray {
                rows: 2,
                cols: 2,
                values: Arc::from([Scalar::Number(1.0)]),
            })),
        },
    ];
    for spec in all_specs() {
        if matches!(spec.body, FnBody::Lazy(_)) {
            continue;
        }
        let Some(def) = registry.lookup(spec.name) else {
            continue;
        };
        let FnBody::Eager(eval) = def.body else {
            continue;
        };
        let n = usize::from(spec.max_args).min(junk.len());
        let _ = eval(&mut ctx, &junk[..n]);
        let _ = eval(&mut ctx, &[]);
    }
}

#[test]
fn max_grid_constants_match_shape_checks() {
    assert_eq!(MAX_ROWS, 1_048_576);
    assert_eq!(MAX_COLS, 16_384);
    assert!(RuntimeArray::checked_len(MAX_ROWS + 1, 1).is_err());
    assert!(RuntimeArray::checked_len(1, u32::from(MAX_COLS) + 1).is_err());
}
