//! Secret resolution: environment variable or command. Never stored in config.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use omacell_conf::schema::AiProvider;

use crate::error::{AiError, codes};

const MAX_SECRET_BYTES: u64 = 8 * 1_024;
const SECRET_CMD_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolve exactly one of `secret_env` or `secret_cmd`.
pub fn resolve_secret(spec: &AiProvider) -> Result<Option<String>, AiError> {
    if spec.secret_env.is_some() && spec.secret_cmd.is_some() {
        return Err(AiError::new(
            codes::SECRET,
            "configure only one of secret_env or secret_cmd",
        ));
    }
    if let Some(name) = &spec.secret_env {
        if name.trim().is_empty() {
            return Err(AiError::new(codes::SECRET, "secret_env is empty"));
        }
        return std::env::var(name).map(Some).map_err(|_| {
            AiError::new(codes::SECRET, format!("secret_env {name} is not set"))
                .with_hint("set the environment variable before sending the request")
        });
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
    let mut child = Command::new(program)
        .args(parts)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| AiError::new(codes::SECRET, err.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AiError::new(codes::SECRET, "secret_cmd stdout is unavailable"))?;
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(MAX_SECRET_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = send.send(result);
    });
    let bytes = match receive.recv_timeout(SECRET_CMD_TIMEOUT) {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(err)) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(AiError::new(codes::SECRET, err.to_string()));
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(AiError::new(codes::SECRET, "secret_cmd timed out"));
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(AiError::new(
                codes::SECRET,
                "secret_cmd output reader stopped",
            ));
        }
    };
    if bytes.len() > MAX_SECRET_BYTES as usize {
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();
        return Err(AiError::new(
            codes::SECRET,
            format!("secret_cmd output exceeds {MAX_SECRET_BYTES} bytes"),
        ));
    }
    let status = child
        .wait()
        .map_err(|err| AiError::new(codes::SECRET, err.to_string()))?;
    let _ = reader.join();
    if !status.success() {
        return Err(AiError::new(
            codes::SECRET,
            format!("secret_cmd exited {status}"),
        ));
    }
    let secret = String::from_utf8(bytes)
        .map_err(|_| AiError::new(codes::SECRET, "secret_cmd output is not UTF-8"))?;
    let secret = secret.trim().to_string();
    if secret.is_empty() {
        return Err(AiError::new(
            codes::SECRET,
            "secret_cmd returned an empty secret",
        ));
    }
    Ok(secret)
}
