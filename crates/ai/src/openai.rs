//! OpenAI-compatible chat completions (Ollama, LM Studio, llama.cpp, vLLM, OpenRouter).

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

/// OpenAI-compatible provider.
pub struct OpenAiCompat {
    name: String,
    endpoint: String,
    local: bool,
    headers: BTreeMap<String, String>,
    secret: AiProvider,
    transport: SharedTransport,
}

impl OpenAiCompat {
    /// From config.
    pub fn new(name: &str, spec: &AiProvider, transport: SharedTransport) -> Result<Self, AiError> {
        validate_provider_spec(spec)?;
        Ok(Self {
            name: name.to_string(),
            endpoint: spec.endpoint.trim_end_matches('/').to_string(),
            local: spec.local || crate::provider::endpoint_is_loopback(&spec.endpoint),
            headers: spec.headers.clone(),
            secret: spec.clone(),
            transport,
        })
    }
}

#[async_trait]
impl Provider for OpenAiCompat {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, AiError> {
        validate_chat_request(&req)?;
        let started = tokio::time::Instant::now();
        let mut body = json!({
            "model": req.model,
            "messages": req.messages.iter().map(openai_message).collect::<Vec<_>>(),
            "stream": req.stream,
        });
        if req.stream {
            body["stream_options"] = json!({"include_usage": true});
        }
        if req.max_output_tokens > 0 {
            body[openai_output_limit_key(&req.model)] = json!(req.max_output_tokens);
        }
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
        let mut headers = self.headers.clone();
        let secret = resolve_request_secret(self.secret.clone(), &req.cancel, req.timeout).await?;
        if let Some(secret) = secret {
            headers.insert("Authorization".into(), format!("Bearer {secret}"));
        }
        let url = format!("{}/chat/completions", self.endpoint);
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
            return assemble_openai_stream(&response.chunks);
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

fn openai_output_limit_key(model: &str) -> &'static str {
    let model = model.rsplit('/').next().unwrap_or(model);
    if model
        .as_bytes()
        .get(0..2)
        .is_some_and(|prefix| prefix[0] == b'o' && prefix[1].is_ascii_digit())
    {
        "max_completion_tokens"
    } else {
        "max_tokens"
    }
}

fn openai_message(message: &crate::provider::ChatMessage) -> Value {
    let mut value = json!({
        "role": match message.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        },
        "content": message.content,
    });
    if let Some(tool_call_id) = &message.tool_call_id {
        value["tool_call_id"] = json!(tool_call_id);
    }
    if !message.tool_calls.is_empty() {
        value["tool_calls"] = json!(
            message
                .tool_calls
                .iter()
                .map(|call| json!({
                    "id": call.id,
                    "type": "function",
                    "function": {
                        "name": call.name,
                        "arguments": call.arguments,
                    }
                }))
                .collect::<Vec<_>>()
        );
    }
    value
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
    let mut tool_calls = Vec::new();
    if let Some(raw_calls) = choice.get("tool_calls") {
        let calls = raw_calls
            .as_array()
            .ok_or_else(|| AiError::new(codes::PROVIDER, "openai tool_calls is not an array"))?;
        if calls.len() > 128 {
            return Err(AiError::new(
                codes::PROVIDER,
                "too many provider tool calls",
            ));
        }
        for call in calls {
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::new(codes::PROVIDER, "openai tool call missing id"))?;
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .ok_or_else(|| AiError::new(codes::PROVIDER, "openai tool call missing name"))?;
            let arguments = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AiError::new(codes::PROVIDER, "openai tool call missing arguments")
                })?;
            validate_tool_call(id, name, arguments, codes::PROVIDER)?;
            tool_calls.push(ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: arguments.into(),
            });
        }
    }
    let usage = Usage {
        prompt_tokens: body
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .try_into()
            .unwrap_or(u32::MAX),
        completion_tokens: body
            .pointer("/usage/completion_tokens")
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

fn assemble_openai_stream(chunks: &[Value]) -> Result<ChatResponse, AiError> {
    let mut text = String::new();
    let mut builders: BTreeMap<u64, ToolBuilder> = BTreeMap::new();
    let mut usage = Usage::default();
    for chunk in chunks {
        if let Some(error) = chunk.get("error") {
            return Err(AiError::new(
                codes::PROVIDER,
                format!("provider stream error: {error}"),
            ));
        }
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
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
                let builder = builders.entry(index).or_default();
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    builder.id.push_str(id);
                }
                if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                    builder.name.push_str(name);
                }
                if let Some(arguments) = call.pointer("/function/arguments").and_then(Value::as_str)
                {
                    builder.arguments.push_str(arguments);
                }
            }
        }
        if let Some(p) = chunk
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64)
        {
            usage.prompt_tokens = p.try_into().unwrap_or(u32::MAX);
        }
        if let Some(c) = chunk
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64)
        {
            usage.completion_tokens = c.try_into().unwrap_or(u32::MAX);
        }
    }
    let tool_calls = builders
        .into_values()
        .map(|builder| {
            if builder.id.is_empty() || builder.name.is_empty() {
                return Err(AiError::new(
                    codes::PROVIDER,
                    "openai stream returned an incomplete tool call",
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
