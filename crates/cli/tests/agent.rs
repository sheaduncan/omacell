//! Hand-off tests with a fake `omarchy` on PATH.

use assert_cmd::Command;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

fn write_fake_omarchy(bin: &std::path::Path, log: &std::path::Path) {
    let script = format!(
        "#!/bin/sh\nlog={log}\nprintf '%s\\n' \"$PWD\" \"$@\" >> \"$log\"\nif [ \"$1\" = default ] && [ \"$2\" = agent ]; then\n  if [ -n \"$OMACELL_FAKE_AGENT\" ]; then printf '%s\\n' \"$OMACELL_FAKE_AGENT\"; fi\n  exit 0\nfi\nexit 0\n",
        log = log.display()
    );
    std::fs::write(bin, script).unwrap();
    std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn handoff_records_args_and_cwd_when_default_agent_set() {
    let home = TempDir::new().unwrap();
    let path_dir = home.path().join("bin");
    std::fs::create_dir_all(&path_dir).unwrap();
    let log = home.path().join("omarchy.log");
    write_fake_omarchy(&path_dir.join("omarchy"), &log);
    let book_dir = home.path().join("wb");
    std::fs::create_dir_all(&book_dir).unwrap();
    let book = book_dir.join("n.xlsx");
    std::fs::write(&book, b"").unwrap();

    let output = Command::cargo_bin("omacell")
        .unwrap()
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", home.path().join("run"))
        .env("PATH", format!("{}:/usr/bin:/bin", path_dir.display()))
        .env("OMACELL_FAKE_AGENT", "claude")
        .args([
            "--json",
            "agent",
            "--book",
            book.to_str().unwrap(),
            "--selection",
            "Sheet1!A1",
            "Reconcile Inputs",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["hidden"], false);
    assert_eq!(json["launched"], true);
    assert_eq!(json["cwd"], book_dir.display().to_string());
    let log_text = std::fs::read_to_string(&log).unwrap();
    assert!(log_text.contains("agent"));
    assert!(log_text.contains("prompt"));
    assert!(log_text.contains("--workbook"));
    assert!(log_text.contains("--selection"));
    assert!(log_text.contains("Sheet1!A1"));
    assert!(log_text.contains("Reconcile Inputs"));
    assert!(
        log_text.contains(&book_dir.display().to_string())
            || log_text.lines().next() == Some(book_dir.to_str().unwrap())
    );
}

#[test]
fn hidden_without_default_agent() {
    let home = TempDir::new().unwrap();
    let path_dir = home.path().join("bin");
    std::fs::create_dir_all(&path_dir).unwrap();
    let log = home.path().join("omarchy.log");
    write_fake_omarchy(&path_dir.join("omarchy"), &log);

    let output = Command::cargo_bin("omacell")
        .unwrap()
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", home.path().join("run"))
        .env("PATH", format!("{}:/usr/bin:/bin", path_dir.display()))
        .env_remove("OMACELL_FAKE_AGENT")
        .args(["--json", "agent", "hello"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["hidden"], true);
    assert_eq!(json["launched"], false);
    assert!(
        json["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "omarchy")
    );
    let log_text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !log_text.contains("prompt"),
        "must not launch omarchy agent prompt when hidden: {log_text}"
    );
}

#[test]
fn hidden_when_omarchy_missing() {
    let home = TempDir::new().unwrap();
    let output = Command::cargo_bin("omacell")
        .unwrap()
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", home.path().join("run"))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "agent", "hello"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["hidden"], true);
}
