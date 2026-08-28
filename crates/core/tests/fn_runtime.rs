//! WP-05F: lazy dispatch, pass context, deterministic random, array limits.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use omacell_core::coerce::Scalar;
use omacell_core::error::ErrorKind;
use omacell_core::eval::{
    ArgVal, EvalCtx, FnDef, FnRegistry, PassEnv, RuntimeArray, RuntimeValue, format_runtime,
};
use omacell_core::formula::Expr;
use omacell_core::graph::CellCoord;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::recalc::RecalcEngine;
use omacell_core::spill::SpillTable;
use omacell_core::workbook::Workbook;

fn display(wb: &Workbook, row: u32, col: u16) -> String {
    omacell_core::recalc::format_cell(wb, wb.active_sheet(), row, col)
}

fn lazy_if(ctx: &mut EvalCtx<'_>, args: &[Option<Expr>]) -> RuntimeValue {
    let Some(Some(test_expr)) = args.first() else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let test = omacell_core::eval::eval_expr(ctx, test_expr);
    let RuntimeValue::Scalar(scalar) = ctx.materialize(test) else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let cond = omacell_core::coerce::to_bool(&scalar).unwrap_or(false);
    let branch = if cond { 1 } else { 2 };
    match args.get(branch) {
        Some(Some(expr)) => omacell_core::eval::eval_expr(ctx, expr),
        _ => RuntimeValue::Scalar(Scalar::Bool(false)),
    }
}

#[test]
fn lazy_if_skips_unselected_error_and_volatile_branch() {
    static FALSE_HITS: AtomicU32 = AtomicU32::new(0);
    fn boom(_ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
        FALSE_HITS.fetch_add(1, Ordering::SeqCst);
        RuntimeValue::error(ErrorKind::Div0)
    }

    let mut registry = FnRegistry::new();
    registry.register(FnDef::lazy("IF", 2, 3, false, lazy_if));
    registry.register(FnDef::eager(
        "BOOM",
        0,
        0,
        true,
        false,
        omacell_core::eval::ArrayLift::None,
        boom,
    ));
    registry.register(FnDef::eager(
        "ASYNC_PROBE",
        0,
        0,
        false,
        true,
        omacell_core::eval::ArrayLift::None,
        boom,
    ));

    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=IF(TRUE,1,1/0)").unwrap();
    wb.set_formula_text(s, 0, 1, "=IF(TRUE,2,BOOM())").unwrap();
    wb.set_formula_text(s, 0, 2, "=IF(FALSE,1/0,3)").unwrap();
    wb.set_formula_text(s, 0, 3, "=IF(TRUE,4,ASYNC_PROBE())")
        .unwrap();
    let mut eng = RecalcEngine::new(registry);
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 0), "1");
    assert_eq!(display(&wb, 0, 1), "2");
    assert_eq!(display(&wb, 0, 2), "3");
    assert_eq!(display(&wb, 0, 3), "4");
    assert_eq!(FALSE_HITS.load(Ordering::SeqCst), 0);

    let mut registry = FnRegistry::new();
    registry.register(FnDef::lazy("IF", 2, 3, false, lazy_if));
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=IF(FALSE,1,1/0)").unwrap();
    let mut eng = RecalcEngine::new(registry);
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 0), "#DIV/0!");
}

#[test]
fn clock_is_pass_stable_and_injectable() {
    fn now(ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
        RuntimeValue::Scalar(Scalar::Number(ctx.clock()))
    }
    let mut registry = FnRegistry::new();
    registry.register(FnDef::eager(
        "NOW",
        0,
        0,
        true,
        false,
        omacell_core::eval::ArrayLift::None,
        now,
    ));
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    for i in 0..1_000u32 {
        let row = i / 40;
        let col = (i % 40) as u16;
        wb.set_formula_text(s, row, col, "=NOW()").unwrap();
    }
    let mut eng = RecalcEngine::new(registry);
    eng.set_clock(Some(44_000.25));
    eng.recalc_full(&mut wb);
    let expected = display(&wb, 0, 0);
    assert!(expected.starts_with("44000"));
    for i in 0..1_000u32 {
        let row = i / 40;
        let col = (i % 40) as u16;
        assert_eq!(display(&wb, row, col), expected, "cell {row},{col}");
    }
}

