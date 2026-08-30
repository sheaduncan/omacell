//! In-process command bus, changesets, and events for Omacell.
//!
//! This crate is the only mutation path for anything outside `omacell-core`.
//! Front-ends, Lua, CLI, IPC, MCP, and models all invoke the same
//! [`CommandRegistry`]. Unix-socket IPC is [`ipc`].
//!
//! Later packages register commands with [`CommandRegistry::register`] without
//! modifying this crate's handler modules.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod args;
mod catalog;
mod changeset;
pub mod chart;
mod commands;
pub mod edit;
mod error;
mod event;
mod handler;
#[cfg(unix)]
pub mod ipc;
mod logical;
mod policy;
mod registry;
mod resolve;
mod restore;
mod runner;
mod session;
mod task;

pub use catalog::{CommandJson, CommandsEnvelope, SCHEMA, commands_json};
pub use changeset::{
    ChangesetStore, MAX_CHANGESET_BYTES, MAX_CHANGESET_COMMANDS, MAX_CHANGESET_STORE_BYTES,
    MAX_CHANGESETS, MAX_EFFECT_RECORDS,
};
pub use chart::register_chart_commands;
pub use commands::register_core;
pub use edit::register_edit_commands;
pub use error::codes;
pub use event::{EventBus, SubscriberId};
pub use handler::{CommandContext, Effect, TaskCtl};
pub use policy::MutationPolicy;
pub use registry::{CommandKind, CommandRegistry, CommandSpec, Exposure, RegisteredCommand};
pub use resolve::MAX_RANGE_CELLS;
pub use runner::{TaskRunner, TaskRunnerHandle, register_hold_command};
pub use session::{Bus, DryRun};
pub use task::{
    CancelHandle, LongOps, ReaderSnapshot, TaskEvent, TaskId, TaskProgress, TaskState, TaskStatus,
};
