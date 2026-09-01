//! In-process command session: execute, propose, apply, revert, dry-run.

use std::sync::Arc;

use omacell_core::changeset::{ChangeSummary, Changeset, ChangesetId, CommandCall};
use omacell_core::command::{Origin, Outcome};
use omacell_core::error::CoreError;
use omacell_core::eval::FnRegistry;
use omacell_core::event::Event;
use omacell_core::recalc::{RecalcEngine, RecalcResult};
use omacell_core::workbook::{CalcMode, Workbook};

use crate::changeset::{ChangesetStore, MAX_EFFECT_RECORDS};
use crate::commands::register_core;
use crate::error as bus_error;
use crate::event::{EventBus, SubscriberId};
use crate::handler::{CommandContext, Effect, TaskCtl};
use crate::policy::MutationPolicy;
use crate::preview::{CellPreview, ChangePreview, ChangePreviewItem};
use crate::registry::{CommandRegistry, Exposure};

/// Outcome of a dry-run. Live session state is not modified.
#[derive(Clone, Debug)]
pub struct DryRun {
    /// Would-be command outcome.
    pub outcome: Outcome,
    /// Would-be change summary.
    pub summary: ChangeSummary,
}

/// Post-commit callback used by command-stream consumers such as the macro
/// recorder.
pub type CommandObserver = Arc<dyn Fn(Origin, &CommandCall) + Send + Sync>;

