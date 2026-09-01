//! Blocking IPC client with request correlation and a subscribe iterator.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use omacell_core::error::CoreError;
use serde_json::Value;

use super::discover::{
    default_runtime_dir, discover_default, discover_focused, discover_newest, discovered_socket,
};
use super::protocol::{
    ControlOp, FrameBuf, IpcLimits, Mode, Reply, ServerRecord, VERSION, check_json_depth,
    encode_command_with_limits, encode_control_with_limits,
};
use crate::error;

/// Default client wait for a correlated reply.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Unix-socket IPC client.
pub struct IpcClient {
    stream: UnixStream,
    next_id: u64,
    timeout: Duration,
    pending_records: VecDeque<ServerRecord>,
    frames: FrameBuf,
    lines: VecDeque<Vec<u8>>,
    limits: IpcLimits,
}

impl IpcClient {
    /// Connect to `path`.
    pub fn connect(path: impl AsRef<Path>) -> Result<Self, CoreError> {
        Self::connect_with_limits(path, IpcLimits::default())
    }

    /// Connect to `path` with a validated runtime frame limit.
    pub fn connect_with_limits(
        path: impl AsRef<Path>,
        limits: IpcLimits,
    ) -> Result<Self, CoreError> {
        let stream = UnixStream::connect(path.as_ref()).map_err(|err| {
            error::ipc_socket(format!("connect {}: {err}", path.as_ref().display()))
        })?;
        stream
            .set_read_timeout(Some(DEFAULT_TIMEOUT))
            .map_err(|err| error::ipc_socket(format!("set timeout: {err}")))?;
        stream
            .set_write_timeout(Some(DEFAULT_TIMEOUT))
            .map_err(|err| error::ipc_socket(format!("set timeout: {err}")))?;
        Ok(Self {
            stream,
            next_id: 1,
            timeout: DEFAULT_TIMEOUT,
            pending_records: VecDeque::new(),
            frames: FrameBuf::with_limits(limits),
            lines: VecDeque::new(),
            limits,
        })
    }

    /// Connect to the newest live owned instance under `dir`.
    pub fn connect_newest(dir: impl AsRef<Path>) -> Result<Self, CoreError> {
        let dir = dir.as_ref();
        let Some(record) = discover_newest(dir)? else {
            return Err(error::ipc_socket(format!(
                "no live Omacell instance in {}",
                dir.display()
            )));
        };
        Self::connect(discovered_socket(dir, &record))
    }

    /// Connect to the focused live owned instance under `dir`.
    pub fn connect_focused(dir: impl AsRef<Path>) -> Result<Self, CoreError> {
        let dir = dir.as_ref();
        let Some(record) = discover_focused(dir)? else {
            return Err(error::ipc_socket(format!(
                "no focused Omacell instance in {}",
                dir.display()
            )));
        };
        Self::connect(discovered_socket(dir, &record))
    }

    /// Connect to the focused instance, or newest live instance as a fallback.
    pub fn connect_default() -> Result<Self, CoreError> {
        Self::connect_default_with_limits(IpcLimits::default())
    }

    /// Connect to the default instance with a validated runtime frame limit.
    pub fn connect_default_with_limits(limits: IpcLimits) -> Result<Self, CoreError> {
        let dir = default_runtime_dir();
        let Some(record) = discover_default(&dir)? else {
            return Err(error::ipc_socket(format!(
                "no live Omacell instance in {}",
                dir.display()
            )));
        };
        Self::connect_with_limits(discovered_socket(&dir, &record), limits)
    }

    /// Override the request timeout.
    pub fn set_timeout(&mut self, timeout: Duration) -> Result<(), CoreError> {
        self.timeout = timeout;
        self.stream
            .set_read_timeout(Some(timeout))
            .map_err(|err| error::ipc_socket(format!("set timeout: {err}")))?;
        self.stream
            .set_write_timeout(Some(timeout))
            .map_err(|err| error::ipc_socket(format!("set timeout: {err}")))?;
        Ok(())
    }

