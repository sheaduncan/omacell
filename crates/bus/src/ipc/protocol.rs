//! IPC v1 JSON-line envelopes, limits, and fail-closed decoder.

use std::collections::BTreeSet;

use omacell_core::error::CoreError;
use omacell_core::event::Event;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error;

/// Envelope version frozen by this package.
pub const VERSION: u32 = 1;

/// Hard maximum JSON-line size including the trailing newline.
pub const MAX_FRAME_BYTES: usize = 16 * 1_048_576;

/// Maximum nesting of `{` / `[` outside strings.
pub const MAX_JSON_DEPTH: u32 = 32;

/// Hard cap on simultaneous accepted clients.
pub const MAX_CONNECTIONS: usize = 32;

/// Maximum event type filters on one subscribe.
pub const MAX_EVENT_FILTERS: usize = 16;

/// Maximum queued events per client.
pub const MAX_EVENT_QUEUE: usize = 64;

/// Maximum queued event payload bytes per client.
pub const MAX_EVENT_QUEUE_BYTES: usize = 262_144;

/// Validated per-server/client frame limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcLimits {
    max_frame_bytes: usize,
}

impl IpcLimits {
    /// Build limits up to the hard 16 MiB protocol ceiling.
    pub fn new(max_frame_bytes: usize) -> Result<Self, CoreError> {
        if max_frame_bytes == 0 || max_frame_bytes > MAX_FRAME_BYTES {
            return Err(error::ipc_limit(format!(
                "IPC frame limit must be in 1..={MAX_FRAME_BYTES} bytes"
            )));
        }
        Ok(Self { max_frame_bytes })
    }

    /// Maximum bytes including the trailing newline.
    #[must_use]
    pub const fn max_frame_bytes(self) -> usize {
        self.max_frame_bytes
    }
}

impl Default for IpcLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: MAX_FRAME_BYTES,
        }
    }
}

/// Allowed subscribe/filter names (frozen `Event` `type` field).
pub const EVENT_TYPES: &[&str] = &[
    "workbook_opened",
    "cell_changed",
    "recalc_done",
    "before_save",
    "file_saved",
    "changeset_proposed",
    "changeset_applied",
    "changeset_reverted",
    "theme_changed",
];

/// How a registry command is dispatched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Create a proposed changeset (default for eligible mutating commands).
    Propose,
    /// Execute immediately. Rejected for changeset-eligible mutating commands.
    Execute,
    /// Validate without touching live state.
    DryRun,
}

/// Control operations that are not registry commands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlOp {
    /// Subscribe to named event types (empty = all).
    Subscribe,
    /// Drop the client's event subscription.
    Unsubscribe,
    /// Apply a proposed changeset.
    #[serde(rename = "changeset.apply")]
    ChangesetApply,
    /// Revert an applied changeset.
    #[serde(rename = "changeset.revert")]
    ChangesetRevert,
    /// List stored changesets.
    #[serde(rename = "changeset.list")]
    ChangesetList,
    /// Fetch one changeset by id.
    #[serde(rename = "changeset.get")]
    ChangesetGet,
    /// Liveness check.
    Ping,
}

/// A v1 request: either a registry command or a control op.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// `cmd` + `args` + optional `mode`.
    Command {
        /// Correlation id echoed in the reply.
        id: u64,
        /// Dotted command id.
        cmd: String,
        /// JSON object arguments.
        args: Value,
        /// Dispatch mode.
        mode: Option<Mode>,
    },
    /// `op` plus op-specific fields.
    Control {
        /// Correlation id echoed in the reply.
        id: u64,
        /// Control operation.
        op: ControlOp,
        /// Subscribe filter (only `subscribe`).
        events: Vec<String>,
        /// Changeset id (`changeset.apply` / `revert` / `get`).
        changeset: Option<String>,
    },
}

impl Request {
    /// Correlation id.
    #[must_use]
    pub fn id(&self) -> u64 {
        match self {
            Self::Command { id, .. } | Self::Control { id, .. } => *id,
        }
    }
}

/// A v1 reply. Exactly one of `result` or `error`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Reply {
    /// Envelope version.
    pub v: u32,
    /// Echoed request id.
    pub id: u64,
    /// Success flag.
    pub ok: bool,
    /// Success payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Failure payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<CoreError>,
}