#[test]
fn random_is_deterministic_across_thread_counts_and_changes_per_pass() {
    fn rand(ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
        RuntimeValue::Scalar(Scalar::Number(ctx.random_unit("RAND", 0)))
    }
    fn pair(_ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
        let values = args
            .iter()
            .map(|arg| match &arg.value {
                RuntimeValue::Scalar(scalar) => scalar.clone(),
                _ => Scalar::Error(ErrorKind::Value),
            })
            .collect();
        RuntimeValue::array(1, 2, values)
    }
    let mut registry = FnRegistry::new();
    registry.register(FnDef::eager(
        "RAND",
        0,
        0,
        true,
        false,
        omacell_core::eval::ArrayLift::None,
        rand,
    ));
    registry.register(FnDef::eager(
        "PAIR",
        2,
        2,
        false,
        false,
        omacell_core::eval::ArrayLift::None,
        pair,
    ));

    let build = || {
        let mut wb = Workbook::new();
        let s = wb.active_sheet();
        for i in 0..64u32 {
            wb.set_formula_text(s, i, 0, "=RAND()").unwrap();
        }
        // These coordinates collided under the old XOR packing because row
        // bit 16 overlapped column bit 0.
        wb.set_formula_text(s, 0, 1, "=RAND()").unwrap();
        wb.set_formula_text(s, 65_536, 0, "=RAND()").unwrap();
        wb.set_formula_text(s, 0, 2, "=PAIR(RAND(),RAND())")
            .unwrap();
        wb
    };

    let mut a = build();
    let mut b = build();
    let mut e1 = RecalcEngine::new(registry.clone());
    let mut e8 = RecalcEngine::new(registry.clone());
    e1.set_threads(1);
    e8.set_threads(8);
    e1.set_random_nonce(Some(0xDEAD_BEEF_CAFE_F00D));
    e8.set_random_nonce(Some(0xDEAD_BEEF_CAFE_F00D));
    e1.recalc_full(&mut a);
    e8.recalc_full(&mut b);
    let snap = |wb: &Workbook| (0..64u32).map(|i| display(wb, i, 0)).collect::<Vec<_>>();
    let s1 = snap(&a);
    assert_eq!(s1, snap(&b));
    let unique: std::collections::BTreeSet<_> = s1.iter().cloned().collect();
    assert!(unique.len() > 8, "random cells collided: {unique:?}");
    assert_ne!(display(&a, 0, 1), display(&a, 65_536, 0));
    assert_ne!(
        display(&a, 0, 2),
        display(&a, 0, 3),
        "repeated RAND calls need distinct call-site streams"
    );
    assert_eq!(display(&a, 0, 2), display(&b, 0, 2));
    assert_eq!(display(&a, 0, 3), display(&b, 0, 3));

    e1.recalc_incremental(&mut a);
    let s2 = snap(&a);
    assert_ne!(s1, s2, "new pass should change RAND");
}

#[test]
fn random_array_indices_use_distinct_streams() {
    let wb = Workbook::new();
    let registry = FnRegistry::new();
    let spill = SpillTable::new();
    let cell = CellCoord::new(wb.active_sheet(), 0, 0);
    let ctx = EvalCtx::new(&wb, &registry, &spill, cell, 1).with_pass_env(PassEnv {
        random_nonce: 7,
        ..PassEnv::default()
    });
    let values: std::collections::BTreeSet<_> = (0..32)
        .map(|index| ctx.random_unit("RANDARRAY", index).to_bits())
        .collect();
    assert_eq!(values.len(), 32);
}

