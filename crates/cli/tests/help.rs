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
    assert_snapshot!("trust", help(&["trust", "--help"]));
    assert_snapshot!("trust_add", help(&["trust", "add", "--help"]));
    assert_snapshot!("trust_remove", help(&["trust", "remove", "--help"]));
    assert_snapshot!("trust_list", help(&["trust", "list", "--help"]));
    assert_snapshot!("fn", help(&["fn", "--help"]));
    assert_snapshot!("fn_list", help(&["fn", "list", "--help"]));
    assert_snapshot!("fn_doc", help(&["fn", "doc", "--help"]));
    assert_snapshot!("config", help(&["config", "--help"]));
    assert_snapshot!("config_check", help(&["config", "check", "--help"]));
    assert_snapshot!("config_edit", help(&["config", "edit", "--help"]));
    assert_snapshot!("config_reset", help(&["config", "reset", "--help"]));
    assert_snapshot!("config_show", help(&["config", "show", "--help"]));
    assert_snapshot!("config_diff", help(&["config", "diff", "--help"]));
    assert_snapshot!("theme", help(&["theme", "--help"]));
    assert_snapshot!("theme_show", help(&["theme", "show", "--help"]));
    assert_snapshot!("theme_reload", help(&["theme", "reload", "--help"]));
    assert_snapshot!("keys", help(&["keys", "--help"]));
    assert_snapshot!("keys_check", help(&["keys", "check", "--help"]));
    assert_snapshot!("setup", help(&["setup", "--help"]));
    assert_snapshot!("setup_omarchy", help(&["setup", "omarchy", "--help"]));
    assert_snapshot!("commands", help(&["commands", "--help"]));
    assert_snapshot!("ipc", help(&["ipc", "--help"]));
    assert_snapshot!("changeset", help(&["changeset", "--help"]));
    assert_snapshot!("changeset_list", help(&["changeset", "list", "--help"]));
    assert_snapshot!("changeset_show", help(&["changeset", "show", "--help"]));
    assert_snapshot!("changeset_apply", help(&["changeset", "apply", "--help"]));
    assert_snapshot!(
        "changeset_discard",
        help(&["changeset", "discard", "--help"])
    );
    assert_snapshot!("changeset_revert", help(&["changeset", "revert", "--help"]));
    assert_snapshot!("changeset_export", help(&["changeset", "export", "--help"]));
    assert_snapshot!("diff", help(&["diff", "--help"]));
    assert_snapshot!("audit", help(&["audit", "--help"]));
    assert_snapshot!("ai", help(&["ai", "--help"]));
    assert_snapshot!("ai_setup", help(&["ai", "setup", "--help"]));
    assert_snapshot!("ai_card", help(&["ai", "card", "--help"]));
    assert_snapshot!("ai_log", help(&["ai", "log", "--help"]));
    assert_snapshot!("ai_usage", help(&["ai", "usage", "--help"]));
    assert_snapshot!("agent", help(&["agent", "--help"]));
    assert_snapshot!("agent_diagnose", help(&["agent", "diagnose", "--help"]));
    assert_snapshot!("mcp", help(&["mcp", "--help"]));
}

#[test]
fn help_describes_the_real_paths_theme_and_formats() {
    let root = help(&["--help"]);
    assert!(root.contains("XDG_CONFIG_HOME"), "{root}");
    assert!(root.contains("role overlay"), "{root}");
    assert!(root.contains(".xlsm"), "{root}");
    assert!(root.contains("Parquet"), "{root}");

    let setup = help(&["setup", "omarchy", "--help"]);
    assert!(setup.contains("--uninstall"), "{setup}");
}
