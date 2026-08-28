//! Socket integration tests for the WP-07b Unix IPC server.

#![cfg(unix)]

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use omacell_bus::Bus;
use omacell_bus::ipc::{IpcClient, Mode, default_runtime_dir, discover_newest, serve};
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;

static UNIQUE: AtomicU64 = AtomicU64::new(1);

fn runtime_dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "omacell-ipc-{}-{}",
        std::process::id(),
        UNIQUE.fetch_add(1, Ordering::SeqCst)
    ))
}

fn bus() -> Bus {
    Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())).expect("bus")
}

fn start() -> (omacell_bus::ipc::IpcHandle, PathBuf) {
    let dir = runtime_dir();
    let handle = serve(dir.clone(), bus()).expect("serve");
    (handle, dir)
}

#[test]
fn ping_and_propose_apply_revert_round_trip() {
    let (handle, dir) = start();
    let mut client = IpcClient::connect(handle.socket_path()).unwrap();
    let pong = client.ping().unwrap();
    assert!(pong.ok, "{:?}", pong.error);

    let proposed = client
        .command(
            "cell.set",
            serde_json::json!({"ref":"A1","input":"7"}),
            Some(Mode::Propose),
        )
        .unwrap();
    assert!(proposed.ok, "{:?}", proposed.error);
    let cs = proposed.result.as_ref().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        proposed.result.as_ref().unwrap()["status"].as_str(),
        Some("proposed")
    );

    let applied = client.apply(&cs).unwrap();
    assert!(applied.ok, "{:?}", applied.error);
    {
        let bus = handle.bus().lock().unwrap();
        let sheet = bus.workbook().active_sheet();
        let slot = bus.workbook().get(sheet, 0, 0).unwrap().unwrap();
        assert_eq!(slot.value, omacell_core::value::Value::Number(7.0));
    }

    let reverted = client.revert(&cs).unwrap();
    assert!(reverted.ok, "{:?}", reverted.error);
    {
        let bus = handle.bus().lock().unwrap();
        let sheet = bus.workbook().active_sheet();
        assert!(bus.workbook().get(sheet, 0, 0).unwrap().is_none());
    }
    let _ = dir;
}

#[test]
fn mutating_execute_is_rejected_internal_ids_are_rejected() {
    let (handle, _dir) = start();
    let mut client = IpcClient::connect(handle.socket_path()).unwrap();
    let err = client
        .command(
            "cell.set",
            serde_json::json!({"ref":"A1","input":"1"}),
            Some(Mode::Execute),
        )
        .unwrap();
    assert!(!err.ok);
    assert_eq!(
        err.error.as_ref().unwrap().code,
        omacell_bus::codes::IPC_MODE
    );

    let err = client
        .command("cell.restore", serde_json::json!({}), None)
        .unwrap();
    assert!(!err.ok);
    assert_eq!(
        err.error.as_ref().unwrap().code,
        omacell_bus::codes::COMMAND_INTERNAL
    );
}

#[test]
fn two_clients_keep_per_client_request_order() {
    let (handle, _dir) = start();
    let mut a = IpcClient::connect(handle.socket_path()).unwrap();
    let mut b = IpcClient::connect(handle.socket_path()).unwrap();
    let a1 = a.ping().unwrap();
    let b1 = b.ping().unwrap();
    let a2 = a.ping().unwrap();
    assert!(a1.ok && b1.ok && a2.ok);
    assert_eq!(a1.id, 1);
    assert_eq!(a2.id, 2);
    assert_eq!(b1.id, 1);
}

#[test]
fn subscribe_receives_changeset_events() {
    let (handle, _dir) = start();
    let mut client = IpcClient::connect(handle.socket_path()).unwrap();
    let sub = client
        .subscribe(&["changeset_proposed".to_string()])
        .unwrap();
    assert!(sub.ok);
    let _ = client
        .command(
            "cell.set",
            serde_json::json!({"ref":"B1","input":"1"}),
            Some(Mode::Propose),
        )
        .unwrap();
    let mut saw = false;
    for _ in 0..20 {
        if let Some(omacell_bus::ipc::ServerRecord::Event { event, .. }) =
            client.poll_record().unwrap()
        {
            if matches!(event, omacell_core::event::Event::ChangesetProposed { .. }) {
                saw = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(saw, "expected changeset_proposed event");
}

#[test]
fn stalled_subscriber_does_not_block_another_client() {
    let (handle, _dir) = start();
    let mut stalled = UnixStream::connect(handle.socket_path()).unwrap();
    stalled
        .write_all(br#"{"v":1,"id":1,"op":"subscribe","events":[]}"#)
        .unwrap();
    stalled.write_all(b"\n").unwrap();
    let mut buf = [0u8; 256];
    let _ = stalled.read(&mut buf);

    let mut live = IpcClient::connect(handle.socket_path()).unwrap();
    for i in 0..8 {
        let reply = live
            .command(
                "cell.set",
                serde_json::json!({"ref":"A1","input":format!("{i}")}),
                Some(Mode::Propose),
            )
            .unwrap();
        assert!(reply.ok, "{:?}", reply.error);
    }
}

#[test]
fn timeout_and_clean_shutdown() {
    let (handle, dir) = start();
    let path = handle.socket_path().to_path_buf();
    let mut client = IpcClient::connect(&path).unwrap();
    client.set_timeout(Duration::from_millis(200)).unwrap();
    assert!(client.ping().unwrap().ok);
    handle.shutdown();
    std::thread::sleep(Duration::from_millis(50));
    assert!(IpcClient::connect(&path).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn runtime_dir_rejects_symlink_and_world_writable() {
    let parent = runtime_dir();
    std::fs::create_dir_all(&parent).unwrap();
    let target = parent.join("real");
    std::fs::create_dir(&target).unwrap();
    let link = parent.join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(serve(link, bus()).is_err());

    let wide = parent.join("wide");
    std::fs::create_dir(&wide).unwrap();
    std::fs::set_permissions(&wide, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(serve(wide, bus()).is_err());
    let _ = std::fs::remove_dir_all(parent);
}

#[test]
fn stale_socket_for_dead_pid_is_removed() {
    let dir = runtime_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    omacell_bus::ipc::prepare_runtime_dir(&dir).unwrap();
    let dead = 4_000_000u32;
    let stale = dir.join(format!("{dead}.sock"));
    std::fs::write(&stale, b"").unwrap();
    omacell_bus::ipc::remove_stale_socket(&dir, dead).unwrap();
    assert!(!stale.exists());
    let handle = serve(dir.clone(), bus()).unwrap();
    assert!(handle.socket_path().exists());
    let newest = discover_newest(&dir).unwrap().unwrap();
    assert_eq!(newest.pid, std::process::id());
    handle.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn default_runtime_dir_is_under_xdg_or_tmp() {
    let dir = default_runtime_dir();
    let s = dir.to_string_lossy();
    assert!(s.contains("omacell"), "{s}");
}
