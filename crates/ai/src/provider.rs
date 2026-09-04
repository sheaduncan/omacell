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
use crate::http::{
    HttpRequest, HttpResponse, MAX_PROVIDER_REQUEST_BYTES, SharedTransport, validate_json,
    validate_request,
};
use crate::openai::OpenAiCompat;
use crate::secrets::resolve_secret;

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
    /// Correlated tool call for a [`Role::Tool`] result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool calls attached to an assistant-history message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
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
    /// Maximum output tokens (`0` uses the provider default).
    pub max_output_tokens: u32,
    /// Cancel flag.
    pub cancel: Cancel,
    /// Per-request timeout.
    pub timeout: Duration,
}

/// Cooperative cancel.
#[derive(Debug, Default)]
struct CancelState {
    flag: AtomicBool,
    notify: tokio::sync::Notify,
}

/// Cooperative cancellation shared between a request and its caller.
#[derive(Clone, Debug, Default)]
pub struct Cancel {
    state: Arc<CancelState>,
}

impl Cancel {
    /// Fresh flag.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancel.
    pub fn cancel(&self) {
        self.state.flag.store(true, Ordering::SeqCst);
        self.state.notify.notify_waiters();
    }

    /// Whether cancel was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.flag.load(Ordering::SeqCst)
    }

    pub(crate) async fn cancelled(&self) {
        loop {
            let notified = self.state.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

pub(crate) fn validate_chat_request(req: &ChatRequest) -> Result<(), AiError> {
    if req.messages.len() > 1_024 {
        return Err(AiError::new(codes::PAYLOAD, "too many chat messages"));
    }
    if req.tools.len() > 128 {
        return Err(AiError::new(codes::PAYLOAD, "too many chat tools"));
    }
    let mut bytes = req.model.len();
    for message in &req.messages {
        bytes = bytes.saturating_add(message.content.len());
        if matches!(message.role, Role::Tool) {
            if message
                .tool_call_id
                .as_deref()
                .is_none_or(|id| id.is_empty() || id.len() > 256)
            {
                return Err(AiError::new(
                    codes::PAYLOAD,
                    "tool-result messages require a bounded tool_call_id",
                ));
            }
        } else if message.tool_call_id.is_some() {
            return Err(AiError::new(
                codes::PAYLOAD,
                "tool_call_id is only valid on tool-result messages",
            ));
        }
        if !message.tool_calls.is_empty() && !matches!(message.role, Role::Assistant) {
            return Err(AiError::new(
                codes::PAYLOAD,
                "tool_calls are only valid on assistant messages",
            ));
        }
        if message.tool_calls.len() > 128 {
            return Err(AiError::new(
                codes::PAYLOAD,
                "too many tool calls on an assistant message",
            ));
        }
        for call in &message.tool_calls {
            validate_tool_call(&call.id, &call.name, &call.arguments, codes::PAYLOAD)?;
            bytes = bytes
                .saturating_add(call.id.len())
                .saturating_add(call.name.len())
                .saturating_add(call.arguments.len());
        }
    }
    for tool in &req.tools {
        if tool.name.len() > 256 || tool.description.len() > 16_384 {
            return Err(AiError::new(
                codes::PAYLOAD,
                "chat tool metadata exceeds its size limit",
            ));
        }
        validate_json(&tool.parameters)?;
        bytes = bytes
            .saturating_add(tool.name.len())
            .saturating_add(tool.description.len())
            .saturating_add(
                serde_json::to_vec(&tool.parameters)
                    .map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))?
                    .len(),
            );
    }
    if let Some(schema) = &req.json_schema {
        validate_json(schema)?;
        bytes = bytes.saturating_add(
            serde_json::to_vec(schema)
                .map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))?
                .len(),
        );
    }
    if bytes > MAX_PROVIDER_REQUEST_BYTES {
        return Err(AiError::new(
            codes::PAYLOAD,
            format!("chat request exceeds {MAX_PROVIDER_REQUEST_BYTES} bytes"),
        ));
    }
    Ok(())
}

