//! Macro recorder: command stream → Lua that calls `omacell.cmd`.

use omacell_core::error::CoreError;
use serde_json::Value;

/// Recorded command stream.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    steps: Vec<(String, Value)>,
    recording: bool,
}

impl Recorder {
    /// Empty recorder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Start capturing.
    pub fn start(&mut self) {
        self.steps.clear();
        self.recording = true;
    }

    /// Stop capturing.
    pub fn stop(&mut self) {
        self.recording = false;
    }

    /// Whether a session is open.
    #[must_use]
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// Record one successful command.
    pub fn push(&mut self, id: &str, args: Value) {
        if self.recording {
            self.steps.push((id.to_string(), args));
        }
    }

    /// Recorded steps.
    #[must_use]
    pub fn steps(&self) -> &[(String, Value)] {
        &self.steps
    }

    /// Emit readable Lua.
    #[must_use]
    pub fn to_lua(&self) -> String {
        let mut out = String::from("-- Recorded by Omacell (WP-20)\n");
        for (id, args) in &self.steps {
            out.push_str("omacell.cmd(");
            out.push_str(&lua_string(id));
            out.push_str(", ");
            out.push_str(&json_to_lua(args));
            out.push_str(")\n");
        }
        out
    }
}

/// Encode a JSON value as a Lua literal.
#[must_use]
pub fn json_to_lua(value: &Value) -> String {
    match value {
        Value::Null => "nil".into(),
        Value::Bool(b) => {
            if *b {
                "true".into()
            } else {
                "false".into()
            }
        }
        Value::Number(n) => n.to_string(),
        Value::String(s) => lua_string(s),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(json_to_lua).collect();
            format!("{{{}}}", inner.join(", "))
        }
        Value::Object(map) => {
            let mut parts: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("[{}] = {}", lua_string(k), json_to_lua(v)))
                .collect();
            parts.sort();
            format!("{{{}}}", parts.join(", "))
        }
    }
}

fn lua_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Replay recorded Lua through a command executor.
pub fn replay_lua(
    source: &str,
    mut exec: impl FnMut(&str, Value) -> Result<Value, CoreError>,
) -> Result<(), CoreError> {
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("--") {
            continue;
        }
        let (id, args) = parse_cmd_line(line)?;
        exec(&id, args)?;
    }
    Ok(())
}

fn parse_cmd_line(line: &str) -> Result<(String, Value), CoreError> {
    let rest = line
        .strip_prefix("omacell.cmd(")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| CoreError::new("macro.parse", "expected omacell.cmd(...)"))?;
    let rest = rest.trim();
    let mut chars = rest.chars();
    if chars.next() != Some('"') {
        return Err(CoreError::new("macro.parse", "expected command id string"));
    }
    let mut id = String::new();
    loop {
        match chars.next() {
            Some('\\') => match chars.next() {
                Some(c) => id.push(c),
                None => break,
            },
            Some('"') => break,
            Some(c) => id.push(c),
            None => {
                return Err(CoreError::new("macro.parse", "unterminated command id"));
            }
        }
    }
    let leftover: String = chars.collect();
    let leftover = leftover.trim().trim_start_matches(',').trim();
    let args = lua_table_to_json(leftover)?;
    Ok((id, args))
}

fn lua_table_to_json(src: &str) -> Result<Value, CoreError> {
    // The recorder emits a restricted Lua table subset we can eval as JSON
    // after replacing Lua syntax.
    let mut s = src.trim().to_string();
    if s == "nil" || s.is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    s = s.replace("[\"", "\"");
    s = s.replace("\"] = ", "\": ");
    s = s.replace(" = ", ": ");
    s = s.replace("nil", "null");
    serde_json::from_str(&s).map_err(|e| CoreError::new("macro.parse", e.to_string()))
}
