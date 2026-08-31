//! Sandbox, caps, and trust tests.

use std::io::Write;

use omacell_bus::Bus;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_lua::{
    BusHost, EmbeddedMode, Profile, Runtime, ScriptPolicy, TrustStore, allow_embedded, sha256_hex,
};

fn host() -> BusHost {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    BusHost::new(Bus::new(Workbook::new(), RecalcEngine::new(registry)).unwrap())
}

#[test]
fn embedded_blocks_io_os_require() {
    let rt = Runtime::new(Profile::Embedded, Box::new(host())).unwrap();
    for src in ["return io", "return os", "return require"] {
        rt.exec(src, "deny.lua").unwrap();
    }
    // The chunk succeeds but the globals are nil; calling them errors.
    let err = rt.exec("io.open('/tmp/x')", "io.lua").unwrap_err();
    assert!(
        err.message.contains("nil") || err.message.contains("attempt"),
        "{err:?}"
    );
    let err = rt.exec("os.execute('true')", "os.lua").unwrap_err();
    assert!(
        err.message.contains("nil") || err.message.contains("attempt"),
        "{err:?}"
    );
    let err = rt.exec("require('os')", "req.lua").unwrap_err();
    assert!(
        err.message.contains("nil") || err.message.contains("attempt"),
        "{err:?}"
    );
}

#[test]
fn embedded_infinite_loop_is_terminated() {
    let rt = Runtime::new(Profile::Embedded, Box::new(host())).unwrap();
    let err = rt.exec("while true do end", "loop.lua").unwrap_err();
    assert!(err.message.contains("instruction limit"), "{err:?}");
}

#[test]
fn embedded_memory_cap_is_enforced() {
    let rt = Runtime::new(Profile::Embedded, Box::new(host())).unwrap();
    let err = rt
        .exec(
            r#"
            local t = {}
            for i = 1, 200000 do
                t[i] = string.rep("x", 256)
            end
            "#,
            "mem.lua",
        )
        .unwrap_err();
    assert!(
        err.message.to_ascii_lowercase().contains("memory")
            || err.message.contains("instruction limit"),
        "{err:?}"
    );
}

#[test]
fn untrusted_embedded_does_not_run() {
    let policy = ScriptPolicy {
        enabled: true,
        embedded: EmbeddedMode::Sandbox,
        trusted_dirs: Vec::new(),
    };
    let store = TrustStore::default();
    let bytes = b"omacell.cmd('cell.set', {ref='A1', input='hacked'})";
    let err =
        allow_embedded(&policy, &store, std::path::Path::new("book.xlsx"), bytes).unwrap_err();
    assert_eq!(err.code, "lua.untrusted");
}

#[test]
fn trusted_embedded_is_allowed() {
    let policy = ScriptPolicy {
        enabled: true,
        embedded: EmbeddedMode::Sandbox,
        trusted_dirs: Vec::new(),
    };
    let bytes = b"return 1";
    let mut store = TrustStore::default();
    store.add(sha256_hex(bytes), Some("book.xlsx".into()));
    allow_embedded(&policy, &store, std::path::Path::new("missing.xlsx"), bytes).unwrap();
}

#[test]
fn deny_policy_blocks_even_trusted() {
    let policy = ScriptPolicy {
        enabled: true,
        embedded: EmbeddedMode::Deny,
        trusted_dirs: Vec::new(),
    };
    let bytes = b"return 1";
    let mut store = TrustStore::default();
    store.add(sha256_hex(bytes), None);
    let err =
        allow_embedded(&policy, &store, std::path::Path::new("book.xlsx"), bytes).unwrap_err();
    assert_eq!(err.code, "lua.embedded");
}

#[test]
fn trust_store_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trust.toml");
    let mut store = TrustStore::default();
    store.add("abcd".into(), Some("/tmp/a.xlsx".into()));
    store.save(&path).unwrap();
    let loaded = TrustStore::load(&path).unwrap();
    assert!(loaded.contains_hash("abcd"));
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(f, "# comment").unwrap();
}
