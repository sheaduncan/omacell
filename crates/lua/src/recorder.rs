//! Macro recorder: command stream → Lua that calls `omacell.cmd`.

use omacell_core::error::CoreError;
use serde_json::Value;

/// Maximum commands retained by one recording session.
pub const MAX_RECORDED_STEPS: usize = 10_000;
/// Maximum estimated command bytes retained by one recording session.
pub const MAX_RECORDED_BYTES: usize = 16 * 1024 * 1024;

/// Recorded command stream.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    steps: Vec<(String, Value)>,
    recording: bool,
    retained_bytes: usize,
    overflowed: bool,
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
        self.retained_bytes = 0;
        self.overflowed = false;
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

    /// Whether capture stopped because its bounded retention limit was hit.
    #[must_use]
    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Record one successful command.
    pub fn push(&mut self, id: &str, args: Value) {
        if !self.recording {
            return;
        }
        let bytes = id
            .len()
            .saturating_add(serde_json::to_vec(&args).map_or(MAX_RECORDED_BYTES + 1, |v| v.len()));
        if self.steps.len() >= MAX_RECORDED_STEPS
            || self.retained_bytes.saturating_add(bytes) > MAX_RECORDED_BYTES
        {
            self.recording = false;
            self.overflowed = true;
            return;
        }
        self.retained_bytes += bytes;
        self.steps.push((id.to_string(), args));
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
            c if c.is_control() => {
                let mut bytes = [0u8; 4];
                for byte in c.encode_utf8(&mut bytes).as_bytes() {
                    out.push_str(&format!("\\{byte:03}"));
                }
            }
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
    let (lua, _counter) = crate::runtime::embedded_lua()?;
    lua.scope(|scope| {
        let omacell = lua.create_table()?;
        let cmd = scope.create_function_mut(|lua, (id, args): (String, mlua::Value)| {
            let args = crate::runtime::lua_to_json(&args).map_err(mlua::Error::external)?;
            let result = exec(&id, args).map_err(mlua::Error::external)?;
            crate::runtime::json_to_lua(lua, &result)
        })?;
        omacell.set("cmd", cmd)?;
        lua.globals().set("omacell", omacell)?;
        lua.load(source).set_name("recorded-macro.lua").exec()
    })
    .map_err(|error| CoreError::new("macro.parse", error.to_string()))
}
