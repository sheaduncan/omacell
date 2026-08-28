//! Handler context and typed effect records.

use omacell_core::changeset::{ChangeSummary, CommandCall};
use omacell_core::command::Origin;
use omacell_core::event::Event;
use omacell_core::graph::CellCoord;
use omacell_core::recalc::{RecalcEngine, RecalcResult};
use omacell_core::workbook::Workbook;

/// Per-invocation borrow of the workbook, engine, and origin.
pub struct CommandContext<'a> {
    workbook: &'a mut Workbook,
    engine: &'a mut RecalcEngine,
    origin: Origin,
}

impl<'a> CommandContext<'a> {
    pub(crate) fn new(
        workbook: &'a mut Workbook,
        engine: &'a mut RecalcEngine,
        origin: Origin,
    ) -> Self {
        Self {
            workbook,
            engine,
            origin,
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

    /// Full recalculation (explicit `calc.recalc`).
    pub fn recalc_full(&mut self) -> RecalcResult {
        self.engine.recalc_full(self.workbook)
    }

    /// Rebuild graph then full recalculation.
    pub fn recalc_rebuild(&mut self) -> RecalcResult {
        self.engine.recalc_rebuild(self.workbook)
    }

    /// Incremental recalculation of the dirty set.
    pub fn recalc_incremental(&mut self) -> RecalcResult {
        self.engine.recalc_incremental(self.workbook)
    }

    /// Trusted origin for this invocation.
    #[must_use]
    pub fn origin(&self) -> Origin {
        self.origin
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
