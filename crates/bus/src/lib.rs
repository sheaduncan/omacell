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
mod commands;
mod error;
mod event;
mod handler;
#[cfg(unix)]
pub mod ipc;
mod logical;
mod policy;
mod registry;
mod resolve;
mod session;

pub use catalog::{CommandJson, CommandsEnvelope, SCHEMA, commands_json};
pub use changeset::ChangesetStore;
pub use commands::register_core;
pub use error::codes;
pub use event::{EventBus, SubscriberId};
pub use handler::{CommandContext, Effect};
pub use policy::MutationPolicy;
pub use registry::{CommandKind, CommandRegistry, CommandSpec, Exposure, RegisteredCommand};
pub use session::{Bus, DryRun};
