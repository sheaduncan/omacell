//! WP-19 audit, find/replace, and explanation corpora.

use std::path::PathBuf;

use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::audit::{audit_workbook, eval_steps, explain_error};
use omacell_core::eval::FnRegistry;
use omacell_core::find::{
    FindSpec, GotoKind, find_cells, goto_spec, goto_special, replace_apply, replace_preview,
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
    let explanations = [
        ("NAME", explain_error(&wb, &engine, CellCoord::new(s, 0, 0))),
        ("REF", explain_error(&wb, &engine, CellCoord::new(s, 1, 0))),
        ("DIV0", explain_error(&wb, &engine, CellCoord::new(s, 2, 0))),
        (
            "SPILL",
            explain_error(&wb, &engine, CellCoord::new(s, 3, 0)),
        ),
    ];
    let corpus = std::fs::read_to_string(corpus("audit/explain.tsv")).unwrap();
    for line in corpus.lines().filter(|line| {
        let line = line.trim();
        !line.is_empty() && !line.starts_with('#') && !line.starts_with("kind\t")
    }) {
        let columns: Vec<_> = line.split('\t').collect();
        let (kind, needle) = (columns[0], columns[2]);
        let explanation = explanations
            .iter()
            .find(|(candidate, _)| *candidate == kind)
            .and_then(|(_, explanation)| explanation.as_ref())
            .unwrap_or_else(|| panic!("missing {kind} explanation"));
        assert!(
            explanation
                .message
                .to_lowercase()
                .contains(&needle.to_lowercase()),
            "{kind}: {}",
            explanation.message
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

#[test]
fn find_rejects_empty_queries_and_unknown_sheets() {
    let wb = Workbook::new();
    let s = wb.active_sheet();
    let err = find_cells(&wb, s, &FindSpec::default()).unwrap_err();
    assert_eq!(err.code, "find.query");

    let err = goto_spec(&wb, "Missing!A1").unwrap_err();
    assert_eq!(err.code, "goto.spec");
}

#[test]
fn find_whole_regex_and_replace_preview_are_exact() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "foobar").unwrap();
    wb.set_text(s, 1, 0, "foo").unwrap();
    let whole_regex = FindSpec {
        query: "foo".into(),
        regex: true,
        whole: true,
        ..FindSpec::default()
    };
    let hits = find_cells(&wb, s, &whole_regex).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].row, hits[0].col), (1, 0));

    let no_op = FindSpec {
        query: "foo".into(),
        whole: true,
        ..FindSpec::default()
    };
    assert_eq!(replace_preview(&wb, s, &no_op, "foo").unwrap(), 0);
    assert_eq!(replace_apply(&mut wb, s, &no_op, "foo").unwrap(), 0);
}

#[test]
fn case_insensitive_replace_is_unicode_safe() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "İX").unwrap();
    let spec = FindSpec {
        query: "x".into(),
        ..FindSpec::default()
    };
    assert_eq!(replace_preview(&wb, s, &spec, "y").unwrap(), 1);
    assert_eq!(replace_apply(&mut wb, s, &spec, "y").unwrap(), 1);
    let slot = wb.get(s, 0, 0).unwrap().unwrap();
    let Value::Text(id) = slot.value else {
        panic!("replacement should remain text");
    };
    assert_eq!(wb.intern().strings.get(id), Some("İy"));
}

#[test]
fn formula_search_skips_non_formula_cells_even_when_regex_matches_empty() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "constant").unwrap();
    wb.set_cell_contents(s, 1, 0, "=1").unwrap();
    let spec = FindSpec {
        query: "^$".into(),
        formulas: true,
        regex: true,
        ..FindSpec::default()
    };
    assert!(find_cells(&wb, s, &spec).unwrap().is_empty());
}

#[test]
fn range_short_uses_the_referenced_column_not_the_sheet_extent() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_number(s, 1, 0, 2.0).unwrap();
    wb.set_cell_contents(s, 99, 3, "=SUM(A1:A2)").unwrap();
    let mut engine = RecalcEngine::new(FnRegistry::new());
    engine.recalc_rebuild(&mut wb);
    let report = audit_workbook(&wb, &engine);
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.id != "audit.range_short"),
        "{:?}",
        report.findings
    );
}

