//! Deterministic command registry and extension API.

use std::collections::BTreeMap;

use omacell_core::command::{CommandDescriptor, CommandId};
use omacell_core::error::CoreError;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::error::{self, codes};
use crate::handler::{CommandContext, Effect};

/// Public vs internal exposure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exposure {
    /// Listed in [`CommandRegistry::commands_json`] and legal as a forward.
    Public,
    /// Restore/inverse only. Excluded from the catalog.
    Internal,
}

/// Mutating vs query classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    /// Changes workbook or engine state.
    Mutating,
    /// Read-only.
    Query,
}

/// Static metadata supplied at registration.
#[derive(Clone, Copy, Debug)]
pub struct CommandSpec {
    /// Dotted id (`cell.set`).
    pub id: &'static str,
    /// One-line documentation.
    pub doc: &'static str,
    /// Kind.
    pub kind: CommandKind,
    /// Whether the command may appear as a changeset forward.
    pub changeset_eligible: bool,
    /// Catalog vs restore-only.
    pub exposure: Exposure,
    /// Default keymap chords (classic).
    pub default_keys: &'static [&'static str],
}

/// One registered command.
pub struct RegisteredCommand {
    /// Frozen WP-01 descriptor (subset of catalog fields).
    pub descriptor: CommandDescriptor,
    /// Default keys copied into the catalog.
    pub default_keys: Vec<String>,
    /// Changeset eligibility.
    pub changeset_eligible: bool,
    /// Public or internal.
    pub exposure: Exposure,
    /// Mutating vs query.
    pub kind: CommandKind,
    snapshot_inverse: bool,
    handler: Handler,
}

type Handler = Box<
    dyn Fn(&mut CommandContext<'_>, serde_json::Value) -> Result<Effect, CoreError> + Send + Sync,
>;

impl RegisteredCommand {
    pub(crate) fn invoke(
        &self,
        ctx: &mut CommandContext<'_>,
        args: serde_json::Value,
    ) -> Result<Effect, CoreError> {
        (self.handler)(ctx, args)
    }

    pub(crate) fn needs_snapshot_inverse(&self) -> bool {
        self.snapshot_inverse
    }
}

/// Sorted registry of command handlers.
pub struct CommandRegistry {
    entries: BTreeMap<String, RegisteredCommand>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandRegistry {
    /// Empty registry. Call [`Self::register`] or [`crate::commands::register_core`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Register a typed command. Duplicate ids fail.
    ///
    /// This is the extension API for later packages (WP-08–11, WP-14, WP-17, …).
    /// Argument type `A` must use `#[serde(deny_unknown_fields)]`.
    pub fn register<A, F>(&mut self, spec: CommandSpec, handler: F) -> Result<(), CoreError>
    where
        A: DeserializeOwned + JsonSchema + Send + 'static,
        F: Fn(&mut CommandContext<'_>, A) -> Result<Effect, CoreError> + Send + Sync + 'static,
    {
        self.register_inner(spec, handler, true)
    }

    /// Register a handler that returns its own bounded logical inverse on
    /// every mutating success path.
    ///
    /// Dispatch skips the compatibility snapshot used by [`Self::register`]
    /// for this command. Query and no-op paths may return an empty inverse.
    pub fn register_with_local_inverse<A, F>(
        &mut self,
        spec: CommandSpec,
        handler: F,
    ) -> Result<(), CoreError>
    where
        A: DeserializeOwned + JsonSchema + Send + 'static,
        F: Fn(&mut CommandContext<'_>, A) -> Result<Effect, CoreError> + Send + Sync + 'static,
    {
        self.register_inner(spec, handler, false)
    }

    fn register_inner<A, F>(
        &mut self,
        spec: CommandSpec,
        handler: F,
        snapshot_inverse: bool,
    ) -> Result<(), CoreError>
    where
        A: DeserializeOwned + JsonSchema + Send + 'static,
        F: Fn(&mut CommandContext<'_>, A) -> Result<Effect, CoreError> + Send + Sync + 'static,
    {
        let id = CommandId::new(spec.id)?;
        if self.entries.contains_key(id.as_str()) {
            return Err(CoreError::new(
                codes::COMMAND_UNKNOWN,
                format!("command {} is already registered", spec.id),
            ));
        }
        let arg_schema = schemars::schema_for!(A);
        let descriptor = CommandDescriptor {
            id: id.clone(),
            doc: spec.doc.to_string(),
            arg_schema,
            mutating: matches!(spec.kind, CommandKind::Mutating),
        };
        let boxed = move |ctx: &mut CommandContext<'_>, args: serde_json::Value| {
            if !args.is_object() && !args.is_null() {
                return Err(error::args("command arguments must be a JSON object"));
            }
            let args = if args.is_null() {
                serde_json::json!({})
            } else {
                args
            };
            let typed: A = serde_json::from_value(args)
                .map_err(|err| error::args(format!("invalid arguments for {}: {err}", spec.id)))?;
            handler(ctx, typed)
        };
        self.entries.insert(
            id.as_str().to_string(),
            RegisteredCommand {
                descriptor,
                default_keys: spec.default_keys.iter().map(|k| (*k).to_string()).collect(),
                changeset_eligible: spec.changeset_eligible,
                exposure: spec.exposure,
                kind: spec.kind,
                snapshot_inverse,
                handler: Box::new(boxed),
            },
        );
        Ok(())
    }

    /// Lookup by id.
    #[must_use]
    pub fn get(&self, id: &CommandId) -> Option<&RegisteredCommand> {
        self.entries.get(id.as_str())
    }

    /// Lookup by dotted string.
    pub fn get_str(&self, id: &str) -> Result<&RegisteredCommand, CoreError> {
        let parsed = CommandId::new(id)?;
        self.entries
            .get(parsed.as_str())
            .ok_or_else(|| error::unknown(parsed.as_str()))
    }

    /// Sorted iterator of public and internal commands.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &RegisteredCommand)> {
        self.entries.iter().map(|(id, cmd)| (id.as_str(), cmd))
    }

    /// Public catalog JSON (schema version 1). Internal commands are omitted.
    pub fn commands_json(&self) -> Result<String, serde_json::Error> {
        crate::catalog::commands_json(self)
    }
}
