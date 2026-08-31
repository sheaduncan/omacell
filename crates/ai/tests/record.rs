use std::sync::Arc;

use omacell_ai::http::{HttpRequest, RecordingTransport, ReplayTransport, Transport};
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn recording_transport_omits_headers_and_writes_replay_shape() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/ai");
    let inner = Arc::new(ReplayTransport::from_dir(fixtures).unwrap());
    let directory = TempDir::new().unwrap();
    let recorder = RecordingTransport::new(inner, directory.path(), "openai_compatible").unwrap();
    recorder
        .send(HttpRequest {
            url: "http://127.0.0.1:11434/v1/chat/completions".into(),
            headers: [("Authorization".into(), "Bearer do-not-record".into())]
                .into_iter()
                .collect(),
            body: json!({
                "model": "qwen",
                "messages": [{"role":"user","content":"return json"}],
                "stream": false,
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "omacell",
                        "schema": {"type":"object","properties":{"ok":{"type":"boolean"}}},
                        "strict": true
                    }
                }
            }),
            stream: false,
        })
        .await
        .unwrap();

    let path = std::fs::read_dir(directory.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("openai_compatible"));
    assert!(!text.contains("Authorization"));
    assert!(!text.contains("do-not-record"));
}
