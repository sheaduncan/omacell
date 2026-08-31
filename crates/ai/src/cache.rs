//! Content-addressed AI-cell cache (workbook part + disk).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use omacell_core::recalc::ContentHash;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AiError, codes};

/// Custom part holding the workbook-local cache.
pub const AICACHE_PART: &str = "xl/omacell/aicache.json";

/// One cached cell result.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CacheEntry {
    /// Task name.
    pub task: String,
    /// Prompt-template version.
    pub template_version: String,
    /// Provider:model.
    pub model: String,
    /// JSON value (scalar or array).
    pub value: Value,
    /// SHA-256 of the prompt.
    pub prompt_hash: String,
    /// Unix ms.
    pub ts: u64,
    /// Prompt tokens.
    pub prompt_tokens: u32,
    /// Completion tokens.
    pub completion_tokens: u32,
    /// Pinned (refresh skips).
    #[serde(default)]
    pub pinned: bool,
}

/// Workbook + disk cache.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AiCache {
    /// Hash hex → entry.
    #[serde(default)]
    pub entries: BTreeMap<String, CacheEntry>,
}

impl AiCache {
    /// Load from a workbook custom part.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        serde_json::from_slice(bytes).unwrap_or_default()
    }

    /// Key for [`ContentHash`].
    #[must_use]
    pub fn key(hash: ContentHash) -> String {
        format!("{:016x}", hash.0)
    }

    /// Lookup.
    #[must_use]
    pub fn get(&self, hash: ContentHash) -> Option<&CacheEntry> {
        self.entries.get(&Self::key(hash))
    }

    /// Insert.
    pub fn insert(&mut self, hash: ContentHash, entry: CacheEntry) {
        self.entries.insert(Self::key(hash), entry);
    }

    /// Remove one key.
    pub fn remove(&mut self, hash: ContentHash) -> bool {
        self.entries.remove(&Self::key(hash)).is_some()
    }

    /// Serialize.
    pub fn to_bytes(&self) -> Result<Vec<u8>, AiError> {
        serde_json::to_vec(self).map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))
    }
}

/// Disk mirror under `cache_dir/ai/`.
pub fn disk_path(cache_dir: &Path, hash: ContentHash) -> PathBuf {
    cache_dir
        .join("ai")
        .join(format!("{}.json", AiCache::key(hash)))
}

/// Write one disk entry (mode 0600).
pub fn write_disk(cache_dir: &Path, hash: ContentHash, entry: &CacheEntry) -> Result<(), AiError> {
    let path = disk_path(cache_dir, hash);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
    }
    let bytes =
        serde_json::to_vec(entry).map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))?;
    std::fs::write(&path, bytes).map_err(|err| AiError::new(codes::LOG, err.to_string()))
}

/// Read one disk entry.
#[must_use]
pub fn read_disk(cache_dir: &Path, hash: ContentHash) -> Option<CacheEntry> {
    let path = disk_path(cache_dir, hash);
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}
