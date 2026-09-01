//! Versioned Unix-socket IPC transport and client (WP-07b).
//!
//! JSON-lines protocol v1. Mutating registry commands default to a proposed
//! changeset. Internal command ids are never dispatched. The server is
//! blocking std threads; tokio is reserved for AI/MCP. GUI and TUI hosts
//! publish an ephemeral focus marker so default clients target the focused
//! instance before falling back to the newest live process.

#[cfg(unix)]
mod client;
#[cfg(unix)]
mod discover;
#[cfg(unix)]
mod dispatch;
#[cfg(unix)]
mod protocol;
#[cfg(unix)]
mod server;

#[cfg(unix)]
pub use client::{DEFAULT_TIMEOUT, IpcClient};
#[cfg(unix)]
pub use discover::{
    default_runtime_dir, discover_default, discover_focused, discover_newest, discovered_socket,
    list_live_instances, prepare_runtime_dir, remove_stale_socket,
};
#[cfg(unix)]
pub use dispatch::{Dispatch, dispatch_bus_request};
#[cfg(unix)]
pub use protocol::{
    ControlOp, Discovery, EVENT_TYPES, FrameBuf, MAX_CONNECTIONS, MAX_EVENT_FILTERS,
    MAX_EVENT_QUEUE, MAX_EVENT_QUEUE_BYTES, MAX_FRAME_BYTES, MAX_JSON_DEPTH, Mode, Reply, Request,
    ServerRecord, VERSION, check_json_depth, decode_request, decode_request_bytes, encode_command,
    encode_control, encode_line, event_type_name,
};
#[cfg(unix)]
pub use server::{IpcHandle, serve, serve_runner, serve_shared};
