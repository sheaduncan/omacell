//! Anthropic Messages API.

use std::collections::BTreeMap;

use async_trait::async_trait;
use omacell_conf::schema::AiProvider;
use serde_json::{Value, json};

use crate::error::{AiError, codes};
use crate::http::{HttpRequest, SharedTransport};
use crate::provider::{
    ChatRequest, ChatResponse, Provider, Role, ToolCall, Usage, resolve_request_secret,
    send_with_controls, validate_chat_request, validate_provider_spec, validate_tool_call,
};

/// Anthropic Messages provider.
pub struct Anthropic {
    name: String,
    endpoint: String,
    local: bool,
    headers: BTreeMap<String, String>,
    secret: AiProvider,
    transport: SharedTransport,
}

impl Anthropic {
    /// From config.
    pub fn new(name: &str, spec: &AiProvider, transport: SharedTransport) -> Result<Self, AiError> {
        validate_provider_spec(spec)?;
        let mut headers = spec.headers.clone();
        headers
            .entry("anthropic-version".into())
            .or_insert_with(|| "2023-06-01".into());
        Ok(Self {
            name: name.to_string(),
            endpoint: spec.endpoint.trim_end_matches('/').to_string(),
            local: spec.local || crate::provider::endpoint_is_loopback(&spec.endpoint),
            headers,
            secret: spec.clone(),
            transport,
        })
    }
}

