//! WP-04 eval corpora: TSV formulas and omc-style workbook fixtures.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use omacell_core::addr::{RangeRef, SheetId, parse_a1, parse_a1_cell};
use omacell_core::coerce::{self, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnDef, FnRegistry, RuntimeValue};
use omacell_core::graph::CellCoord;
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::recalc::{AsyncNodeProvider, RecalcEngine, RecalcResult, format_cell};
use omacell_core::storage::CellFlags;
use omacell_core::tables::Table;
use omacell_core::value::Value;
use omacell_core::workbook::{CalcMode, Workbook};

fn corpus(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(rel)
}

fn read_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .map(ToOwned::to_owned)
        .collect()
}

fn read_tsv(path: &Path) -> Vec<Vec<String>> {
    read_lines(path)
        .into_iter()
        .filter(|line| {
            let t = line.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .map(|line| line.split('\t').map(ToOwned::to_owned).collect())
        .collect()
}

fn corpus_registry() -> FnRegistry {
    let mut r = FnRegistry::new();
    r.register(FnDef {
        name: "SUM",
        min_args: 1,
        max_args: 255,
        volatile: false,
        async_node: false,
        array_lift: omacell_core::eval::ArrayLift::None,
        eval: eval_sum,
    });
    r.register(FnDef {
        name: "IF",
        min_args: 2,
        max_args: 3,
        volatile: false,
        async_node: false,
        array_lift: omacell_core::eval::ArrayLift::None,
        eval: eval_if,
    });
    r.register(FnDef {
        name: "INDIRECT",
        min_args: 1,
        max_args: 2,
        volatile: true,
        async_node: false,
        array_lift: omacell_core::eval::ArrayLift::None,
        eval: eval_indirect,
    });
    r.register(FnDef {
        name: "NOW",
        min_args: 0,
        max_args: 0,
        volatile: true,
        async_node: false,
        array_lift: omacell_core::eval::ArrayLift::None,
        eval: eval_now,
    });
    r.register(FnDef {
        name: "AI",
        min_args: 1,
        max_args: 8,
        volatile: false,
        async_node: true,
        array_lift: omacell_core::eval::ArrayLift::None,
        eval: eval_ai_stub,
    });
    r
}

fn eval_sum(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let mut acc = 0.0;
    let mut err = None;
    let mut add = |s: Scalar| {
        if err.is_some() {
            return;
        }
        if let Some(e) = s.error() {
            err = Some(e);
            return;
        }
        if matches!(s, Scalar::Text(_) | Scalar::Empty) {
            return;
        }
        match coerce::to_number(&s) {
            Ok(n) => acc += n,
            Err(e) => err = Some(e),
        }
    };
    for a in args {
        if a.omitted {
            continue;
        }
        match &a.value {
            RuntimeValue::Ref(r) => ctx.for_each_cell(r, &mut add),
            RuntimeValue::Scalar(s) => add(s.clone()),
            RuntimeValue::Array(ar) => {
                for s in ar.values.iter() {
                    add(s.clone());
                }
            }
            RuntimeValue::Lambda(_) => return RuntimeValue::error(ErrorKind::Value),
        }
    }
    match err {
        Some(e) => RuntimeValue::error(e),
        None => RuntimeValue::Scalar(Scalar::Number(acc)),
    }
}

fn eval_if(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    if args.is_empty() {
        return RuntimeValue::error(ErrorKind::Value);
    }
    let cond = ctx.materialize(args[0].value.clone());
    let b = match cond {
        RuntimeValue::Scalar(s) => match coerce::to_bool(&s) {
            Ok(v) => v,
            Err(e) => return RuntimeValue::error(e),
        },
        other => {
            if let Some(e) = other.error_kind() {
                return RuntimeValue::error(e);
            }
            return RuntimeValue::error(ErrorKind::Value);
        }
    };
    if b {
        args.get(1)
            .map(|a| ctx.materialize(a.value.clone()))
            .unwrap_or(RuntimeValue::Scalar(Scalar::Bool(true)))
    } else if let Some(a) = args.get(2) {
        ctx.materialize(a.value.clone())
    } else {
        RuntimeValue::Scalar(Scalar::Bool(false))
    }
}

fn eval_indirect(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let v = ctx.materialize(args[0].value.clone());
    let RuntimeValue::Scalar(s) = v else {
        return RuntimeValue::error(ErrorKind::Ref);
    };
    let text = match coerce::to_text(&s) {
        Ok(t) => t,
        Err(e) => return RuntimeValue::error(e),
    };
    let parsed = match parse_a1(&text) {
        Ok(p) => p,
        Err(_) => return RuntimeValue::error(ErrorKind::Ref),
    };
    let kind = match ctx.workbook().resolve_parsed(parsed) {
        Ok(k) => k,
        Err(_) => return RuntimeValue::error(ErrorKind::Ref),
    };
    let r = match kind {
        omacell_core::addr::RefKind::Cell(c) => {
            let sheet = c.sheet.unwrap_or(ctx.coord().sheet);
            omacell_core::eval::Reference::Range {
                sheet,
                start_row: c.row,
                start_col: c.col,
                end_row: c.row,
                end_col: c.col,
            }
        }
        omacell_core::addr::RefKind::Range(range) => {
            let sheet = range.start.sheet.unwrap_or(ctx.coord().sheet);
            omacell_core::eval::Reference::Range {
                sheet,
                start_row: range.start.row,
                start_col: range.start.col,
                end_row: range.end.row,
                end_col: range.end.col,
            }
        }
    };
    ctx.record_dynamic_ref(r.clone());
    RuntimeValue::Ref(r)
}

fn eval_now(ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    RuntimeValue::Scalar(Scalar::Number(f64::from(ctx.pass())))
}

fn eval_ai_stub(_ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    RuntimeValue::error(ErrorKind::Na)
}

fn engine() -> RecalcEngine {
    RecalcEngine::new(corpus_registry())
}

fn display(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> String {
    format_cell(wb, sheet, row, col)
}

fn values_match(got: &str, expected: &str) -> bool {
    let exp = expected.trim();
    if exp.is_empty() || exp == "(empty)" {
        return got.is_empty();
    }
    if exp.eq_ignore_ascii_case("TRUE") || exp.eq_ignore_ascii_case("FALSE") {
        return got.eq_ignore_ascii_case(exp);
    }
    if exp.starts_with('#') {
        return got == exp;
    }
    if let Some(inner) = exp.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return got == inner;
    }
    if exp.starts_with('{') {
        return got == exp;
    }
    if let (Ok(e), Ok(g)) = (exp.parse::<f64>(), got.parse::<f64>()) {
        return e == g || (e.is_finite() && g.is_finite() && (e - g).abs() <= 1e-9);
    }
    got == exp
}

fn parse_cell_token(tok: &str, default_sheet: SheetId, wb: &Workbook) -> (SheetId, u32, u16) {
    if let Some((sh, rest)) = tok.split_once('!') {
        let id = wb.resolve_sheet_name(sh).unwrap_or(default_sheet);
        let c = parse_a1_cell(rest).unwrap_or_else(|_| panic!("bad cell {tok}"));
        (id, c.row, c.col)
    } else {
        let c = parse_a1_cell(tok).unwrap_or_else(|_| panic!("bad cell {tok}"));
        (default_sheet, c.row, c.col)
    }
}

fn set_literal_or_formula(wb: &mut Workbook, sheet: SheetId, row: u32, col: u16, rest: &str) {
    let rest = rest.trim();
    if rest == "(empty)" {
        let _ = wb.clear_cell(sheet, row, col);
        return;
    }
    if rest.starts_with('=') {
        wb.set_formula_text(sheet, row, col, rest)
            .unwrap_or_else(|e| panic!("set formula {rest}: {e}"));
        return;
    }
    if rest.eq_ignore_ascii_case("TRUE") {
        let _ = wb.set_slot(
            sheet,
            row,
            col,
            omacell_core::storage::CellSlot {
                value: Value::Bool(true),
                formula: None,
                style: omacell_core::style::StyleId::DEFAULT,
                flags: CellFlags::DEFAULT,
            },
        );
        return;
    }
    if rest.eq_ignore_ascii_case("FALSE") {
        let _ = wb.set_slot(
            sheet,
            row,
            col,
            omacell_core::storage::CellSlot {
                value: Value::Bool(false),
                formula: None,
                style: omacell_core::style::StyleId::DEFAULT,
                flags: CellFlags::DEFAULT,
            },
        );
        return;
    }
    if rest.starts_with('#') {
        if let Some(e) = ErrorKind::from_display(rest) {
            let _ = wb.set_slot(
                sheet,
                row,
                col,
                omacell_core::storage::CellSlot {
                    value: Value::Error(e),
                    formula: None,
                    style: omacell_core::style::StyleId::DEFAULT,
                    flags: CellFlags::DEFAULT,
                },
            );
            return;
        }
    }
    if let Some(inner) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        let _ = wb.set_text(sheet, row, col, inner);
        return;
    }
    if let Ok(n) = rest.parse::<f64>() {
        let _ = wb.set_number(sheet, row, col, n);
        return;
    }
    let _ = wb.set_text(sheet, row, col, rest);
}

struct Expect {
    sheet: SheetId,
    row: u32,
    col: u16,
    value: String,
}

fn run_omc(path: &Path) {
    let mut wb = Workbook::new();
    wb.undo_log_mut().set_enabled(false);
    let mut current = wb.active_sheet();
    let mut expects = Vec::new();
    let mut circular = Vec::new();
    let mut volatile = Vec::new();
    let mut first_sheet = true;
    let mut eng = engine();
    let mut pending_recalc = false;

    let flush = |wb: &mut Workbook,
                 eng: &mut RecalcEngine,
                 expects: &mut Vec<Expect>,
                 circular: &mut Vec<CellCoord>,
                 path: &Path| {
        let result = eng.recalc_full(wb);
        check_expects(path, wb, expects);
        if !circular.is_empty() {
            let mut got = result.circular.clone();
            got.sort();
            let mut want = circular.clone();
            want.sort();
            assert_eq!(got, want, "{} circular set", path.display());
        }
        expects.clear();
        circular.clear();
    };

    for (i, line) in read_lines(path).into_iter().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("").trim();
        match cmd {
            "sheet" => {
                if first_sheet && rest.eq_ignore_ascii_case("Sheet1") {
                    current = wb.active_sheet();
                    first_sheet = false;
                } else if let Some(id) = wb.sheet_by_name(rest).map(|s| s.id) {
                    current = id;
                    first_sheet = false;
                } else {
                    current = wb.add_sheet(rest).unwrap_or_else(|e| panic!("sheet: {e}"));
                    first_sheet = false;
                }
                pending_recalc = true;
            }
            "set" => {
                let (addr, value) = rest
                    .split_once(char::is_whitespace)
                    .unwrap_or_else(|| panic!("line {}: set needs address and value", i + 1));
                let (sid, row, col) = parse_cell_token(addr, current, &wb);
                current = sid;
                set_literal_or_formula(&mut wb, sid, row, col, value);
                pending_recalc = true;
            }
            "flag" => {
                let mut it = rest.split_whitespace();
                let addr = it.next().expect("flag addr");
                let kind = it.next().unwrap_or("");
                let (sid, row, col) = parse_cell_token(addr, current, &wb);
                if kind == "array"
                    && let Ok(Some(mut slot)) = wb.get(sid, row, col).map(|s| s.copied())
                {
                    slot.flags = slot.flags.with(CellFlags::ARRAY, true);
                    let _ = wb.set_slot(sid, row, col, slot);
                }
                pending_recalc = true;
            }
            "table" => {
                let mut it = rest.split_whitespace();
                let name = it.next().expect("table name");
                let range = it.next().expect("table range");
                let headers: Vec<String> = it.map(ToOwned::to_owned).collect();
                let parsed = parse_a1(range).unwrap_or_else(|e| panic!("table range: {e}"));
                let omacell_core::addr::RefKind::Range(r) = parsed.kind else {
                    panic!("table range must be a range");
                };
                let mut t = Table::new(
                    omacell_core::tables::TableId::new(0),
                    name,
                    current,
                    r.start.row,
                    r.start.col,
                    r.end.row,
                    r.end.col,
                );
                if !headers.is_empty() {
                    t.columns = headers
                        .into_iter()
                        .map(|n| omacell_core::tables::TableColumn { name: n })
                        .collect();
                }
                wb.add_table(t).unwrap_or_else(|e| panic!("table: {e}"));
                pending_recalc = true;
            }
            "name" => {
                let (nm, referent) = rest
                    .split_once(char::is_whitespace)
                    .unwrap_or_else(|| panic!("name needs referent"));
                let referent = parse_referent(&wb, current, referent);
                wb.define_name(DefinedName {
                    name: nm.to_string(),
                    scope: NameScope::Workbook,
                    referent,
                    comment: None,
                })
                .unwrap_or_else(|e| panic!("define name: {e}"));
                pending_recalc = true;
            }
            "settings" => apply_settings(&mut wb, rest),
            "expect" => {
                let (addr, value) = rest
                    .split_once(char::is_whitespace)
                    .unwrap_or_else(|| panic!("expect needs value"));
                let (sid, row, col) = parse_cell_token(addr, current, &wb);
                expects.push(Expect {
                    sheet: sid,
                    row,
                    col,
                    value: value.to_string(),
                });
            }
            "expect_circular" => {
                for tok in rest.split_whitespace() {
                    let (sid, row, col) = parse_cell_token(tok, current, &wb);
                    circular.push(CellCoord::new(sid, row, col));
                }
            }
            "expect_volatile" => {
                for tok in rest.split_whitespace() {
                    let (sid, row, col) = parse_cell_token(tok, current, &wb);
                    volatile.push(CellCoord::new(sid, row, col));
                }
            }
            "expect_stale" => {}
            "recalc" => {
                flush(&mut wb, &mut eng, &mut expects, &mut circular, path);
                pending_recalc = false;
            }
            other => panic!("{}: unknown command {other}", i + 1),
        }
    }
    if pending_recalc || !expects.is_empty() || !circular.is_empty() {
        flush(&mut wb, &mut eng, &mut expects, &mut circular, path);
    }
    if !volatile.is_empty() {
        let before: Vec<String> = volatile
            .iter()
            .map(|c| display(&wb, c.sheet, c.row, c.col))
            .collect();
        let _ = eng.recalc_incremental(&mut wb);
        for (c, old) in volatile.iter().zip(before) {
            let now = display(&wb, c.sheet, c.row, c.col);
            assert_ne!(now, old, "{} should be volatile", path.display());
        }
    }
}

