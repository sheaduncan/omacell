//! Session persistence (`~/.local/state/omacell/session.toml`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error;
use omacell_core::error::CoreError;

/// Restored session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| error::session(format!("{}: {e}", path.display())))?;
        toml::from_str(&text).map_err(|e| error::session(format!("session.toml: {e}")))
    }

    /// Atomic-enough write (temp + rename).
    pub fn save(&self, state_dir: &Path) -> Result<(), CoreError> {
        std::fs::create_dir_all(state_dir).map_err(|e| error::session(e.to_string()))?;
        let path = Self::path(state_dir);
        let tmp = path.with_extension("toml.tmp");
        let text = toml::to_string_pretty(self)
            .map_err(|e| error::session(format!("session.toml: {e}")))?;
        std::fs::write(&tmp, text).map_err(|e| error::session(e.to_string()))?;
        std::fs::rename(&tmp, &path).map_err(|e| error::session(e.to_string()))?;
        Ok(())
    }

    /// Remember a file path.
    pub fn touch_file(&mut self, path: &str) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_string());
        self.recent_files.truncate(20);
    }
}
