//! Content-addressed AI-cell cache (workbook part + disk).

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

use omacell_core::recalc::ContentHash;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AiError, codes};

/// Custom part holding the workbook-local cache.
pub const AICACHE_PART: &str = "xl/omacell/aicache.json";

const MAX_CACHE_BYTES: usize = 16 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 100_000;

/// One cached cell result.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CacheEntry {
    /// Task name.
    pub task: String,
    /// Prompt-template version.
    pub template_version: String,
    /// Provider name.
    #[serde(default)]
    pub provider: String,
    /// Model name.
    pub model: String,
    /// JSON value (scalar or array).
    pub value: Value,
    /// SHA-256 of the prompt.
    pub prompt_hash: String,
    /// SHA-256 of the evaluated arguments (guards the compact cache key).
    #[serde(default)]
    pub input_hash: String,
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
        if bytes.len() > MAX_CACHE_BYTES {
            return Self::default();
        }
        let cache: Self = serde_json::from_slice(bytes).unwrap_or_default();
        if cache.entries.len() > MAX_CACHE_ENTRIES {
            Self::default()
        } else {
            cache
        }
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
        let key = Self::key(hash);
        if !self.entries.contains_key(&key) && self.entries.len() >= MAX_CACHE_ENTRIES {
            let removable = self
                .entries
                .iter()
                .find_map(|(key, entry)| (!entry.pinned).then(|| key.clone()));
            let Some(removable) = removable else {
                return;
            };
            self.entries.remove(&removable);
        }
        self.entries.insert(key, entry);
    }

    /// Remove one key.
    pub fn remove(&mut self, hash: ContentHash) -> bool {
        self.entries.remove(&Self::key(hash)).is_some()
    }

    /// Serialize.
    pub fn to_bytes(&self) -> Result<Vec<u8>, AiError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))?;
        if bytes.len() > MAX_CACHE_BYTES {
            return Err(AiError::new(
                codes::PAYLOAD,
                "AI workbook cache exceeds the 16 MiB limit",
            ));
        }
        Ok(bytes)
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
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder
            .create(parent)
            .map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
        #[cfg(unix)]
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
    }
    let bytes =
        serde_json::to_vec(entry).map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&path)
        .map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
    #[cfg(unix)]
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
    file.write_all(&bytes)
        .map_err(|err| AiError::new(codes::LOG, err.to_string()))
}

/// Read one disk entry.
#[must_use]
pub fn read_disk(cache_dir: &Path, hash: ContentHash) -> Option<CacheEntry> {
    let path = disk_path(cache_dir, hash);
    if std::fs::metadata(&path).ok()?.len() > MAX_CACHE_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}
