//! Secret shapes must not appear in config or logs after AI operations.

use omacell_ai::audit::{AuditLog, LogRecord, hash_json, now_ms};
use omacell_ai::provider::Usage;
use omacell_ai::setup::{SetupPatch, apply_setup_patch};
use serde_json::json;
use tempfile::TempDir;

const SECRET: &str = "sk-test-secret-leaky-value";

fn walk_files(root: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
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

#[test]
fn secret_does_not_land_in_config_or_logs() {
    let home = TempDir::new().unwrap();
    let config = home.path().join(".config/omacell/config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "schema = 1\n[ai]\nenabled = false\n").unwrap();
    let patch = SetupPatch {
        enabled: true,
        providers: vec![omacell_ai::DetectedProvider {
            name: "ollama".into(),
            kind: "openai_compatible".into(),
            endpoint: "http://127.0.0.1:11434/v1".into(),
            reachable: true,
        }],
    };
    apply_setup_patch(&config, &patch).unwrap();

    let log = AuditLog::open(&home.path().join(".local/state/omacell")).unwrap();
    log.append(&LogRecord {
        ts: now_ms(),
        task: "chat".into(),
        provider: "ollama".into(),
        model: "qwen".into(),
        request_bytes: 10,
        response_bytes: 10,
        request_hash: hash_json(&json!({"secret": SECRET})),
        latency_ms: 1,
        usage: Usage::default(),
        content: None,
    })
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let log_path = home.path().join(".local/state/omacell/ai/log.jsonl");
        let dir_mode = std::fs::metadata(log_path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let file_mode = std::fs::metadata(&log_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);
    }

    let mut files = Vec::new();
    walk_files(home.path(), &mut files);
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        assert!(
            !text.contains(SECRET),
            "{} leaked the secret",
            path.display()
        );
    }
}
