//! Exit codes and JSON error payloads.

use std::io;

use omacell_core::error::CoreError;
use serde::Serialize;

/// Success.
pub const EXIT_OK: i32 = 0;
/// Operational failure (I/O, config, command).
pub const EXIT_ERROR: i32 = 1;
/// Usage / clap parse error.
pub const EXIT_USAGE: i32 = 2;
/// Stub / not yet implemented.
pub const EXIT_NYI: i32 = 3;

/// CLI failure with a stable `{code, message, hint}` body.
#[derive(Debug, Clone, Serialize)]
pub struct CliError {
    /// Stable dotted code.
    pub code: String,
    /// Human message.
    pub message: String,
    /// Optional recovery hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Process exit code.
    #[serde(skip)]
    pub exit: i32,
}

impl CliError {
    /// Construct an operational error.
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: None,
            exit: EXIT_ERROR,
        }
    }

    /// Attach a hint.
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Override the exit code.
    #[must_use]
    pub fn exit(mut self, exit: i32) -> Self {
        self.exit = exit;
        self
    }

    /// Stub that names the owning work package.
    #[must_use]
    #[allow(dead_code)]
    pub fn nyi(feature: &str, wp: &str) -> Self {
        Self::new("cli.not_yet", format!("{feature} arrives in {wp}"))
            .hint(format!("see docs/build/wp/{wp}-*.md"))
            .exit(EXIT_NYI)
    }

    /// JSON object for stderr in `--json` mode.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code,
            "message": self.message,
            "hint": self.hint,
        })
    }
}

impl From<io::Error> for CliError {
    fn from(err: io::Error) -> Self {
        Self::new("cli.io", err.to_string())
    }
}

impl From<omacell_ai::AiError> for CliError {
    fn from(err: omacell_ai::AiError) -> Self {
        Self {
            code: err.code,
            message: err.message,
            hint: err.hint,
            exit: EXIT_ERROR,
        }
    }
}

impl From<CoreError> for CliError {
    fn from(err: CoreError) -> Self {
        Self {
            code: err.code,
            message: err.message,
            hint: err.hint,
            exit: EXIT_ERROR,
        }
    }
}

impl From<clap::Error> for CliError {
    fn from(err: clap::Error) -> Self {
        let exit = if err.use_stderr() {
            EXIT_USAGE
        } else {
            EXIT_OK
        };
        Self {
            code: "cli.usage".into(),
            message: err.to_string(),
            hint: Some("try omacell --help".into()),
            exit,
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