impl<'de> Deserialize<'de> for Reply {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ReplyWire::deserialize(deserializer)?;
        let (result, error) = match (wire.ok, wire.result, wire.error) {
            (true, Present::Value(result), Present::Missing) => (Some(result), None),
            (false, Present::Missing, Present::Value(error))
                if !error.code.is_empty() && !error.message.is_empty() =>
            {
                (
                    None,
                    Some(CoreError {
                        code: error.code,
                        message: error.message,
                        hint: error.hint,
                    }),
                )
            }
            (false, Present::Missing, Present::Value(_)) => {
                return Err(serde::de::Error::custom(
                    "reply error code and message must be non-empty",
                ));
            }
            _ => {
                return Err(serde::de::Error::custom(
                    "reply must contain exactly one of result or error",
                ));
            }
        };
        Ok(Self {
            v: wire.v,
            id: wire.id,
            ok: wire.ok,
            result,
            error,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyWire {
    v: u32,
    id: u64,
    ok: bool,
    #[serde(default)]
    result: Present<Value>,
    #[serde(default)]
    error: Present<WireError>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireError {
    code: String,
    message: String,
    #[serde(default)]
    hint: Option<String>,
}

#[derive(Default)]
enum Present<T> {
    #[default]
    Missing,
    Value(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Present<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self::Value)
    }
}

impl Reply {
    /// Successful reply.
    #[must_use]
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            v: VERSION,
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Failed reply.
    #[must_use]
    pub fn err(id: u64, error: CoreError) -> Self {
        Self {
            v: VERSION,
            id,
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}

/// Unsolicited record written on a subscribed connection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServerRecord {
    /// A bus event that passed the client's filter.
    Event {
        /// Envelope version.
        v: u32,
        /// Event payload (internally tagged `type`).
        event: Event,
    },
    /// The per-client queue overflowed; the connection will close.
    Overflow {
        /// Envelope version.
        v: u32,
        /// Events dropped from this client's queue.
        dropped: u64,
    },
}

impl ServerRecord {
    /// Event record.
    #[must_use]
    pub fn event(event: Event) -> Self {
        Self::Event { v: VERSION, event }
    }

    /// Overflow record.
    #[must_use]
    pub fn overflow(dropped: u64) -> Self {
        Self::Overflow {
            v: VERSION,
            dropped,
        }
    }
}

/// Instance discovery file (`<pid>.instance`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Discovery {
    /// Envelope version.
    pub v: u32,
    /// Process id of the listening instance.
    pub pid: u32,
    /// Socket file name (`{pid}.sock`) relative to the runtime directory.
    pub socket: String,
    /// Unix epoch milliseconds when the instance bound the socket.
    pub started_unix_ms: u64,
}

/// Byte-oriented line assembler with a hard frame cap.
#[derive(Clone, Debug, Default)]
pub struct FrameBuf {
    buf: Vec<u8>,
    limits: IpcLimits,
}

impl FrameBuf {
    /// Empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(IpcLimits::default())
    }

    /// Empty buffer using a validated runtime frame limit.
    #[must_use]
    pub fn with_limits(limits: IpcLimits) -> Self {
        Self {
            buf: Vec::new(),
            limits,
        }
    }

    /// Append bytes and return every complete line (without the newline).
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, CoreError> {
        let mut lines = Vec::new();
        let mut rest = bytes;
        while let Some(idx) = rest.iter().position(|b| *b == b'\n') {
            if self.buf.len().saturating_add(idx).saturating_add(1) > self.limits.max_frame_bytes {
                self.buf.clear();
                return Err(frame_limit_error(self.limits, "IPC frame"));
            }
            self.buf.extend_from_slice(&rest[..idx]);
            let mut line = std::mem::take(&mut self.buf);
            if line.last().copied() == Some(b'\r') {
                line.pop();
            }
            lines.push(line);
            rest = &rest[idx + 1..];
        }
        if self.buf.len().saturating_add(rest.len()) >= self.limits.max_frame_bytes {
            self.buf.clear();
            return Err(frame_limit_error(
                self.limits,
                "IPC frame without a newline",
            ));
        }
        self.buf.extend_from_slice(rest);
        Ok(lines)
    }
}

/// Decode one UTF-8 JSON object as a v1 request.
pub fn decode_request(text: &str) -> Result<Request, CoreError> {
    decode_request_with_limits(text, IpcLimits::default())
}

/// Decode one request under a validated runtime frame limit.
pub fn decode_request_with_limits(text: &str, limits: IpcLimits) -> Result<Request, CoreError> {
    if text.len() >= limits.max_frame_bytes {
        return Err(frame_limit_error(limits, "IPC frame"));
    }
    let text = text.trim();
    if text.is_empty() {
        return Err(error::ipc_frame("empty IPC frame"));
    }
    check_json_depth(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|err| error::ipc_frame(format!("invalid JSON: {err}")))?;
    let obj = value
        .as_object()
        .ok_or_else(|| error::ipc_protocol("IPC request must be a JSON object"))?;
    decode_request_object(obj)
}

/// Decode bytes as a request (optional trailing newline).
pub fn decode_request_bytes(data: &[u8]) -> Result<Request, CoreError> {
    decode_request_bytes_with_limits(data, IpcLimits::default())
}

/// Decode request bytes under a validated runtime frame limit.
pub fn decode_request_bytes_with_limits(
    data: &[u8],
    limits: IpcLimits,
) -> Result<Request, CoreError> {
    if data.len() > limits.max_frame_bytes
        || (data.len() == limits.max_frame_bytes && data.last().copied() != Some(b'\n'))
    {
        return Err(frame_limit_error(limits, "IPC frame"));
    }
    let text = std::str::from_utf8(data).map_err(|_| error::ipc_frame("IPC frame is not UTF-8"))?;
    let text = text.trim_end_matches(['\n', '\r']);
    decode_request_with_limits(text, limits)
}

fn decode_request_object(obj: &Map<String, Value>) -> Result<Request, CoreError> {
    let v = required_u32(obj, "v")?;
    if v != VERSION {
        return Err(error::ipc_version(format!("unsupported IPC version {v}")));
    }
    let id = required_u64(obj, "id")?;
    let has_cmd = obj.contains_key("cmd");
    let has_op = obj.contains_key("op");
    if has_cmd && has_op {
        return Err(error::ipc_protocol("request cannot have both cmd and op"));
    }
    if !has_cmd && !has_op {
        return Err(error::ipc_protocol("request must have cmd or op"));
    }
    if has_cmd {
        allow_keys(obj, &["v", "id", "cmd", "args", "mode"])?;
        let cmd = required_string(obj, "cmd")?;
        if omacell_core::command::CommandId::new(&cmd).is_err() {
            return Err(error::ipc_protocol(format!("invalid command id {cmd:?}")));
        }
        let args = match obj.get("args") {
            None => Value::Object(Map::new()),
            Some(Value::Object(_)) => obj.get("args").cloned().unwrap_or(Value::Null),
            Some(_) => return Err(error::ipc_protocol("args must be a JSON object")),
        };
        let mode = match obj.get("mode") {
            None => None,
            Some(Value::String(s)) => Some(parse_mode(s)?),
            Some(_) => return Err(error::ipc_protocol("mode must be a string")),
        };
        return Ok(Request::Command {
            id,
            cmd,
            args,
            mode,
        });
    }
    allow_keys(obj, &["v", "id", "op", "events", "changeset"])?;
    let op = parse_op(&required_string(obj, "op")?)?;
    let events = match obj.get("events") {
        None => Vec::new(),
        Some(Value::Array(items)) => {
            if items.len() > MAX_EVENT_FILTERS {
                return Err(error::ipc_limit(format!(
                    "subscribe allows at most {MAX_EVENT_FILTERS} event filters"
                )));
            }
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let Some(name) = item.as_str() else {
                    return Err(error::ipc_protocol("events entries must be strings"));
                };
                if !EVENT_TYPES.contains(&name) {
                    return Err(error::ipc_protocol(format!("unknown event type {name:?}")));
                }
                out.push(name.to_string());
            }
            out
        }
        Some(_) => return Err(error::ipc_protocol("events must be an array of strings")),
    };
    let changeset = match obj.get("changeset") {
        None => None,
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(_) => return Err(error::ipc_protocol("changeset must be a non-empty string")),
    };
    match op {
        ControlOp::Subscribe => {
            if obj.contains_key("changeset") {
                return Err(error::ipc_protocol("unexpected field for this op"));
            }
        }
        ControlOp::Unsubscribe | ControlOp::ChangesetList | ControlOp::Ping => {
            if obj.contains_key("events") || obj.contains_key("changeset") {
                return Err(error::ipc_protocol("unexpected field for this op"));
            }
        }
        ControlOp::ChangesetApply | ControlOp::ChangesetRevert | ControlOp::ChangesetGet => {
            if obj.contains_key("events") {
                return Err(error::ipc_protocol("unexpected field for this op"));
            }
            if changeset.is_none() {
                return Err(error::ipc_protocol("changeset id is required"));
            }
        }
    }
    Ok(Request::Control {
        id,
        op,
        events,
        changeset,
    })
}

fn parse_mode(s: &str) -> Result<Mode, CoreError> {
    match s {
        "propose" => Ok(Mode::Propose),
        "execute" => Ok(Mode::Execute),
        "dry_run" => Ok(Mode::DryRun),
        other => Err(error::ipc_protocol(format!("unknown mode {other:?}"))),
    }
}

fn parse_op(s: &str) -> Result<ControlOp, CoreError> {
    match s {
        "subscribe" => Ok(ControlOp::Subscribe),
        "unsubscribe" => Ok(ControlOp::Unsubscribe),
        "changeset.apply" => Ok(ControlOp::ChangesetApply),
        "changeset.revert" => Ok(ControlOp::ChangesetRevert),
        "changeset.list" => Ok(ControlOp::ChangesetList),
        "changeset.get" => Ok(ControlOp::ChangesetGet),
        "ping" => Ok(ControlOp::Ping),
        other => Err(error::ipc_protocol(format!("unknown op {other:?}"))),
    }
}

fn allow_keys(obj: &Map<String, Value>, allowed: &[&str]) -> Result<(), CoreError> {
    let allowed: BTreeSet<&str> = allowed.iter().copied().collect();
    for key in obj.keys() {
        if !allowed.contains(key.as_str()) {
            return Err(error::ipc_protocol(format!("unknown field {key:?}")));
        }
    }
    Ok(())
}

fn required_u32(obj: &Map<String, Value>, key: &str) -> Result<u32, CoreError> {
    match obj.get(key) {
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| error::ipc_protocol(format!("{key} must be a non-negative integer"))),
        Some(_) => Err(error::ipc_protocol(format!(
            "{key} must be a non-negative integer"
        ))),
        None => Err(error::ipc_protocol(format!("missing {key}"))),
    }
}

