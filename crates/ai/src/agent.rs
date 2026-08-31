//! In-app agent loop (tool calling, review by default).

use std::path::Path;

use serde_json::{Value, json};

use crate::error::{AiError, codes};
use crate::plan::{forbidden, parse_plan};
use crate::provider::ToolSpec;

/// Tools the in-app agent may request.
#[must_use]
pub fn agent_tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "command_run".into(),
        description: "Propose a registry command (review before apply).".into(),
        parameters: json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": {"type": "string"},
                "args": {"type": "object"}
            }
        }),
    }]
}

/// Validate a tool call. Autopilot still cannot run forbidden ids.
pub fn validate_tool(name: &str, arguments: &str, autopilot: bool) -> Result<Value, AiError> {
    if name != "command_run" {
        return Err(AiError::new(codes::PAYLOAD, format!("unknown tool {name}")));
    }
    let args: Value = serde_json::from_str(arguments)
        .map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))?;
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| AiError::new(codes::PAYLOAD, "tool missing id"))?;
    if forbidden(id) {
        return Err(AiError::new(
            codes::PAYLOAD,
            format!("agent cannot run {id}"),
        ));
    }
    if autopilot && (id.starts_with("file.") || id == "script.run") {
        return Err(AiError::new(
            codes::PAYLOAD,
            format!("autopilot blocked {id}"),
        ));
    }
    let _ = parse_plan(
        &json!({"commands":[{"id": id, "args": args.get("args").cloned().unwrap_or(json!({}))}]}),
        &Default::default(),
    )?;
    Ok(args)
}

/// Conversation record persisted under the state dir.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct Conversation {
    /// Turns.
    pub turns: Vec<Value>,
}

/// One skill directory (`SKILL.md`).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Skill {
    /// Directory name.
    pub name: String,
    /// File body.
    pub body: String,
}

/// Load ADR-006 `SKILL.md` directories from `dir`.
#[must_use]
pub fn load_skills(dir: &Path) -> Vec<Skill> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        if !path.is_dir() {
            continue;
        }
        let skill = path.join("SKILL.md");
        let Ok(body) = std::fs::read_to_string(&skill) else {
            continue;
        };
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("skill")
            .to_string();
        out.push(Skill { name, body });
    }
    out
}

/// Conversation JSON path.
#[must_use]
pub fn conversation_path(state_dir: &Path) -> std::path::PathBuf {
    state_dir.join("ai").join("conversation.json")
}

/// Load a persisted conversation (empty if missing).
#[must_use]
pub fn load_conversation(state_dir: &Path) -> Conversation {
    let path = conversation_path(state_dir);
    std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Persist a conversation (mode 0600).
pub fn save_conversation(state_dir: &Path, conv: &Conversation) -> Result<(), AiError> {
    let path = conversation_path(state_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
    }
    let bytes =
        serde_json::to_vec(conv).map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))?;
    std::fs::write(&path, bytes).map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}
