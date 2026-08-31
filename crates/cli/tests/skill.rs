//! Skill-drift: every CLI command mentioned in SKILL.md exists with those flags.

use assert_cmd::Command;
use omacell_conf::paths::Paths;
use omacell_conf::setup::{SetupOptions, setup_omarchy};
use std::path::PathBuf;

fn skill_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../default/agents/skills/omacell/SKILL.md")
}

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
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn assert_skill_matches_cli(text: &str) {
    let root = help(&["--help"]);
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed
            .strip_prefix("omacell ")
            .or_else(|| trimmed.strip_prefix("`omacell "))
        else {
            continue;
        };
        let rest = rest.trim_matches('`');
        let tokens: Vec<&str> = rest
            .split_whitespace()
            .map(|t| t.trim_matches(|c| c == '`' || c == '"' || c == ',' || c == '.' || c == ')'))
            .filter(|t| {
                !t.is_empty() && !t.starts_with('<') && !t.ends_with('>') && *t != "\"<prompt>\""
            })
            .collect();
        if tokens.is_empty() {
            continue;
        }
        if tokens[0].starts_with('-') {
            continue;
        }
        let cmd = tokens[0];
        if cmd == "--help" {
            continue;
        }
        assert!(
            root.contains(cmd),
            "SKILL.md command `{cmd}` missing from omacell --help"
        );
        let flags: Vec<&str> = tokens
            .iter()
            .copied()
            .filter(|t| t.starts_with("--") && *t != "--help")
            .collect();
        if flags.is_empty() {
            continue;
        }
        let mut args = vec![cmd, "--help"];
        if cmd == "agent" && tokens.get(1) == Some(&"diagnose") {
            args = vec!["agent", "diagnose", "--help"];
        } else if cmd == "changeset" && tokens.len() > 1 && !tokens[1].starts_with('-') {
            args = vec!["changeset", tokens[1], "--help"];
        } else if cmd == "setup" && tokens.get(1) == Some(&"omarchy") {
            args = vec!["setup", "omarchy", "--help"];
        }
        let page = help(&args);
        for flag in flags {
            let flag = flag.trim_end_matches('>');
            assert!(
                page.contains(flag),
                "SKILL.md flag `{flag}` missing from `omacell {}`",
                args.join(" ")
            );
        }
    }
}

#[test]
fn skill_commands_match_cli_help() {
    let text = std::fs::read_to_string(skill_path()).unwrap();
    assert!(!text.contains("TODO(WP-21)"));
    assert_skill_matches_cli(&text);
}

#[test]
fn setup_links_skill_and_drift_uses_that_file() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    setup_omarchy(
        &paths,
        SetupOptions {
            confirm_menu: false,
            link_skill: true,
        },
    )
    .unwrap();
    let linked = dir.path().join(".agents/skills/omacell/SKILL.md");
    let text = std::fs::read_to_string(&linked).unwrap();
    assert_eq!(text, std::fs::read_to_string(skill_path()).unwrap());
    assert_skill_matches_cli(&text);
}
