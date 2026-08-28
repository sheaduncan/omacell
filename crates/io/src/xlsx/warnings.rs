//! Recoverable open warnings (UI/CLI surface).

use serde::{Deserialize, Serialize};

/// One recoverable problem while opening a workbook.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileWarning {
    /// Stable dotted code (`xlsx.formula`, `xlsx.part`, …).
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Optional recovery hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Zip part the warning refers to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}

/// List of [`FileWarning`]s from a successful open.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileWarnings {
    /// Warnings in encounter order.
    pub items: Vec<FileWarning>,
}

impl FileWarnings {
    /// Empty list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a warning.
    pub fn push(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        part: Option<String>,
    ) {
        self.items.push(FileWarning {
            code: code.into(),
            message: message.into(),
            hint: None,
            part,
        });
    }

    /// True when nothing was reported.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
