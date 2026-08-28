//! WP-05b: locale matrix, pass-stable clock, regex limits, fuzz smoke.

use omacell_core::coerce::Scalar;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, FnRegistry, RuntimeValue};
use omacell_core::graph::CellCoord;
use omacell_core::locale::LocaleId;
use omacell_core::recalc::{RecalcEngine, format_cell};
use omacell_core::spill::SpillTable;
use omacell_core::workbook::{DateSystem, Workbook};
use omacell_fn::{all_specs, register_all};

fn eval_formula(formula: &str, locale: LocaleId, date_system: DateSystem, clock: f64) -> String {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    let mut wb = Workbook::new();
    wb.settings_mut().date_system = date_system;
    let sheet = wb.active_sheet();
    wb.set_formula_text(sheet, 0, 0, formula).unwrap();
    let mut engine = RecalcEngine::new(registry);
    engine.set_clock(Some(clock));
    engine.set_locale(locale);
    engine.recalc_full(&mut wb);
    format_cell(&wb, sheet, 0, 0)
}

#[test]
fn locale_matrix_text_value_datevalue() {
    let clock = 45_000.5;
    assert_eq!(
        eval_formula(
            r#"=VALUE("1/2/2024")"#,
            LocaleId::EN_US,
            DateSystem::Excel1900,
            clock
        ),
        "45293"
    );
    assert_eq!(
        eval_formula(
            r#"=VALUE("1/2/2024")"#,
            LocaleId::EN_GB,
            DateSystem::Excel1900,
            clock
        ),
        "45323"
    );
    assert_eq!(
        eval_formula(
            r#"=DATEVALUE("1.2.2024")"#,
            LocaleId::DE_DE,
            DateSystem::Excel1900,
            clock
        ),
        "45323"
    );
    assert_eq!(
        eval_formula(
            r##"=TEXT(1234.5, "#,##0.00")"##,
            LocaleId::EN_US,
            DateSystem::Excel1900,
            clock
        ),
        "1,234.50"
    );
    assert_eq!(
        eval_formula(
            r##"=TEXT(1234.5, "#,##0.00")"##,
            LocaleId::DE_DE,
            DateSystem::Excel1900,
            clock
        ),
        "1.234,50"
    );
    assert_eq!(
        eval_formula(
            r#"=NUMBERVALUE("1.234,56")"#,
            LocaleId::DE_DE,
            DateSystem::Excel1900,
            clock
        ),
        "1234.56"
    );
}

#[test]
fn date_system_1904_date_and_year() {
    let clock = 45_000.5;
    assert_eq!(
        eval_formula(
            "=DATE(2024,1,1)",
            LocaleId::EN_US,
            DateSystem::Excel1904,
            clock
        ),
        "43830"
    );
    assert_eq!(
        eval_formula("=YEAR(0)", LocaleId::EN_US, DateSystem::Excel1904, clock),
        "1904"
    );
    assert_eq!(
        eval_formula("=DAY(0)", LocaleId::EN_US, DateSystem::Excel1904, clock),
        "1"
    );
}

#[test]
fn now_and_today_are_pass_stable() {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    for i in 0..64u32 {
        wb.set_formula_text(s, i, 0, "=NOW()").unwrap();
        wb.set_formula_text(s, i, 1, "=TODAY()").unwrap();
    }
    let mut eng = RecalcEngine::new(registry);
    eng.set_clock(Some(12_345.75));
    eng.recalc_full(&mut wb);
    let now = format_cell(&wb, s, 0, 0);
    let today = format_cell(&wb, s, 0, 1);
    assert_eq!(now, format_cell(&wb, s, 63, 0));
    assert_eq!(today, format_cell(&wb, s, 63, 1));
    assert_eq!(today, "12345");
}

#[test]
fn regex_oversized_pattern_is_value_error() {
    let pat = "a".repeat(300);
    let formula = format!(r#"=REGEXTEST("a", "{pat}")"#);
    assert_eq!(
        eval_formula(&formula, LocaleId::EN_US, DateSystem::Excel1900, 1.0),
        "#VALUE!"
    );
    assert_eq!(
        eval_formula(
            r#"=REGEXTEST("abc", "(")"#,
            LocaleId::EN_US,
            DateSystem::Excel1900,
            1.0
        ),
        "#VALUE!"
    );
}

#[test]
fn eager_functions_do_not_panic_on_random_args() {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    let wb = Workbook::new();
    let spill = SpillTable::new();
    let cell = CellCoord::new(wb.active_sheet(), 0, 0);
    let mut ctx = EvalCtx::new(&wb, &registry, &spill, cell, 1);
    let samples = [
        Scalar::Empty,
        Scalar::Number(0.0),
        Scalar::Number(-1.0),
        Scalar::Number(1e20),
        Scalar::Bool(true),
        Scalar::Text("abc".into()),
        Scalar::Text("(".into()),
        Scalar::Error(omacell_core::error::ErrorKind::Value),
    ];
    for spec in all_specs() {
        let FnBody::Eager(eval) = spec.body else {
            continue;
        };
        let n = usize::from(spec.min_args);
        for sample in &samples {
            let args: Vec<ArgVal> = (0..n)
                .map(|_| ArgVal {
                    omitted: false,
                    value: RuntimeValue::Scalar(sample.clone()),
                })
                .collect();
            let _ = eval(&mut ctx, &args);
        }
        let omitted: Vec<ArgVal> = (0..n)
            .map(|_| ArgVal {
                omitted: true,
                value: RuntimeValue::Scalar(Scalar::Empty),
            })
            .collect();
        let _ = eval(&mut ctx, &omitted);
    }
}

#[test]
fn lotus_leap_date_parts() {
    assert_eq!(
        eval_formula(
            "=DATE(1900,2,29)",
            LocaleId::EN_US,
            DateSystem::Excel1900,
            1.0
        ),
        "60"
    );
    assert_eq!(
        eval_formula("=YEAR(60)", LocaleId::EN_US, DateSystem::Excel1900, 1.0),
        "1900"
    );
    assert_eq!(
        eval_formula("=MONTH(60)", LocaleId::EN_US, DateSystem::Excel1900, 1.0),
        "2"
    );
    assert_eq!(
        eval_formula("=DAY(60)", LocaleId::EN_US, DateSystem::Excel1900, 1.0),
        "29"
    );
    assert_eq!(
        eval_formula(
            "=DATE(1900,3,1)",
            LocaleId::EN_US,
            DateSystem::Excel1900,
            1.0
        ),
        "61"
    );
}
