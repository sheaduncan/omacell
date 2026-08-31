//! OpenAI-compatible chat completions (Ollama, LM Studio, llama.cpp, vLLM, OpenRouter).

use std::collections::BTreeMap;

use async_trait::async_trait;
use omacell_conf::schema::AiProvider;
use serde_json::{Value, json};

use crate::error::{AiError, codes};
use crate::http::{HttpRequest, SharedTransport};
use crate::provider::{ChatRequest, ChatResponse, Provider, Role, ToolCall, Usage};
use crate::secrets::resolve_secret;

/// OpenAI-compatible provider.
pub struct OpenAiCompat {
    name: String,
    endpoint: String,
    local: bool,
    headers: BTreeMap<String, String>,
    transport: SharedTransport,
}

impl OpenAiCompat {
    /// From config.
    pub fn new(name: &str, spec: &AiProvider, transport: SharedTransport) -> Result<Self, AiError> {
        let mut headers = spec.headers.clone();
        if let Some(secret) = resolve_secret(spec)? {
            headers
                .entry("Authorization".into())
                .or_insert_with(|| format!("Bearer {secret}"));
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
impl Provider for OpenAiCompat {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, AiError> {
        if req.cancel.is_cancelled() {
            return Err(AiError::new(codes::CANCELLED, "request cancelled"));
        }
        let mut body = json!({
            "model": req.model,
            "messages": req.messages.iter().map(|m| json!({
                "role": match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                },
                "content": m.content,
            })).collect::<Vec<_>>(),
            "stream": req.stream,
        });
        if let Some(schema) = &req.json_schema {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": { "name": "omacell", "schema": schema, "strict": true }
            });
        }
        if !req.tools.is_empty() {
            body["tools"] = json!(
                req.tools
                    .iter()
                    .map(|tool| json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        }
                    }))
                    .collect::<Vec<_>>()
            );
        }
        let url = format!("{}/chat/completions", self.endpoint);
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
            return Ok(assemble_openai_stream(&response.chunks));
        }
        parse_openai_json(&response.body)
    }

    fn is_local(&self) -> bool {
        self.local
    }

    fn name(&self) -> &str {
        &self.name
    }
}

fn parse_openai_json(body: &Value) -> Result<ChatResponse, AiError> {
    let choice = body
        .pointer("/choices/0/message")
        .ok_or_else(|| AiError::new(codes::PROVIDER, "openai response missing choices"))?;
    let text = choice
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let tool_calls = choice
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    Some(ToolCall {
                        id: call.get("id")?.as_str()?.to_string(),
                        name: call.pointer("/function/name")?.as_str()?.to_string(),
                        arguments: call
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let usage = Usage {
        prompt_tokens: body
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        completion_tokens: body
            .pointer("/usage/completion_tokens")
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

fn assemble_openai_stream(chunks: &[Value]) -> ChatResponse {
    let mut text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut usage = Usage::default();
    for chunk in chunks {
        if let Some(delta) = chunk
            .pointer("/choices/0/delta/content")
            .and_then(Value::as_str)
        {
            text.push_str(delta);
        }
        if let Some(calls) = chunk
            .pointer("/choices/0/delta/tool_calls")
            .and_then(Value::as_array)
        {
            for call in calls {
                if let (Some(id), Some(name)) = (
                    call.get("id").and_then(Value::as_str),
                    call.pointer("/function/name").and_then(Value::as_str),
                ) {
                    tool_calls.push(ToolCall {
                        id: id.to_string(),
                        name: name.to_string(),
                        arguments: call
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}")
                            .to_string(),
                    });
                }
            }
        }
        if let Some(p) = chunk
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64)
        {
            usage.prompt_tokens = p as u32;
        }
        if let Some(c) = chunk
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64)
        {
            usage.completion_tokens = c as u32;
        }
    }
    ChatResponse {
        text,
        tool_calls,
        usage,
        streamed: true,
    }
}
