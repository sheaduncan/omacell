//! Provider trait, chat types, slots, and routing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use omacell_conf::schema::{AiProvider, Config};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::anthropic::Anthropic;
use crate::error::{AiError, codes};
use crate::http::SharedTransport;
use crate::openai::OpenAiCompat;

/// Task slots in `[ai.models]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Slot {
    /// Fast / cheap.
    Fast,
    /// Default.
    Default,
    /// Strong.
    Strong,
    /// Agent.
    Agent,
    /// Vision.
    Vision,
}

impl Slot {
    /// Parse a slot name.
    pub fn parse(name: &str) -> Result<Self, AiError> {
        match name {
            "fast" => Ok(Self::Fast),
            "default" => Ok(Self::Default),
            "strong" => Ok(Self::Strong),
            "agent" => Ok(Self::Agent),
            "vision" => Ok(Self::Vision),
            other => Err(AiError::new(
                codes::KIND,
                format!("unknown model slot {other}"),
            )),
        }
    }
}

/// Chat role.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// System.
    System,
    /// User.
    User,
    /// Assistant.
    Assistant,
    /// Tool result.
    Tool,
}

/// One message.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    /// Role.
    pub role: Role,
    /// Text content.
    pub content: String,
}

/// Tool definition for tool-calling.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolSpec {
    /// Function name.
    pub name: String,
    /// Description.
    pub description: String,
    /// JSON Schema parameters.
    pub parameters: Value,
}

/// Model-requested tool call.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
    /// Call id.
    pub id: String,
    /// Function name.
    pub name: String,
    /// JSON arguments.
    pub arguments: String,
}

/// Token usage.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    /// Prompt tokens.
    pub prompt_tokens: u32,
    /// Completion tokens.
    pub completion_tokens: u32,
}

/// Chat request. Workbook content must already have passed [`crate::policy`].
#[derive(Clone, Debug)]
pub struct ChatRequest {
    /// Provider-qualified model (`qwen2.5:14b`).
    pub model: String,
    /// Messages.
    pub messages: Vec<ChatMessage>,
    /// Optional structured-output schema.
    pub json_schema: Option<Value>,
    /// Optional tools.
    pub tools: Vec<ToolSpec>,
    /// Stream deltas.
    pub stream: bool,
    /// Cancel flag.
    pub cancel: Cancel,
    /// Per-request timeout.
    pub timeout: Duration,
}

/// Cooperative cancel.
#[derive(Clone, Debug, Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
}

impl Cancel {
    /// Fresh flag.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancel.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Whether cancel was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// Chat response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChatResponse {
    /// Assistant text.
    pub text: String,
    /// Tool calls.
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// Usage.
    pub usage: Usage,
    /// Whether the body was assembled from a stream.
    pub streamed: bool,
}

/// A model endpoint.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Chat (JSON schema + tools + stream).
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, AiError>;
    /// Loopback / process-local.
    fn is_local(&self) -> bool;
    /// Config name.
    fn name(&self) -> &str;
}

/// Build a provider from a config block.
pub fn provider_from_config(
    name: &str,
    spec: &AiProvider,
    transport: SharedTransport,
) -> Result<Box<dyn Provider>, AiError> {
    match spec.kind.as_str() {
        "openai_compatible" => Ok(Box::new(OpenAiCompat::new(name, spec, transport)?)),
        "anthropic" => Ok(Box::new(Anthropic::new(name, spec, transport)?)),
        other => Err(
            AiError::new(codes::KIND, format!("unknown provider kind {other}"))
                .with_hint("kind is openai_compatible or anthropic"),
        ),
    }
}

/// Resolve `provider:model` for a slot. Empty strong/agent/vision fall back to default.
#[must_use]
pub fn route_slot(config: &Config, slot: Slot) -> (String, String) {
    let models = &config.ai.models;
    let spec = match slot {
        Slot::Fast => &models.fast,
        Slot::Default => &models.default,
        Slot::Strong if !models.strong.is_empty() => &models.strong,
        Slot::Agent if !models.agent.is_empty() => &models.agent,
        Slot::Vision if !models.vision.is_empty() => &models.vision,
        _ => &models.default,
    };
    split_route(spec)
}

fn split_route(spec: &str) -> (String, String) {
    match spec.split_once(':') {
        Some((provider, model)) => (provider.to_string(), model.to_string()),
        None => (spec.to_string(), spec.to_string()),
    }
}

/// Per-provider timeout. `0` means 30 seconds.
#[must_use]
pub fn provider_timeout(spec: &AiProvider) -> Duration {
    if spec.timeout == 0 {
        Duration::from_secs(30)
    } else {
        Duration::from_millis(u64::from(spec.timeout))
    }
}

/// Host is loopback.
#[must_use]
pub fn endpoint_is_loopback(endpoint: &str) -> bool {
    let rest = endpoint
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(endpoint);
    let host = rest.split('/').next().unwrap_or(rest);
    let host = host.split('@').next_back().unwrap_or(host);
    let host = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    let host = host.trim_matches(|c| c == '[' || c == ']');
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0")
}
