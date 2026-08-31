//! HTTP transport: production `reqwest` and recorded replay (no network in tests).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AiError, codes};

/// Maximum serialized provider request body.
pub const MAX_PROVIDER_REQUEST_BYTES: usize = 8 * 1_048_576;
/// Maximum provider response body, including an SSE stream.
pub const MAX_PROVIDER_RESPONSE_BYTES: usize = 8 * 1_048_576;
/// Maximum JSON nesting accepted at the provider boundary.
pub const MAX_PROVIDER_JSON_DEPTH: usize = 64;
const MAX_PROVIDER_HEADERS: usize = 64;
const MAX_PROVIDER_HEADER_BYTES: usize = 16 * 1_024;
const MAX_REPLAY_FIXTURE_BYTES: u64 = 32 * 1_048_576;

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
        validate_request(&req)?;
        let body = serde_json::to_vec(&req.body)
            .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?;
        let mut builder = self
            .client
            .post(&req.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
        for (key, value) in &req.headers {
            builder = builder.header(key, value);
        }
        let mut response = builder
            .send()
            .await
            .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_PROVIDER_RESPONSE_BYTES {
                return Err(AiError::new(
                    codes::PROVIDER,
                    format!("provider response exceeds {MAX_PROVIDER_RESPONSE_BYTES} bytes"),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        if req.stream {
            let chunks = parse_sse(&bytes)?;
            return Ok(HttpResponse {
                status,
                body: Value::Null,
                chunks,
            });
        }
        let body = match serde_json::from_slice(&bytes) {
            Ok(value) => {
                validate_json(&value)?;
                value
            }
            Err(_) => Value::String(String::from_utf8_lossy(&bytes).into_owned()),
        };
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

/// Explicit human-operated recording wrapper. Authorization headers are never
/// written, but request and response bodies may contain workbook data.
pub struct RecordingTransport {
    inner: SharedTransport,
    directory: PathBuf,
    protocol: String,
    next: AtomicU64,
}

impl RecordingTransport {
    /// Wrap a real transport and write replay fixtures under `directory`.
    pub fn new(
        inner: SharedTransport,
        directory: impl Into<PathBuf>,
        protocol: impl Into<String>,
    ) -> Result<Self, AiError> {
        let directory = directory.into();
        std::fs::create_dir_all(&directory)
            .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?;
        }
        Ok(Self {
            inner,
            directory,
            protocol: protocol.into(),
            next: AtomicU64::new(0),
        })
    }
}

#[async_trait]
impl Transport for RecordingTransport {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, AiError> {
        validate_request(&req)?;
        let response = self.inner.send(req.clone()).await?;
        let exchange = RecordedExchange {
            protocol: self.protocol.clone(),
            request: RecordedRequest {
                method: "POST".into(),
                path: url_path(&req.url),
                body: req.body,
            },
            response: RecordedResponse {
                status: response.status,
                headers: BTreeMap::new(),
                body: if req.stream {
                    Value::Array(response.chunks.clone())
                } else {
                    response.body.clone()
                },
                stream: req.stream,
                delay_ms: 0,
            },
        };
        let sequence = self.next.fetch_add(1, Ordering::Relaxed);
        let path = self
            .directory
            .join(format!("recording-{}-{sequence}.json", std::process::id()));
        tokio::task::spawn_blocking(move || write_recording(&path, &exchange))
            .await
            .map_err(|err| {
                AiError::new(codes::PROVIDER, format!("recording writer stopped: {err}"))
            })??;
        Ok(response)
    }
}

fn write_recording(path: &Path, exchange: &RecordedExchange) -> Result<(), AiError> {
    let body = serde_json::to_vec_pretty(exchange)
        .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?;
    if body.len() as u64 > MAX_REPLAY_FIXTURE_BYTES {
        return Err(AiError::new(
            codes::PROVIDER,
            "recorded provider exchange exceeds the fixture size limit",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?;
    use std::io::Write as _;
    file.write_all(&body)
        .and_then(|()| file.sync_all())
        .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))
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
            let size = std::fs::metadata(&path)
                .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?
                .len();
            if size > MAX_REPLAY_FIXTURE_BYTES {
                return Err(AiError::new(
                    codes::PROVIDER,
                    format!("{} exceeds the replay fixture limit", path.display()),
                ));
            }
            let text = std::fs::read_to_string(&path)
                .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?;
            let parsed: RecordedExchange = serde_json::from_str(&text).map_err(|err| {
                AiError::new(codes::PROVIDER, format!("{}: {err}", path.display()))
            })?;
            validate_json(&parsed.request.body)?;
            validate_json(&parsed.response.body)?;
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
        validate_request(&req)?;
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

fn parse_sse(bytes: &[u8]) -> Result<Vec<Value>, AiError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| AiError::new(codes::PROVIDER, "provider SSE is not UTF-8"))?;
    let mut out = Vec::new();
    let mut data_lines = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            push_sse_data(&mut out, &mut data_lines)?;
            continue;
        }
        if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.trim_start());
        }
    }
    push_sse_data(&mut out, &mut data_lines)?;
    Ok(out)
}