#[test]
fn array_limits_reject_invalid_shapes_without_panic() {
    assert_eq!(
        RuntimeArray::try_new(0, 1, vec![]).unwrap_err(),
        ErrorKind::Num
    );
    assert_eq!(
        RuntimeArray::try_new(1, 0, vec![]).unwrap_err(),
        ErrorKind::Num
    );
    assert_eq!(
        RuntimeArray::try_new(MAX_ROWS + 1, 1, vec![Scalar::Empty]).unwrap_err(),
        ErrorKind::Num
    );
    assert_eq!(
        RuntimeArray::try_new(1, u32::from(MAX_COLS) + 1, vec![Scalar::Empty]).unwrap_err(),
        ErrorKind::Num
    );
    assert_eq!(
        RuntimeArray::checked_len(MAX_ROWS, 17).unwrap_err(),
        ErrorKind::Num
    );
    assert_eq!(
        RuntimeArray::try_new(2, 2, vec![Scalar::Empty]).unwrap_err(),
        ErrorKind::Value
    );
    assert!(RuntimeArray::try_new(1, 1, vec![Scalar::Number(1.0)]).is_ok());
    let ok = RuntimeValue::array(2, 1, vec![Scalar::Number(1.0), Scalar::Number(2.0)]);
    assert_eq!(format_runtime(&ok), "{1;2}");
    let malformed = RuntimeValue::Array(Arc::new(RuntimeArray {
        rows: 3,
        cols: 1,
        values: Arc::from(vec![Scalar::Number(1.0)]),
    }));
    assert_eq!(format_runtime(&malformed), "#VALUE!");
    let enormous = RuntimeValue::Array(Arc::new(RuntimeArray {
        rows: u32::MAX,
        cols: u32::MAX,
        values: Arc::from(Vec::new()),
    }));
    assert_eq!(format_runtime(&enormous), "#NUM!");
}

#[test]
fn malformed_function_array_is_rejected_before_spill_iteration() {
    fn malformed(_ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
        RuntimeValue::Array(Arc::new(RuntimeArray {
            rows: u32::MAX,
            cols: u32::MAX,
            values: Arc::from(Vec::new()),
        }))
    }
    let mut registry = FnRegistry::new();
    registry.register(FnDef::eager(
        "MALFORMED",
        0,
        0,
        false,
        false,
        omacell_core::eval::ArrayLift::None,
        malformed,
    ));
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=MALFORMED()").unwrap();
    let mut eng = RecalcEngine::new(registry);
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 0), "#NUM!");
}

#[test]
fn valid_sequence_spills() {
    fn seq(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
        let RuntimeValue::Scalar(s) = ctx.materialize(args[0].value.clone()) else {
            return RuntimeValue::error(ErrorKind::Value);
        };
        let n = omacell_core::coerce::to_number(&s).unwrap() as u32;
        let values: Vec<_> = (1..=n).map(|i| Scalar::Number(f64::from(i))).collect();
        RuntimeValue::array(n, 1, values)
    }
    let mut registry = FnRegistry::new();
    registry.register(FnDef::eager(
        "SEQUENCE",
        1,
        1,
        false,
        false,
        omacell_core::eval::ArrayLift::None,
        seq,
    ));
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=SEQUENCE(3)").unwrap();
    let mut eng = RecalcEngine::new(registry);
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 0), "1");
    assert_eq!(display(&wb, 1, 0), "2");
    assert_eq!(display(&wb, 2, 0), "3");
}

#[test]
fn locale_is_visible_on_eval_ctx() {
    fn loc(ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
        RuntimeValue::Scalar(Scalar::Number(f64::from(ctx.locale().lcid())))
    }
    let mut registry = FnRegistry::new();
    registry.register(FnDef::eager(
        "LOC",
        0,
        0,
        false,
        false,
        omacell_core::eval::ArrayLift::None,
        loc,
    ));
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=LOC()").unwrap();
    let mut eng = RecalcEngine::new(registry);
    eng.set_locale(omacell_core::locale::LocaleId::DE_DE);
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 0), "1031");
}