fn parse_referent(wb: &Workbook, sheet: SheetId, s: &str) -> NameReferent {
    let s = s.trim();
    if s.starts_with('=') {
        return NameReferent::Formula(s.to_string());
    }
    if s.eq_ignore_ascii_case("TRUE") {
        return NameReferent::Constant(Value::Bool(true));
    }
    if s.eq_ignore_ascii_case("FALSE") {
        return NameReferent::Constant(Value::Bool(false));
    }
    if let Some(inner) = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        // Constant text needs an intern id — store as formula `"text"`.
        return NameReferent::Formula(format!("=\"{inner}\""));
    }
    if let Ok(n) = s.parse::<f64>() {
        return NameReferent::Constant(Value::Number(n));
    }
    if let Ok(p) = parse_a1(s) {
        match wb.resolve_parsed(p) {
            Ok(omacell_core::addr::RefKind::Cell(c)) => {
                let mut r = RangeRef::from_corners(c, c);
                r.start.sheet = Some(c.sheet.unwrap_or(sheet));
                r.end.sheet = r.start.sheet;
                NameReferent::Range(r)
            }
            Ok(omacell_core::addr::RefKind::Range(mut r)) => {
                if r.start.sheet.is_none() {
                    r.start.sheet = Some(sheet);
                    r.end.sheet = Some(sheet);
                }
                NameReferent::Range(r)
            }
            Err(_) => NameReferent::Formula(format!("={s}")),
        }
    } else {
        NameReferent::Formula(format!("={s}"))
    }
}

