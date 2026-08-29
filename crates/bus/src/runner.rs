//! Single-writer command task runner (spec §10.2, §11.5).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use omacell_core::changeset::{Changeset, ChangesetId, CommandCall};
use omacell_core::command::{Origin, Outcome};
use omacell_core::error::CoreError;

use crate::args::EmptyArgs;
use crate::error;
use crate::handler::TaskCtl;
use crate::registry::{CommandKind, CommandSpec, Exposure};
use crate::session::{Bus, DryRun};
use crate::task::{
    CancelHandle, LongOps, MAX_SUBMIT_QUEUE, MAX_TASK_EVENTS, ReaderSnapshot, TaskEvent, TaskId,
    TaskProgress, TaskState, TaskStatus,
};

enum WorkerMsg {
    Execute {
        id: TaskId,
        origin: Origin,
        cmd: String,
        args: serde_json::Value,
        cancel: Arc<AtomicBool>,
        reply: Option<SyncSender<Outcome>>,
    },
    Propose {
        origin: Origin,
        forward: Vec<CommandCall>,
        reply: SyncSender<Result<Changeset, CoreError>>,
    },
    Apply {
        origin: Origin,
        id: ChangesetId,
        reply: SyncSender<Result<Changeset, CoreError>>,
    },
    DryRun {
        origin: Origin,
        cmd: String,
        args: serde_json::Value,
        reply: SyncSender<Result<DryRun, CoreError>>,
    },
    Shutdown,
}

struct Shared {
    snapshot: Mutex<Arc<ReaderSnapshot>>,
    tasks: Mutex<BTreeMap<TaskId, TaskState>>,
    cancels: Mutex<BTreeMap<TaskId, Arc<AtomicBool>>>,
    events: Mutex<VecDeque<TaskEvent>>,
    dropped: AtomicU64,
    running: Mutex<Option<TaskId>>,
    command_ids: BTreeSet<String>,
    long_ops: LongOps,
    shutdown: AtomicBool,
}

/// Cloneable handle for UI, IPC, and tests.
#[derive(Clone)]
pub struct TaskRunnerHandle {
    shared: Arc<Shared>,
    submit: SyncSender<WorkerMsg>,
    next_id: Arc<AtomicU64>,
}

/// Owns the writer thread. Dropping it shuts the worker down.
pub struct TaskRunner {
    handle: TaskRunnerHandle,
    worker: Option<JoinHandle<()>>,
}

impl TaskRunner {
    /// Spawn a worker that exclusively owns `bus`.
    pub fn spawn(bus: Bus, long_ops: LongOps) -> Result<Self, CoreError> {
        let snapshot = Arc::new(ReaderSnapshot {
            workbook: bus.workbook().clone(),
            spill: bus.engine().spill().clone(),
        });
        let command_ids = bus
            .registry()
            .iter()
            .map(|(id, _)| id.to_string())
            .collect();
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(snapshot),
            tasks: Mutex::new(BTreeMap::new()),
            cancels: Mutex::new(BTreeMap::new()),
            events: Mutex::new(VecDeque::new()),
            dropped: AtomicU64::new(0),
            running: Mutex::new(None),
            command_ids,
            long_ops,
            shutdown: AtomicBool::new(false),
        });
        let (submit, rx) = mpsc::sync_channel(MAX_SUBMIT_QUEUE);
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("omacell-cmd-worker".into())
            .spawn(move || worker_loop(bus, rx, worker_shared))
            .map_err(|err| CoreError::new("task.spawn", format!("spawn command worker: {err}")))?;
        Ok(Self {
            handle: TaskRunnerHandle {
                shared,
                submit,
                next_id: Arc::new(AtomicU64::new(1)),
            },
            worker: Some(worker),
        })
    }

    /// Shareable handle.
    #[must_use]
    pub fn handle(&self) -> TaskRunnerHandle {
        self.handle.clone()
    }
}

