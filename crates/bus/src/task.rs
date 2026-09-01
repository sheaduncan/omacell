//! Additive UI task-runner types (not frozen `Event` / IPC).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use omacell_core::addr::SheetId;
use omacell_core::condfmt::{CfOverlay, ResolvedCfOverlay};
use omacell_core::error::CoreError;
use omacell_core::spill::SpillTable;
use omacell_core::workbook::Workbook;

/// Stable id of a queued or running command task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(u64);

impl TaskId {
    /// Numeric id.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn new(id: u64) -> Self {
        Self(id)
    }
}

/// Lifecycle of one runner task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    /// Accepted, not yet the writer.
    Queued,
    /// Worker is executing the command.
    Running,
    /// Cancel requested; worker has not yet unwound.
    Cancelling,
    /// Finished successfully.
    Completed,
    /// Finished with an error (including cooperative cancel).
    Failed,
}

/// Bounded progress for the status line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskProgress {
    /// Units completed.
    pub done: u64,
    /// Optional total.
    pub total: Option<u64>,
    /// Short label (`recalc`, `import`, `save`).
    pub label: String,
}

/// Public snapshot of one task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskState {
    /// Id.
    pub id: TaskId,
    /// Registry command id.
    pub command: String,
    /// Lifecycle.
    pub status: TaskStatus,
    /// Latest coalesced progress.
    pub progress: Option<TaskProgress>,
}

/// Cooperative cancellation handle for a task.
#[derive(Clone)]
pub struct CancelHandle {
    id: TaskId,
    flag: Arc<AtomicBool>,
    on_cancel: Arc<dyn Fn(TaskId) + Send + Sync>,
}

impl CancelHandle {
    pub(crate) fn new(
        id: TaskId,
        flag: Arc<AtomicBool>,
        on_cancel: Arc<dyn Fn(TaskId) + Send + Sync>,
    ) -> Self {
        Self {
            id,
            flag,
            on_cancel,
        }
    }

    /// Task this handle cancels.
    #[must_use]
    pub fn id(&self) -> TaskId {
        self.id
    }

    /// Request cooperative cancel. The writer finishes the current transaction
    /// atomically (commit or restore), then marks the task failed.
    pub fn cancel(&self) {
        if !self.flag.swap(true, Ordering::SeqCst) {
            (self.on_cancel)(self.id);
        }
    }

    /// Whether cancel has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

impl std::fmt::Debug for CancelHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelHandle")
            .field("id", &self.id)
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

/// Copy-on-write reader view published after each writer commit.
#[derive(Clone, Debug)]
pub struct ReaderSnapshot {
    /// Workbook as of the last committed command.
    pub workbook: Workbook,
    /// Spill table as of that commit.
    pub spill: SpillTable,
}

/// Worker-resolved conditional-format overlays for one reader snapshot.
#[derive(Clone, Debug)]
pub struct ConditionalFormatSnapshot {
    pub(crate) reader: Arc<ReaderSnapshot>,
    pub(crate) sheet: SheetId,
    pub(crate) overlays: Vec<ResolvedCfOverlay>,
    pub(crate) error: Option<CoreError>,
}

impl ConditionalFormatSnapshot {
    /// Effective conditional-format overlay for one cell in the cached viewport.
    #[must_use]
    pub fn get(&self, row: u32, col: u16) -> Option<CfOverlay> {
        self.overlays
            .iter()
            .find_map(|overlay| overlay.get(row, col))
    }

    /// Number of rectangular viewport caches retained by this snapshot.
    #[must_use]
    pub fn range_count(&self) -> usize {
        self.overlays.len()
    }

    /// Resolution failure, if the worker could not build this viewport cache.
    #[must_use]
    pub fn error(&self) -> Option<&CoreError> {
        self.error.as_ref()
    }
}

/// Composition-layer set of long-running commands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LongOps {
    ids: BTreeSet<String>,
}

impl LongOps {
    /// Production defaults: recalc and file I/O.
    #[must_use]
    pub fn production() -> Self {
        Self {
            ids: [
                "calc.recalc",
                "file.open",
                "file.save",
                "file.saveas",
                "file.export",
                "file.print",
                "chart.export",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    /// Add a command id (tests register `test.hold`).
    #[must_use]
    pub fn with(mut self, id: impl Into<String>) -> Self {
        self.ids.insert(id.into());
        self
    }

    /// Whether `id` is classified as a long operation.
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }
}

/// Events the UI drains without blocking the worker.
#[derive(Clone, Debug)]
pub enum TaskEvent {
    /// Task accepted onto the writer queue.
    Queued(TaskState),
    /// Worker started the command.
    Running(TaskState),
    /// Cooperative cancellation was requested.
    Cancelling(TaskState),
    /// Coalesced progress for the running task.
    Progress(TaskState),
    /// Terminal success.
    Completed {
        /// Terminal task snapshot.
        state: TaskState,
        /// Command result used by frontends to reconcile dirty state.
        outcome: omacell_core::command::Outcome,
    },
    /// Terminal failure or cancel.
    Failed {
        /// Task snapshot.
        state: TaskState,
        /// Machine error code.
        code: String,
        /// Human message.
        message: String,
    },
}

pub(crate) const MAX_SUBMIT_QUEUE: usize = 32;
pub(crate) const MAX_TASK_EVENTS: usize = 64;