fn push_sse_data(out: &mut Vec<Value>, lines: &mut Vec<&str>) -> Result<(), AiError> {
    if lines.is_empty() {
        return Ok(());
    }
    let data = lines.join("\n");
    lines.clear();
    if data.trim().is_empty() || data.trim() == "[DONE]" {
        return Ok(());
    }
    let value: Value = serde_json::from_str(&data).map_err(|err| {
        AiError::new(codes::PROVIDER, format!("invalid provider SSE JSON: {err}"))
    })?;
    validate_json(&value)?;
    out.push(value);
    Ok(())
}

pub(crate) fn validate_request(req: &HttpRequest) -> Result<(), AiError> {
    if req.url.len() > 8_192 {
        return Err(AiError::new(codes::PROVIDER, "provider URL is too long"));
    }
    let url = reqwest::Url::parse(&req.url)
        .map_err(|err| AiError::new(codes::PROVIDER, format!("invalid provider URL: {err}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AiError::new(
            codes::PROVIDER,
            "provider URL must be absolute HTTP or HTTPS",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AiError::new(
            codes::PROVIDER,
            "provider URL must not contain credentials",
        ));
    }
    if req.headers.len() > MAX_PROVIDER_HEADERS {
        return Err(AiError::new(codes::PROVIDER, "too many provider headers"));
    }
    let header_bytes = req
        .headers
        .iter()
        .try_fold(0usize, |total, (name, value)| {
            let next = total.saturating_add(name.len()).saturating_add(value.len());
            if name.contains('\r')
                || name.contains('\n')
                || value.contains('\r')
                || value.contains('\n')
            {
                return Err(AiError::new(
                    codes::PROVIDER,
                    "provider header contains a line break",
                ));
            }
            Ok(next)
        })?;
    if header_bytes > MAX_PROVIDER_HEADER_BYTES {
        return Err(AiError::new(
            codes::PROVIDER,
            "provider headers exceed the size limit",
        ));
    }
    validate_json(&req.body)?;
    let bytes = serde_json::to_vec(&req.body)
        .map_err(|err| AiError::new(codes::PROVIDER, err.to_string()))?;
    if bytes.len() > MAX_PROVIDER_REQUEST_BYTES {
        return Err(AiError::new(
            codes::PROVIDER,
            format!("provider request exceeds {MAX_PROVIDER_REQUEST_BYTES} bytes"),
        ));
    }
    Ok(())
}

pub(crate) fn validate_json(value: &Value) -> Result<(), AiError> {
    let mut stack = vec![(value, 1usize)];
    while let Some((current, depth)) = stack.pop() {
        if depth > MAX_PROVIDER_JSON_DEPTH {
            return Err(AiError::new(
                codes::PROVIDER,
                format!("provider JSON exceeds depth {MAX_PROVIDER_JSON_DEPTH}"),
            ));
        }
        match current {
            Value::Array(items) => {
                stack.extend(items.iter().map(|item| (item, depth.saturating_add(1))));
            }
            Value::Object(map) => {
                stack.extend(map.values().map(|item| (item, depth.saturating_add(1))));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Shared transport handle.
pub type SharedTransport = Arc<dyn Transport>;
