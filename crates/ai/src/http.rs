//! HTTP transport: production `reqwest` and recorded replay (no network in tests).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AiError, codes};

/// One recorded HTTP exchange.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordedExchange {
    /// Protocol label (`openai_compatible` / `anthropic`).
    #[serde(default)]
    pub protocol: String,
    /// Request to match.
    pub request: RecordedRequest,
    /// Response to replay.
    pub response: RecordedResponse,
}

/// Matched request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordedRequest {
    /// HTTP method.
    pub method: String,
    /// URL path (and query).
    pub path: String,
    /// JSON body.
    #[serde(default)]
    pub body: Value,
}

/// Recorded response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordedResponse {
    /// Status code.
    pub status: u16,
    /// Response headers.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// JSON body, or a JSON array of SSE payloads when `stream` is true.
    pub body: Value,
    /// Server-sent events.
    #[serde(default)]
    pub stream: bool,
    /// Artificial delay before completing (milliseconds). Tests use this for timeouts.
    #[serde(default)]
    pub delay_ms: u64,
}

/// Outgoing JSON POST.
#[derive(Clone, Debug)]
pub struct HttpRequest {
    /// Full URL.
    pub url: String,
    /// Headers including authorization.
    pub headers: BTreeMap<String, String>,
    /// JSON body.
    pub body: Value,
    /// Stream the response.
    pub stream: bool,
}

/// HTTP response (possibly streamed as chunks).
#[derive(Clone, Debug)]
pub struct HttpResponse {
    /// Status.
    pub status: u16,
    /// JSON body for non-stream.
    pub body: Value,
    /// SSE JSON payloads for stream.
    pub chunks: Vec<Value>,
}

/// Pluggable HTTP.
#[async_trait]
pub trait Transport: Send + Sync {
    /// POST JSON.
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, AiError>;
}

/// Production `reqwest` client.
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    /// Build with a default timeout (overridden per request by `tokio::time`).
    pub fn new() -> Result<Self, AiError> {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .build()
            .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl Transport for ReqwestTransport {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, AiError> {
        let mut builder = self.client.post(&req.url).json(&req.body);
        for (key, value) in &req.headers {
            builder = builder.header(key, value);
        }
        let response = builder
            .send()
            .await
            .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?;
        let status = response.status().as_u16();
        let text = response
            .text()
            .await
            .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?;
        if req.stream {
            let chunks = parse_sse(&text);
            return Ok(HttpResponse {
                status,
                body: Value::Null,
                chunks,
            });
        }
        let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
        Ok(HttpResponse {
            status,
            body,
            chunks: Vec::new(),
        })
    }
}

/// Replay recorded fixtures. Never opens a socket.
pub struct ReplayTransport {
    exchanges: Vec<RecordedExchange>,
}

impl ReplayTransport {
    /// Load every `*.json` in a directory, sorted by name.
    pub fn from_dir(dir: impl AsRef<Path>) -> Result<Self, AiError> {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir.as_ref())
            .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        paths.sort();
        let mut exchanges = Vec::new();
        for path in paths {
            let text = std::fs::read_to_string(&path)
                .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?;
            let parsed: RecordedExchange = serde_json::from_str(&text).map_err(|err| {
                AiError::new(codes::PROVIDER, format!("{}: {err}", path.display()))
            })?;
            exchanges.push(parsed);
        }
        Ok(Self { exchanges })
    }

    /// One in-memory exchange (unit tests).
    #[must_use]
    pub fn from_exchanges(exchanges: Vec<RecordedExchange>) -> Self {
        Self { exchanges }
    }
}

#[async_trait]
impl Transport for ReplayTransport {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, AiError> {
        let path = url_path(&req.url);
        let body = canonicalize(&req.body);
        let found = self.exchanges.iter().find(|ex| {
            ex.request.method.eq_ignore_ascii_case("POST")
                && (ex.request.path == path || req.url.ends_with(&ex.request.path))
                && canonicalize(&ex.request.body) == body
        });
        let Some(ex) = found else {
            return Err(AiError::new(
                codes::PROVIDER,
                format!("no recorded fixture for POST {path}"),
            )
            .with_hint("add a file under tests/fixtures/ai/"));
        };
        if ex.response.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(ex.response.delay_ms)).await;
        }
        if ex.response.stream || req.stream {
            let chunks = match &ex.response.body {
                Value::Array(items) => items.clone(),
                other => vec![other.clone()],
            };
            return Ok(HttpResponse {
                status: ex.response.status,
                body: Value::Null,
                chunks,
            });
        }
        Ok(HttpResponse {
            status: ex.response.status,
            body: ex.response.body.clone(),
            chunks: Vec::new(),
        })
    }
}

fn url_path(url: &str) -> String {
    url.split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|i| rest[i..].to_string()))
        .unwrap_or_else(|| url.to_string())
}

fn canonicalize(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn parse_sse(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        if let Ok(value) = serde_json::from_str(data) {
            out.push(value);
        }
    }
    out
}

/// Shared transport handle.
pub type SharedTransport = Arc<dyn Transport>;
