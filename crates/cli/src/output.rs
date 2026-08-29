//! Human and `--json` writers.

use std::io::{self, Write};

use crate::error::CliError;

/// How a command should print.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Output {
    /// Machine-readable stdout/stderr.
    pub json: bool,
    /// Suppress non-error human text.
    pub quiet: bool,
}

impl Output {
    /// Write a success JSON object or human line.
    pub fn success(&self, json: serde_json::Value, human: &str) -> io::Result<()> {
        let mut out = io::stdout().lock();
        if self.json {
            writeln!(
                out,
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string())
            )
        } else if self.quiet || human.is_empty() {
            Ok(())
        } else {
            writeln!(out, "{human}")
        }
    }

    /// Write `{code,message,hint}` to stderr in JSON mode, else a human line.
    pub fn error(&self, err: &CliError) -> io::Result<()> {
        let mut err_out = io::stderr().lock();
        if self.json {
            writeln!(
                err_out,
                "{}",
                serde_json::to_string_pretty(&err.to_json())
                    .unwrap_or_else(|_| err.to_json().to_string())
            )
        } else {
            match &err.hint {
                Some(hint) => writeln!(
                    err_out,
                    "omacell: {} ({})\nhint: {hint}",
                    err.message, err.code
                ),
                None => writeln!(err_out, "omacell: {} ({})", err.message, err.code),
            }
        }
    }
}
