//! In-app agent loop (tool calling, review by default).

use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

use serde_json::{Value, json};

use crate::error::{AiError, codes};
use crate::plan::{forbidden, parse_plan};
use crate::provider::ToolSpec;

const MAX_SKILLS: usize = 64;
const MAX_SKILL_BYTES: u64 = 64 * 1024;
const MAX_CONVERSATION_BYTES: usize = 4 * 1024 * 1024;

/// Tools the in-app agent may request.
#[must_use]
pub fn agent_tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "command_run".into(),
        description: "Propose a registry command (review before apply).".into(),
        parameters: json!({
            "type": "object",
            "required": ["id"],
            "additionalProperties": false,
            "properties": {
                "id": {"type": "string"},
                "args": {"type": "object"}
            }
        }),
    }]
}

/// Validate a tool call against the frozen changeset-eligible catalog.
pub fn validate_tool(
    name: &str,
    arguments: &str,
    autopilot: bool,
    catalog: &BTreeSet<String>,
) -> Result<Value, AiError> {
    if name != "command_run" {
        return Err(AiError::new(codes::PAYLOAD, format!("unknown tool {name}")));
    }
    let args: Value = serde_json::from_str(arguments)
        .map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))?;
    let object = args
        .as_object()
        .ok_or_else(|| AiError::new(codes::PAYLOAD, "tool arguments must be an object"))?;
    if object.keys().any(|key| key != "id" && key != "args") {
        return Err(AiError::new(
            codes::PAYLOAD,
            "tool arguments contain unknown fields",
        ));
    }
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
    if !catalog.contains(id) {
        return Err(AiError::new(
            codes::PAYLOAD,
            format!("agent command {id} is not changeset-eligible"),
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
        catalog,
    )?;
    Ok(args)
}

/// Conversation record persisted under the state dir.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct Conversation {
    /// Turns.
    pub turns: Vec<Value>,
}

impl Conversation {
    /// Append a turn, evicting oldest turns to keep the private state file bounded.
    pub fn push_bounded(&mut self, turn: Value) -> Result<(), AiError> {
        self.turns.push(turn);
        loop {
            let bytes = serde_json::to_vec(self)
                .map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))?;
            if bytes.len() <= MAX_CONVERSATION_BYTES {
                return Ok(());
            }
            if self.turns.len() <= 1 {
                self.turns.clear();
                return Err(AiError::new(
                    codes::PAYLOAD,
                    "AI conversation turn exceeds the 4 MiB limit",
                ));
            }
            self.turns.remove(0);
        }
    }
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
    let expanded;
    let dir = if let Some(rest) = dir.to_str().and_then(|path| path.strip_prefix("~/")) {
        let Some(home) = std::env::var_os("HOME") else {
            return out;
        };
        expanded = Path::new(&home).join(rest);
        expanded.as_path()
    } else {
        dir
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        if out.len() >= MAX_SKILLS {
            break;
        }
        if !path.is_dir() {
            continue;
        }
        let skill = path.join("SKILL.md");
        let Ok(metadata) = std::fs::metadata(&skill) else {
            continue;
        };
        if metadata.len() > MAX_SKILL_BYTES {
            continue;
        }
        let Ok(file) = std::fs::File::open(&skill) else {
            continue;
        };
        let mut body = String::new();
        if file
            .take(MAX_SKILL_BYTES + 1)
            .read_to_string(&mut body)
            .is_err()
            || body.len() as u64 > MAX_SKILL_BYTES
        {
            continue;
        }
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
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_CONVERSATION_BYTES as u64)
    {
        return Conversation::default();
    }
    std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Persist a conversation (mode 0600).
pub fn save_conversation(state_dir: &Path, conv: &Conversation) -> Result<(), AiError> {
    let path = conversation_path(state_dir);
    if let Some(parent) = path.parent() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder
            .create(parent)
            .map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
        #[cfg(unix)]
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
    }
    let bytes =
        serde_json::to_vec(conv).map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))?;
    if bytes.len() > MAX_CONVERSATION_BYTES {
        return Err(AiError::new(
            codes::PAYLOAD,
            "AI conversation exceeds the 4 MiB limit",
        ));
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&path)
        .map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
    #[cfg(unix)]
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|err| AiError::new(codes::LOG, err.to_string()))?;
    file.write_all(&bytes)
        .map_err(|err| AiError::new(codes::LOG, err.to_string()))
}
