//! Session persistence (`~/.local/state/omacell/session.toml`).

use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::error;
use omacell_core::error::CoreError;

/// Restored session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionState {
    /// Recent files, newest first.
    #[serde(default)]
    pub recent_files: Vec<String>,
    /// Last active sheet name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    /// Last cursor A1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Open panel id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<String>,
    /// Zoom.
    #[serde(default = "one")]
    pub zoom: f64,
}

const MAX_SESSION_BYTES: u64 = 1024 * 1024;
static SAVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl Default for SessionState {
    fn default() -> Self {
        Self {
            recent_files: Vec::new(),
            sheet: None,
            cursor: None,
            panel: None,
            zoom: 1.0,
        }
    }
}

fn one() -> f64 {
    1.0
}

impl SessionState {
    /// Path under `state_dir`.
    #[must_use]
    pub fn path(state_dir: &Path) -> PathBuf {
        state_dir.join("session.toml")
    }

    /// Load, or default if missing.
    pub fn load(state_dir: &Path) -> Result<Self, CoreError> {
        let path = Self::path(state_dir);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(error::session("session.toml must not be a symlink"));
            }
            Ok(metadata) if !metadata.is_file() => {
                return Err(error::session("session.toml is not a regular file"));
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(error::session(format!("{}: {err}", path.display()))),
        }
        let file = std::fs::File::open(&path)
            .map_err(|err| error::session(format!("{}: {err}", path.display())))?;
        let mut bytes = Vec::new();
        file.take(MAX_SESSION_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|err| error::session(format!("{}: {err}", path.display())))?;
        if bytes.len() as u64 > MAX_SESSION_BYTES {
            return Err(error::session(format!(
                "session.toml exceeds {MAX_SESSION_BYTES} bytes"
            )));
        }
        let text = String::from_utf8(bytes)
            .map_err(|err| error::session(format!("session.toml is not UTF-8: {err}")))?;
        let state: Self =
            toml::from_str(&text).map_err(|e| error::session(format!("session.toml: {e}")))?;
        state.validate()?;
        Ok(state)
    }

    /// Atomic-enough write (temp + rename).
    pub fn save(&self, state_dir: &Path) -> Result<(), CoreError> {
        self.validate()?;
        std::fs::create_dir_all(state_dir).map_err(|e| error::session(e.to_string()))?;
        let path = Self::path(state_dir);
        let sequence = SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let tmp = state_dir.join(format!(
            ".session.toml.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        let text = toml::to_string_pretty(self)
            .map_err(|e| error::session(format!("session.toml: {e}")))?;
        if text.len() as u64 > MAX_SESSION_BYTES {
            return Err(error::session(format!(
                "session.toml exceeds {MAX_SESSION_BYTES} bytes"
            )));
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .map_err(|err| error::session(err.to_string()))?;
        if let Err(err) = file
            .write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
        {
            let _ = std::fs::remove_file(&tmp);
            return Err(error::session(err.to_string()));
        }
        if let Err(err) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(error::session(err.to_string()));
        }
        Ok(())
    }

    /// Remember a file path.
    pub fn touch_file(&mut self, path: &str) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_string());
        self.recent_files.truncate(20);
    }

    fn validate(&self) -> Result<(), CoreError> {
        if !self.zoom.is_finite() || !(0.25..=8.0).contains(&self.zoom) {
            return Err(error::session("zoom must be finite and in 0.25..=8.0"));
        }
        if self.recent_files.len() > 20 {
            return Err(error::session("recent_files may contain at most 20 paths"));
        }
        Ok(())
    }
}