    /// Allocate the next request id.
    fn next(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    /// Send a registry command.
    pub fn command(
        &mut self,
        cmd: &str,
        args: Value,
        mode: Option<Mode>,
    ) -> Result<Reply, CoreError> {
        let id = self.next();
        let line = encode_command_with_limits(id, cmd, &args, mode, self.limits)?;
        self.write_all(line.as_bytes())?;
        self.read_reply(id)
    }

    /// Send a control operation.
    pub fn control(
        &mut self,
        op: ControlOp,
        events: &[String],
        changeset: Option<&str>,
    ) -> Result<Reply, CoreError> {
        let id = self.next();
        let line = encode_control_with_limits(id, op, events, changeset, self.limits)?;
        self.write_all(line.as_bytes())?;
        self.read_reply(id)
    }

    /// `ping` control op.
    pub fn ping(&mut self) -> Result<Reply, CoreError> {
        self.control(ControlOp::Ping, &[], None)
    }

    /// Subscribe to `events` (empty = all).
    pub fn subscribe(&mut self, events: &[String]) -> Result<Reply, CoreError> {
        self.control(ControlOp::Subscribe, events, None)
    }

    /// Apply a proposed changeset.
    pub fn apply(&mut self, changeset: &str) -> Result<Reply, CoreError> {
        self.control(ControlOp::ChangesetApply, &[], Some(changeset))
    }

    /// Revert an applied changeset.
    pub fn revert(&mut self, changeset: &str) -> Result<Reply, CoreError> {
        self.control(ControlOp::ChangesetRevert, &[], Some(changeset))
    }

    /// Next buffered or incoming unsolicited record.
    pub fn poll_record(&mut self) -> Result<Option<ServerRecord>, CoreError> {
        if let Some(record) = self.pending_records.pop_front() {
            return Ok(Some(record));
        }
        self.stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .map_err(|err| error::ipc_socket(format!("set timeout: {err}")))?;
        let result = self.read_one_record();
        let _ = self.stream.set_read_timeout(Some(self.timeout));
        match result {
            Ok(Some(Wire::Record(r))) => Ok(Some(r)),
            Ok(Some(Wire::Reply(reply))) => {
                // Unexpected reply; surface as protocol error.
                Err(error::ipc_protocol(format!(
                    "unexpected reply for id {}",
                    reply.id
                )))
            }
            Ok(None) => Ok(None),
            Err(err) if err.code == error::codes::IPC_TIMEOUT => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), CoreError> {
        self.stream
            .write_all(bytes)
            .map_err(|_| error::ipc_disconnected())
    }

    fn read_reply(&mut self, want: u64) -> Result<Reply, CoreError> {
        loop {
            match self.read_one_record()? {
                Some(Wire::Reply(reply)) if reply.id == want => return Ok(reply),
                Some(Wire::Reply(reply)) => {
                    return Err(error::ipc_protocol(format!(
                        "unexpected reply id {}, want {want}",
                        reply.id
                    )));
                }
                Some(Wire::Record(record)) => self.pending_records.push_back(record),
                None => {
                    return Err(error::ipc_timeout(format!(
                        "timed out waiting for reply {want}"
                    )));
                }
            }
        }
    }

    fn read_one_record(&mut self) -> Result<Option<Wire>, CoreError> {
        if let Some(line) = self.lines.pop_front() {
            return parse_wire(&line).map(Some);
        }
        let mut chunk = [0u8; 8192];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err(error::ipc_disconnected()),
                Ok(n) => {
                    let mut lines = self.frames.push(&chunk[..n])?;
                    if lines.is_empty() {
                        continue;
                    }
                    let first = lines.remove(0);
                    self.lines.extend(lines);
                    return parse_wire(&first).map(Some);
                }
                Err(err)
                    if err.kind() == std::io::ErrorKind::WouldBlock
                        || err.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return Err(error::ipc_timeout("IPC read timed out"));
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(error::ipc_disconnected()),
            }
        }
    }
}

enum Wire {
    Reply(Reply),
    Record(ServerRecord),
}

fn parse_wire(line: &[u8]) -> Result<Wire, CoreError> {
    let text = std::str::from_utf8(line)
        .map_err(|_| error::ipc_frame("IPC frame is not UTF-8"))?
        .trim();
    if text.is_empty() {
        return Err(error::ipc_frame("empty IPC frame"));
    }
    check_json_depth(text)?;
    let value: Value = serde_json::from_str(text)
        .map_err(|err| error::ipc_frame(format!("invalid JSON: {err}")))?;
    let obj = value
        .as_object()
        .ok_or_else(|| error::ipc_protocol("IPC record must be a JSON object"))?;
    if obj.contains_key("ok") {
        let reply: Reply = serde_json::from_value(value)
            .map_err(|err| error::ipc_protocol(format!("invalid reply: {err}")))?;
        if reply.v != VERSION {
            return Err(error::ipc_version(format!(
                "unsupported IPC version {}",
                reply.v
            )));
        }
        return Ok(Wire::Reply(reply));
    }
    if obj.get("kind").and_then(Value::as_str) == Some("event")
        || obj.get("kind").and_then(Value::as_str) == Some("overflow")
    {
        let record: ServerRecord = serde_json::from_value(value)
            .map_err(|err| error::ipc_protocol(format!("invalid server record: {err}")))?;
        let (version, valid_payload) = match &record {
            ServerRecord::Event { v, .. } => (*v, true),
            ServerRecord::Overflow { v, dropped } => (*v, *dropped > 0),
        };
        if version != VERSION {
            return Err(error::ipc_version(format!(
                "unsupported IPC version {version}"
            )));
        }
        if !valid_payload {
            return Err(error::ipc_protocol(
                "overflow record must report at least one dropped event",
            ));
        }
        return Ok(Wire::Record(record));
    }
    Err(error::ipc_protocol("unrecognized IPC record"))
}

impl Drop for IpcClient {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}
