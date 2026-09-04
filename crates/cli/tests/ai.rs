//! `omacell ai setup|card|log|usage` in a temp `$HOME`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    let mut command = Command::cargo_bin("omacell").unwrap();
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME");
    command
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

fn fake_model_server(port: u16) -> bool {
    let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) else {
        return false;
    };
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            );
        }
    });
    true
}

fn model_server_responds(port: u16, path: &str) -> bool {
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_millis(80),
    ) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(100)));
    if write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .is_err()
    {
        return false;
    }
    let mut response = [0u8; 64];
    let Ok(read) = stream.read(&mut response) else {
        return false;
    };
    let line_end = response[..read]
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(read);
    let Ok(line) = std::str::from_utf8(&response[..line_end]) else {
        return false;
    };
    let mut fields = line.split_ascii_whitespace();
    fields
        .next()
        .is_some_and(|version| version.starts_with("HTTP/1."))
        && fields
            .next()
            .and_then(|status| status.parse::<u16>().ok())
            .is_some_and(|status| (200..300).contains(&status))
}

#[test]
fn setup_with_fake_local_servers_writes_only_config() {
    let home = TempDir::new().unwrap();
    let runtime = TempDir::new().unwrap();
    let config_home = home.path().join(".config");
    let fake_ollama = fake_model_server(11434);
    let fake_lm = fake_model_server(1234);
    let ollama_available = fake_ollama || model_server_responds(11434, "/api/tags");
    let lm_available = fake_lm || model_server_responds(1234, "/v1/models");
    assert!(
        ollama_available || lm_available,
        "need a loopback model endpoint on 11434 or 1234"
    );

    let mut cmd = bin();
    cmd.env("HOME", home.path());
    cmd.env("XDG_CONFIG_HOME", &config_home);
    cmd.env("XDG_RUNTIME_DIR", runtime.path());
    cmd.args(["--json", "ai", "setup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("127.0.0.1"));

    let config = config_home.join("omacell/config.toml");
    assert!(config.is_file(), "setup must write {}", config.display());
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(text.contains("openai_compatible"), "{text}");
    assert!(text.contains("local = true"), "{text}");
    assert!(!text.contains("secret_env"), "{text}");
    assert!(!text.contains("secret_cmd"), "{text}");
    if ollama_available {
        assert!(text.contains("11434"), "{text}");
    }
    if lm_available {
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