fn apply_settings(wb: &mut Workbook, rest: &str) {
    let mut it = rest.split_whitespace();
    let key = it.next().unwrap_or("");
    match key {
        "iteration" => {
            let on = it.next().is_some_and(|s| s == "on");
            wb.settings_mut().iteration.enabled = on;
            for tok in it {
                if let Some(v) = tok.strip_prefix("max_iterations=")
                    && let Ok(n) = v.parse()
                {
                    wb.settings_mut().iteration.max_iterations = n;
                }
                if let Some(v) = tok.strip_prefix("max_change=")
                    && let Ok(n) = v.parse()
                {
                    wb.settings_mut().iteration.max_change = n;
                }
            }
        }
        "calc_mode" => {
            wb.settings_mut().calc_mode = match it.next().unwrap_or("") {
                "manual" => CalcMode::Manual,
                "automatic_except_tables" => CalcMode::AutomaticExceptTables,
                _ => CalcMode::Automatic,
            };
        }
        _ => {}
    }
}

fn check_expects(path: &Path, wb: &Workbook, expects: &[Expect]) {
    let mut failed = Vec::new();
    for e in expects {
        let got = display(wb, e.sheet, e.row, e.col);
        if !values_match(&got, &e.value) {
            failed.push(format!(
                "{} r{}c{}: got {got:?} expected {:?}",
                e.sheet.index(),
                e.row,
                e.col,
                e.value
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "{} failures in {}:\n{}",
        failed.len(),
        path.display(),
        failed.join("\n")
    );
}

fn run_tsv(path: &Path) {
    let rows = read_tsv(path);
    let mut failed = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        assert!(row.len() >= 3, "row {}: {row:?}", i + 2);
        let formula = &row[0];
        let expected = &row[1];
        let mut wb = Workbook::new();
        wb.undo_log_mut().set_enabled(false);
        let sheet = wb.active_sheet();
        wb.set_formula_text(sheet, 0, 0, formula)
            .unwrap_or_else(|e| panic!("{formula}: {e}"));
        let mut eng = engine();
        eng.recalc_full(&mut wb);
        let got = if expected.trim().starts_with('{') {
            spill_display(&eng, &wb, sheet, 0, 0)
        } else {
            display(&wb, sheet, 0, 0)
        };
        if !values_match(&got, expected) {
            failed.push(format!(
                "row {} {formula}: got {got:?} expected {expected:?}",
                i + 2
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "{} TSV failures in {}:\n{}",
        failed.len(),
        path.display(),
        failed.join("\n")
    );
}

fn spill_display(eng: &RecalcEngine, wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> String {
    if let Some(r) = eng.spill().get(CellCoord::new(sheet, row, col)) {
        let mut cells = Vec::new();
        for dr in 0..r.rows {
            let mut row_s = Vec::new();
            for dc in 0..r.cols {
                row_s.push(display(wb, sheet, row + dr, col + dc as u16));
            }
            cells.push(row_s.join(","));
        }
        return format!("{{{}}}", cells.join(";"));
    }
    display(wb, sheet, row, col)
}

#[test]
fn coerce_corpus() {
    run_tsv(&corpus("eval/coerce.tsv"));
}

#[test]
fn operators_corpus() {
    run_tsv(&corpus("eval/operators.tsv"));
}

#[test]
fn let_lambda_corpus() {
    run_omc(&corpus("eval/let_lambda.omc.txt"));
}

#[test]
fn spill_corpus() {
    run_omc(&corpus("eval/spill.omc.txt"));
}

#[test]
fn implicit_intersection_corpus() {
    run_omc(&corpus("eval/implicit_intersection.omc.txt"));
}

#[test]
fn threed_corpus() {
    run_omc(&corpus("eval/threed.omc.txt"));
}

#[test]
fn structured_corpus() {
    run_omc(&corpus("eval/structured.omc.txt"));
}

#[test]
fn names_corpus() {
    run_omc(&corpus("eval/names.omc.txt"));
}

#[test]
fn volatile_corpus() {
    run_omc(&corpus("eval/volatile.omc.txt"));
}

#[test]
fn cycles_corpus() {
    run_omc(&corpus("eval/cycles.omc.txt"));
}

#[test]
fn cycles_iter_corpus() {
    run_omc(&corpus("eval/cycles_iter.omc.txt"));
}

#[test]
fn stub_registry_unknown_is_name() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=NOTAREALFUNC(1)").unwrap();
    let mut eng = RecalcEngine::new(FnRegistry::new());
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, s, 0, 0), "#NAME?");
}

struct CountingProvider {
    inner: omacell_core::recalc::MockAsyncProvider,
}

impl CountingProvider {
    fn new() -> Self {
        Self {
            inner: omacell_core::recalc::MockAsyncProvider::new(Value::Number(42.0)),
        }
    }
}

impl AsyncNodeProvider for CountingProvider {
    fn evaluate(
        &self,
        key: omacell_core::recalc::ContentHash,
        req: &omacell_core::recalc::AsyncRequest,
    ) -> omacell_core::recalc::AsyncState {
        self.inner.evaluate(key, req)
    }
}

#[test]
fn async_mock_second_wave() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=AI(\"hi\")").unwrap();
    wb.set_formula_text(s, 1, 0, "=A1+1").unwrap();
    let mut eng = engine();
    eng.set_async_provider(Arc::new(CountingProvider::new()));
    let r1 = eng.recalc_full(&mut wb);
    assert!(
        !r1.pending_async.is_empty()
            || display(&wb, s, 0, 0) == "#GETTING_DATA"
            || display(&wb, s, 0, 0) == "#N/A"
    );
    // Dependents of a pending AI cell should still have evaluated against the
    // pending value (GettingData / N/A).
    let r2 = eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, s, 0, 0), "42");
    assert_eq!(display(&wb, s, 1, 0), "43");
    let _ = r2;
}

#[allow(dead_code)]
fn _result_ty(_: RecalcResult) {}
