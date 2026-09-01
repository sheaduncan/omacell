//! `assert_cmd` coverage for every subcommand, including stubs.

use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("omacell").unwrap()
}

fn home() -> (TempDir, Command) {
    let dir = TempDir::new().unwrap();
    let mut cmd = bin();
    cmd.env("HOME", dir.path());
    cmd.env("XDG_RUNTIME_DIR", dir.path().join("run"));
    (dir, cmd)
}

fn corpus_xlsx() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/xlsx/l1_values.xlsx")
}

fn corpus_xls() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/xls/l1_values.xls")
}

#[test]
fn version_and_usage() {
    bin()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("omacell"));
    bin().arg("--not-a-flag").assert().code(2);

    let output = bin().args(["--json", "--not-a-flag"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "cli.usage");
    assert!(error["message"].as_str().unwrap().contains("not-a-flag"));
}

#[test]
fn ipc_all_and_socket_are_mutually_exclusive() {
    bin()
        .args(["ipc", "ping", "--all", "--socket", "/does/not/matter.sock"])
        .assert()
        .code(2);
}

#[test]
fn gui_without_display_exits_error() {
    let (_home, mut cmd) = home();
    cmd.env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .arg("book.xlsx")
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "requires a Wayland or X11 display",
        ));
}

#[test]
fn gui_rejects_ambiguous_or_ignored_arguments_before_display_setup() {
    for args in [vec!["one.xlsx", "two.xlsx"], vec!["--dry-run"]] {
        let (_home, mut cmd) = home();
        cmd.env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("cli.usage"));
    }
}

#[test]
fn tui_without_tty_exits_error() {
    let (_home, mut cmd) = home();
    cmd.args(["--tui"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("requires a terminal"));
}

#[test]
fn tui_rejects_ambiguous_or_ignored_arguments_before_tty_setup() {
    for args in [
        vec!["--tui", "one.xlsx", "two.xlsx"],
        vec!["--tui", "config", "check"],
        vec!["--tui", "--dry-run"],
    ] {
        let (_home, mut cmd) = home();
        cmd.args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("cli.usage"));
    }
}

#[test]
fn json_error_shape() {
    let (_home, mut cmd) = home();
    cmd.args(["--json", "query", "/no/such/omacell-missing.xlsx", "A1"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("\"code\""))
        .stderr(predicate::str::contains("\"message\""));
}

#[test]
fn query_xls_without_external_tools() {
    let (dir, mut cmd) = home();
    cmd.env("PATH", dir.path())
        .args(["--json", "query", corpus_xls().to_str().unwrap(), "A1:C1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1.5"))
        .stdout(predicate::str::contains("hello"));
}

#[test]
fn fn_list_and_doc() {
    let (_home, mut cmd) = home();
    cmd.args(["--json", "fn", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"schema\""))
        .stdout(predicate::str::contains("SUM"));
    let (_home, mut cmd) = home();
    cmd.args(["fn", "doc", "XLOOKUP"])
        .assert()
        .success()
        .stdout(predicate::str::contains("XLOOKUP"));
}

#[test]
fn commands_json_includes_file_and_theme() {
    let (_home, mut cmd) = home();
    cmd.args(["--json", "commands"])
        .assert()
        .success()
        .stdout(predicate::str::contains("file.open"))
        .stdout(predicate::str::contains("file.save"))
        .stdout(predicate::str::contains("file.export"))
        .stdout(predicate::str::contains("file.print"))
        .stdout(predicate::str::contains("theme.reload"));
}

#[test]
fn deferred_and_composition_commands_match_the_live_catalog() {
    let (_home, mut cmd) = home();
    let output = cmd.args(["--json", "commands"]).output().unwrap();
    assert!(output.status.success());
    let catalog: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let registered: std::collections::BTreeSet<&str> = catalog["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|command| command["id"].as_str())
        .collect();

    let stale: Vec<_> = omacell_ui::DEFERRED_COMMANDS
        .iter()
        .filter(|command| registered.contains(command.id))
        .map(|command| command.id)
        .collect();
    assert!(
        stale.is_empty(),
        "registered commands must not remain deferred: {stale:?}"
    );

    let missing_composition: Vec<_> = omacell_ui::COMPOSITION_COMMANDS
        .iter()
        .copied()
        .filter(|id| !registered.contains(id))
        .collect();
    assert!(
        missing_composition.is_empty(),
        "composition commands must be present in the live catalog: {missing_composition:?}"
    );
}

#[test]
fn query_eval_set_recalc_convert() {
    let dir = TempDir::new().unwrap();
    let book = dir.path().join("book.xlsx");
    std::fs::copy(corpus_xlsx(), &book).unwrap();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["--json", "query", book.to_str().unwrap(), "A1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("rows"));

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["eval", book.to_str().unwrap(), "=1+1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2"));

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["set", book.to_str().unwrap(), "Z1", "7"])
        .assert()
        .success();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["set", book.to_str().unwrap(), "Z2", "-1"])
        .assert()
        .success();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["recalc", book.to_str().unwrap()])
        .assert()
        .success();

    let out = dir.path().join("out.csv");
    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["convert", book.to_str().unwrap(), out.to_str().unwrap()])
        .assert()
        .success();
    assert!(out.is_file());

    let pdf = dir.path().join("out.pdf");
    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["convert", book.to_str().unwrap(), pdf.to_str().unwrap()])
        .assert()
        .success();
    let pdf_bytes = std::fs::read(&pdf).unwrap();
    assert!(pdf_bytes.starts_with(b"%PDF-"));
}

