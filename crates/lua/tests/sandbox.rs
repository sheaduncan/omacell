//! Sandbox, caps, and trust tests.

use std::io::Write;
use std::sync::{Arc, Mutex};

use omacell_bus::Bus;
use omacell_core::error::CoreError;
use omacell_core::eval::DynamicFn;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_lua::{
    BusHost, EmbeddedMode, Profile, Runtime, ScriptHost, ScriptPolicy, TrustStore, allow_embedded,
    load_user_scripts, sha256_hex,
};
use serde_json::Value as Json;

fn host() -> BusHost {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    BusHost::new(Bus::new(Workbook::new(), RecalcEngine::new(registry)).unwrap())
}

#[test]
fn embedded_blocks_io_os_require() {
    let rt = Runtime::new(Profile::Embedded, Box::new(host())).unwrap();
    for src in [
        "assert(io == nil)",
        "assert(os == nil)",
        "assert(require == nil)",
        "assert(coroutine == nil)",
        "assert(pcall == nil)",
        "assert(xpcall == nil)",
    ] {
        rt.exec(src, "deny.lua").unwrap();
    }
    // The chunk succeeds but the globals are nil; calling them errors.
    let err = rt.exec("io.open('/blocked/x')", "io.lua").unwrap_err();
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

struct EscapeHost {
    workbook: Workbook,
    calls: Arc<Mutex<Vec<String>>>,
}

impl ScriptHost for EscapeHost {
    fn execute(&mut self, id: &str, _args: Json) -> Result<Json, CoreError> {
        self.calls.lock().unwrap().push(id.to_string());
        Ok(Json::Null)
    }

    fn workbook(&self) -> &Workbook {
        &self.workbook
    }

    fn register_function(&mut self, _def: DynamicFn) -> Result<(), CoreError> {
        Ok(())
    }

    fn prompt(&mut self, _message: &str) -> Result<String, CoreError> {
        Ok(String::new())
    }

    fn status(&mut self, _message: &str) {}

    fn notify(&mut self, _message: &str) {}
}

#[test]
fn embedded_cannot_escape_through_external_effect_commands() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let host = EscapeHost {
        workbook: Workbook::new(),
        calls: Arc::clone(&calls),
    };
    let rt = Runtime::new(Profile::Embedded, Box::new(host)).unwrap();
    for id in [
        "file.open",
        "file.save",
        "file.export",
        "file.print",
        "macro.save",
        "script.source",
        "theme.reload",
        // `edit.repeat` can indirectly replay the last file/macro/theme
        // mutation and therefore must not bypass the namespace filter.
        "edit.repeat",
    ] {
        let source = format!("omacell.cmd({id:?}, {{path = 'escape'}})");
        let err = rt.exec(&source, "escape.lua").unwrap_err();
        assert_eq!(err.code, "lua.exec");
        assert!(
            err.message.contains("not available to embedded scripts"),
            "{err:?}"
        );
    }
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn user_profile_keeps_io_and_os() {
    let rt = Runtime::new(Profile::User, Box::new(host())).unwrap();
    rt.exec(
        "assert(type(io) == 'table'); assert(type(os) == 'table')",
        "stdlib.lua",
    )
    .unwrap();
}

#[test]
fn user_scripts_load_init_then_sorted_plugins() {
    let dir = tempfile::Builder::new()
        .prefix("user-scripts-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap();
    std::fs::create_dir_all(dir.path().join("plugins/z-last")).unwrap();
    std::fs::create_dir_all(dir.path().join("plugins/a-first")).unwrap();
    std::fs::write(dir.path().join("init.lua"), "order = 'init'").unwrap();
    std::fs::write(
        dir.path().join("plugins/a-first/init.lua"),
        "order = order .. ',a'",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("plugins/z-last/init.lua"),
        "order = order .. ',z'",
    )
    .unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let policy = ScriptPolicy {
        enabled: true,
        embedded: EmbeddedMode::Sandbox,
        trusted_dirs: vec![root],
    };
    let rt = Runtime::new(Profile::User, Box::new(host())).unwrap();
    let loaded = load_user_scripts(&rt, dir.path(), &policy).unwrap();
    assert_eq!(loaded.len(), 3);
    rt.exec("assert(order == 'init,a,z')", "load-order.lua")
        .unwrap();
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
    store
        .add(sha256_hex(bytes), Some("book.xlsx".into()))
        .unwrap();
    allow_embedded(&policy, &store, std::path::Path::new("missing.xlsx"), bytes).unwrap();
}

#[test]
fn trust_is_checked_against_the_exact_opened_bytes() {
    let dir = tempfile::Builder::new()
        .prefix("trust-exact-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap();
    let path = dir.path().join("book.xlsx");
    std::fs::write(&path, b"trusted-on-disk").unwrap();
    let policy = ScriptPolicy {
        enabled: true,
        embedded: EmbeddedMode::Sandbox,
        trusted_dirs: Vec::new(),
    };
    let mut store = TrustStore::default();
    store
        .add(
            sha256_hex(b"trusted-on-disk"),
            Some(path.display().to_string()),
        )
        .unwrap();
    let err = allow_embedded(&policy, &store, &path, b"bytes-that-were-parsed").unwrap_err();
    assert_eq!(err.code, "lua.untrusted");
}

#[test]
fn policy_reload_only_retains_still_trusted_directories() {
    let a = std::path::PathBuf::from("/a");
    let b = std::path::PathBuf::from("/b");
    let c = std::path::PathBuf::from("/c");
    let mut current = ScriptPolicy {
        enabled: true,
        embedded: EmbeddedMode::Sandbox,
        trusted_dirs: vec![a.clone(), b],
    };
    current.tighten(&ScriptPolicy {
        enabled: true,
        embedded: EmbeddedMode::Sandbox,
        trusted_dirs: vec![a.clone(), c],
    });
    assert_eq!(current.trusted_dirs, vec![a]);
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
    store.add(sha256_hex(bytes), None).unwrap();
    let err =
        allow_embedded(&policy, &store, std::path::Path::new("book.xlsx"), bytes).unwrap_err();
    assert_eq!(err.code, "lua.embedded");
}

#[test]
fn trust_store_round_trips() {
    let dir = tempfile::Builder::new()
        .prefix("trust-roundtrip-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap();
    let path = dir.path().join("trust.toml");
    let mut store = TrustStore::default();
    let hash = sha256_hex(b"workbook");
    let informational_path = "line one\nline\t\\\"two".to_string();
    store
        .add(hash.clone(), Some(informational_path.clone()))
        .unwrap();
    store.save(&path).unwrap();
    let loaded = TrustStore::load(&path).unwrap();
    assert!(loaded.contains_hash(&hash));
    assert_eq!(
        loaded.files[0].path.as_deref(),
        Some(informational_path.as_str())
    );
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(f, "# comment").unwrap();
}
