//! End-to-end Lua/Python CLI boundaries.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;

fn scratch(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
}

fn corpus_xlsx() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/xlsx/l1_values.xlsx")
}

fn command(home: &Path) -> Command {
    let mut command = Command::cargo_bin("omacell").unwrap();
    command.env("HOME", home);
    command.env("XDG_RUNTIME_DIR", home.join("run"));
    command
}

#[test]
fn user_programs_reject_dry_run_and_profiles_are_exclusive() {
    let dir = scratch("cli-lua-args-");
    command(dir.path())
        .args(["--dry-run", "run", "script.lua", "book.xlsx"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot safely execute"));
    command(dir.path())
        .args(["run", "--embedded", "--python", "book.xlsx"])
        .assert()
        .code(2);
    command(dir.path())
        .args(["run", "--embedded", "book.xlsx", "ignored.xlsx"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("only path"));
}

#[test]
fn embedded_script_requires_exact_file_hash_trust_then_saves() {
    let dir = scratch("cli-embedded-");
    let book = dir.path().join("book.xlsx");
    let mut document = omacell_io::xlsx::open(&corpus_xlsx()).unwrap();
    document.workbook.custom_parts.insert(
        omacell_lua::EMBEDDED_PART.into(),
        br#"
omacell.on_before_save(function()
    omacell.cmd("cell.set", {ref = "A2", input = "654"})
end)
omacell.cmd("cell.set", {ref = "A1", input = "321"})
"#
        .to_vec(),
    );
    std::fs::write(&book, omacell_io::xlsx::save_bytes(&document).unwrap()).unwrap();
    let before = std::fs::read(&book).unwrap();

    command(dir.path())
        .args(["run", "--embedded", book.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not in the trust store"));
    assert_eq!(std::fs::read(&book).unwrap(), before);

    command(dir.path())
        .args(["trust", "add", book.to_str().unwrap()])
        .assert()
        .success();
    command(dir.path())
        .args(["run", "--embedded", book.to_str().unwrap()])
        .assert()
        .success();
    command(dir.path())
        .args(["query", book.to_str().unwrap(), "A1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("321"));
    command(dir.path())
        .args(["query", book.to_str().unwrap(), "A2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("654"));
}

#[test]
fn python_bridge_uses_versioned_ipc_envelopes_without_a_workbook() {
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let dir = scratch("cli-python-");
    let script = dir.path().join("bridge.py");
    std::fs::write(
        &script,
        r#"import json
import sys

print(json.dumps({"v": 1, "id": 7, "op": "ping"}), flush=True)
reply = json.loads(sys.stdin.readline())
assert reply == {"v": 1, "id": 7, "ok": True, "result": {"pong": True}}

print(json.dumps({"v": 1, "id": 8, "cmd": "cell.set", "args": {"ref": "A1", "input": "x" * 1100000}, "mode": "propose"}), flush=True)
reply = json.loads(sys.stdin.readline())
assert reply["id"] == 8
assert reply["ok"] is False
assert reply["error"]["code"] != "ipc.frame"
"#,
    )
    .unwrap();

    command(dir.path())
        .args(["run", "--python", script.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));

    command(dir.path())
        .args([
            "--set",
            "ipc.max_frame_bytes=1048576",
            "run",
            "--python",
            script.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("ipc.frame"));
}