#[test]
fn query_human_csv_and_markdown_outputs_escape_cells() {
    let dir = TempDir::new().unwrap();
    let book = dir.path().join("book.csv");
    std::fs::write(
        &book,
        "\"a,b\",\"quote\"\"here\",\"pipe|slash\\\",\"line\nbreak\"\n",
    )
    .unwrap();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["query", book.to_str().unwrap(), "A1:D1", "--format", "csv"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"a,b\""))
        .stdout(predicate::str::contains("\"quote\"\"here\""))
        .stdout(predicate::str::contains("\"line\nbreak\""));

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["query", book.to_str().unwrap(), "A1:D1", "--format", "md"])
        .assert()
        .success()
        .stdout(predicate::str::contains("pipe\\|slash\\\\"))
        .stdout(predicate::str::contains("line<br>break"));

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["query", book.to_str().unwrap(), "A1"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("["))
        .stdout(predicate::str::contains("a,b"));
}

#[test]
fn convert_accepts_the_shared_csv_import_plan() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("input.csv");
    let plan = dir.path().join("plan.json");
    let output = dir.path().join("output.xlsx");
    std::fs::write(&input, "00123\n").unwrap();
    std::fs::write(
        &plan,
        r#"{"delimiter":",","columns":[{"ty":{"kind":"text"}}]}"#,
    )
    .unwrap();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args([
            "convert",
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "--plan",
            plan.to_str().unwrap(),
        ])
        .assert()
        .success();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["--json", "query", output.to_str().unwrap(), "A1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("00123"));
}

#[test]
fn theme_show_and_keys_check_and_setup() {
    let (dir, mut cmd) = home();
    cmd.args(["--json", "theme", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("theme"));

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["keys", "check"])
        .assert()
        .success();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["setup", "omarchy", "--show-hyprland"])
        .assert()
        .success()
        .stdout(predicate::str::contains("o.bind"));

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["setup", "omarchy"])
        .assert()
        .success();
    assert!(
        dir.path()
            .join(".config/omarchy/hooks/theme-set.d/omacell")
            .is_file()
    );
}

#[test]
fn diff_two_identical_copies() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.xlsx");
    let b = dir.path().join("b.xlsx");
    std::fs::copy(corpus_xlsx(), &a).unwrap();
    std::fs::copy(corpus_xlsx(), &b).unwrap();
    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["--json", "diff", a.to_str().unwrap(), b.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"empty\": true"));
}