/// In-process command bus. The only mutation path outside `omacell-core`.
pub struct Bus {
    workbook: Workbook,
    engine: RecalcEngine,
    registry: CommandRegistry,
    changesets: ChangesetStore,
    events: EventBus,
    last_repeatable: Option<CommandCall>,
    command_observers: Vec<CommandObserver>,
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
            last_repeatable: None,
            command_observers: Vec::new(),
        })
    }

    /// Borrow the workbook.
    #[must_use]
    pub fn workbook(&self) -> &Workbook {
        &self.workbook
    }

    /// Mutable workbook (AI cache persist, composition-root settle).
    pub fn workbook_mut(&mut self) -> &mut Workbook {
        &mut self.workbook
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

    pub(crate) fn recalc_after_registry_change(&mut self) {
        let result = self.engine.recalc_rebuild(&mut self.workbook);
        self.events.emit(Event::RecalcDone {
            cells: result.cells_evaluated,
            elapsed_ms: result.elapsed_ms,
        });
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

    /// Observe successful direct commands after their transaction commits.
    ///
    /// Dry-runs, proposals, failed commands, and internal preflight passes are
    /// never observed. Observers must not call back into this bus.
    pub fn observe_commands(&mut self, observer: CommandObserver) {
        self.command_observers.push(observer);
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

    /// Subscribe an IPC client with count/byte limits and a wire-event filter.
    pub(crate) fn subscribe_ipc(
        &mut self,
        cap: usize,
        byte_cap: usize,
        filter: &[String],
    ) -> SubscriberId {
        self.events.subscribe_filtered(cap, byte_cap, filter)
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
        self.execute_with_task(origin, id, args, crate::handler::TaskCtl::default())
    }

    /// Execute with cooperative cancel/progress (task runner).
    pub fn execute_with_task(
        &mut self,
        origin: Origin,
        id: &str,
        args: serde_json::Value,
        task: crate::handler::TaskCtl,
    ) -> Outcome {
        if id == "edit.repeat" {
            let repeat_args = if args.is_null() {
                serde_json::json!({})
            } else {
                args
            };
            let repeat: crate::edit::RepeatArgs = match serde_json::from_value(repeat_args) {
                Ok(value) => value,
                Err(error) => {
                    return Outcome::failure(bus_error::args(format!(
                        "invalid arguments for edit.repeat: {error}"
                    )));
                }
            };
            if repeat.count == 0 || repeat.count > 1_000 {
                return Outcome::failure(bus_error::args("edit.repeat count must be in 1..=1000"));
            }
            let Some(call) = self.last_repeatable.clone() else {
                return Outcome::failure(bus_error::args("there is no prior mutation to repeat"));
            };
            let calls = vec![call; repeat.count as usize];
            return match self.run_with_task(origin, &calls, Run::direct(), task) {
                Ok(effect) => {
                    self.notify_command_observers(origin, &calls);
                    Outcome::success(effect.result)
                }
                Err(error) => Outcome::failure(error),
            };
        }
        let call = match command_call(id, args) {
            Ok(call) => call,
            Err(err) => return Outcome::failure(err),
        };
        let repeatable = self.registry.get(&call.id).is_some_and(|command| {
            command.descriptor.mutating
                && command.exposure == Exposure::Public
                && !matches!(call.id.as_str(), "edit.undo" | "edit.redo" | "edit.repeat")
        });
        match self.run_with_task(origin, std::slice::from_ref(&call), Run::direct(), task) {
            Ok(effect) => {
                let opened_workbook = effect
                    .events
                    .iter()
                    .any(|event| matches!(event, Event::WorkbookOpened { .. }));
                if opened_workbook {
                    // Changesets and repeat state belong to the workbook they
                    // were computed against. Never carry them across an open.
                    self.changesets = ChangesetStore::new();
                    self.last_repeatable = None;
                } else if repeatable {
                    self.last_repeatable = Some(call.clone());
                }
                self.notify_command_observers(origin, std::slice::from_ref(&call));
                Outcome::success(effect.result)
            }
            Err(err) => Outcome::failure(err),
        }
    }

    fn notify_command_observers(&self, origin: Origin, calls: &[CommandCall]) {
        for call in calls {
            for observer in &self.command_observers {
                observer(origin, call);
            }
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
        self.changesets.ensure_can_propose(&forward)?;
        let effect = self.run(origin, &forward, Run::propose())?;
        let changeset =
            self.changesets
                .insert_proposed(origin, forward, effect.inverse, effect.summary)?;
        self.events.emit(Event::ChangesetProposed {
            id: changeset.id.clone(),
        });
        Ok(changeset)
    }

    /// Replace the accepted command subset of a proposal without touching live state.
    pub fn revise_proposal(
        &mut self,
        origin: Origin,
        id: &ChangesetId,
        forward: Vec<CommandCall>,
    ) -> Result<Changeset, CoreError> {
        if !MutationPolicy::allow_apply(origin) {
            return Err(bus_error::denied(
                "this origin cannot revise a changeset proposal",
            ));
        }
        let proposal_origin = self.changesets.get(id)?.origin;
        let effect = self.run(proposal_origin, &forward, Run::propose())?;
        let changeset =
            self.changesets
                .replace_proposed(id, forward, effect.inverse, effect.summary)?;
        self.events.emit(Event::ChangesetProposed {
            id: changeset.id.clone(),
        });
        Ok(changeset)
    }

    /// Reject and remove a proposal without touching live state.
    pub fn discard_proposal(
        &mut self,
        origin: Origin,
        id: &ChangesetId,
    ) -> Result<Changeset, CoreError> {
        if !MutationPolicy::allow_apply(origin) {
            return Err(bus_error::denied(
                "this origin cannot discard a changeset proposal",
            ));
        }
        self.changesets.remove_proposed(id)
    }

    /// Build bounded command-local before/after data for a proposal.
    pub fn preview_changeset(&self, id: &ChangesetId) -> Result<ChangePreview, CoreError> {
        use std::collections::BTreeSet;

        let changeset = self.changesets.get(id)?.clone();
        let forward = self.changesets.forward_for_apply(id)?.to_vec();
        let mut workbook = self.workbook.clone();
        let mut engine = clone_engine(&self.engine);
        let mut items = Vec::with_capacity(forward.len());
        for command in forward {
            let before = workbook.clone();
            let effect = dispatch(
                &self.registry,
                &mut workbook,
                &mut engine,
                changeset.origin,
                std::slice::from_ref(&command),
                false,
                true,
                &TaskCtl::default(),
            )?;
            // Effects already enforce MAX_EFFECT_RECORDS. Building the preview
            // from their bounded coordinates keeps review proportional to the
            // command, rather than scanning an arbitrarily large workbook.
            let coordinates = effect
                .events
                .iter()
                .filter_map(|event| match event {
                    Event::CellChanged { sheet, row, col } => Some((sheet.index(), *row, *col)),
                    _ => None,
                })
                .chain(
                    effect
                        .dirty
                        .iter()
                        .map(|cell| (cell.sheet.index(), cell.row, cell.col)),
                )
                .collect::<BTreeSet<_>>();
            if coordinates.len() > MAX_EFFECT_RECORDS {
                return Err(bus_error::changeset_limit(format!(
                    "preview has more than {MAX_EFFECT_RECORDS} changed cells"
                )));
            }
            let mut cells = Vec::new();
            for (sheet_index, row, col) in coordinates {
                let sheet = omacell_core::addr::SheetId::new(sheet_index);
                let old = before.get(sheet, row, col).ok().flatten();
                let new = workbook.get(sheet, row, col).ok().flatten();
                if old == new {
                    continue;
                }
                let name = workbook
                    .sheet(sheet)
                    .or_else(|| before.sheet(sheet))
                    .map_or_else(
                        || format!("Sheet{}", sheet.index()),
                        |sheet| sheet.name.clone(),
                    );
                cells.push(CellPreview {
                    sheet: name,
                    row,
                    col,
                    before: old.map(|slot| crate::logical::slot_input(&before, slot)),
                    after: new.map(|slot| crate::logical::slot_input(&workbook, slot)),
                    style_changed: old.map(|slot| slot.style) != new.map(|slot| slot.style),
                });
            }
            items.push(ChangePreviewItem {
                command,
                summary: effect.summary,
                cells,
            });
        }
        Ok(ChangePreview {
            id: changeset.id,
            origin: changeset.origin,
            summary: changeset.summary,
            items,
        })
    }

    /// Apply a proposed changeset as one undo unit and recalculate once.
    pub fn apply(&mut self, origin: Origin, id: &ChangesetId) -> Result<Changeset, CoreError> {
        if !MutationPolicy::allow_apply(origin) {
            return Err(bus_error::denied("this origin cannot apply a changeset"));
        }
        // Validate the lifecycle before dispatch. Running first and rejecting in
        // `mark_applied` would let an invalid second apply mutate live state.
        let forward = self.changesets.forward_for_apply(id)?.to_vec();
        let effect = self.run(origin, &forward, Run::apply(id.clone()))?;
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
        self.run_with_task(origin, calls, how, crate::handler::TaskCtl::default())
    }

    fn run_with_task(
        &mut self,
        origin: Origin,
        calls: &[CommandCall],
        how: Run,
        task: crate::handler::TaskCtl,
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
            true,
            how.scratch_only,
            &task,
        )?;
        if let Some(id) = &how.applied_changeset {
            self.changesets
                .ensure_applied_fits(id, &preflight.inverse, &preflight.summary)?;
        }
        if how.scratch_only {
            if how.recalc {
                apply_recalc(&mut scratch_wb, &mut scratch_engine, &preflight, &task)?;
            }
            return Ok(preflight);
        }
        let live = {
            let Bus {
                workbook,
                engine,
                registry,
                changesets,
                ..
            } = self;
            workbook.transact_try(|wb| {
                let live_effect =
                    dispatch(registry, wb, engine, origin, calls, false, false, &task)?;
                if let Some(id) = &how.applied_changeset {
                    changesets.ensure_applied_fits(
                        id,
                        &live_effect.inverse,
                        &live_effect.summary,
                    )?;
                }
                let extra = apply_recalc(wb, engine, &live_effect, &task)?;
                Ok((live_effect, extra))
            })
        };
        let (live_effect, extra) = match live {
            Ok(committed) => committed,
            Err(err) => {
                self.engine.rebuild_after_rollback(&self.workbook);
                return Err(err);
            }
        };
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
    applied_changeset: Option<ChangesetId>,
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
            applied_changeset: None,
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
            applied_changeset: None,
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
            applied_changeset: None,
        }
    }

    fn apply(id: ChangesetId) -> Self {
        Self {
            allow_internal: false,
            require_eligible: true,
            require_direct: false,
            scratch_only: false,
            emit: true,
            recalc: true,
            applied_changeset: Some(id),
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
            applied_changeset: None,
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
    for def in engine.registry().iter_dynamic() {
        registry.register_dynamic(def.clone());
    }
    let mut cloned = RecalcEngine::new(registry);
    cloned.set_threads(engine.threads());
    cloned
}

#[allow(clippy::too_many_arguments)]
fn dispatch(
    registry: &CommandRegistry,
    workbook: &mut Workbook,
    engine: &mut RecalcEngine,
    origin: Origin,
    calls: &[CommandCall],
    preflight: bool,
    dry_run: bool,
    task: &crate::handler::TaskCtl,
) -> Result<Effect, CoreError> {
    let mut combined = Effect {
        auto_recalc: false,
        ..Effect::default()
    };
    for call in calls {
        let cmd = registry
            .get(&call.id)
            .ok_or_else(|| bus_error::unknown(call.id.as_str()))?;
        let before = (cmd.descriptor.mutating && cmd.changeset_eligible).then(|| workbook.clone());
        let mut ctx =
            CommandContext::with_task(workbook, engine, origin, preflight, dry_run, task.clone());
        let mut effect = cmd.invoke(&mut ctx, call.args.clone())?;
        if effect.inverse.is_empty()
            && let Some(before) = before
        {
            effect
                .inverse
                .push(crate::restore::exact_inverse(&before, workbook)?);
        }
        ensure_effect_fits(&effect)?;
        combined.append(effect);
        ensure_effect_fits(&combined)?;
    }
    combined.inverse.reverse();
    Ok(combined)
}

fn ensure_effect_fits(effect: &Effect) -> Result<(), CoreError> {
    const MAX_EFFECT_SUMMARY_BYTES: usize = 64 * 1024;
    for (kind, count) in [
        ("inverse commands", effect.inverse.len()),
        ("events", effect.events.len()),
        ("dirty cells", effect.dirty.len()),
    ] {
        if count > MAX_EFFECT_RECORDS {
            return Err(bus_error::changeset_limit(format!(
                "effect has {count} {kind}; maximum is {MAX_EFFECT_RECORDS}"
            )));
        }
    }
    if effect.summary.text.len() > MAX_EFFECT_SUMMARY_BYTES {
        return Err(bus_error::changeset_limit(format!(
            "effect summary is {} bytes; maximum is {MAX_EFFECT_SUMMARY_BYTES}",
            effect.summary.text.len()
        )));
    }
    Ok(())
}

fn apply_recalc(
    workbook: &mut Workbook,
    engine: &mut RecalcEngine,
    effect: &Effect,
    task: &crate::handler::TaskCtl,
) -> Result<Option<Event>, CoreError> {
    if effect.rebuild {
        engine.rebuild(workbook);
    }
    for coord in &effect.dirty {
        engine.notify_edit(workbook, *coord);
    }
    if !effect.auto_recalc {
        return Ok(None);
    }
    if workbook.settings().calc_mode == CalcMode::Manual {
        return Ok(None);
    }
    let result =
        engine.recalc_incremental_with_ctl(workbook, task.cancel.as_deref(), task.progress.clone());
    if result.cancelled {
        return Err(bus_error::task_cancelled());
    }
    let RecalcResult {
        cells_evaluated,
        elapsed_ms,
        ..
    } = result;
    Ok(Some(Event::RecalcDone {
        cells: cells_evaluated,
        elapsed_ms,
    }))
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
