//! Stable `{code, message, hint}` errors for the AI crate.

use omacell_core::error::CoreError;
use thiserror::Error;

/// AI-layer failure.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct AiError {
    /// Stable dotted code.
    pub code: String,
    /// Human message.
    pub message: String,
    /// Optional recovery hint.
    pub hint: Option<String>,
}

impl AiError {
    /// Construct a code + message.
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: None,
        }
    }

    /// Attach a hint.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl From<AiError> for CoreError {
    fn from(err: AiError) -> Self {
        let mut core = CoreError::new(err.code, err.message);
        if let Some(hint) = err.hint {
            core = core.with_hint(hint);
        }
        core
    }
}

impl From<CoreError> for AiError {
    fn from(err: CoreError) -> Self {
        Self {
            code: err.code,
            message: err.message,
            hint: err.hint,
        }
    }
}

/// Machine codes.
pub mod codes {
    /// Provider kind is not `openai_compatible` or `anthropic`.
    pub const KIND: &str = "ai.kind";
    /// HTTP or protocol failure.
    pub const PROVIDER: &str = "ai.provider";
    /// Request cancelled.
    pub const CANCELLED: &str = "ai.cancelled";
    /// Deadline exceeded.
    pub const TIMEOUT: &str = "ai.timeout";
    /// Rate or cell budget exceeded.
    pub const BUDGET: &str = "ai.budget";
    /// Secret resolution failed.
    pub const SECRET: &str = "ai.secret";
    /// Payload/card construction failed.
    pub const PAYLOAD: &str = "ai.payload";
    /// Audit log I/O.
    pub const LOG: &str = "ai.log";
    /// Setup / config write.
    pub const SETUP: &str = "ai.setup";
    /// AI is disabled.
    pub const DISABLED: &str = "ai.disabled";
    /// Per-session autopilot policy denied a command.
    pub const AUTOPILOT: &str = "ai.autopilot";
}
