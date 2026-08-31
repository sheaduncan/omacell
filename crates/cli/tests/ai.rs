//! `omacell ai setup|card|log|usage` in a temp `$HOME`.

use std::net::TcpListener;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("omacell").unwrap()
}

fn walk_files(root: &Path, out: &mut Vec<std::path::PathBuf>) {
    if root.is_file() {
        out.push(root.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        walk_files(&entry.path(), out);
    }
}

fn port_open(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        std::time::Duration::from_millis(80),
    )
    .is_ok()
}

#[test]
fn setup_with_fake_local_servers_writes_only_config() {
    let home = TempDir::new().unwrap();
    let runtime = TempDir::new().unwrap();
    let _ollama = TcpListener::bind("127.0.0.1:11434").ok();
    let _lm = TcpListener::bind("127.0.0.1:1234").ok();
    assert!(
        _ollama.is_some() || _lm.is_some() || port_open(11434) || port_open(1234),
        "need a loopback listener on 11434 or 1234"
    );

    let mut cmd = bin();
    cmd.env("HOME", home.path());
    cmd.env("XDG_RUNTIME_DIR", runtime.path());
    cmd.args(["--json", "ai", "setup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("127.0.0.1"));

    let config = home.path().join(".config/omacell/config.toml");
    assert!(config.is_file(), "setup must write {}", config.display());
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(text.contains("openai_compatible"), "{text}");
    assert!(text.contains("local = true"), "{text}");
    assert!(!text.contains("secret_env"), "{text}");
    assert!(!text.contains("secret_cmd"), "{text}");
    if _ollama.is_some() || port_open(11434) {
        assert!(text.contains("11434"), "{text}");
    }
    if _lm.is_some() || port_open(1234) {
        assert!(text.contains("1234"), "{text}");
    }

    let mut files = Vec::new();
    walk_files(home.path(), &mut files);
    for path in &files {
        let rel = path.strip_prefix(home.path()).unwrap();
        assert_eq!(
            rel,
            Path::new(".config/omacell/config.toml"),
            "setup wrote extra file {}",
            rel.display()
        );
    }
}

#[test]
fn card_log_usage_json() {
    let home = TempDir::new().unwrap();
    let runtime = TempDir::new().unwrap();
    let book = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/xlsx/l1_values.xlsx");
    let mut cmd = bin();
    cmd.env("HOME", home.path());
    cmd.env("XDG_RUNTIME_DIR", runtime.path());
    cmd.args([
        "--json",
        "ai",
        "card",
        book.to_str().unwrap(),
        "--level",
        "summary",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("\"schema\""))
    .stdout(predicate::str::contains("\"kind\""));

    let mut cmd = bin();
    cmd.env("HOME", home.path());
    cmd.env("XDG_RUNTIME_DIR", runtime.path());
    cmd.args(["--json", "ai", "log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"records\""));

    let mut cmd = bin();
    cmd.env("HOME", home.path());
    cmd.env("XDG_RUNTIME_DIR", runtime.path());
    cmd.args(["--json", "ai", "usage"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"providers\""));
}
