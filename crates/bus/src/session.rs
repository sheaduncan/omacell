//! In-process command session: execute, propose, apply, revert, dry-run.

use omacell_core::changeset::{ChangeSummary, Changeset, ChangesetId, CommandCall};
use omacell_core::command::{Origin, Outcome};
use omacell_core::error::CoreError;
use omacell_core::eval::FnRegistry;
use omacell_core::event::Event;
use omacell_core::recalc::{RecalcEngine, RecalcResult};
use omacell_core::workbook::{CalcMode, Workbook};

use crate::changeset::ChangesetStore;
use crate::commands::register_core;
use crate::error as bus_error;
use crate::event::{EventBus, SubscriberId};
use crate::handler::{CommandContext, Effect};
use crate::policy::MutationPolicy;
use crate::registry::{CommandRegistry, Exposure};

/// Outcome of a dry-run. Live session state is not modified.
#[derive(Clone, Debug)]
pub struct DryRun {
    /// Would-be command outcome.
    pub outcome: Outcome,
    /// Would-be change summary.
    pub summary: ChangeSummary,
}

/// In-process command bus. The only mutation path outside `omacell-core`.
pub struct Bus {
    workbook: Workbook,
    engine: RecalcEngine,
    registry: CommandRegistry,
    changesets: ChangesetStore,
    events: EventBus,
}

impl Bus {
    /// Session with the WP-07a core command set registered.
    pub fn new(workbook: Workbook, engine: RecalcEngine) -> Result<Self, CoreError> {
        let mut registry = CommandRegistry::new();
        register_core(&mut registry)?;
        Ok(Self {
            workbook,
            engine,
            registry,
            changesets: ChangesetStore::new(),
            events: EventBus::new(),
        })
    }

    /// Borrow the workbook.
    #[must_use]
    pub fn workbook(&self) -> &Workbook {
        &self.workbook
    }

    /// Borrow the recalc engine.
    #[must_use]
    pub fn engine(&self) -> &RecalcEngine {
        &self.engine
    }

    /// Mutable engine (startup registry, thread count, locale injectors).
    pub fn engine_mut(&mut self) -> &mut RecalcEngine {
        &mut self.engine
    }

    /// Command registry.
    #[must_use]
    pub fn registry(&self) -> &CommandRegistry {
        &self.registry
    }

    /// Mutable registry so later packages can register commands.
    pub fn registry_mut(&mut self) -> &mut CommandRegistry {
        &mut self.registry
    }

    /// Changeset store.
    #[must_use]
    pub fn changesets(&self) -> &ChangesetStore {
        &self.changesets
    }

    /// Public command catalog (schema version 1).
    pub fn commands_json(&self) -> Result<String, serde_json::Error> {
        self.registry.commands_json()
    }

    /// Subscribe to events with a bounded queue.
    pub fn subscribe(&mut self, cap: usize) -> SubscriberId {
        self.events.subscribe(cap)
    }

    /// Drain queued events for a subscriber.
    pub fn drain(&mut self, id: SubscriberId) -> Vec<Event> {
        self.events.drain(id)
    }

    /// Overflow count for a stalled subscriber.
    #[must_use]
    pub fn dropped(&self, id: SubscriberId) -> u64 {
        self.events.dropped(id)
    }

    /// Drop an event subscriber.
    pub fn unsubscribe(&mut self, id: SubscriberId) {
        self.events.unsubscribe(id);
    }

    /// Execute a single command. Model origins cannot directly mutate.
    pub fn execute(&mut self, origin: Origin, id: &str, args: serde_json::Value) -> Outcome {
        let call = match command_call(id, args) {
            Ok(call) => call,
            Err(err) => return Outcome::failure(err),
        };
        match self.run(origin, std::slice::from_ref(&call), Run::direct()) {
            Ok(effect) => Outcome::success(effect.result),
            Err(err) => Outcome::failure(err),
        }
    }

    /// Validate and describe a command without touching live state.
    pub fn dry_run(
        &mut self,
        origin: Origin,
        id: &str,
        args: serde_json::Value,
    ) -> Result<DryRun, CoreError> {
        let call = command_call(id, args)?;
        match self.run(origin, std::slice::from_ref(&call), Run::dry()) {
            Ok(effect) => Ok(DryRun {
                outcome: Outcome::success(effect.result.clone()),
                summary: effect.summary,
            }),
            Err(err) => Ok(DryRun {
                outcome: Outcome::failure(err),
                summary: ChangeSummary::default(),
            }),
        }
    }

