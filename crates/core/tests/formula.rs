//! WP-03: formula corpora, printer stability, rewrite, deps.

use std::path::PathBuf;

use omacell_core::formula::{
    BinOp, ExprKind, MAX_FORMULA_DEPTH, ParseOptions, PostfixOp, PrefixOp, RefStyle, RewriteOp,
    TableColumns, collect_deps, parse, parse_editor, parse_with, print, rewrite_print,
};
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, FileFailurePersistence};

fn corpus(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(rel)
}

fn read_tsv(path: &std::path::Path) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|line| {
            let t = line.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .map(|line| line.split('\t').map(ToOwned::to_owned).collect())
        .collect()
}

fn parse_mode(
    src: &str,
    mode: &str,
) -> Result<omacell_core::formula::Formula, omacell_core::formula::ParseError> {
    if mode == "r1c1" {
        parse_with(
            src,
            ParseOptions {
                style: RefStyle::R1C1,
                ..ParseOptions::default()
            },
        )
    } else {
        parse(src)
    }
}

#[test]
fn excel_unary_precedence_shapes_the_ast() {
    let negated_power = parse("=-2^2").expect("negated power");
    assert!(matches!(
        negated_power.ast.kind,
        ExprKind::Binary {
            op: BinOp::Pow,
            left,
            ..
        } if matches!(
            left.kind,
            ExprKind::Prefix {
                op: PrefixOp::Minus,
                ..
            }
        )
    ));

    let negative_exponent = parse("=2^-2").expect("negative exponent");
    assert!(matches!(
        negative_exponent.ast.kind,
        ExprKind::Binary {
            op: BinOp::Pow,
            right,
            ..
        } if matches!(
            right.kind,
            ExprKind::Prefix {
                op: PrefixOp::Minus,
                ..
            }
        )
    ));

    let negated_percent = parse("=-5%").expect("negated percent");
    assert!(matches!(
        negated_percent.ast.kind,
        ExprKind::Postfix {
            op: PostfixOp::Percent,
            expr,
        } if matches!(
            expr.kind,
            ExprKind::Prefix {
                op: PrefixOp::Minus,
                ..
            }
        )
    ));
}

#[test]
fn numeric_literals_remain_finite_and_stable_at_excel_boundary() {
    let parsed = parse("=1.7976931348623158e308").expect("largest formula result");
    let ExprKind::Number(number) = parsed.ast.kind else {
        panic!("expected a numeric literal");
    };
    assert_eq!(number, f64::MAX);
    assert!(number.is_finite());

    let canonical = print(&parsed);
    let reparsed = parse(&canonical).expect("canonical maximum must reparse");
    assert_eq!(print(&reparsed), canonical);
}

#[test]
fn structured_columns_use_excel_prefix_escapes() {
    for (formula, expected) in [
        ("=DeptSalesFYSummary['#OfItems]", "#OfItems"),
        ("=Sales[O''Brien]", "O'Brien"),
        ("=Sales['[Bracket']]", "[Bracket]"),
        ("=Sales['@Owner]", "@Owner"),
    ] {
        let parsed = parse(formula).unwrap_or_else(|error| panic!("{formula}: {error:?}"));
        let ExprKind::Structured(reference) = &parsed.ast.kind else {
            panic!("{formula}: expected a structured reference");
        };
        assert_eq!(
            reference.columns,
            Some(TableColumns::One(expected.to_string())),
            "{formula}"
        );
        assert_eq!(print(&parsed), formula);
    }
}

