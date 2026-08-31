//! Local AI audit log (`~/.local/state/omacell/ai/log.jsonl`).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AiError, codes};
use crate::provider::Usage;

/// One log record. Content is omitted unless `log_content`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LogRecord {
    /// Unix ms.
    pub ts: u64,
    /// Task name (`card`, `chat`, `setup`).
    pub task: String,
    /// Provider name.
    pub provider: String,
    /// Model id.
    pub model: String,
    /// Request bytes.
    pub request_bytes: u64,
    /// Response bytes.
    pub response_bytes: u64,
    /// SHA-256 of the request JSON.
    pub request_hash: String,
    /// Latency ms.
    pub latency_ms: u64,
    /// Token usage.
    pub usage: Usage,
    /// Optional content (only when configured).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
}

/// Append-only JSONL log.
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    /// `state_dir/ai/log.jsonl`.
    #[must_use]
    pub fn path(state_dir: &Path) -> PathBuf {
        state_dir.join("ai").join("log.jsonl")
    }

    /// Open (creating the directory).
    pub fn open(state_dir: &Path) -> Result<Self, AiError> {
        let path = Self::path(state_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
        }
        Ok(Self { path })
    }

    /// Append one record.
    pub fn append(&self, record: &LogRecord) -> Result<(), AiError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
        let mut line =
            serde_json::to_vec(record).map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
        line.push(b'\n');
        file.write_all(&line)
            .map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
        Ok(())
    }

    /// Read all records (for `omacell ai log`).
    pub fn read(&self) -> Result<Vec<LogRecord>, AiError> {
        if !self.path.is_file() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&self.path)
            .map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
        let mut out = Vec::new();
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            out.push(
                serde_json::from_str(line)
                    .map_err(|err| AiError::new(codes::LOG, err.to_string()))?,
            );
        }
        Ok(out)
    }
}

/// SHA-256 hex of canonical JSON.
#[must_use]
pub fn hash_json(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Current unix ms.
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Session counters for the status line.
#[derive(Clone, Debug, Default, Serialize)]
pub struct SessionStats {
    /// Requests this process.
    pub requests: u64,
    /// Bytes sent this process.
    pub bytes_sent: u64,
}

impl SessionStats {
    /// Record one send.
    pub fn record(&mut self, bytes: u64) {
        self.requests += 1;
        self.bytes_sent += bytes;
    }
}

/// Status-line segment data (provider, privacy, session sends).
#[derive(Clone, Debug, Serialize)]
pub struct StatusSegment {
    /// Active provider name.
    pub provider: String,
    /// Loopback.
    pub local: bool,
    /// Effective send level.
    pub level: String,
    /// Session request count.
    pub requests: u64,
    /// Session bytes sent.
    pub bytes_sent: u64,
}

impl StatusSegment {
    /// Build from routing + policy + counters.
    #[must_use]
    pub fn new(
        provider: impl Into<String>,
        local: bool,
        level: impl Into<String>,
        stats: &SessionStats,
    ) -> Self {
        Self {
            provider: provider.into(),
            local,
            level: level.into(),
            requests: stats.requests,
            bytes_sent: stats.bytes_sent,
        }
    }
}