    /// Propose a batch. Live workbook, engine, undo, and events stay untouched.
    pub fn propose(
        &mut self,
        origin: Origin,
        forward: Vec<CommandCall>,
    ) -> Result<Changeset, CoreError> {
        if !MutationPolicy::allow_propose(origin) {
            return Err(bus_error::denied("this origin cannot propose a changeset"));
        }
        let effect = self.run(origin, &forward, Run::propose())?;
        let changeset =
            self.changesets
                .insert_proposed(origin, forward, effect.inverse, effect.summary)?;
        self.events.emit(Event::ChangesetProposed {
            id: changeset.id.clone(),
        });
        Ok(changeset)
    }

    /// Apply a proposed changeset as one undo unit and recalculate once.
    pub fn apply(&mut self, origin: Origin, id: &ChangesetId) -> Result<Changeset, CoreError> {
        if !MutationPolicy::allow_apply(origin) {
            return Err(bus_error::denied("this origin cannot apply a changeset"));
        }
        // Validate the lifecycle before dispatch. Running first and rejecting in
        // `mark_applied` would let an invalid second apply mutate live state.
        let forward = self.changesets.forward_for_apply(id)?.to_vec();
        let effect = self.run(origin, &forward, Run::apply())?;
        let changeset = self
            .changesets
            .mark_applied(id, effect.inverse, effect.summary)?;
        self.events.emit(Event::ChangesetApplied {
            id: changeset.id.clone(),
        });
        Ok(changeset)
    }

    /// Revert an applied changeset as one undo unit and recalculate once.
    pub fn revert(&mut self, origin: Origin, id: &ChangesetId) -> Result<Changeset, CoreError> {
        if !MutationPolicy::allow_apply(origin) {
            return Err(bus_error::denied("this origin cannot revert a changeset"));
        }
        // A proposed or already-reverted changeset must fail before any trusted
        // inverse command reaches the workbook.
        let inverse = self.changesets.inverse_for_revert(id)?.to_vec();
        let _ = self.run(origin, &inverse, Run::revert())?;
        let changeset = self.changesets.mark_reverted(id)?;
        self.events.emit(Event::ChangesetReverted {
            id: changeset.id.clone(),
        });
        Ok(changeset)
    }

    /// List changesets in insertion order.
    #[must_use]
    pub fn list_changesets(&self) -> Vec<Changeset> {
        self.changesets.list()
    }

    /// Fetch one changeset.
    pub fn get_changeset(&self, id: &ChangesetId) -> Result<&Changeset, CoreError> {
        self.changesets.get(id)
    }

    fn run(
        &mut self,
        origin: Origin,
        calls: &[CommandCall],
        how: Run,
    ) -> Result<Effect, CoreError> {
        self.check_calls(origin, calls, &how)?;
        let mut scratch_wb = self.workbook.clone();
        let mut scratch_engine = clone_engine(&self.engine);
        let preflight = dispatch(
            &self.registry,
            &mut scratch_wb,
            &mut scratch_engine,
            origin,
            calls,
        )?;
        if how.scratch_only {
            if how.recalc {
                apply_recalc(&mut scratch_wb, &mut scratch_engine, &preflight);
            }
            return Ok(preflight);
        }
        let live_effect = {
            let Bus {
                workbook,
                engine,
                registry,
                ..
            } = self;
            let mut live_effect = Effect::default();
            workbook.transact_try(|wb| {
                live_effect = dispatch(registry, wb, engine, origin, calls)?;
                Ok(())
            })?;
            live_effect
        };
        let extra = apply_recalc(&mut self.workbook, &mut self.engine, &live_effect);
        if how.emit {
            emit_events(&mut self.events, &live_effect, extra);
        }
        Ok(live_effect)
    }

