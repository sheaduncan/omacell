//! Anthropic Messages API.

use std::collections::BTreeMap;

use async_trait::async_trait;
use omacell_conf::schema::AiProvider;
use serde_json::{Value, json};

use crate::error::{AiError, codes};
use crate::http::{HttpRequest, SharedTransport};
use crate::provider::{ChatRequest, ChatResponse, Provider, Role, ToolCall, Usage};
use crate::secrets::resolve_secret;

/// Anthropic Messages provider.
pub struct Anthropic {
    name: String,
    endpoint: String,
    local: bool,
    headers: BTreeMap<String, String>,
    transport: SharedTransport,
}

impl Anthropic {
    /// From config.
    pub fn new(name: &str, spec: &AiProvider, transport: SharedTransport) -> Result<Self, AiError> {
        let mut headers = spec.headers.clone();
        headers
            .entry("anthropic-version".into())
            .or_insert_with(|| "2023-06-01".into());
        if let Some(secret) = resolve_secret(spec)? {
            headers.entry("x-api-key".into()).or_insert(secret);
        }
        Ok(Self {
            name: name.to_string(),
            endpoint: spec.endpoint.trim_end_matches('/').to_string(),
            local: spec.local || crate::provider::endpoint_is_loopback(&spec.endpoint),
            headers,
            transport,
        })
    }
}

#[async_trait]
impl Provider for Anthropic {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, AiError> {
        if req.cancel.is_cancelled() {
            return Err(AiError::new(codes::CANCELLED, "request cancelled"));
        }
        let system = req
            .messages
            .iter()
            .find(|m| matches!(m.role, Role::System))
            .map(|m| m.content.clone());
        let messages: Vec<Value> = req
            .messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .map(|m| {
                json!({
                    "role": match m.role {
                        Role::Assistant => "assistant",
                        _ => "user",
                    },
                    "content": m.content,
                })
            })
            .collect();
        let mut body = json!({
            "model": req.model,
            "max_tokens": 1024,
            "messages": messages,
            "stream": req.stream,
        });
        if let Some(system) = system {
            body["system"] = json!(system);
        }
        if let Some(schema) = &req.json_schema {
            body["output_format"] = json!({"type": "json_schema", "schema": schema});
        }
        if !req.tools.is_empty() {
            body["tools"] = json!(
                req.tools
                    .iter()
                    .map(|tool| json!({
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.parameters,
                    }))
                    .collect::<Vec<_>>()
            );
        }
        let url = format!("{}/v1/messages", self.endpoint);
        let http = HttpRequest {
            url,
            headers: self.headers.clone(),
            body,
            stream: req.stream,
        };
        let send = self.transport.send(http);
        let response = tokio::time::timeout(req.timeout, send)
            .await
            .map_err(|_| AiError::new(codes::TIMEOUT, "provider deadline exceeded"))??;
        if req.cancel.is_cancelled() {
            return Err(AiError::new(codes::CANCELLED, "request cancelled"));
        }
        if !(200..300).contains(&response.status) {
            return Err(AiError::new(
                codes::PROVIDER,
                format!("provider HTTP {}", response.status),
            ));
        }
        if req.stream {
            return Ok(assemble_anthropic_stream(&response.chunks));
        }
        parse_anthropic_json(&response.body)
    }

    fn is_local(&self) -> bool {
        self.local
    }

    fn name(&self) -> &str {
        &self.name
    }
}

fn parse_anthropic_json(body: &Value) -> Result<ChatResponse, AiError> {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    if let Some(blocks) = body.get("content").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(Value::as_str) {
                        text.push_str(t);
                    }
                }
                Some("tool_use") => {
                    tool_calls.push(ToolCall {
                        id: block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        name: block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        arguments: block
                            .get("input")
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "{}".into()),
                    });
                }
                _ => {}
            }
        }
    }
    let usage = Usage {
        prompt_tokens: body
            .pointer("/usage/input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        completion_tokens: body
            .pointer("/usage/output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
    };
    Ok(ChatResponse {
        text,
        tool_calls,
        usage,
        streamed: false,
    })
}

fn assemble_anthropic_stream(chunks: &[Value]) -> ChatResponse {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut usage = Usage::default();
    for chunk in chunks {
        match chunk.get("type").and_then(Value::as_str) {
            Some("content_block_delta") => {
                if let Some(t) = chunk.pointer("/delta/text").and_then(Value::as_str) {
                    text.push_str(t);
                }
            }
            Some("content_block_start") => {
                if chunk.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use")
                {
                    tool_calls.push(ToolCall {
                        id: chunk
                            .pointer("/content_block/id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        name: chunk
                            .pointer("/content_block/name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        arguments: chunk
                            .pointer("/content_block/input")
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "{}".into()),
                    });
                }
            }
            Some("message_delta") => {
                if let Some(p) = chunk.pointer("/usage/input_tokens").and_then(Value::as_u64) {
                    usage.prompt_tokens = p as u32;
                }
                if let Some(c) = chunk
                    .pointer("/usage/output_tokens")
                    .and_then(Value::as_u64)
                {
                    usage.completion_tokens = c as u32;
                }
            }
            _ => {}
        }
    }
    ChatResponse {
        text,
        tool_calls,
        usage,
        streamed: true,
    }
}
