//! Omarchy default-agent detection and `omacell agent` hand-off (spec A-5.3).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use omacell_core::error::CoreError;
use serde::Serialize;

/// Detected Omarchy default agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DefaultAgent {
    /// Name printed by `omarchy default agent`.
    pub name: String,
}

/// Prepared hand-off. Either launched or printed for a foreign harness.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HandOff {
    /// `true` when no default agent is configured (palette/status hide).
    pub hidden: bool,
    /// Working directory used for the spawn (workbook directory).
    pub cwd: String,
    /// Exact argv, including `omarchy`.
    pub argv: Vec<String>,
    /// Whether the process was started.
    pub launched: bool,
}

/// Inputs for [`hand_off`].
#[derive(Clone, Debug, Default)]
pub struct HandOffRequest {
    /// User prompt.
    pub prompt: String,
    /// Open workbook, if any.
    pub workbook: Option<PathBuf>,
    /// Current selection (`Sheet!A1`).
    pub selection: Option<String>,
    /// Optional diagnostic bundle path (`omacell agent diagnose`).
    pub diagnose: Option<PathBuf>,
}

/// `omarchy` is on `PATH` and `omarchy default agent` prints a non-empty name.
#[must_use]
pub fn detect_default_agent() -> Option<DefaultAgent> {
    if !on_path("omarchy") {
        return None;
    }
    let output = Command::new("omarchy")
        .args(["default", "agent"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(DefaultAgent { name })
    }
}

/// Build `omarchy agent prompt …` argv. Spawns only when a default agent exists.
pub fn hand_off(req: HandOffRequest) -> Result<HandOff, CoreError> {
    let cwd = req
        .workbook
        .as_ref()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let mut argv = vec!["omarchy".into(), "agent".into(), "prompt".into()];
    if let Some(book) = &req.workbook {
        argv.push("--workbook".into());
        argv.push(book.display().to_string());
    }
    if let Some(sel) = &req.selection {
        argv.push("--selection".into());
        argv.push(sel.clone());
    }
    argv.push("--skill".into());
    argv.push("omacell".into());
    if let Some(path) = &req.diagnose {
        argv.push("--diagnose".into());
        argv.push(path.display().to_string());
    }
    argv.push("--".into());
    argv.push(req.prompt);
    let hidden = detect_default_agent().is_none();
    if hidden {
        return Ok(HandOff {
            hidden: true,
            cwd: cwd.display().to_string(),
            argv,
            launched: false,
        });
    }
    let mut child = Command::new(&argv[0]);
    child.args(&argv[1..]).current_dir(&cwd);
    let status = child
        .status()
        .map_err(|err| CoreError::new("agent.spawn", err.to_string()))?;
    if !status.success() {
        return Err(CoreError::new(
            "agent.spawn",
            format!("omarchy agent prompt exited {status}"),
        ));
    }
    Ok(HandOff {
        hidden: false,
        cwd: cwd.display().to_string(),
        argv,
        launched: true,
    })
}

/// Whether `name` is an executable on `PATH`.
#[must_use]
pub fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(name);
        candidate.is_file()
    })
}
