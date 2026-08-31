//! File-hash trust store (`~/.local/state/omacell/trust.toml`).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use omacell_core::error::CoreError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One trusted file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustEntry {
    /// Lowercase hex SHA-256 of the file bytes.
    pub sha256: String,
    /// Path at grant time (informational).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// On-disk trust file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustStore {
    /// Granted files.
    #[serde(default)]
    pub files: Vec<TrustEntry>,
}

/// SHA-256 hex of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Hash a file.
pub fn hash_path(path: &Path) -> Result<String, CoreError> {
    let bytes = fs::read(path).map_err(|e| CoreError::new("trust.io", e.to_string()))?;
    Ok(sha256_hex(&bytes))
}

impl TrustStore {
    /// Load from `path`, or empty if missing.
    pub fn load(path: &Path) -> Result<Self, CoreError> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text =
            fs::read_to_string(path).map_err(|e| CoreError::new("trust.io", e.to_string()))?;
        toml_from_str(&text)
    }

    /// Write atomically-ish (temp + rename).
    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| CoreError::new("trust.io", e.to_string()))?;
        }
        let text = toml_to_string(self)?;
        let tmp = path.with_extension("toml.tmp");
        {
            let mut f =
                fs::File::create(&tmp).map_err(|e| CoreError::new("trust.io", e.to_string()))?;
            f.write_all(text.as_bytes())
                .map_err(|e| CoreError::new("trust.io", e.to_string()))?;
        }
        fs::rename(&tmp, path).map_err(|e| CoreError::new("trust.io", e.to_string()))?;
        Ok(())
    }

    /// Whether this hash is trusted.
    #[must_use]
    pub fn contains_hash(&self, sha256: &str) -> bool {
        let want = sha256.to_ascii_lowercase();
        self.files.iter().any(|e| e.sha256 == want)
    }

    /// Grant a hash.
    pub fn add(&mut self, sha256: String, path: Option<String>) {
        let sha256 = sha256.to_ascii_lowercase();
        if self.contains_hash(&sha256) {
            return;
        }
        self.files.push(TrustEntry { sha256, path });
        self.files.sort_by(|a, b| a.sha256.cmp(&b.sha256));
    }

    /// Revoke a hash.
    pub fn remove(&mut self, sha256: &str) -> bool {
        let want = sha256.to_ascii_lowercase();
        let before = self.files.len();
        self.files.retain(|e| e.sha256 != want);
        self.files.len() != before
    }
}

/// Default trust file under a state dir.
#[must_use]
pub fn trust_path(state_dir: &Path) -> PathBuf {
    state_dir.join("trust.toml")
}

fn toml_from_str(text: &str) -> Result<TrustStore, CoreError> {
    parse_simple_toml(text)
}

fn toml_to_string(store: &TrustStore) -> Result<String, CoreError> {
    let mut out = String::from("# Omacell embedded-script trust store (WP-20)\n");
    for e in &store.files {
        out.push_str("\n[[files]]\n");
        out.push_str(&format!("sha256 = \"{}\"\n", e.sha256));
        if let Some(p) = &e.path {
            out.push_str(&format!("path = \"{}\"\n", escape_toml(p)));
        }
    }
    Ok(out)
}

fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn parse_simple_toml(text: &str) -> Result<TrustStore, CoreError> {
    let mut store = TrustStore::default();
    let mut current: Option<TrustEntry> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[files]]" {
            if let Some(e) = current.take() {
                store.add(e.sha256, e.path);
            }
            current = Some(TrustEntry {
                sha256: String::new(),
                path: None,
            });
            continue;
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        if let Some(v) = line.strip_prefix("sha256") {
            entry.sha256 = parse_toml_string(v)?;
        } else if let Some(v) = line.strip_prefix("path") {
            entry.path = Some(parse_toml_string(v)?);
        }
    }
    if let Some(e) = current.take() {
        store.add(e.sha256, e.path);
    }
    Ok(store)
}

fn parse_toml_string(rest: &str) -> Result<String, CoreError> {
    let rest = rest.trim().trim_start_matches('=').trim();
    if let Some(inner) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        Ok(inner.replace("\\\"", "\"").replace("\\\\", "\\"))
    } else {
        Err(CoreError::new("trust.parse", "expected a quoted string"))
    }
}