#[test]
fn audit_json_validates_against_committed_schema() {
    let dir = TempDir::new().unwrap();
    let book = dir.path().join("book.xlsx");
    std::fs::copy(corpus_xlsx(), &book).unwrap();
    let output = bin()
        .env("HOME", dir.path())
        .args(["--json", "audit", book.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let schema_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/schemas/audit.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(schema_path).unwrap()).unwrap();
    validate_audit_schema(&json, &schema, &schema, "$").unwrap();
}

fn validate_audit_schema(
    value: &serde_json::Value,
    schema: &serde_json::Value,
    root: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    if let Some(reference) = schema.get("$ref").and_then(serde_json::Value::as_str) {
        let name = reference
            .strip_prefix("#/$defs/")
            .ok_or_else(|| format!("{path}: unsupported ref {reference}"))?;
        let resolved = root
            .get("$defs")
            .and_then(|defs| defs.get(name))
            .ok_or_else(|| format!("{path}: unresolved ref {reference}"))?;
        return validate_audit_schema(value, resolved, root, path);
    }
    if let Some(expected) = schema.get("const")
        && value != expected
    {
        return Err(format!("{path}: expected const {expected}, got {value}"));
    }
    if let Some(choices) = schema.get("enum").and_then(serde_json::Value::as_array)
        && !choices.contains(value)
    {
        return Err(format!("{path}: {value} is not in enum"));
    }
    let types: Vec<&str> = match schema.get("type") {
        Some(serde_json::Value::String(kind)) => vec![kind],
        Some(serde_json::Value::Array(kinds)) => {
            kinds.iter().filter_map(serde_json::Value::as_str).collect()
        }
        _ => Vec::new(),
    };
    if !types.is_empty() && !types.iter().any(|kind| schema_type_matches(value, kind)) {
        return Err(format!("{path}: value does not match type {types:?}"));
    }
    if let Some(object) = value.as_object() {
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object);
        if let Some(required) = schema.get("required").and_then(serde_json::Value::as_array) {
            for key in required.iter().filter_map(serde_json::Value::as_str) {
                if !object.contains_key(key) {
                    return Err(format!("{path}: missing {key}"));
                }
            }
        }
        if schema.get("additionalProperties") == Some(&serde_json::Value::Bool(false))
            && let Some(properties) = properties
        {
            for key in object.keys() {
                if !properties.contains_key(key) {
                    return Err(format!("{path}: unexpected property {key}"));
                }
            }
        }
        if let Some(properties) = properties {
            for (key, child) in object {
                if let Some(child_schema) = properties.get(key) {
                    validate_audit_schema(child, child_schema, root, &format!("{path}.{key}"))?;
                }
            }
        }
    }
    if let Some(array) = value.as_array()
        && let Some(items) = schema.get("items")
    {
        for (index, item) in array.iter().enumerate() {
            validate_audit_schema(item, items, root, &format!("{path}[{index}]"))?;
        }
    }
    if let Some(string) = value.as_str() {
        if let Some(minimum) = schema.get("minLength").and_then(serde_json::Value::as_u64)
            && string.chars().count() < minimum as usize
        {
            return Err(format!("{path}: string is too short"));
        }
        if schema.get("pattern").and_then(serde_json::Value::as_str) == Some("^audit\\.[a-z0-9_]+$")
            && (!string.starts_with("audit.")
                || string.len() == "audit.".len()
                || !string["audit.".len()..]
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'))
        {
            return Err(format!("{path}: invalid audit finding id {string:?}"));
        }
    }
    Ok(())
}

fn schema_type_matches(value: &serde_json::Value, kind: &str) -> bool {
    match kind {
        "array" => value.is_array(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "null" => value.is_null(),
        "object" => value.is_object(),
        "string" => value.is_string(),
        _ => false,
    }
}
