//! Handler context and typed effect records.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use omacell_core::changeset::{ChangeSummary, CommandCall};
use omacell_core::command::Origin;
use omacell_core::event::Event;
use omacell_core::graph::CellCoord;
use omacell_core::recalc::{RecalcEngine, RecalcResult};
use omacell_core::workbook::Workbook;

/// Cooperative cancel / progress hooks supplied by the task runner.
#[derive(Clone, Default)]
pub struct TaskCtl {
    /// Cooperative cancel flag.
    pub cancel: Option<Arc<AtomicBool>>,
    /// Progress sink `(done, total, label)`.
    pub progress: Option<Arc<omacell_core::recalc::RecalcProgress>>,
}

/// Per-invocation borrow of the workbook, engine, and origin.
pub struct CommandContext<'a> {
    workbook: &'a mut Workbook,
    engine: &'a mut RecalcEngine,
    origin: Origin,
    preflight: bool,
    dry_run: bool,
    task: TaskCtl,
}

impl<'a> CommandContext<'a> {
    pub(crate) fn with_task(
        workbook: &'a mut Workbook,
        engine: &'a mut RecalcEngine,
        origin: Origin,
        preflight: bool,
        dry_run: bool,
        task: TaskCtl,
    ) -> Self {
        Self {
            workbook,
            engine,
            origin,
            preflight,
            dry_run,
            task,
        }
    }

    /// Workbook being mutated or inspected.
    pub fn workbook(&mut self) -> &mut Workbook {
        self.workbook
    }

    /// Shared workbook borrow.
    #[must_use]
    pub fn workbook_ref(&self) -> &Workbook {
        self.workbook
    }

    /// Recalculation engine. Handlers other than `calc.recalc` should not run it.
    pub fn engine(&mut self) -> &mut RecalcEngine {
        self.engine
    }

    /// Simultaneous workbook and engine borrows (Goal Seek).
    pub fn workbook_and_engine(&mut self) -> (&mut Workbook, &mut RecalcEngine) {
        (self.workbook, self.engine)
    }

    /// Shared engine borrow for queries (WP-19).
    #[must_use]
    pub fn engine_ref(&self) -> &RecalcEngine {
        self.engine
    }

    /// Full recalculation (explicit `calc.recalc`).
    pub fn recalc_full(&mut self) -> RecalcResult {
        self.engine.recalc_full_with_ctl(
            self.workbook,
            self.task.cancel.as_deref(),
            self.task.progress.clone(),
        )
    }

    /// Rebuild graph then full recalculation.
    pub fn recalc_rebuild(&mut self) -> RecalcResult {
        self.engine.recalc_rebuild_with_ctl(
            self.workbook,
            self.task.cancel.as_deref(),
            self.task.progress.clone(),
        )
    }

    /// Rebuild and recalculate a staged workbook before atomically installing it.
    pub fn recalc_staged(&mut self, workbook: &mut Workbook) -> RecalcResult {
        let live_context = self.engine.session_context();
        self.engine.reset_session_context();
        let result = self.engine.recalc_rebuild_with_ctl(
            workbook,
            self.task.cancel.as_deref(),
            self.task.progress.clone(),
        );
        self.engine.restore_session_context(live_context);
        result
    }

    /// Install a successfully recalculated staged workbook as a fresh session.
    pub fn install_staged_workbook(&mut self, workbook: Workbook) {
        *self.workbook = workbook;
        self.engine.reset_session_context();
    }

    /// Incremental recalculation of the dirty set.
    pub fn recalc_incremental(&mut self) -> RecalcResult {
        self.engine.recalc_incremental_with_ctl(
            self.workbook,
            self.task.cancel.as_deref(),
            self.task.progress.clone(),
        )
    }

    /// Trusted origin for this invocation.
    #[must_use]
    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// Whether the caller requested a dry-run or changeset proposal.
    #[must_use]
    pub fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    /// Whether this is the non-committing preflight invocation.
    ///
    /// Handlers with effects outside the workbook must perform those effects
    /// only when this returns false.
    #[must_use]
    pub fn is_preflight(&self) -> bool {
        self.preflight
    }

    /// Cooperative cancel flag from the task runner, if any.
    #[must_use]
    pub fn cancel_flag(&self) -> Option<&Arc<AtomicBool>> {
        self.task.cancel.as_ref()
    }

    /// Whether the runner has requested cancel.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.task
            .cancel
            .as_ref()
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
    }

    /// Report coalesced progress for the running task.
    pub fn report_progress(&self, done: u64, total: Option<u64>, label: &str) {
        if let Some(progress) = &self.task.progress {
            progress(done, total, label);
        }
    }

    /// Clone the progress sink for an adapter callback that must be `'static`.
    #[must_use]
    pub fn progress_sink(&self) -> Option<Arc<omacell_core::recalc::RecalcProgress>> {
        self.task.progress.clone()
    }
}

/// Typed result of a handler. Summary construction uses this record, never a
/// full workbook scan.
#[derive(Clone, Debug)]
pub struct Effect {
    /// Inverse commands in execution order for this handler (batch reverses).
    pub inverse: Vec<CommandCall>,
    /// Events to emit if the outer command or batch succeeds.
    pub events: Vec<Event>,
    /// Affected-structure counts and one-line text.
    pub summary: ChangeSummary,
    /// Cells whose formula or value changed, for `notify_edit`.
    pub dirty: Vec<CellCoord>,
    /// Success payload for [`omacell_core::command::Outcome`].
    pub result: serde_json::Value,
    /// Whether the bus should auto-recalc after this effect.
    pub auto_recalc: bool,
    /// Whether the dependency graph should be rebuilt (names, sheets, mode).
    pub rebuild: bool,
}

impl Default for Effect {
    fn default() -> Self {
        Self {
            inverse: Vec::new(),
            events: Vec::new(),
            summary: ChangeSummary::default(),
            dirty: Vec::new(),
            result: serde_json::json!({}),
            auto_recalc: true,
            rebuild: false,
        }
    }
}

impl Effect {
    /// Query or no-op effect that does not trigger recalc.
    #[must_use]
    pub fn query(result: serde_json::Value) -> Self {
        Self {
            result,
            auto_recalc: false,
            ..Self::default()
        }
    }

    /// Merge `other` into this effect (later handler).
    pub fn append(&mut self, other: Self) {
        self.inverse.extend(other.inverse);
        self.events.extend(other.events);
        self.summary.cells += other.summary.cells;
        self.summary.rows += other.summary.rows;
        self.summary.columns += other.summary.columns;
        self.summary.sheets += other.summary.sheets;
        self.summary.styles += other.summary.styles;
        if !other.summary.text.is_empty() {
            if self.summary.text.is_empty() {
                self.summary.text = other.summary.text;
            } else {
                self.summary.text.push_str("; ");
                self.summary.text.push_str(&other.summary.text);
            }
        }
        self.dirty.extend(other.dirty);
        self.auto_recalc |= other.auto_recalc;
        self.rebuild |= other.rebuild;
        self.result = other.result;
    }
}