fn required_u64(obj: &Map<String, Value>, key: &str) -> Result<u64, CoreError> {
    match obj.get(key) {
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| error::ipc_protocol(format!("{key} must be a non-negative integer"))),
        Some(_) => Err(error::ipc_protocol(format!(
            "{key} must be a non-negative integer"
        ))),
        None => Err(error::ipc_protocol(format!("missing {key}"))),
    }
}

fn required_string(obj: &Map<String, Value>, key: &str) -> Result<String, CoreError> {
    match obj.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Ok(s.clone()),
        Some(_) => Err(error::ipc_protocol(format!(
            "{key} must be a non-empty string"
        ))),
        None => Err(error::ipc_protocol(format!("missing {key}"))),
    }
}

/// Reject JSON whose `{`/`[` nesting exceeds [`MAX_JSON_DEPTH`].
pub fn check_json_depth(text: &str) -> Result<(), CoreError> {
    let mut depth = 0u32;
    let mut in_str = false;
    let mut escape = false;
    for c in text.chars() {
        if in_str {
            if escape {
                escape = false;
                continue;
            }
            match c {
                '\\' => escape = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '[' => {
                depth = depth.saturating_add(1);
                if depth > MAX_JSON_DEPTH {
                    return Err(error::ipc_limit(format!(
                        "JSON nesting exceeds {MAX_JSON_DEPTH}"
                    )));
                }
            }
            '}' | ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

/// Frozen `Event` tag.
#[must_use]
pub fn event_type_name(event: &Event) -> &'static str {
    crate::event::event_type_name(event).unwrap_or("unknown")
}

/// Encode a value as a JSON line (trailing newline).
pub fn encode_line<T: Serialize>(value: &T) -> Result<String, CoreError> {
    encode_line_with_limits(value, IpcLimits::default())
}

/// Encode one JSON line under a validated runtime frame limit.
pub fn encode_line_with_limits<T: Serialize>(
    value: &T,
    limits: IpcLimits,
) -> Result<String, CoreError> {
    let mut text = serde_json::to_string(value)
        .map_err(|err| error::ipc_frame(format!("serialize IPC record: {err}")))?;
    text.push('\n');
    if text.len() > limits.max_frame_bytes {
        return Err(frame_limit_error(limits, "encoded IPC frame"));
    }
    Ok(text)
}

/// Encode a reply, replacing an oversized result with a correlated frame error.
pub fn encode_reply_with_limits(reply: &Reply, limits: IpcLimits) -> Result<String, CoreError> {
    match encode_line_with_limits(reply, limits) {
        Ok(line) => Ok(line),
        Err(err) if err.code == error::codes::IPC_FRAME => {
            encode_line_with_limits(&Reply::err(reply.id, err), limits)
        }
        Err(err) => Err(err),
    }
}

fn frame_limit_error(limits: IpcLimits, label: &str) -> CoreError {
    error::ipc_frame(format!(
        "{label} exceeds the configured {}-byte limit",
        limits.max_frame_bytes
    ))
}

/// Encode a command request as a JSON line.
pub fn encode_command(
    id: u64,
    cmd: &str,
    args: &Value,
    mode: Option<Mode>,
) -> Result<String, CoreError> {
    encode_command_with_limits(id, cmd, args, mode, IpcLimits::default())
}

/// Encode a command request under a validated runtime frame limit.
pub fn encode_command_with_limits(
    id: u64,
    cmd: &str,
    args: &Value,
    mode: Option<Mode>,
    limits: IpcLimits,
) -> Result<String, CoreError> {
    let mut map = Map::new();
    map.insert("v".into(), Value::from(VERSION));
    map.insert("id".into(), Value::from(id));
    map.insert("cmd".into(), Value::from(cmd));
    map.insert("args".into(), args.clone());
    if let Some(mode) = mode {
        let label = match mode {
            Mode::Propose => "propose",
            Mode::Execute => "execute",
            Mode::DryRun => "dry_run",
        };
        map.insert("mode".into(), Value::from(label));
    }
    encode_line_with_limits(&Value::Object(map), limits)
}

/// Encode a control request as a JSON line.
pub fn encode_control(
    id: u64,
    op: ControlOp,
    events: &[String],
    changeset: Option<&str>,
) -> Result<String, CoreError> {
    encode_control_with_limits(id, op, events, changeset, IpcLimits::default())
}

/// Encode a control request under a validated runtime frame limit.
pub fn encode_control_with_limits(
    id: u64,
    op: ControlOp,
    events: &[String],
    changeset: Option<&str>,
    limits: IpcLimits,
) -> Result<String, CoreError> {
    let mut map = Map::new();
    map.insert("v".into(), Value::from(VERSION));
    map.insert("id".into(), Value::from(id));
    let op_s = match op {
        ControlOp::Subscribe => "subscribe",
        ControlOp::Unsubscribe => "unsubscribe",
        ControlOp::ChangesetApply => "changeset.apply",
        ControlOp::ChangesetRevert => "changeset.revert",
        ControlOp::ChangesetList => "changeset.list",
        ControlOp::ChangesetGet => "changeset.get",
        ControlOp::Ping => "ping",
    };
    map.insert("op".into(), Value::from(op_s));
    if matches!(op, ControlOp::Subscribe) {
        map.insert(
            "events".into(),
            Value::Array(events.iter().cloned().map(Value::from).collect()),
        );
    }
    if let Some(cs) = changeset {
        map.insert("changeset".into(), Value::from(cs));
    }
    encode_line_with_limits(&Value::Object(map), limits)
}
