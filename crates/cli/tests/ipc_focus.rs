//! Default IPC routing follows the focused frontend instead of process age.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;

use assert_cmd::Command;
use omacell_bus::Bus;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use predicates::prelude::*;

#[test]
fn ipc_defaults_to_the_focused_instance() {
    let xdg = tempfile::tempdir().unwrap();
    let runtime = xdg.path().join("omacell");
    let bus = Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())).unwrap();
    let server = omacell_bus::ipc::serve(runtime.clone(), bus).unwrap();

    // PID 1 is live and this synthetic socket has the newest discovery time,
    // so an age-based client would connect here and time out.
    let newest_path = runtime.join("1.sock");
    let _newest_listener = UnixListener::bind(&newest_path).unwrap();
    std::fs::set_permissions(&newest_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(
        runtime.join("1.instance"),
        r#"{"v":1,"pid":1,"socket":"1.sock","started_unix_ms":18446744073709551615}"#,
    )
    .unwrap();
    server.set_focused(true).unwrap();

    Command::cargo_bin("omacell")
        .unwrap()
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env("HOME", xdg.path())
        .env("XDG_RUNTIME_DIR", xdg.path())
        .args(["--json", "ipc", "ping"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""pong": true"#));
}