#[async_trait]
impl Provider for Anthropic {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, AiError> {
        validate_chat_request(&req)?;
        let started = tokio::time::Instant::now();
        let system = req
            .messages
            .iter()
            .filter(|m| matches!(m.role, Role::System))
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let messages: Vec<Value> = req
            .messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .map(anthropic_message)
            .collect::<Result<_, _>>()?;
        let mut body = json!({
            "model": req.model,
            "max_tokens": if req.max_output_tokens == 0 { 1024 } else { req.max_output_tokens },
            "messages": messages,
            "stream": req.stream,
        });
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if let Some(schema) = &req.json_schema {
            body["output_config"] = json!({"format": {"type": "json_schema", "schema": schema}});
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
        let mut headers = self.headers.clone();
        let secret = resolve_request_secret(self.secret.clone(), &req.cancel, req.timeout).await?;
        if let Some(secret) = secret {
            headers.insert("x-api-key".into(), secret);
        }
        let url = format!("{}/v1/messages", self.endpoint);
        let http = HttpRequest {
            url,
            headers,
            body,
            stream: req.stream,
        };
        let remaining = req
            .timeout
            .checked_sub(started.elapsed())
            .ok_or_else(|| AiError::new(codes::TIMEOUT, "provider deadline exceeded"))?;
        let response = send_with_controls(&self.transport, http, &req.cancel, remaining).await?;
        if !(200..300).contains(&response.status) {
            return Err(AiError::new(
                codes::PROVIDER,
                format!("provider HTTP {}", response.status),
            ));
        }
        if req.stream {
            return assemble_anthropic_stream(&response.chunks);
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

fn anthropic_message(message: &crate::provider::ChatMessage) -> Result<Value, AiError> {
    match message.role {
        Role::System => unreachable!("system messages are filtered before encoding"),
        Role::User => Ok(json!({"role": "user", "content": message.content})),
        Role::Tool => Ok(json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": message.tool_call_id,
                "content": message.content,
            }],
        })),
        Role::Assistant if message.tool_calls.is_empty() => {
            Ok(json!({"role": "assistant", "content": message.content}))
        }
        Role::Assistant => {
            let mut content = Vec::new();
            if !message.content.is_empty() {
                content.push(json!({"type": "text", "text": message.content}));
            }
            for call in &message.tool_calls {
                let input: Value = serde_json::from_str(&call.arguments).map_err(|err| {
                    AiError::new(
                        codes::PAYLOAD,
                        format!("anthropic tool input is not JSON: {err}"),
                    )
                })?;
                content.push(json!({
                    "type": "tool_use",
                    "id": call.id,
                    "name": call.name,
                    "input": input,
                }));
            }
            Ok(json!({"role": "assistant", "content": content}))
        }
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
                    let id = block.get("id").and_then(Value::as_str).ok_or_else(|| {
                        AiError::new(codes::PROVIDER, "anthropic tool use missing id")
                    })?;
                    let name = block.get("name").and_then(Value::as_str).ok_or_else(|| {
                        AiError::new(codes::PROVIDER, "anthropic tool use missing name")
                    })?;
                    let arguments = block.get("input").ok_or_else(|| {
                        AiError::new(codes::PROVIDER, "anthropic tool use missing input")
                    })?;
                    let arguments = arguments.to_string();
                    validate_tool_call(id, name, &arguments, codes::PROVIDER)?;
                    tool_calls.push(ToolCall {
                        id: id.into(),
                        name: name.into(),
                        arguments,
                    });
                }
                _ => {}
            }
        }
    }
    if tool_calls.len() > 128 {
        return Err(AiError::new(
            codes::PROVIDER,
            "too many provider tool calls",
        ));
    }
    let usage = Usage {
        prompt_tokens: body
            .pointer("/usage/input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .try_into()
            .unwrap_or(u32::MAX),
        completion_tokens: body
            .pointer("/usage/output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .try_into()
            .unwrap_or(u32::MAX),
    };
    Ok(ChatResponse {
        text,
        tool_calls,
        usage,
        streamed: false,
    })
}

#[derive(Default)]
struct ToolBuilder {
    id: String,
    name: String,
    arguments: String,
}

fn assemble_anthropic_stream(chunks: &[Value]) -> Result<ChatResponse, AiError> {
    let mut text = String::new();
    let mut builders: BTreeMap<u64, ToolBuilder> = BTreeMap::new();
    let mut usage = Usage::default();
    for chunk in chunks {
        match chunk.get("type").and_then(Value::as_str) {
            Some("error") => {
                return Err(AiError::new(
                    codes::PROVIDER,
                    format!("provider stream error: {}", chunk["error"]),
                ));
            }
            Some("message_start") => {
                if let Some(p) = chunk
                    .pointer("/message/usage/input_tokens")
                    .and_then(Value::as_u64)
                {
                    usage.prompt_tokens = p.try_into().unwrap_or(u32::MAX);
                }
            }
            Some("content_block_delta") => {
                if let Some(t) = chunk.pointer("/delta/text").and_then(Value::as_str) {
                    text.push_str(t);
                }
                if chunk.pointer("/delta/type").and_then(Value::as_str) == Some("input_json_delta")
                {
                    let index = chunk.get("index").and_then(Value::as_u64).unwrap_or(0);
                    if let Some(partial) =
                        chunk.pointer("/delta/partial_json").and_then(Value::as_str)
                    {
                        builders
                            .entry(index)
                            .or_default()
                            .arguments
                            .push_str(partial);
                    }
                }
            }
            Some("content_block_start") => {
                if chunk.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use")
                {
                    let index = chunk.get("index").and_then(Value::as_u64).unwrap_or(0);
                    let builder = builders.entry(index).or_default();
                    builder.id = chunk
                        .pointer("/content_block/id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    builder.name = chunk
                        .pointer("/content_block/name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if let Some(input) = chunk.pointer("/content_block/input")
                        && input.as_object().is_some_and(|object| !object.is_empty())
                    {
                        builder.arguments = input.to_string();
                    }
                }
            }
            Some("message_delta") => {
                if let Some(c) = chunk
                    .pointer("/usage/output_tokens")
                    .and_then(Value::as_u64)
                {
                    usage.completion_tokens = c.try_into().unwrap_or(u32::MAX);
                }
            }
            _ => {}
        }
    }
    let tool_calls = builders
        .into_values()
        .map(|builder| {
            if builder.id.is_empty() || builder.name.is_empty() {
                return Err(AiError::new(
                    codes::PROVIDER,
                    "anthropic stream returned an incomplete tool call",
                ));
            }
            Ok(ToolCall {
                id: builder.id,
                name: builder.name,
                arguments: if builder.arguments.is_empty() {
                    "{}".into()
                } else {
                    builder.arguments
                },
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if tool_calls.len() > 128 {
        return Err(AiError::new(
            codes::PROVIDER,
            "too many provider tool calls",
        ));
    }
    for call in &tool_calls {
        validate_tool_call(&call.id, &call.name, &call.arguments, codes::PROVIDER)?;
    }
    Ok(ChatResponse {
        text,
        tool_calls,
        usage,
        streamed: true,
    })
}
