//! Smoke test: the binary prints its version.
use std::process::Command;

#[test]
fn binary_prints_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_omacell"))
        .arg("--version")
        .output()
        .expect("failed to execute omacell");
    assert!(
        output.status.success(),
        "omacell --version failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "version output did not contain package version: {stdout:?}"
    );
    assert!(
        stdout.contains("omacell"),
        "version output did not contain product name: {stdout:?}"
    );
}