impl Drop for TaskRunner {
    fn drop(&mut self) {
        self.handle.shutdown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl TaskRunnerHandle {
    fn alloc(&self) -> TaskId {
        TaskId::new(self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Known registry ids at spawn (keymap reload).
    #[must_use]
    pub fn command_ids(&self) -> &BTreeSet<String> {
        &self.shared.command_ids
    }

    /// Long-operation classifier used by this runner.
    #[must_use]
    pub fn long_ops(&self) -> &LongOps {
        &self.shared.long_ops
    }

    /// Latest committed reader snapshot (`Arc` clone is O(1)).
    #[must_use]
    pub fn snapshot(&self) -> Arc<ReaderSnapshot> {
        self.shared
            .snapshot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Running task, if any.
    #[must_use]
    pub fn running(&self) -> Option<TaskState> {
        let id = *self
            .shared
            .running
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let id = id?;
        self.shared
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
            .cloned()
    }

    /// Whether the writer is occupied.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.running().is_some()
    }

    /// Cancel handle for the running task, if it is still cancellable.
    #[must_use]
    pub fn running_cancel(&self) -> Option<CancelHandle> {
        let state = self.running()?;
        if matches!(
            state.status,
            TaskStatus::Running | TaskStatus::Queued | TaskStatus::Cancelling
        ) {
            // Flag lives in the task map via a side table.
            self.cancel_flag(state.id)
                .map(|flag| CancelHandle::new(state.id, flag))
        } else {
            None
        }
    }

    fn cancel_flag(&self, id: TaskId) -> Option<Arc<AtomicBool>> {
        self.cancels()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
            .cloned()
    }

    fn cancels(&self) -> &Mutex<BTreeMap<TaskId, Arc<AtomicBool>>> {
        // stored on Shared — add field
        &self.shared.cancels
    }

    /// Events dropped because the UI did not drain.
    #[must_use]
    pub fn dropped_events(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    /// Drain pending task events (non-blocking).
    pub fn drain_events(&self) -> Vec<TaskEvent> {
        let mut q = self.shared.events.lock().unwrap_or_else(|p| p.into_inner());
        q.drain(..).collect()
    }

    /// Queue a command. Does not wait. Long TUI work uses this.
    pub fn submit(
        &self,
        origin: Origin,
        cmd: &str,
        args: serde_json::Value,
    ) -> Result<(TaskId, CancelHandle), CoreError> {
        let id = self.alloc();
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = CancelHandle::new(id, Arc::clone(&cancel));
        let state = TaskState {
            id,
            command: cmd.to_string(),
            status: TaskStatus::Queued,
            progress: None,
        };
        self.shared
            .cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, Arc::clone(&cancel));
        self.shared
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, state.clone());
        self.push_event(TaskEvent::Queued(state));
        let msg = WorkerMsg::Execute {
            id,
            origin,
            cmd: cmd.to_string(),
            args,
            cancel,
            reply: None,
        };
        match self.submit.try_send(msg) {
            Ok(()) => Ok((id, handle)),
            Err(TrySendError::Full(_)) => {
                self.fail_submit(id, "task.queue", "command queue is full")?;
                Err(error::task_queue())
            }
            Err(TrySendError::Disconnected(_)) => {
                self.fail_submit(id, "task.shutdown", "command worker stopped")?;
                Err(error::task_shutdown())
            }
        }
    }

    fn fail_submit(&self, id: TaskId, code: &str, message: &str) -> Result<(), CoreError> {
        let mut tasks = self.shared.tasks.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(state) = tasks.get_mut(&id) {
            state.status = TaskStatus::Failed;
            self.push_event(TaskEvent::Failed {
                state: state.clone(),
                code: code.into(),
                message: message.into(),
            });
        }
        Ok(())
    }

    /// Queue and wait for the outcome (short commands and IPC).
    pub fn submit_wait(&self, origin: Origin, cmd: &str, args: serde_json::Value) -> Outcome {
        let id = self.alloc();
        let cancel = Arc::new(AtomicBool::new(false));
        let state = TaskState {
            id,
            command: cmd.to_string(),
            status: TaskStatus::Queued,
            progress: None,
        };
        self.shared
            .cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, Arc::clone(&cancel));
        self.shared
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, state.clone());
        self.push_event(TaskEvent::Queued(state));
        let (tx, rx) = mpsc::sync_channel(1);
        let msg = WorkerMsg::Execute {
            id,
            origin,
            cmd: cmd.to_string(),
            args,
            cancel,
            reply: Some(tx),
        };
        if self.submit.send(msg).is_err() {
            return Outcome::failure(error::task_shutdown());
        }
        rx.recv()
            .unwrap_or_else(|_| Outcome::failure(error::task_shutdown()))
    }

    /// Propose through the writer.
    pub fn propose(
        &self,
        origin: Origin,
        forward: Vec<CommandCall>,
    ) -> Result<Changeset, CoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit
            .send(WorkerMsg::Propose {
                origin,
                forward,
                reply: tx,
            })
            .map_err(|_| error::task_shutdown())?;
        rx.recv().map_err(|_| error::task_shutdown())?
    }