#[test]
fn valid_corpus() {
    let path = corpus("formulas/valid.tsv");
    let rows = read_tsv(&path);
    assert!(
        rows.len() >= 500,
        "valid.tsv has {} rows, need ≥ 500",
        rows.len()
    );
    let mut failed = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        assert!(row.len() >= 4, "row {}: {row:?}", i + 2);
        let input = &row[0];
        let expected = &row[1];
        let mode = &row[2];
        match parse_mode(input, mode) {
            Ok(f) => {
                let got = print(&f);
                if got != *expected {
                    failed.push(format!(
                        "row {} {input:?} mode={mode}: got {got:?} expected {expected:?}",
                        i + 2
                    ));
                }
                let again = parse_mode(&got, mode).map(|g| print(&g));
                if again.as_deref() != Ok(got.as_str()) {
                    failed.push(format!("row {} stability {got:?} -> {again:?}", i + 2));
                }
            }
            Err(e) => failed.push(format!(
                "row {} {input:?} parse error {} at {}",
                i + 2,
                e.error,
                e.offset
            )),
        }
        if failed.len() >= 40 {
            break;
        }
    }
    assert!(
        failed.is_empty(),
        "{} failures:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

#[test]
fn invalid_corpus() {
    let path = corpus("formulas/invalid.tsv");
    let rows = read_tsv(&path);
    assert!(
        rows.len() >= 100,
        "invalid.tsv has {} rows, need ≥ 100",
        rows.len()
    );
    let update = std::env::var("UPDATE_INVALID").is_ok();
    let mut new_rows = Vec::new();
    let mut failed = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        assert!(row.len() >= 4, "row {}: {row:?}", i + 2);
        let input = &row[0];
        let offset: usize = row[1].parse().expect("offset");
        let code = &row[2];
        let note = &row[3];
        match parse(input) {
            Ok(f) => {
                if update {
                    continue;
                }
                failed.push(format!(
                    "row {} {input:?} unexpectedly parsed as {}",
                    i + 2,
                    print(&f)
                ));
            }
            Err(e) => {
                if update {
                    new_rows.push(format!("{input}\t{}\t{}\t{note}", e.offset, e.error.code));
                } else if e.error.code != *code || e.offset != offset {
                    failed.push(format!(
                        "row {} {input:?}: offset {} expected {offset} code {} expected {code} ({})",
                        i + 2,
                        e.offset,
                        e.error.code,
                        e.error.message
                    ));
                }
            }
        }
        if !update && failed.len() >= 80 {
            break;
        }
    }
    if update {
        let mut text = String::from("# input\toffset\tcode\tnote\n");
        for r in &new_rows {
            text.push_str(r);
            text.push('\n');
        }
        std::fs::write(&path, text).unwrap();
        eprintln!("updated {} invalid rows", new_rows.len());
    }
    assert!(
        failed.is_empty(),
        "{} failures:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

#[test]
fn rewrite_corpus() {
    let path = corpus("formulas/rewrite.tsv");
    let rows = read_tsv(&path);
    assert!(!rows.is_empty(), "rewrite.tsv is empty");
    let mut failed = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        assert!(row.len() >= 7, "row {}: {row:?}", i + 2);
        let op_name = &row[0];
        let src = &row[1];
        let a1 = &row[2];
        let a2 = &row[3];
        let expected = &row[5];
        let op = match op_name.as_str() {
            "copy" => RewriteOp::Copy {
                dcol: a1.parse().expect("dcol"),
                drow: a2.parse().expect("drow"),
            },
            "move" => RewriteOp::Move {
                src: a1.clone(),
                dest: a2.clone(),
            },
            "insert_rows" => RewriteOp::InsertRows {
                at: a1.parse().expect("at"),
                count: a2.parse().expect("count"),
            },
            "delete_rows" => RewriteOp::DeleteRows {
                at: a1.parse().expect("at"),
                count: a2.parse().expect("count"),
            },
            "insert_cols" => RewriteOp::InsertCols {
                at: a1.clone(),
                count: a2.parse().expect("count"),
            },
            "delete_cols" => RewriteOp::DeleteCols {
                at: a1.clone(),
                count: a2.parse().expect("count"),
            },
            "sheet_rename" => RewriteOp::SheetRename {
                old: a1.clone(),
                new: a2.clone(),
            },
            "table_rename" => RewriteOp::TableRename {
                old: a1.clone(),
                new: a2.clone(),
            },
            other => panic!("unknown op {other}"),
        };
        match rewrite_print(src, &op) {
            Ok(got) if got == *expected => {}
            Ok(got) => failed.push(format!(
                "row {} {op_name} {src:?}: got {got:?} expected {expected:?}",
                i + 2
            )),
            Err(e) => failed.push(format!("row {} {op_name} {src:?}: {e}", i + 2)),
        }
        if failed.len() >= 30 {
            break;
        }
    }
    assert!(
        failed.is_empty(),
        "{} failures:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

#[test]
fn printer_stability_on_valid_corpus() {
    let rows = read_tsv(&corpus("formulas/valid.tsv"));
    for row in rows {
        if row.get(2).map(String::as_str) != Some("a1") {
            continue;
        }
        let input = &row[0];
        let Ok(f1) = parse(input) else {
            continue;
        };
        let p1 = print(&f1);
        let f2 = parse(&p1).unwrap_or_else(|e| panic!("{input}: {e}"));
        let p2 = print(&f2);
        assert_eq!(p1, p2, "print(parse(print(parse(x)))) for {input}");
    }
}

#[test]
fn editor_mode_keeps_partial_sum() {
    let p = parse_editor("=SUM(A1,");
    assert!(p.error.is_some());
    assert!(p.expr.is_some());
}

#[test]
fn deps_flags_volatile_and_dynamic() {
    let f = parse("=NOW()+INDIRECT(\"A1\")+A2").unwrap();
    let d = collect_deps(&f.ast);
    assert!(d.volatile);
    assert!(d.dynamic);
    assert!(!d.ranges.is_empty());
}

#[test]
fn three_d_dependencies_keep_the_sheet_span() {
    let f = parse("=Sheet1:Sheet3!A1").unwrap();
    let deps = collect_deps(&f.ast);
    assert_eq!(deps.ranges.len(), 1);
    let sheets = deps.ranges[0].0.as_ref().expect("3-D sheet span");
    assert_eq!(sheets.start, "Sheet1");
    assert_eq!(sheets.end.as_deref(), Some("Sheet3"));
}

#[test]
fn function_depth_matches_excel_and_is_independent_from_syntax_depth() {
    let nested = |levels: usize, inner: &str| {
        format!("={}{}{}", "SUM(".repeat(levels), inner, ")".repeat(levels))
    };

    let exactly_64 = nested(MAX_FORMULA_DEPTH as usize, "1");
    parse(&exactly_64).expect("Excel permits 64 nested function levels");

    let too_many = nested(MAX_FORMULA_DEPTH as usize + 1, "1");
    let err = parse(&too_many).expect_err("the 65th nested function must fail");
    assert_eq!(err.error.code, omacell_core::formula::codes::DEPTH);
    assert_eq!(err.offset, 1 + MAX_FORMULA_DEPTH as usize * 4);

    let unary_inside_63 = nested(MAX_FORMULA_DEPTH as usize - 1, "-1");
    parse(&unary_inside_63).expect("an operator does not consume a function level");

    let grouped = format!("={}1{}", "(".repeat(70), ")".repeat(70));
    parse(&grouped).expect("parentheses do not consume function levels");
}

#[test]
fn parser_recursion_limit_covers_array_prefixes() {
    let src = format!("={{{}1}}", "-".repeat(300));
    let err = parse(&src).unwrap_err();
    assert_eq!(err.error.code, omacell_core::formula::codes::DEPTH);
}

#[test]
fn r1c1_rejects_an_invalid_base_cell() {
    let err = parse_with(
        "=R",
        ParseOptions {
            style: RefStyle::R1C1,
            base_row: MAX_ROWS,
            base_col: MAX_COLS,
            lenient: false,
        },
    )
    .unwrap_err();
    assert_eq!(err.error.code, omacell_core::formula::codes::PARSE);
}

#[test]
fn rewrite_rejects_lossy_or_out_of_grid_operations() {
    assert!(
        rewrite_print(
            "=A1",
            &RewriteOp::Move {
                src: "Data!A1".into(),
                dest: "B2".into(),
            },
        )
        .is_err()
    );
    assert!(
        rewrite_print(
            "=A1",
            &RewriteOp::InsertRows {
                at: MAX_ROWS,
                count: 2,
            },
        )
        .is_err()
    );
}

fn arb_cell() -> impl Strategy<Value = String> {
    (0u32..20, 0u16..10).prop_map(|(r, c)| {
        let letters = omacell_core::addr::col_to_letters(c).unwrap();
        format!("{letters}{}", r + 1)
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        cases: 64,
        ..ProptestConfig::default()
    })]

    #[test]
    fn print_parse_print_stable(
        a in arb_cell(),
        b in arb_cell(),
        n in -100i32..100,
        op in prop::sample::select(vec!["+", "-", "*", "/", "&"])
    ) {
        let src = format!("={a}{op}{n}+{b}");
        let f1 = parse(&src).expect("parse");
        let p1 = print(&f1);
        let f2 = parse(&p1).expect("parse2");
        let p2 = print(&f2);
        prop_assert_eq!(p1, p2);
    }
}
