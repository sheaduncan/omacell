//! Recorded-fixture tests for both wire protocols.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use omacell_ai::http::ReplayTransport;
use omacell_ai::provider::{
    Cancel, ChatMessage, ChatRequest, Provider, Role, ToolCall, ToolSpec, provider_from_config,
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
            tool_call_id: None,
            tool_calls: vec![],
        }],
        json_schema: None,
        tools: vec![],
        stream,
        max_output_tokens: 0,
        cancel: Cancel::new(),
        timeout: Duration::from_secs(2),
    }
}

fn tool_result_request(model: &str) -> ChatRequest {
    ChatRequest {
        model: model.into(),
        messages: vec![
            ChatMessage {
                role: Role::User,
                content: "continue tool".into(),
                tool_call_id: None,
                tool_calls: vec![],
            },
            ChatMessage {
                role: Role::Assistant,
                content: String::new(),
                tool_call_id: None,
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "sum".into(),
                    arguments: "{\"a\":1}".into(),
                }],
            },
            ChatMessage {
                role: Role::Tool,
                content: "1".into(),
                tool_call_id: Some("call_1".into()),
                tool_calls: vec![],
            },
        ],
        json_schema: None,
        tools: vec![ToolSpec {
            name: "sum".into(),
            description: "add".into(),
            parameters: json!({"type":"object"}),
        }],
        stream: false,
        max_output_tokens: 0,
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
    assert_eq!(out.tool_calls[0].id, "call_1");
    assert_eq!(out.tool_calls[0].arguments, "{\"a\":1}");
    assert_eq!(out.usage.prompt_tokens, 1);
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
async fn openai_encodes_correlated_tool_results() {
    let out = openai().chat(tool_result_request("qwen")).await.unwrap();
    assert_eq!(out.text, "done");
}

#[tokio::test]
async fn openai_sends_output_limit_and_rejects_malformed_tool_calls() {
    let mut request = req("invalid tool response", false);
    request.max_output_tokens = 321;
    let err = openai().chat(request).await.unwrap_err();
    assert_eq!(err.code, "ai.provider");
}

#[tokio::test]
async fn openai_uses_current_output_limit_for_o_series() {
    let mut request = req("o-series limit", false);
    request.model = "openai/o3".into();
    request.max_output_tokens = 321;
    let out = openai().chat(request).await.unwrap();
    assert_eq!(out.text, "bounded");
}

#[tokio::test]
async fn openai_cancel_interrupts_in_flight_transport() {
    let p = openai();
    let r = req("slow", false);
    let cancel = r.cancel.clone();
    let task = tokio::spawn(async move { p.chat(r).await });
    tokio::time::sleep(Duration::from_millis(10)).await;
    cancel.cancel();
    let err = task.await.unwrap().unwrap_err();
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
            tool_call_id: None,
            tool_calls: vec![],
        }],
        json_schema: Some(json!({"type":"object","properties":{"ok":{"type":"boolean"}}})),
        tools: vec![],
        stream: false,
        max_output_tokens: 0,
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
            tool_call_id: None,
            tool_calls: vec![],
        }],
        json_schema: None,
        tools: vec![ToolSpec {
            name: "sum".into(),
            description: "add".into(),
            parameters: json!({"type":"object"}),
        }],
        stream: false,
        max_output_tokens: 0,
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
            tool_call_id: None,
            tool_calls: vec![],
        }],
        json_schema: None,
        tools: vec![],
        stream: true,
        max_output_tokens: 0,
        cancel: Cancel::new(),
        timeout: Duration::from_secs(2),
    };
    let out = p.chat(stream).await.unwrap();
    assert_eq!(out.text, "Hi");
    assert_eq!(out.tool_calls[0].id, "tool_1");
    assert_eq!(out.tool_calls[0].arguments, "{\"a\":1}");
    assert_eq!(out.usage.prompt_tokens, 2);
    assert!(out.streamed);

    let fail = ChatRequest {
        model: "claude".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: "fail".into(),
            tool_call_id: None,
            tool_calls: vec![],
        }],
        json_schema: None,
        tools: vec![],
        stream: false,
        max_output_tokens: 0,
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
            tool_call_id: None,
            tool_calls: vec![],
        }],
        json_schema: None,
        tools: vec![],
        stream: false,
        max_output_tokens: 0,
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
            tool_call_id: None,
            tool_calls: vec![],
        }],
        json_schema: Some(json!({"type":"object","properties":{"ok":{"type":"boolean"}}})),
        tools: vec![],
        stream: false,
        max_output_tokens: 0,
        cancel: Cancel::new(),
        timeout: Duration::from_secs(2),
    };
    r.cancel.cancel();
    let err = p.chat(r).await.unwrap_err();
    assert_eq!(err.code, "ai.cancelled");
}

