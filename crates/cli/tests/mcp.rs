//! rmcp client contract tests against `omacell mcp`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin;
use omacell_bus::mcp::tool_names;
use rmcp::model::{CallToolRequestParams, CallToolResult, ReadResourceRequestParams};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Map, Value, json};
use tempfile::TempDir;
use tokio::net::UnixStream;

fn corpus_xlsx() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/xlsx/l1_values.xlsx")
}

fn wait_socket(path: &std::path::Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_ipc(runtime: &std::path::Path) {
    let start = Instant::now();
    loop {
        if runtime.exists()
            && std::fs::read_dir(runtime).is_ok_and(|mut d| {
                d.any(|e| {
                    e.ok()
                        .is_some_and(|e| e.path().extension().is_some_and(|ext| ext == "sock"))
                })
            })
        {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for IPC in {}",
            runtime.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

async fn connect(path: PathBuf) -> rmcp::service::RunningService<RoleClient, ()> {
    wait_socket(&path);
    let stream = UnixStream::connect(&path)
        .await
        .unwrap_or_else(|err| panic!("connect {}: {err}", path.display()));
    ().serve(stream).await.expect("mcp handshake")
}

fn args_map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

async fn call_tool(
    client: &rmcp::service::RunningService<RoleClient, ()>,
    name: &str,
    args: Value,
) -> Result<Value, String> {
    let result: CallToolResult = client
        .call_tool(CallToolRequestParams::new(name.to_string()).with_arguments(args_map(args)))
        .await
        .map_err(|err| err.to_string())?;
    let text = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("");
    if result.is_error.unwrap_or(false) {
        Err(text)
    } else {
        Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
}

#[tokio::test]
async fn rmcp_client_exercises_every_tool_and_resource() {
    let home = TempDir::new().unwrap();
    let runtime = home.path().join("run");
    std::fs::create_dir_all(&runtime).unwrap();
    let socket = home.path().join("mcp.sock");
    let book = corpus_xlsx();
    let mut child = std::process::Command::new(cargo_bin("omacell"))
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", &runtime)
        .args([
            "mcp",
            "--socket",
            socket.to_str().unwrap(),
            "--book",
            book.to_str().unwrap(),
        ])
        .spawn()
        .unwrap();

    let client = connect(socket.clone()).await;
    let listed = client.list_tools(Default::default()).await.unwrap();
    let names: Vec<String> = listed.tools.iter().map(|t| t.name.to_string()).collect();
    for tool in tool_names() {
        assert!(names.contains(&tool.to_string()), "missing {tool}");
    }

    let _ = call_tool(
        &client,
        "workbook_open",
        json!({"path": book.display().to_string()}),
    )
    .await;

    for tool in [
        "workbook_list",
        "sheet_list",
        "commands_list",
        "recalc",
        "audit",
        "card",
        "changeset_list",
    ] {
        call_tool(&client, tool, json!({}))
            .await
            .unwrap_or_else(|err| panic!("{tool}: {err}"));
    }

    let page = call_tool(
        &client,
        "range_read",
        json!({"range": "A1:A1", "offset": 0, "limit": 1}),
    )
    .await
    .unwrap();
    assert!(page.get("rows").is_some());

    let err = call_tool(
        &client,
        "range_read",
        json!({"range": "A1", "fields": ["nope"]}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("mcp.args") || err.contains("unknown range_read"));

    let proposed = call_tool(
        &client,
        "range_write",
        json!({"range": "Z1:Z1", "values": [["mcp"]]}),
    )
    .await
    .unwrap();
    assert_eq!(proposed["status"], "proposed");

    let resources = client.list_resources(Default::default()).await.unwrap();
    assert!(!resources.resources.is_empty());
    let uri = resources.resources[0].uri.clone();
    let read = client
        .read_resource(ReadResourceRequestParams::new(uri))
        .await
        .unwrap();
    assert!(!read.contents.is_empty());

    let render_err = call_tool(&client, "render", json!({"range": "A1"}))
        .await
        .unwrap_err();
    assert!(render_err.contains("GUI not running"));

    drop(client);
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn headless_proposal_appears_in_changeset_cli() {
    let home = TempDir::new().unwrap();
    let runtime = home.path().join("run");
    std::fs::create_dir_all(&runtime).unwrap();
    let socket = home.path().join("mcp.sock");
    let book = corpus_xlsx();
    let mut child = std::process::Command::new(cargo_bin("omacell"))
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", &runtime)
        .args([
            "mcp",
            "--socket",
            socket.to_str().unwrap(),
            "--book",
            book.to_str().unwrap(),
        ])
        .spawn()
        .unwrap();

    let client = connect(socket.clone()).await;
    let proposed = call_tool(
        &client,
        "range_write",
        json!({"range": "A1:A1", "values": [["from-mcp"]]}),
    )
    .await
    .unwrap();
    let id = proposed["id"].as_str().unwrap().to_string();
    wait_ipc(&runtime.join("omacell"));

    let listed = std::process::Command::new(cargo_bin("omacell"))
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", &runtime)
        .args(["--json", "changeset", "list"])
        .output()
        .unwrap();
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let json: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert!(json.as_array().unwrap().iter().any(|cs| cs["id"] == id));

    let applied = std::process::Command::new(cargo_bin("omacell"))
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", &runtime)
        .args(["--json", "changeset", "apply", &id])
        .output()
        .unwrap();
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let applied_json: Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(applied_json["status"], "applied");

    let reverted = std::process::Command::new(cargo_bin("omacell"))
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", &runtime)
        .args(["--json", "changeset", "revert", &id])
        .output()
        .unwrap();
    assert!(
        reverted.status.success(),
        "{}",
        String::from_utf8_lossy(&reverted.stderr)
    );
    let reverted_json: Value = serde_json::from_slice(&reverted.stdout).unwrap();
    assert_eq!(reverted_json["status"], "reverted");

    drop(client);
    let _ = child.kill();
    let _ = child.wait();
}