    /// Apply a changeset through the writer.
    pub fn apply(&self, origin: Origin, id: &ChangesetId) -> Result<Changeset, CoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit
            .send(WorkerMsg::Apply {
                origin,
                id: id.clone(),
                reply: tx,
            })
            .map_err(|_| error::task_shutdown())?;
        rx.recv().map_err(|_| error::task_shutdown())?
    }

    /// Dry-run through the writer.
    pub fn dry_run(
        &self,
        origin: Origin,
        cmd: &str,
        args: serde_json::Value,
    ) -> Result<DryRun, CoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit
            .send(WorkerMsg::DryRun {
                origin,
                cmd: cmd.to_string(),
                args,
                reply: tx,
            })
            .map_err(|_| error::task_shutdown())?;
        rx.recv().map_err(|_| error::task_shutdown())?
    }

    /// Request worker shutdown. In-flight work finishes or cancels atomically.
    pub fn shutdown(&self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.running_cancel() {
            handle.cancel();
        }
        let _ = self.submit.try_send(WorkerMsg::Shutdown);
    }

    fn push_event(&self, event: TaskEvent) {
        push_event(&self.shared, event);
    }
}

fn push_event(shared: &Shared, event: TaskEvent) {
    let mut q = shared.events.lock().unwrap_or_else(|p| p.into_inner());
    if q.len() >= MAX_TASK_EVENTS {
        q.pop_front();
        shared.dropped.fetch_add(1, Ordering::Relaxed);
    }
    q.push_back(event);
}

