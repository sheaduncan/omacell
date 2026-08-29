//! Additive UI task-runner types (not frozen `Event` / IPC).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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
#[derive(Clone, Debug)]
pub struct CancelHandle {
    id: TaskId,
    flag: Arc<AtomicBool>,
}

impl CancelHandle {
    pub(crate) fn new(id: TaskId, flag: Arc<AtomicBool>) -> Self {
        Self { id, flag }
    }

    /// Task this handle cancels.
    #[must_use]
    pub fn id(&self) -> TaskId {
        self.id
    }

    /// Request cooperative cancel. The writer finishes the current transaction
    /// atomically (commit or restore), then marks the task failed.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Whether cancel has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
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
            ids: ["calc.recalc", "file.open", "file.save", "file.export"]
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
    /// Coalesced progress for the running task.
    Progress(TaskState),
    /// Terminal success.
    Completed(TaskState),
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
