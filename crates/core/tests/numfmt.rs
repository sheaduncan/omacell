//! WP-06 corpus: number formats, General, locales, builtin ids.

use std::path::PathBuf;

use omacell_core::dates::DateSystem;
use omacell_core::error::ErrorKind;
use omacell_core::locale::LocaleId;
use omacell_core::numfmt::{
    FormatOptions, FormatValue, MAX_FORMAT_LEN, builtin_format, format_with, general_for_width,
    parse,
};
use omacell_core::style::NumFmtId;

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

fn parse_locale(s: &str) -> LocaleId {
    LocaleId::parse_tag(s).unwrap_or(LocaleId::EN_US)
}

fn parse_system(s: &str) -> DateSystem {
    match s {
        "1904" => DateSystem::Excel1904,
        _ => DateSystem::Excel1900,
    }
}

fn run_format_file(rel: &str) {
    let rows = read_tsv(&corpus(rel));
    assert!(!rows.is_empty(), "{rel} is empty");
    let mut failures = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        assert!(row.len() >= 6, "{rel}:{i} {row:?}");
        let value_s = &row[0];
        let fmt = &row[1];
        let locale = parse_locale(&row[2]);
        let system = parse_system(&row[3]);
        let want = &row[4];
        let note = row.get(5).map(String::as_str).unwrap_or("");
        let value = match value_s.as_str() {
            "TRUE" => FormatValue::Bool(true),
            "FALSE" => FormatValue::Bool(false),
            "" => FormatValue::Empty,
            s if ErrorKind::from_display(s).is_some() => {
                FormatValue::Error(ErrorKind::from_display(s).unwrap())
            }
            s if s.starts_with('"') => FormatValue::Text(s.trim_matches('"')),
            s if s == "-0" || s == "-0.0" => FormatValue::Number(-0.0),
            s => match s.parse::<f64>() {
                Ok(n) => FormatValue::Number(n),
                Err(_) => FormatValue::Text(s),
            },
        };
        let opts = FormatOptions {
            locale,
            date_system: system,
            width: None,
        };
        let got = format_with(value, fmt, &opts);
        if got.text != *want {
            failures.push(format!(
                "{rel}:{} value={value_s:?} fmt={fmt:?} locale={} sys={} got={:?} want={want:?} ({note})",
                i + 2,
                row[2],
                row[3],
                got.text
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn corpus_general() {
    run_format_file("numfmt/general.tsv");
}
#[test]
fn corpus_numbers() {
    run_format_file("numfmt/numbers.tsv");
}
#[test]
fn corpus_sections() {
    run_format_file("numfmt/sections.tsv");
}
#[test]
fn corpus_scientific() {
    run_format_file("numfmt/scientific.tsv");
}
#[test]
fn corpus_fractions() {
    run_format_file("numfmt/fractions.tsv");
}
#[test]
fn corpus_dates() {
    run_format_file("numfmt/dates.tsv");
}
#[test]
fn corpus_dates_boundary() {
    run_format_file("numfmt/dates_boundary.tsv");
}
#[test]
fn corpus_times() {
    run_format_file("numfmt/times.tsv");
}
#[test]
fn corpus_text_bool_error() {
    run_format_file("numfmt/text_bool_error.tsv");
}
#[test]
fn corpus_locales() {
    run_format_file("numfmt/locales.tsv");
}
#[test]
fn corpus_misc() {
    run_format_file("numfmt/misc.tsv");
}

#[test]
fn corpus_has_at_least_400_format_rows() {
    let mut n = 0usize;
    for name in [
        "general",
        "numbers",
        "sections",
        "scientific",
        "fractions",
        "dates",
        "dates_boundary",
        "times",
        "text_bool_error",
        "locales",
        "misc",
    ] {
        n += read_tsv(&corpus(&format!("numfmt/{name}.tsv"))).len();
    }
    assert!(n >= 400, "only {n} format corpus rows");
}

#[test]
fn builtin_ids_0_49() {
    let rows = read_tsv(&corpus("numfmt/builtin.tsv"));
    assert!(rows.len() >= 50, "need every id 0–49");
    let mut seen = [false; 50];
    for row in &rows {
        let id: u32 = row[0].parse().unwrap();
        let locale = parse_locale(&row[1]);
        let want = &row[2];
        let note = row.get(3).map(String::as_str).unwrap_or("");
        let got = builtin_format(id, locale).unwrap_or_else(|| panic!("id {id} ({note})"));
        assert_eq!(got.as_ref(), want, "id {id} {} ({note})", row[1]);
        if locale == LocaleId::EN_US && id < 50 {
            seen[id as usize] = true;
        }
    }
    for (id, ok) in seen.iter().enumerate() {
        assert!(ok, "missing en-US builtin id {id}");
    }
    assert_eq!(NumFmtId::GENERAL.index(), 0);
}

#[test]
fn parse_caps_length() {
    let long = "0".repeat(MAX_FORMAT_LEN + 1);
    assert!(parse(&long).is_err());
    assert!(parse("General").is_ok());
}

#[test]
fn parse_never_panics_on_junk() {
    for s in ["", "[", "[[", "\"", ";;;;;;;;;;;", "*_*_[$-zzzz]", "@@@"] {
        let _ = parse(s);
    }
}

#[test]
fn malformed_format_delimiters_are_rejected() {
    for code in [
        "[Red",
        "Red]",
        "\"unterminated",
        "0;0;0;@;extra",
        "0\\",
        "0_",
        "0*",
    ] {
        assert!(parse(code).is_err(), "unexpectedly accepted {code:?}");
    }
}

#[test]
fn general_for_width_never_exceeds_the_budget() {
    assert_eq!(general_for_width(1.0e100, 3), "###");
    assert!(general_for_width(12.345, 5).chars().count() <= 5);
}