#[tokio::test]
async fn anthropic_encodes_correlated_tool_results() {
    let out = anthropic()
        .chat(tool_result_request("claude"))
        .await
        .unwrap();
    assert_eq!(out.text, "done");
}

#[tokio::test]
async fn anthropic_sends_output_limit_and_rejects_malformed_tool_calls() {
    let mut request = req("invalid claude tool response", false);
    request.max_output_tokens = 321;
    let err = anthropic().chat(request).await.unwrap_err();
    assert_eq!(err.code, "ai.provider");
}

#[tokio::test]
async fn anthropic_cancel_interrupts_in_flight_transport() {
    let p = anthropic();
    let r = ChatRequest {
        model: "claude".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: "slow".into(),
            tool_call_id: None,
            tool_calls: vec![],
        }],
        json_schema: None,
        tools: vec![],
        stream: false,
        max_output_tokens: 0,
        cancel: Cancel::new(),
        timeout: Duration::from_secs(2),
    };
    let cancel = r.cancel.clone();
    let task = tokio::spawn(async move { p.chat(r).await });
    tokio::time::sleep(Duration::from_millis(10)).await;
    cancel.cancel();
    let err = task.await.unwrap().unwrap_err();
    assert_eq!(err.code, "ai.cancelled");
}

#[test]
fn routing_and_loopback() {
    assert!(omacell_ai::endpoint_is_loopback(
        "http://127.0.0.1:11434/v1"
    ));
    assert!(omacell_ai::endpoint_is_loopback("http://localhost:1234/v1"));
    assert!(!omacell_ai::endpoint_is_loopback("http://0.0.0.0:1234/v1"));
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

#[tokio::test]
async fn secret_resolution_is_deferred_until_send() {
    let spec = AiProvider {
        kind: "openai_compatible".into(),
        endpoint: "http://127.0.0.1:11434/v1".into(),
        local: true,
        secret_env: Some("OMACELL_TEST_SECRET_THAT_IS_NOT_SET".into()),
        secret_cmd: None,
        timeout: 0,
        headers: Default::default(),
    };
    let transport = Arc::new(ReplayTransport::from_dir(fixture_dir()).unwrap());
    let provider = provider_from_config("test", &spec, transport).unwrap();
    let mut request = req("return json", false);
    request.json_schema = Some(json!({"type":"object","properties":{"ok":{"type":"boolean"}}}));
    let err = provider.chat(request).await.unwrap_err();
    assert_eq!(err.code, "ai.secret");
}

#[test]
fn plaintext_secret_headers_are_rejected() {
    let spec = AiProvider {
        kind: "openai_compatible".into(),
        endpoint: "http://127.0.0.1:11434/v1".into(),
        local: true,
        secret_env: None,
        secret_cmd: None,
        timeout: 0,
        headers: [("Authorization".into(), "Bearer secret".into())]
            .into_iter()
            .collect(),
    };
    let transport = Arc::new(ReplayTransport::from_dir(fixture_dir()).unwrap());
    match provider_from_config("test", &spec, transport) {
        Ok(_) => panic!("expected plaintext secret header to fail"),
        Err(err) => assert_eq!(err.code, "ai.secret"),
    }
}