fn worker_loop(mut bus: Bus, rx: Receiver<WorkerMsg>, shared: Arc<Shared>) {
    while let Ok(msg) = rx.recv() {
        if shared.shutdown.load(Ordering::SeqCst) && matches!(msg, WorkerMsg::Shutdown) {
            break;
        }
        match msg {
            WorkerMsg::Shutdown => break,
            WorkerMsg::Execute {
                id,
                origin,
                cmd,
                args,
                cancel,
                reply,
            } => {
                {
                    let mut tasks = shared.tasks.lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(state) = tasks.get_mut(&id) {
                        state.status = if cancel.load(Ordering::SeqCst) {
                            TaskStatus::Cancelling
                        } else {
                            TaskStatus::Running
                        };
                        push_event(&shared, TaskEvent::Running(state.clone()));
                    }
                    *shared.running.lock().unwrap_or_else(|p| p.into_inner()) = Some(id);
                }
                let progress_shared = Arc::clone(&shared);
                let progress_id = id;
                let ctl = TaskCtl {
                    cancel: Some(Arc::clone(&cancel)),
                    progress: Some(Arc::new(move |done, total, label: &str| {
                        coalesce_progress(&progress_shared, progress_id, done, total, label);
                    })),
                };
                let outcome = bus.execute_with_task(origin, &cmd, args, ctl);
                publish_snapshot(&shared, &bus);
                let failed = !outcome.ok;
                {
                    let mut tasks = shared.tasks.lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(state) = tasks.get_mut(&id) {
                        if failed {
                            state.status = TaskStatus::Failed;
                            let (code, message) = outcome
                                .error
                                .as_ref()
                                .map(|e| (e.code.clone(), e.message.clone()))
                                .unwrap_or_else(|| ("task.failed".into(), "command failed".into()));
                            push_event(
                                &shared,
                                TaskEvent::Failed {
                                    state: state.clone(),
                                    code,
                                    message,
                                },
                            );
                        } else {
                            state.status = TaskStatus::Completed;
                            push_event(&shared, TaskEvent::Completed(state.clone()));
                        }
                    }
                    *shared.running.lock().unwrap_or_else(|p| p.into_inner()) = None;
                }
                if let Some(reply) = reply {
                    let _ = reply.send(outcome);
                }
            }
            WorkerMsg::Propose {
                origin,
                forward,
                reply,
            } => {
                let result = bus.propose(origin, forward);
                if result.is_ok() {
                    publish_snapshot(&shared, &bus);
                }
                let _ = reply.send(result);
            }
            WorkerMsg::Apply { origin, id, reply } => {
                let result = bus.apply(origin, &id);
                if result.is_ok() {
                    publish_snapshot(&shared, &bus);
                }
                let _ = reply.send(result);
            }
            WorkerMsg::DryRun {
                origin,
                cmd,
                args,
                reply,
            } => {
                let _ = reply.send(bus.dry_run(origin, &cmd, args));
            }
        }
        if shared.shutdown.load(Ordering::SeqCst) {
            break;
        }
    }
}

fn publish_snapshot(shared: &Shared, bus: &Bus) {
    let next = Arc::new(ReaderSnapshot {
        workbook: bus.workbook().clone(),
        spill: bus.engine().spill().clone(),
    });
    *shared.snapshot.lock().unwrap_or_else(|p| p.into_inner()) = next;
}

fn coalesce_progress(shared: &Shared, id: TaskId, done: u64, total: Option<u64>, label: &str) {
    let mut tasks = shared.tasks.lock().unwrap_or_else(|p| p.into_inner());
    let Some(state) = tasks.get_mut(&id) else {
        return;
    };
    state.progress = Some(TaskProgress {
        done,
        total,
        label: label.to_string(),
    });
    let snapshot = state.clone();
    drop(tasks);
    let mut q = shared.events.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(TaskEvent::Progress(existing)) = q.back_mut()
        && existing.id == id
    {
        *existing = snapshot;
        return;
    }
    if q.len() >= MAX_TASK_EVENTS {
        q.pop_front();
        shared.dropped.fetch_add(1, Ordering::Relaxed);
    }
    q.push_back(TaskEvent::Progress(snapshot));
}

/// Test helper: register `test.hold` which waits on `start` then spins until
/// `release` or cancel. Synchronization uses a barrier and atomics, not sleeps.
pub fn register_hold_command(
    registry: &mut crate::registry::CommandRegistry,
    start: Arc<std::sync::Barrier>,
    release: Arc<AtomicBool>,
) -> Result<(), CoreError> {
    registry.register::<EmptyArgs, _>(
        CommandSpec {
            id: "test.hold",
            doc: "Test-only barrier hold (WP-15a)",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, _args| {
            if ctx.is_preflight() {
                return Ok(crate::handler::Effect::query(serde_json::json!({})));
            }
            start.wait();
            loop {
                if ctx.is_cancelled() {
                    return Err(error::task_cancelled());
                }
                if release.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::yield_now();
            }
            Ok(crate::handler::Effect {
                result: serde_json::json!({"held": true}),
                ..crate::handler::Effect::default()
            })
        },
    )
}
