//! Stable `{code, message, hint}` codes for the command bus.

use omacell_core::error::CoreError;

/// Machine codes for bus errors. CLI, IPC, and MCP mirror these strings.
pub mod codes {
    /// Command id is not registered.
    pub const COMMAND_UNKNOWN: &str = "command.unknown";
    /// Arguments failed structural JSON validation.
    pub const COMMAND_ARGS: &str = "command.args";
    /// Origin is not allowed to perform this operation.
    pub const COMMAND_DENIED: &str = "command.denied";
    /// Internal restore used as an external forward command.
    pub const COMMAND_INTERNAL: &str = "command.internal";
    /// Command is not legal in a changeset proposal.
    pub const COMMAND_INELIGIBLE: &str = "command.ineligible";
    /// Range covers more cells than the bus will iterate.
    pub const RANGE_SIZE: &str = "range.size";
    /// Changeset id is not in the store.
    pub const CHANGESET_NOT_FOUND: &str = "changeset.not_found";
    /// Changeset lifecycle does not allow this operation.
    pub const CHANGESET_STATE: &str = "changeset.state";
    /// Changeset count, command count, retained bytes, or effect records exceeded a limit.
    pub const CHANGESET_LIMIT: &str = "changeset.limit";
    /// IPC envelope version is missing or not 1.
    pub const IPC_VERSION: &str = "ipc.version";
    /// Frame is oversized, not UTF-8, or not a single JSON line.
    pub const IPC_FRAME: &str = "ipc.frame";
    /// Unknown field, op, mode, or mutually exclusive cmd/op.
    pub const IPC_PROTOCOL: &str = "ipc.protocol";
    /// `mode` is not allowed for this command.
    pub const IPC_MODE: &str = "ipc.mode";
    /// Connection, nesting, or queue limit exceeded.
    pub const IPC_LIMIT: &str = "ipc.limit";
    /// Socket path, permissions, owner, or symlink check failed.
    pub const IPC_SOCKET: &str = "ipc.socket";
    /// Client request timed out.
    pub const IPC_TIMEOUT: &str = "ipc.timeout";
    /// Peer closed the connection.
    pub const IPC_DISCONNECTED: &str = "ipc.disconnected";
    /// Per-client event queue overflowed.
    pub const IPC_OVERFLOW: &str = "ipc.overflow";
    /// Task-runner submit queue is full.
    pub const TASK_QUEUE: &str = "task.queue";
    /// Task-runner worker is shut down.
    pub const TASK_SHUTDOWN: &str = "task.shutdown";
    /// Cooperative cancel completed without committing.
    pub const TASK_CANCELLED: &str = "task.cancelled";
}

pub(crate) fn unknown(id: &str) -> CoreError {
    CoreError::new(codes::COMMAND_UNKNOWN, format!("unknown command {id:?}"))
        .with_hint("call commands_json() for the public catalog")
}

pub(crate) fn args(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::COMMAND_ARGS, message)
        .with_hint("arguments must match the command schema; unknown fields are rejected")
}

pub(crate) fn denied(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::COMMAND_DENIED, message)
        .with_hint("model origins propose mutating work as a changeset")
}

pub(crate) fn internal(id: &str) -> CoreError {
    CoreError::new(
        codes::COMMAND_INTERNAL,
        format!("command {id} is an internal restore handler"),
    )
    .with_hint("internal commands cannot be used as external forwards")
}

pub(crate) fn ineligible(id: &str) -> CoreError {
    CoreError::new(
        codes::COMMAND_INELIGIBLE,
        format!("command {id} cannot appear in a changeset proposal"),
    )
}

pub(crate) fn range_size(area: u64) -> CoreError {
    CoreError::new(
        codes::RANGE_SIZE,
        format!(
            "range covers {area} cells; maximum is {}",
            crate::resolve::MAX_RANGE_CELLS
        ),
    )
    .with_hint("narrow the range or issue multiple commands")
}

pub(crate) fn changeset_not_found(id: &str) -> CoreError {
    CoreError::new(
        codes::CHANGESET_NOT_FOUND,
        format!("changeset {id:?} does not exist"),
    )
}

pub(crate) fn changeset_state(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::CHANGESET_STATE, message)
}

pub(crate) fn changeset_limit(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::CHANGESET_LIMIT, message)
        .with_hint("split the work into smaller reviewed changesets or start a new session")
}

pub(crate) fn ipc_version(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::IPC_VERSION, message).with_hint("IPC v1 requires \"v\":1")
}

pub(crate) fn ipc_frame(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::IPC_FRAME, message)
        .with_hint("send one UTF-8 JSON object per line, at most 1 MiB")
}

pub(crate) fn ipc_protocol(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::IPC_PROTOCOL, message)
        .with_hint("unknown fields, versions, modes, and ops are rejected")
}

pub(crate) fn ipc_mode(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::IPC_MODE, message).with_hint(
        "mutating registry commands default to propose; execute is not allowed on the socket",
    )
}

pub(crate) fn ipc_limit(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::IPC_LIMIT, message)
}

pub(crate) fn ipc_socket(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::IPC_SOCKET, message)
        .with_hint("runtime dir must be 0700, sockets 0600, owned, and not a symlink")
}

pub(crate) fn ipc_timeout(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::IPC_TIMEOUT, message)
}

pub(crate) fn ipc_disconnected() -> CoreError {
    CoreError::new(codes::IPC_DISCONNECTED, "IPC peer closed the connection")
}

pub(crate) fn task_queue() -> CoreError {
    CoreError::new(codes::TASK_QUEUE, "command queue is full")
        .with_hint("wait for the current long operation to finish")
}

pub(crate) fn task_shutdown() -> CoreError {
    CoreError::new(codes::TASK_SHUTDOWN, "command worker stopped")
}

pub(crate) fn task_cancelled() -> CoreError {
    CoreError::new(codes::TASK_CANCELLED, "operation cancelled")
        .with_hint("the live workbook and destination file were left unchanged")
}