pub(crate) fn validate_tool_call(
    id: &str,
    name: &str,
    arguments: &str,
    error_code: &'static str,
) -> Result<(), AiError> {
    if id.is_empty() || id.len() > 256 || name.is_empty() || name.len() > 256 {
        return Err(AiError::new(
            error_code,
            "tool-call metadata exceeds its size limit",
        ));
    }
    let arguments: Value = serde_json::from_str(arguments).map_err(|err| {
        AiError::new(
            error_code,
            format!("tool-call arguments are not JSON: {err}"),
        )
    })?;
    validate_json(&arguments).map_err(|err| AiError::new(error_code, err.message))?;
    if !arguments.is_object() {
        return Err(AiError::new(
            error_code,
            "tool-call arguments must be a JSON object",
        ));
    }
    Ok(())
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

pub(crate) fn validate_provider_spec(spec: &AiProvider) -> Result<(), AiError> {
    let url = reqwest::Url::parse(&spec.endpoint).map_err(|err| {
        AiError::new(codes::PROVIDER, format!("invalid provider endpoint: {err}"))
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AiError::new(
            codes::PROVIDER,
            "provider endpoint must be absolute HTTP or HTTPS",
        ));
    }
    if url.scheme() == "http" && !endpoint_is_loopback(&spec.endpoint) {
        return Err(AiError::new(
            codes::PROVIDER,
            "plaintext HTTP provider endpoints must use a loopback host",
        )
        .with_hint("use HTTPS for remote providers"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AiError::new(
            codes::PROVIDER,
            "provider endpoint must not contain credentials",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(AiError::new(
            codes::PROVIDER,
            "provider endpoint must not contain a query or fragment",
        ));
    }
    if spec.secret_env.is_some() && spec.secret_cmd.is_some() {
        return Err(AiError::new(
            codes::SECRET,
            "configure only one of secret_env or secret_cmd",
        ));
    }
    if spec
        .secret_env
        .as_deref()
        .is_some_and(|name| name.trim().is_empty())
    {
        return Err(AiError::new(codes::SECRET, "secret_env is empty"));
    }
    if spec
        .secret_cmd
        .as_deref()
        .is_some_and(|cmd| cmd.trim().is_empty())
    {
        return Err(AiError::new(codes::SECRET, "secret_cmd is empty"));
    }
    for name in spec.headers.keys() {
        let lower = name.to_ascii_lowercase();
        if [
            "authorization",
            "proxy-authorization",
            "cookie",
            "set-cookie",
            "x-api-key",
        ]
        .contains(&lower.as_str())
            || lower.contains("secret")
            || lower.contains("token")
        {
            return Err(AiError::new(
                codes::SECRET,
                format!("provider header {name} may contain a plaintext secret"),
            )
            .with_hint("use secret_env or secret_cmd"));
        }
    }
    Ok(())
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
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

pub(crate) async fn send_with_controls(
    transport: &SharedTransport,
    request: HttpRequest,
    cancel: &Cancel,
    timeout: Duration,
) -> Result<HttpResponse, AiError> {
    validate_request(&request)?;
    if cancel.is_cancelled() {
        return Err(AiError::new(codes::CANCELLED, "request cancelled"));
    }
    let send = transport.send(request);
    tokio::pin!(send);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(AiError::new(codes::CANCELLED, "request cancelled")),
        _ = &mut deadline => Err(AiError::new(codes::TIMEOUT, "provider deadline exceeded")),
        result = &mut send => result,
    }
}

pub(crate) async fn resolve_request_secret(
    spec: AiProvider,
    cancel: &Cancel,
    timeout: Duration,
) -> Result<Option<String>, AiError> {
    if spec.secret_env.is_none() && spec.secret_cmd.is_none() {
        return Ok(None);
    }
    let resolver = tokio::task::spawn_blocking(move || resolve_secret(&spec));
    tokio::pin!(resolver);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(AiError::new(codes::CANCELLED, "request cancelled")),
        _ = &mut deadline => Err(AiError::new(codes::TIMEOUT, "provider deadline exceeded while resolving its secret")),
        result = &mut resolver => result
            .map_err(|err| AiError::new(codes::SECRET, format!("secret resolver stopped: {err}")))?,
    }
}
