//! File-hash trust store (`~/.local/state/omacell/trust.toml`).

use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use omacell_core::error::CoreError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_TRUST_BYTES: u64 = 1024 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// One trusted file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustEntry {
    /// Lowercase hex SHA-256 of the file bytes.
    pub sha256: String,
    /// Path at grant time (informational).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// On-disk trust file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    let mut file = fs::File::open(path).map_err(|e| CoreError::new("trust.io", e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| CoreError::new("trust.io", e.to_string()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    Ok(out)
}

impl TrustStore {
    /// Parse and validate bounded trust-store TOML text.
    pub fn parse(text: &str) -> Result<Self, CoreError> {
        if text.len() as u64 > MAX_TRUST_BYTES {
            return Err(CoreError::new("trust.limit", "trust store exceeds 1 MiB"));
        }
        let decoded: Self = toml::from_str(text)
            .map_err(|error| CoreError::new("trust.parse", error.to_string()))?;
        let mut validated = Self::default();
        for entry in decoded.files {
            validated.add(entry.sha256, entry.path)?;
        }
        Ok(validated)
    }

    /// Load from `path`, or empty if missing.
    pub fn load(path: &Path) -> Result<Self, CoreError> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(CoreError::new("trust.io", error.to_string())),
        };
        let metadata = file
            .metadata()
            .map_err(|error| CoreError::new("trust.io", error.to_string()))?;
        if !metadata.is_file() {
            return Err(CoreError::new(
                "trust.io",
                "trust store is not a regular file",
            ));
        }
        if metadata.len() > MAX_TRUST_BYTES {
            return Err(CoreError::new("trust.limit", "trust store exceeds 1 MiB"));
        }
        let mut text = String::new();
        file.take(MAX_TRUST_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|error| CoreError::new("trust.io", error.to_string()))?;
        if text.len() as u64 > MAX_TRUST_BYTES {
            return Err(CoreError::new("trust.limit", "trust store exceeds 1 MiB"));
        }
        Self::parse(&text)
    }

    /// Write atomically (exclusive sibling temp, fsync, rename).
    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(directory).map_err(|e| CoreError::new("trust.io", e.to_string()))?;
        let text = toml_to_string(self)?;
        let name = path
            .file_name()
            .ok_or_else(|| CoreError::new("trust.io", "destination has no file name"))?;
        let (mut file, temp) = loop {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let mut temp_name = OsString::from(".");
            temp_name.push(name);
            temp_name.push(format!(".omacell-{}-{sequence}.tmp", std::process::id()));
            let temp = directory.join(temp_name);
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            match options.open(&temp) {
                Ok(file) => break (file, temp),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(CoreError::new("trust.io", error.to_string())),
            }
        };
        let write_result = file
            .write_all(text.as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        let write_result = write_result
            .and_then(|()| fs::rename(&temp, path))
            .and_then(|()| fs::File::open(directory)?.sync_all());
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temp);
            return Err(CoreError::new("trust.io", error.to_string()));
        }
        Ok(())
    }

    /// Whether this hash is trusted.
    #[must_use]
    pub fn contains_hash(&self, sha256: &str) -> bool {
        if !valid_sha256(sha256) {
            return false;
        }
        let want = sha256.to_ascii_lowercase();
        self.files.iter().any(|e| e.sha256 == want)
    }

    /// Grant a hash.
    pub fn add(&mut self, sha256: String, path: Option<String>) -> Result<(), CoreError> {
        if !valid_sha256(&sha256) {
            return Err(CoreError::new(
                "trust.hash",
                "SHA-256 must be exactly 64 hexadecimal characters",
            ));
        }
        let sha256 = sha256.to_ascii_lowercase();
        if self.contains_hash(&sha256) {
            return Ok(());
        }
        self.files.push(TrustEntry { sha256, path });
        self.files.sort_by(|a, b| a.sha256.cmp(&b.sha256));
        Ok(())
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

fn toml_to_string(store: &TrustStore) -> Result<String, CoreError> {
    for e in &store.files {
        if !valid_sha256(&e.sha256) {
            return Err(CoreError::new(
                "trust.hash",
                "refusing to save an invalid SHA-256 trust entry",
            ));
        }
    }
    let encoded =
        toml::to_string(store).map_err(|error| CoreError::new("trust.parse", error.to_string()))?;
    Ok(format!(
        "# Omacell embedded-script trust store (WP-20)\n{encoded}"
    ))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
