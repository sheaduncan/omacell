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
