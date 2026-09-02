//! Shared IPC request policy and dispatch.

use omacell_core::changeset::{Changeset, ChangesetId, CommandCall};
use omacell_core::command::{CommandId, Origin, Outcome};
use omacell_core::error::CoreError;
use serde_json::Value;

use super::protocol::{ControlOp, Mode, Reply, Request};
use crate::error;
use crate::registry::{CommandKind, Exposure};
use crate::runner::TaskRunnerHandle;
use crate::session::{Bus, DryRun};

/// Result of shared request dispatch.
///
/// Socket-only subscription changes are returned to the connection loop; all
/// command, mode, origin, and changeset policy is resolved here.
pub enum Dispatch {
    /// A complete protocol reply.
    Reply(Reply),
    /// Replace the connection's event subscription.
    Subscribe {
        /// Correlation id.
        id: u64,
        /// Validated event names.
        events: Vec<String>,
    },
    /// Remove the connection's event subscription.
    Unsubscribe {
        /// Correlation id.
        id: u64,
    },
}

impl Dispatch {
    /// Return a reply, rejecting subscription operations when a transport has
    /// no persistent connection on which to deliver unsolicited records.
    #[must_use]
    pub fn reject_subscriptions(self, message: &str) -> Reply {
        match self {
            Self::Reply(reply) => reply,
            Self::Subscribe { id, .. } | Self::Unsubscribe { id } => {
                Reply::err(id, error::ipc_protocol(message))
            }
        }
    }
}

/// Dispatch a decoded request against an in-process bus.
///
/// `origin` is selected by the transport caller and used consistently for
/// execute, dry-run, propose, apply, and revert.
pub fn dispatch_bus_request(bus: &mut Bus, origin: Origin, request: Request) -> Dispatch {
    dispatch_request(Target::Bus(bus), origin, request)
}

pub(crate) fn dispatch_runner_request(
    runner: &TaskRunnerHandle,
    origin: Origin,
    request: Request,
) -> Dispatch {
    dispatch_request(Target::Runner(runner), origin, request)
}

enum Target<'a> {
    Bus(&'a mut Bus),
    Runner(&'a TaskRunnerHandle),
}

impl Target<'_> {
    fn command_policy(&self, command: &str) -> Result<(CommandKind, bool), CoreError> {
        match self {
            Self::Bus(bus) => {
                let registered = bus.registry().get_str(command)?;
                if registered.exposure == Exposure::Internal {
                    return Err(error::internal(command));
                }
                Ok((registered.kind, registered.changeset_eligible))
            }
            Self::Runner(runner) => runner.ipc_command_policy(command),
        }
    }

    fn execute(&mut self, origin: Origin, command: &str, args: Value) -> Outcome {
        match self {
            Self::Bus(bus) => bus.execute(origin, command, args),
            Self::Runner(runner) => runner.submit_wait(origin, command, args),
        }
    }

    fn dry_run(&mut self, origin: Origin, command: &str, args: Value) -> Result<DryRun, CoreError> {
        match self {
            Self::Bus(bus) => bus.dry_run(origin, command, args),
            Self::Runner(runner) => runner.dry_run(origin, command, args),
        }
    }

    fn propose(
        &mut self,
        origin: Origin,
        commands: Vec<CommandCall>,
    ) -> Result<Changeset, CoreError> {
        match self {
            Self::Bus(bus) => bus.propose(origin, commands),
            Self::Runner(runner) => runner.propose(origin, commands),
        }
    }

    fn list_changesets(&mut self) -> Result<Vec<Changeset>, CoreError> {
        match self {
            Self::Bus(bus) => Ok(bus.list_changesets()),
            Self::Runner(runner) => runner.list_changesets(),
        }
    }

    fn get_changeset(&mut self, id: &ChangesetId) -> Result<Changeset, CoreError> {
        match self {
            Self::Bus(bus) => bus.get_changeset(id).cloned(),
            Self::Runner(runner) => runner.get_changeset(id),
        }
    }

    fn apply(&mut self, origin: Origin, id: &ChangesetId) -> Result<Changeset, CoreError> {
        match self {
            Self::Bus(bus) => bus.apply(origin, id),
            Self::Runner(runner) => runner.apply(origin, id),
        }
    }

    fn revert(&mut self, origin: Origin, id: &ChangesetId) -> Result<Changeset, CoreError> {
        match self {
            Self::Bus(bus) => bus.revert(origin, id),
            Self::Runner(runner) => runner.revert(origin, id),
        }
    }
}

