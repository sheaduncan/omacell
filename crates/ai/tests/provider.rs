//! Recorded-fixture tests for both wire protocols.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use omacell_ai::http::ReplayTransport;
use omacell_ai::provider::{
    Cancel, ChatMessage, ChatRequest, Provider, Role, ToolSpec, provider_from_config,
};
use omacell_conf::schema::AiProvider;
use serde_json::json;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/ai")
}

fn openai() -> Box<dyn Provider> {
    let spec = AiProvider {
        kind: "openai_compatible".into(),
        endpoint: "http://127.0.0.1:11434/v1".into(),
        local: true,
        secret_env: None,
        secret_cmd: None,
        timeout: 0,
        headers: Default::default(),
    };
    let transport = Arc::new(ReplayTransport::from_dir(fixture_dir()).unwrap());
    provider_from_config("ollama", &spec, transport).unwrap()
}

fn anthropic() -> Box<dyn Provider> {
    let spec = AiProvider {
        kind: "anthropic".into(),
        endpoint: "http://127.0.0.1:9".into(),
        local: true,
        secret_env: None,
        secret_cmd: None,
        timeout: 0,
        headers: Default::default(),
    };
    let transport = Arc::new(ReplayTransport::from_dir(fixture_dir()).unwrap());
    provider_from_config("anthropic", &spec, transport).unwrap()
}

fn req(content: &str, stream: bool) -> ChatRequest {
    ChatRequest {
        model: if content.contains("claude") {
            "claude".into()
        } else {
            "qwen".into()
        },
        messages: vec![ChatMessage {
            role: Role::User,
            content: content.into(),
        }],
        json_schema: None,
        tools: vec![],
        stream,
        cancel: Cancel::new(),
        timeout: Duration::from_secs(2),
    }
}

#[tokio::test]
async fn openai_structured_tools_stream_error_timeout() {
    let p = openai();
    let mut structured = req("return json", false);
    structured.model = "qwen".into();
    structured.json_schema = Some(json!({"type":"object","properties":{"ok":{"type":"boolean"}}}));
    let out = p.chat(structured).await.unwrap();
    assert_eq!(out.text, "{\"ok\":true}");
    assert_eq!(out.usage.prompt_tokens, 4);

    let mut tools = req("call tool", false);
    tools.tools = vec![ToolSpec {
        name: "sum".into(),
        description: "add".into(),
        parameters: json!({"type":"object"}),
    }];
    let out = p.chat(tools).await.unwrap();
    assert_eq!(out.tool_calls[0].name, "sum");

    let stream = req("stream please", true);
    let out = p.chat(stream).await.unwrap();
    assert_eq!(out.text, "Hello");
    assert!(out.streamed);

    let err = p.chat(req("fail", false)).await.unwrap_err();
    assert_eq!(err.code, "ai.provider");

    let mut slow = req("slow", false);
    slow.timeout = Duration::from_millis(20);
    let err = p.chat(slow).await.unwrap_err();
    assert_eq!(err.code, "ai.timeout");
}

#[tokio::test]
async fn openai_cancel_before_send() {
    let p = openai();
    let mut r = req("return json", false);
    r.json_schema = Some(json!({"type":"object","properties":{"ok":{"type":"boolean"}}}));
    r.cancel.cancel();
    let err = p.chat(r).await.unwrap_err();
    assert_eq!(err.code, "ai.cancelled");
}

#[tokio::test]
async fn anthropic_structured_tools_stream_error() {
    let p = anthropic();
    let structured = ChatRequest {
        model: "claude".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: "return json".into(),
        }],
        json_schema: Some(json!({"type":"object","properties":{"ok":{"type":"boolean"}}})),
        tools: vec![],
        stream: false,
        cancel: Cancel::new(),
        timeout: Duration::from_secs(2),
    };
    let out = p.chat(structured).await.unwrap();
    assert_eq!(out.text, "{\"ok\":true}");

    let tools = ChatRequest {
        model: "claude".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: "call tool".into(),
        }],
        json_schema: None,
        tools: vec![ToolSpec {
            name: "sum".into(),
            description: "add".into(),
            parameters: json!({"type":"object"}),
        }],
        stream: false,
        cancel: Cancel::new(),
        timeout: Duration::from_secs(2),
    };
    let out = p.chat(tools).await.unwrap();
    assert_eq!(out.tool_calls[0].name, "sum");

    let stream = ChatRequest {
        model: "claude".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: "stream please".into(),
        }],
        json_schema: None,
        tools: vec![],
        stream: true,
        cancel: Cancel::new(),
        timeout: Duration::from_secs(2),
    };
    let out = p.chat(stream).await.unwrap();
    assert_eq!(out.text, "Hi");
    assert!(out.streamed);

    let fail = ChatRequest {
        model: "claude".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: "fail".into(),
        }],
        json_schema: None,
        tools: vec![],
        stream: false,
        cancel: Cancel::new(),
        timeout: Duration::from_secs(2),
    };
    let err = p.chat(fail).await.unwrap_err();
    assert_eq!(err.code, "ai.provider");

    let slow = ChatRequest {
        model: "claude".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: "slow".into(),
        }],
        json_schema: None,
        tools: vec![],
        stream: false,
        cancel: Cancel::new(),
        timeout: Duration::from_millis(20),
    };
    let err = p.chat(slow).await.unwrap_err();
    assert_eq!(err.code, "ai.timeout");
}

#[tokio::test]
async fn anthropic_cancel_before_send() {
    let p = anthropic();
    let r = ChatRequest {
        model: "claude".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: "return json".into(),
        }],
        json_schema: Some(json!({"type":"object","properties":{"ok":{"type":"boolean"}}})),
        tools: vec![],
        stream: false,
        cancel: Cancel::new(),
        timeout: Duration::from_secs(2),
    };
    r.cancel.cancel();
    let err = p.chat(r).await.unwrap_err();
    assert_eq!(err.code, "ai.cancelled");
}

#[test]
fn routing_and_loopback() {
    assert!(omacell_ai::endpoint_is_loopback(
        "http://127.0.0.1:11434/v1"
    ));
    assert!(omacell_ai::endpoint_is_loopback("http://localhost:1234/v1"));
    assert!(!omacell_ai::endpoint_is_loopback(
        "https://api.openai.com/v1"
    ));
    let spec = AiProvider {
        kind: "mystery".into(),
        endpoint: "http://127.0.0.1:9".into(),
        local: true,
        secret_env: None,
        secret_cmd: None,
        timeout: 0,
        headers: Default::default(),
    };
    let transport = Arc::new(ReplayTransport::from_dir(fixture_dir()).unwrap());
    match provider_from_config("x", &spec, transport) {
        Ok(_) => panic!("expected unknown kind"),
        Err(err) => assert_eq!(err.code, "ai.kind"),
    }
}
