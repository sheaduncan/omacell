//! WP-19 audit, find/replace, and explanation corpora.

use std::path::PathBuf;

use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::audit::{audit_workbook, eval_steps, explain_error};
use omacell_core::eval::FnRegistry;
use omacell_core::find::{
    FindSpec, GotoKind, find_cells, goto_special, replace_apply, replace_preview,
};
use omacell_core::graph::CellCoord;
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::ops::merge;
use omacell_core::recalc::RecalcEngine;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;

fn corpus(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(rel)
}

fn range(r0: u32, c0: u16, r1: u32, c1: u16) -> RangeRef {
    RangeRef::from_corners(CellRef::new(r0, c0).unwrap(), CellRef::new(r1, c1).unwrap())
}

fn seeded() -> (Workbook, RecalcEngine) {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_number(s, 1, 0, 2.0).unwrap();
    wb.set_number(s, 2, 0, 3.0).unwrap();
    wb.set_cell_contents(s, 0, 1, "=A1+2").unwrap();
    wb.set_cell_contents(s, 0, 2, "=A1+1").unwrap();
    wb.set_cell_contents(s, 1, 2, "=A2+1").unwrap();
    wb.set_cell_contents(s, 2, 2, "=A3+1").unwrap();
    wb.set_cell_contents(s, 3, 2, "=A4+99").unwrap();
    wb.set_cell_contents(s, 0, 3, "=SUM(A1:A2)").unwrap();
    wb.define_name(DefinedName {
        name: "Orphan".into(),
        scope: NameScope::Workbook,
        referent: NameReferent::Constant(Value::Number(1.0)),
        comment: None,
    })
    .unwrap();
    wb.set_cell_contents(s, 0, 4, "=E2").unwrap();
    wb.set_cell_contents(s, 1, 4, "=E1").unwrap();
    wb.set_cell_contents(s, 0, 5, "=[Other.xlsx]Sheet1!A1")
        .unwrap();
    wb.set_cell_contents(s, 0, 6, "=NOW()").unwrap();
    wb.set_text(s, 9, 0, "Item").unwrap();
    wb.set_text(s, 10, 0, "a").unwrap();
    wb.create_table(s, range(9, 0, 11, 0), "Sales").unwrap();
    merge(&mut wb, s, range(9, 0, 9, 1)).unwrap();
    let mut engine = RecalcEngine::new(FnRegistry::new());
    engine.recalc_rebuild(&mut wb);
    (wb, engine)
}

fn read_expected() -> Vec<(String, String, String, String)> {
    let text = std::fs::read_to_string(corpus("audit/checks.tsv")).unwrap();
    text.lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#') && !t.starts_with("id\t")
        })
        .map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (
                c[0].to_string(),
                c[1].to_string(),
                c[2].to_string(),
                c[3].to_string(),
            )
        })
        .collect()
}

#[test]
fn seeded_defect_corpus_has_full_precision_and_recall() {
    let (wb, engine) = seeded();
    let report = audit_workbook(&wb, &engine);
    let expected = read_expected();
    let got: Vec<(String, String, String)> = report
        .findings
        .iter()
        .map(|f| (f.id.clone(), f.sheet.clone(), f.cell_ref.clone()))
        .collect();
    let want: Vec<(String, String, String)> = expected
        .iter()
        .map(|(id, _, sheet, r)| (id.clone(), sheet.clone(), r.clone()))
        .collect();
    for row in &want {
        assert!(got.contains(row), "missing finding {row:?} in {got:?}");
    }
    for row in &got {
        assert!(want.contains(row), "unexpected finding {row:?}");
    }
    assert_eq!(got.len(), want.len());
}

#[test]
fn explanations_corpus_needles() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_cell_contents(s, 0, 0, "=NO_SUCH_FN(1)").unwrap();
    wb.set_cell_contents(s, 1, 0, "=#REF!").unwrap();
    wb.set_cell_contents(s, 2, 0, "=1/0").unwrap();
    wb.set_cell_contents(s, 3, 0, "={1;2;3}").unwrap();
    wb.set_number(s, 4, 0, 1.0).unwrap();
    let mut engine = RecalcEngine::new(FnRegistry::new());
    engine.recalc_rebuild(&mut wb);
    let name = explain_error(&wb, &engine, CellCoord::new(s, 0, 0)).unwrap();
    assert!(name.message.contains("NO_SUCH_FN"), "{}", name.message);
    let rf = explain_error(&wb, &engine, CellCoord::new(s, 1, 0)).unwrap();
    assert!(rf.message.contains("#REF!"), "{}", rf.message);
    let div = explain_error(&wb, &engine, CellCoord::new(s, 2, 0)).unwrap();
    assert!(
        div.message.to_lowercase().contains("divisor") || div.kind == "#DIV/0!",
        "{}",
        div.message
    );
    let spill = explain_error(&wb, &engine, CellCoord::new(s, 3, 0));
    if let Some(exp) = spill {
        assert!(
            exp.message.to_lowercase().contains("block"),
            "{}",
            exp.message
        );
    }
}

#[test]
fn eval_steps_include_intermediates() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 2.0).unwrap();
    wb.set_cell_contents(s, 0, 1, "=A1+3").unwrap();
    let mut engine = RecalcEngine::new(FnRegistry::new());
    engine.recalc_rebuild(&mut wb);
    let steps = eval_steps(&wb, &engine, CellCoord::new(s, 0, 1));
    assert!(steps.iter().any(|st| st.expr.contains('3')));
}

#[test]
fn find_replace_preview_equals_applied() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "foo").unwrap();
    wb.set_text(s, 1, 0, "foo bar").unwrap();
    wb.set_text(s, 2, 0, "baz").unwrap();
    let spec = FindSpec {
        query: "foo".into(),
        ..FindSpec::default()
    };
    let preview = replace_preview(&wb, s, &spec, "qux").unwrap();
    let applied = replace_apply(&mut wb, s, &spec, "qux").unwrap();
    assert_eq!(preview, applied);
    assert_eq!(applied, 2);
}

#[test]
fn regex_pathological_pattern_times_out_cleanly() {
    let spec = FindSpec {
        query: "a".repeat(300),
        regex: true,
        ..FindSpec::default()
    };
    let wb = Workbook::new();
    let err = find_cells(&wb, wb.active_sheet(), &spec).unwrap_err();
    assert_eq!(err.code, "find.timeout");
}

#[test]
fn goto_special_blanks_and_formulas() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_cell_contents(s, 0, 1, "=A1").unwrap();
    let formulas = goto_special(&wb, s, GotoKind::Formulas, false).unwrap();
    assert_eq!(formulas.len(), 1);
    let numbers = goto_special(&wb, s, GotoKind::Numbers, false).unwrap();
    assert_eq!(numbers.len(), 1);
}
