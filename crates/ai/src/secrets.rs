//! Secret resolution: environment variable or command. Never stored in config.

use std::process::Command;

use omacell_conf::schema::AiProvider;

use crate::error::{AiError, codes};

/// Resolve `secret_env` then `secret_cmd`. Missing env is `Ok(None)`.
pub fn resolve_secret(spec: &AiProvider) -> Result<Option<String>, AiError> {
    if let Some(name) = &spec.secret_env {
        return Ok(std::env::var(name).ok());
    }
    if let Some(cmd) = &spec.secret_cmd {
        return run_secret_cmd(cmd).map(Some);
    }
    Ok(None)
}

fn run_secret_cmd(cmd: &str) -> Result<String, AiError> {
    let mut parts = cmd.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| AiError::new(codes::SECRET, "secret_cmd is empty"))?;
    let output = Command::new(program)
        .args(parts)
        .output()
        .map_err(|err| AiError::new(codes::SECRET, err.to_string()))?;
    if !output.status.success() {
        return Err(AiError::new(
            codes::SECRET,
            format!("secret_cmd exited {}", output.status),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