#[test]
fn circular_audit_reports_each_strongly_connected_set_separately() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_cell_contents(s, 0, 0, "=B1").unwrap();
    wb.set_cell_contents(s, 0, 1, "=A1").unwrap();
    wb.set_cell_contents(s, 0, 3, "=E1").unwrap();
    wb.set_cell_contents(s, 0, 4, "=D1").unwrap();
    let mut engine = RecalcEngine::new(FnRegistry::new());
    engine.recalc_rebuild(&mut wb);
    assert_eq!(
        engine.graph().circular_set(&engine.graph().formula_cells()),
        [
            CellCoord::new(s, 0, 0),
            CellCoord::new(s, 0, 1),
            CellCoord::new(s, 0, 3),
            CellCoord::new(s, 0, 4),
        ]
    );
    let report = audit_workbook(&wb, &engine);
    let a1 = report
        .findings
        .iter()
        .find(|finding| finding.id == "audit.circular" && finding.cell_ref == "A1")
        .unwrap();
    assert!(a1.message.contains("Sheet1!A1"), "{}", a1.message);
    assert!(a1.message.contains("Sheet1!B1"), "{}", a1.message);
    assert!(!a1.message.contains("Sheet1!D1"), "{}", a1.message);
}

#[test]
fn inconsistent_formula_audit_checks_contiguous_rows() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_cell_contents(s, 1, 0, "=A1").unwrap();
    wb.set_cell_contents(s, 1, 1, "=B1").unwrap();
    wb.set_cell_contents(s, 1, 2, "=C1").unwrap();
    wb.set_cell_contents(s, 1, 3, "=A1").unwrap();
    let mut engine = RecalcEngine::new(FnRegistry::new());
    engine.recalc_rebuild(&mut wb);
    let anomalies: Vec<_> = audit_workbook(&wb, &engine)
        .findings
        .into_iter()
        .filter(|finding| finding.id == "audit.inconsistent_formula")
        .map(|finding| finding.cell_ref)
        .collect();
    assert_eq!(anomalies, ["D2"]);
}

#[test]
fn unused_names_respect_sheet_scope_and_fix_scope() {
    let mut wb = Workbook::new();
    let sheet1 = wb.active_sheet();
    let sheet2 = wb.add_sheet("Second").unwrap();
    for sheet in [sheet1, sheet2] {
        wb.define_name(DefinedName {
            name: "LocalValue".into(),
            scope: NameScope::Sheet(sheet),
            referent: NameReferent::Constant(Value::Number(1.0)),
            comment: None,
        })
        .unwrap();
    }
    wb.set_cell_contents(sheet1, 0, 0, "=LocalValue").unwrap();
    let mut engine = RecalcEngine::new(FnRegistry::new());
    engine.recalc_rebuild(&mut wb);

    let unused: Vec<_> = audit_workbook(&wb, &engine)
        .findings
        .into_iter()
        .filter(|finding| finding.id == "audit.unused_name")
        .collect();
    assert_eq!(unused.len(), 1, "{unused:?}");
    assert_eq!(unused[0].sheet, "Second");
    assert_eq!(
        unused[0].fix.as_ref().unwrap().args,
        serde_json::json!({"name": "LocalValue", "sheet": "Second"})
    );
}

#[test]
fn goto_named_range_uses_the_ranges_sheet() {
    let mut wb = Workbook::new();
    let second = wb.add_sheet("Second Sheet").unwrap();
    let mut named_range = range(2, 3, 2, 3);
    named_range.start.sheet = Some(second);
    named_range.end.sheet = Some(second);
    wb.define_name(DefinedName {
        name: "Target".into(),
        scope: NameScope::Workbook,
        referent: NameReferent::Range(named_range),
        comment: None,
    })
    .unwrap();
    assert_eq!(goto_spec(&wb, "Target").unwrap(), (second, 2, 3));

    wb.define_name(DefinedName {
        name: "LocalTarget".into(),
        scope: NameScope::Sheet(second),
        referent: NameReferent::Range(range(4, 5, 4, 5)),
        comment: None,
    })
    .unwrap();
    assert_eq!(
        goto_spec(&wb, "'Second Sheet'!LocalTarget").unwrap(),
        (second, 4, 5)
    );
}
