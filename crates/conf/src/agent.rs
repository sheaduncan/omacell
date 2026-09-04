//! Omarchy default-agent detection and `omacell agent` hand-off (spec A-5.3).

use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use omacell_core::error::CoreError;
use serde::Serialize;

static DIAGNOSTIC_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static HANDOFF_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    /// Private file containing the user prompt and workbook context.
    pub prompt_file: String,
    /// Whether the process was started.
    pub launched: bool,
}

/// Inputs for [`hand_off`].
#[derive(Clone, Debug)]
pub struct HandOffRequest {
    /// User prompt.
    pub prompt: String,
    /// Open workbook, if any.
    pub workbook: Option<PathBuf>,
    /// Current selection (`Sheet!A1`).
    pub selection: Option<String>,
    /// Optional diagnostic bundle path (`omacell agent diagnose`).
    pub diagnose: Option<PathBuf>,
    /// Omacell state directory for the private hand-off request.
    pub state_dir: PathBuf,
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
    let workbook = req.workbook.as_ref().map(|path| absolute_path(path));
    let cwd = workbook
        .as_ref()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let prompt_file = write_handoff_prompt(
        &req.state_dir,
        contextual_prompt(&req, workbook.as_deref()).as_bytes(),
    )?;
    // Omarchy's prompt helper has no stdin mode and does not consistently
    // support a `--` sentinel. Keep all user/workbook-derived text out of
    // process argv and pass only a fixed instruction plus a private path.
    let argv = vec![
        "omarchy".into(),
        "agent".into(),
        "prompt".into(),
        format!(
            "Read the private Omacell hand-off request at {} and follow it.",
            prompt_file.display()
        ),
    ];
    let hidden = detect_default_agent().is_none();
    if hidden {
        return Ok(HandOff {
            hidden: true,
            cwd: cwd.display().to_string(),
            argv,
            prompt_file: prompt_file.display().to_string(),
            launched: false,
        });
    }
    let mut child = Command::new(&argv[0]);
    child
        .args(&argv[1..])
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
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
        prompt_file: prompt_file.display().to_string(),
        launched: true,
    })
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn contextual_prompt(req: &HandOffRequest, workbook: Option<&Path>) -> String {
    let mut prompt = req.prompt.clone();
    prompt.push_str("\n\nOmacell hand-off context:\n- Use the installed omacell skill.\n");
    if let Some(path) = workbook {
        prompt.push_str(&format!("- Workbook: {}\n", path.display()));
    }
    if let Some(selection) = &req.selection {
        prompt.push_str(&format!("- Current selection: {selection}\n"));
    }
    if let Some(path) = &req.diagnose {
        prompt.push_str(&format!(
            "- Read the diagnostic bundle: {}\n",
            absolute_path(path).display()
        ));
    }
    prompt.push_str(
        "- Propose workbook edits as changesets; do not apply them without the user's review.",
    );
    prompt
}

fn write_handoff_prompt(state_dir: &Path, bytes: &[u8]) -> Result<PathBuf, CoreError> {
    let dir = state_dir.join("handoffs");
    std::fs::create_dir_all(&dir).map_err(|err| CoreError::new("agent.io", err.to_string()))?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|err| CoreError::new("agent.io", err.to_string()))?;
    for _ in 0..1_024 {
        let sequence = HANDOFF_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("request-{}-{sequence}.md", std::process::id()));
        let opened = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path);
        match opened {
            Ok(mut file) => {
                file.write_all(bytes)
                    .map_err(|err| CoreError::new("agent.io", err.to_string()))?;
                file.write_all(b"\n")
                    .map_err(|err| CoreError::new("agent.io", err.to_string()))?;
                return Ok(path);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(CoreError::new("agent.io", err.to_string())),
        }
    }
    Err(CoreError::new(
        "agent.io",
        "could not allocate a unique hand-off request path",
    ))
}

/// Render argv as a copy/paste-safe POSIX shell command.
#[must_use]
pub fn shell_command(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| {
            if !arg.is_empty()
                && arg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&b))
            {
                arg.clone()
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Persist a private diagnostic bundle under the Omacell state directory.
pub fn write_diagnostic_bundle(
    state_dir: &Path,
    bundle: &serde_json::Value,
) -> Result<PathBuf, CoreError> {
    let dir = state_dir.join("diagnose");
    std::fs::create_dir_all(&dir).map_err(|err| CoreError::new("agent.io", err.to_string()))?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|err| CoreError::new("agent.io", err.to_string()))?;
    let bytes = serde_json::to_vec_pretty(bundle)
        .map_err(|err| CoreError::new("agent.json", err.to_string()))?;
    for _ in 0..1_024 {
        let sequence = DIAGNOSTIC_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("bundle-{}-{sequence}.json", std::process::id()));
        let opened = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path);
        match opened {
            Ok(mut file) => {
                file.write_all(&bytes)
                    .map_err(|err| CoreError::new("agent.io", err.to_string()))?;
                file.write_all(b"\n")
                    .map_err(|err| CoreError::new("agent.io", err.to_string()))?;
                return Ok(path);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(CoreError::new("agent.io", err.to_string())),
        }
    }
    Err(CoreError::new(
        "agent.io",
        "could not allocate a unique diagnostic bundle path",
    ))
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
