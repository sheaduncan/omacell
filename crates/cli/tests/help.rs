//! Help snapshots for every command (agents and docs depend on these).

use assert_cmd::Command;
use insta::assert_snapshot;

fn help(args: &[&str]) -> String {
    let output = Command::cargo_bin("omacell")
        .unwrap()
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

#[test]
fn help_snapshots() {
    assert_snapshot!("root", help(&["--help"]));
    assert_snapshot!("convert", help(&["convert", "--help"]));
    assert_snapshot!("query", help(&["query", "--help"]));
    assert_snapshot!("set", help(&["set", "--help"]));
    assert_snapshot!("eval", help(&["eval", "--help"]));
    assert_snapshot!("recalc", help(&["recalc", "--help"]));
    assert_snapshot!("run", help(&["run", "--help"]));
    assert_snapshot!("fn", help(&["fn", "--help"]));
    assert_snapshot!("fn_list", help(&["fn", "list", "--help"]));
    assert_snapshot!("fn_doc", help(&["fn", "doc", "--help"]));
    assert_snapshot!("config", help(&["config", "--help"]));
    assert_snapshot!("config_show", help(&["config", "show", "--help"]));
    assert_snapshot!("theme", help(&["theme", "--help"]));
    assert_snapshot!("keys", help(&["keys", "check", "--help"]));
    assert_snapshot!("setup", help(&["setup", "omarchy", "--help"]));
    assert_snapshot!("commands", help(&["commands", "--help"]));
    assert_snapshot!("ipc", help(&["ipc", "--help"]));
    assert_snapshot!("changeset", help(&["changeset", "--help"]));
    assert_snapshot!("diff", help(&["diff", "--help"]));
    assert_snapshot!("audit", help(&["audit", "--help"]));
    assert_snapshot!("ai", help(&["ai", "--help"]));
    assert_snapshot!("agent", help(&["agent", "--help"]));
    assert_snapshot!("mcp", help(&["mcp", "--help"]));
}