fn dispatch_request(mut target: Target<'_>, origin: Origin, request: Request) -> Dispatch {
    match request {
        Request::Command {
            id,
            cmd,
            args,
            mode,
        } => Dispatch::Reply(dispatch_command(&mut target, origin, id, &cmd, args, mode)),
        Request::Control {
            id,
            op,
            events,
            changeset,
        } => match op {
            ControlOp::Subscribe => Dispatch::Subscribe { id, events },
            ControlOp::Unsubscribe => Dispatch::Unsubscribe { id },
            _ => Dispatch::Reply(dispatch_control(&mut target, origin, id, op, changeset)),
        },
    }
}

fn dispatch_command(
    target: &mut Target<'_>,
    origin: Origin,
    id: u64,
    command: &str,
    args: Value,
    mode: Option<Mode>,
) -> Reply {
    let (kind, eligible) = match target.command_policy(command) {
        Ok(policy) => policy,
        Err(error) => return Reply::err(id, error),
    };
    let mode = mode.unwrap_or(match kind {
        CommandKind::Query => Mode::Execute,
        CommandKind::Mutating if eligible => Mode::Propose,
        CommandKind::Mutating => Mode::Execute,
    });
    if kind == CommandKind::Mutating && eligible && mode == Mode::Execute {
        return Reply::err(
            id,
            error::ipc_mode("changeset-eligible mutating commands cannot use mode execute"),
        );
    }
    // Repeat is a meta-command: the session expands it into the last direct
    // mutation only after descriptor policy has been resolved here.
    if origin == Origin::Ipc && command == "edit.repeat" && mode == Mode::Execute {
        return Reply::err(
            id,
            error::ipc_mode(
                "edit.repeat cannot use mode execute over IPC; submit the original mutation as a proposal",
            ),
        );
    }
    match mode {
        Mode::Execute => outcome_reply(id, target.execute(origin, command, args)),
        Mode::DryRun => match target.dry_run(origin, command, args) {
            Ok(dry) if dry.outcome.ok => Reply::ok(
                id,
                serde_json::json!({
                    "dry_run": true,
                    "summary": dry.summary,
                    "result": dry.outcome.result,
                }),
            ),
            Ok(dry) => Reply::err(
                id,
                dry.outcome.error.unwrap_or_else(|| {
                    error::ipc_protocol("dry-run failed without an error payload")
                }),
            ),
            Err(error) => Reply::err(id, error),
        },
        Mode::Propose => {
            let call = CommandId::new(command).map(|id| CommandCall { id, args });
            match call.and_then(|call| target.propose(origin, vec![call])) {
                Ok(changeset) => serialized_reply(id, changeset, "changeset"),
                Err(error) => Reply::err(id, error),
            }
        }
    }
}

fn dispatch_control(
    target: &mut Target<'_>,
    origin: Origin,
    id: u64,
    operation: ControlOp,
    changeset: Option<String>,
) -> Reply {
    match operation {
        ControlOp::Ping => Reply::ok(id, serde_json::json!({"pong": true})),
        ControlOp::ChangesetList => match target.list_changesets() {
            Ok(changesets) => serialized_reply(id, changesets, "changesets"),
            Err(error) => Reply::err(id, error),
        },
        ControlOp::ChangesetGet | ControlOp::ChangesetApply | ControlOp::ChangesetRevert => {
            let Some(changeset) = changeset else {
                return Reply::err(id, error::ipc_protocol("changeset id is required"));
            };
            let changeset = match ChangesetId::new(changeset) {
                Ok(changeset) => changeset,
                Err(error) => return Reply::err(id, error),
            };
            let result = match operation {
                ControlOp::ChangesetGet => target.get_changeset(&changeset),
                ControlOp::ChangesetApply => target.apply(origin, &changeset),
                ControlOp::ChangesetRevert => target.revert(origin, &changeset),
                _ => return Reply::err(id, error::ipc_protocol("invalid changeset operation")),
            };
            match result {
                Ok(changeset) => serialized_reply(id, changeset, "changeset"),
                Err(error) => Reply::err(id, error),
            }
        }
        ControlOp::Subscribe | ControlOp::Unsubscribe => {
            Reply::err(id, error::ipc_protocol("invalid subscription dispatch"))
        }
    }
}

fn serialized_reply(id: u64, value: impl serde::Serialize, label: &str) -> Reply {
    match serde_json::to_value(value) {
        Ok(value) => Reply::ok(id, value),
        Err(error) => Reply::err(id, error::ipc_frame(format!("serialize {label}: {error}"))),
    }
}

fn outcome_reply(id: u64, outcome: Outcome) -> Reply {
    if outcome.ok {
        Reply::ok(id, outcome.result.unwrap_or(Value::Null))
    } else {
        Reply::err(
            id,
            outcome
                .error
                .unwrap_or_else(|| error::ipc_protocol("command failed without an error payload")),
        )
    }
}
