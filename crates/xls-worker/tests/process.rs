//! Companion-process protocol smoke tests.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn stdio_worker_converts_a_committed_biff_fixture() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/xls/l1_values.xls");
    let input = std::fs::read(fixture).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_omacell-xls-worker"))
        .arg("--stdio-v1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let workbook = omacell_io::xlsx::open_bytes(&output.stdout)
        .unwrap()
        .workbook;
    assert!(
        workbook
            .get(workbook.active_sheet(), 0, 0)
            .unwrap()
            .is_some()
    );
}

#[test]
fn worker_rejects_non_protocol_invocation() {
    let output = Command::new(env!("CARGO_BIN_EXE_omacell-xls-worker"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid XLS worker invocation"));
}