    fn check_calls(
        &self,
        origin: Origin,
        calls: &[CommandCall],
        how: &Run,
    ) -> Result<(), CoreError> {
        for call in calls {
            let cmd = self
                .registry
                .get(&call.id)
                .ok_or_else(|| bus_error::unknown(call.id.as_str()))?;
            if cmd.exposure == Exposure::Internal && !how.allow_internal {
                return Err(bus_error::internal(call.id.as_str()));
            }
            if how.require_eligible && !cmd.changeset_eligible {
                return Err(bus_error::ineligible(call.id.as_str()));
            }
            if cmd.descriptor.mutating
                && how.require_direct
                && !MutationPolicy::allow_direct_mutate(origin)
            {
                return Err(bus_error::denied(format!(
                    "origin {origin:?} cannot directly execute mutating command {}",
                    call.id
                )));
            }
        }
        Ok(())
    }
}

struct Run {
    allow_internal: bool,
    require_eligible: bool,
    require_direct: bool,
    scratch_only: bool,
    emit: bool,
    recalc: bool,
}

impl Run {
    fn direct() -> Self {
        Self {
            allow_internal: false,
            require_eligible: false,
            require_direct: true,
            scratch_only: false,
            emit: true,
            recalc: true,
        }
    }

    fn dry() -> Self {
        Self {
            allow_internal: false,
            require_eligible: false,
            require_direct: true,
            scratch_only: true,
            emit: false,
            recalc: true,
        }
    }

    fn propose() -> Self {
        Self {
            allow_internal: false,
            require_eligible: true,
            require_direct: false,
            scratch_only: true,
            emit: false,
            recalc: false,
        }
    }

    fn apply() -> Self {
        Self {
            allow_internal: false,
            require_eligible: true,
            require_direct: false,
            scratch_only: false,
            emit: true,
            recalc: true,
        }
    }

    fn revert() -> Self {
        Self {
            allow_internal: true,
            require_eligible: false,
            require_direct: false,
            scratch_only: false,
            emit: true,
            recalc: true,
        }
    }
}

fn command_call(id: &str, args: serde_json::Value) -> Result<CommandCall, CoreError> {
    Ok(CommandCall {
        id: omacell_core::command::CommandId::new(id)?,
        args,
    })
}

fn clone_engine(engine: &RecalcEngine) -> RecalcEngine {
    let mut registry = FnRegistry::new();
    for def in engine.registry().iter() {
        registry.register(*def);
    }
    let mut cloned = RecalcEngine::new(registry);
    cloned.set_threads(engine.threads());
    cloned
}

fn dispatch(
    registry: &CommandRegistry,
    workbook: &mut Workbook,
    engine: &mut RecalcEngine,
    origin: Origin,
    calls: &[CommandCall],
) -> Result<Effect, CoreError> {
    let mut combined = Effect {
        auto_recalc: false,
        ..Effect::default()
    };
    for call in calls {
        let cmd = registry
            .get(&call.id)
            .ok_or_else(|| bus_error::unknown(call.id.as_str()))?;
        let mut ctx = CommandContext::new(workbook, engine, origin);
        let effect = cmd.invoke(&mut ctx, call.args.clone())?;
        combined.append(effect);
    }
    combined.inverse.reverse();
    Ok(combined)
}

fn apply_recalc(
    workbook: &mut Workbook,
    engine: &mut RecalcEngine,
    effect: &Effect,
) -> Option<Event> {
    if effect.rebuild {
        engine.rebuild(workbook);
    }
    for coord in &effect.dirty {
        engine.notify_edit(workbook, *coord);
    }
    if !effect.auto_recalc {
        return None;
    }
    if workbook.settings().calc_mode == CalcMode::Manual {
        return None;
    }
    let RecalcResult {
        cells_evaluated,
        elapsed_ms,
        ..
    } = engine.recalc_incremental(workbook);
    Some(Event::RecalcDone {
        cells: cells_evaluated,
        elapsed_ms,
    })
}

fn emit_events(bus: &mut EventBus, effect: &Effect, extra_recalc: Option<Event>) {
    let mut cells: Vec<Event> = effect
        .events
        .iter()
        .filter(|event| matches!(event, Event::CellChanged { .. }))
        .cloned()
        .collect();
    cells.sort_by_key(|event| match event {
        Event::CellChanged { sheet, row, col } => (sheet.index(), *row, *col),
        _ => (u32::MAX, u32::MAX, u16::MAX),
    });
    for event in cells {
        bus.emit(event);
    }
    for event in &effect.events {
        if !matches!(event, Event::CellChanged { .. }) {
            bus.emit(event.clone());
        }
    }
    if let Some(event) = extra_recalc {
        bus.emit(event);
    }
}
