//! Single-writer command task runner (spec §10.2, §11.5).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, Weak};
use std::thread::{self, JoinHandle};

use omacell_core::addr::{RangeRef, SheetId};
use omacell_core::changeset::{Changeset, ChangesetId, CommandCall};
use omacell_core::command::{Origin, Outcome};
use omacell_core::condfmt::{MAX_CF_OVERLAY_CELLS, resolve_overlay_with_registry};
use omacell_core::error::CoreError;
use omacell_core::eval::{DynamicFn, FnRegistry};
use omacell_core::event::Event;

#[cfg(feature = "test-util")]
use crate::args::EmptyArgs;
use crate::error;
use crate::event::{EventBus, SubscriberId};
use crate::handler::TaskCtl;
use crate::preview::ChangePreview;
#[cfg(feature = "test-util")]
use crate::registry::CommandSpec;
use crate::registry::{CommandKind, Exposure};
use crate::session::{Bus, DryRun};
use crate::task::{
    CancelHandle, ConditionalFormatSnapshot, LongOps, MAX_SUBMIT_QUEUE, MAX_TASK_EVENTS,
    ReaderSnapshot, TaskEvent, TaskId, TaskProgress, TaskState, TaskStatus,
};

const MAX_CF_VIEWPORT_RANGES: usize = 4;

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
    ReviseProposal {
        origin: Origin,
        id: ChangesetId,
        forward: Vec<CommandCall>,
        reply: SyncSender<Result<Changeset, CoreError>>,
    },
    DiscardProposal {
        origin: Origin,
        id: ChangesetId,
        reply: SyncSender<Result<Changeset, CoreError>>,
    },
    PreviewChangeset {
        id: ChangesetId,
        reply: SyncSender<Result<ChangePreview, CoreError>>,
    },
    Apply {
        origin: Origin,
        id: ChangesetId,
        reply: SyncSender<Result<Changeset, CoreError>>,
    },
    Revert {
        origin: Origin,
        id: ChangesetId,
        reply: SyncSender<Result<Changeset, CoreError>>,
    },
    ListChangesets {
        reply: SyncSender<Result<Vec<Changeset>, CoreError>>,
    },
    GetChangeset {
        id: ChangesetId,
        reply: SyncSender<Result<Changeset, CoreError>>,
    },
    DryRun {
        origin: Origin,
        cmd: String,
        args: serde_json::Value,
        reply: SyncSender<Result<DryRun, CoreError>>,
    },
    RegisterFunction {
        def: DynamicFn,
        reply: SyncSender<Result<(), CoreError>>,
    },
    RefreshFunctions {
        reply: SyncSender<Result<(), CoreError>>,
    },
    ReplaceFunctions {
        previous: BTreeSet<String>,
        current: Vec<DynamicFn>,
        reply: SyncSender<Result<(), CoreError>>,
    },
    Shutdown,
}

enum CfWorkerMsg {
    Resolve,
    Shutdown,
}

#[derive(Clone)]
struct CfRequest {
    reader: Arc<ReaderSnapshot>,
    registry: Arc<FnRegistry>,
    sheet: SheetId,
    ranges: Vec<RangeRef>,
}

#[derive(Default)]
struct CfRequestState {
    pending: Option<CfRequest>,
    last: Option<CfRequest>,
    wake_queued: bool,
}

struct PublishedReader {
    reader: Arc<ReaderSnapshot>,
    registry: Arc<FnRegistry>,
}

struct Shared {
    snapshot: Mutex<PublishedReader>,
    conditional_formats: Mutex<Option<Arc<ConditionalFormatSnapshot>>>,
    cf_requests: Mutex<CfRequestState>,
    tasks: Mutex<BTreeMap<TaskId, TaskState>>,
    cancels: Mutex<BTreeMap<TaskId, Arc<AtomicBool>>>,
    events: Mutex<VecDeque<TaskEvent>>,
    bus_events: Mutex<VecDeque<Event>>,
    ipc_events: Mutex<EventBus>,
    event_waker: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    dropped: AtomicU64,
    task_slots: AtomicUsize,
    running: Mutex<Option<TaskId>>,
    writer_busy: AtomicBool,
    command_ids: BTreeSet<String>,
    command_policy: BTreeMap<String, (CommandKind, bool, Exposure)>,
    long_ops: LongOps,
    shutdown: AtomicBool,
}

/// Cloneable handle for UI, IPC, and tests.
#[derive(Clone)]
pub struct TaskRunnerHandle {
    shared: Arc<Shared>,
    submit: SyncSender<WorkerMsg>,
    cf_submit: Sender<CfWorkerMsg>,
    next_id: Arc<AtomicU64>,
}

/// Owns the writer thread. Dropping it shuts the worker down.
pub struct TaskRunner {
    handle: TaskRunnerHandle,
    worker: Option<JoinHandle<()>>,
    cf_worker: Option<JoinHandle<()>>,
}

impl TaskRunner {
    /// Spawn a worker that exclusively owns `bus`.
    pub fn spawn(mut bus: Bus, long_ops: LongOps) -> Result<Self, CoreError> {
        let reader = Arc::new(ReaderSnapshot {
            workbook: bus.workbook().clone(),
            spill: bus.engine().spill().clone(),
        });
        let snapshot = PublishedReader {
            reader,
            registry: Arc::new(bus.engine().registry().clone()),
        };
        let command_ids = bus
            .registry()
            .iter()
            .map(|(id, _)| id.to_string())
            .collect();
        let command_policy = bus
            .registry()
            .iter()
            .map(|(id, command)| {
                (
                    id.to_string(),
                    (command.kind, command.changeset_eligible, command.exposure),
                )
            })
            .collect();
        let bus_event_subscriber = bus.subscribe(crate::changeset::MAX_EFFECT_RECORDS + 1);
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(snapshot),
            conditional_formats: Mutex::new(None),
            cf_requests: Mutex::new(CfRequestState::default()),
            tasks: Mutex::new(BTreeMap::new()),
            cancels: Mutex::new(BTreeMap::new()),
            events: Mutex::new(VecDeque::new()),
            bus_events: Mutex::new(VecDeque::new()),
            ipc_events: Mutex::new(EventBus::new()),
            event_waker: Mutex::new(None),
            dropped: AtomicU64::new(0),
            task_slots: AtomicUsize::new(0),
            running: Mutex::new(None),
            writer_busy: AtomicBool::new(false),
            command_ids,
            command_policy,
            long_ops,
            shutdown: AtomicBool::new(false),
        });
        let (submit, rx) = mpsc::sync_channel(MAX_SUBMIT_QUEUE);
        let (cf_submit, cf_rx) = mpsc::channel();
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("omacell-cmd-worker".into())
            .spawn(move || worker_loop(bus, bus_event_subscriber, rx, worker_shared))
            .map_err(|err| CoreError::new("task.spawn", format!("spawn command worker: {err}")))?;
        let cf_shared = Arc::clone(&shared);
        let cf_worker = match thread::Builder::new()
            .name("omacell-cf-worker".into())
            .spawn(move || cf_worker_loop(cf_rx, cf_shared))
        {
            Ok(worker) => worker,
            Err(err) => {
                shared.shutdown.store(true, Ordering::SeqCst);
                let _ = submit.send(WorkerMsg::Shutdown);
                let _ = worker.join();
                return Err(CoreError::new(
                    "task.spawn",
                    format!("spawn conditional-format worker: {err}"),
                ));
            }
        };
        Ok(Self {
            handle: TaskRunnerHandle {
                shared,
                submit,
                cf_submit,
                next_id: Arc::new(AtomicU64::new(1)),
            },
            worker: Some(worker),
            cf_worker: Some(cf_worker),
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
        if let Some(worker) = self.cf_worker.take() {
            let _ = worker.join();
        }
    }
}

impl TaskRunnerHandle {
    fn alloc(&self) -> TaskId {
        TaskId::new(self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    fn reserve_task(&self) -> Result<(), CoreError> {
        self.shared
            .task_slots
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                (current < MAX_SUBMIT_QUEUE + 1).then_some(current + 1)
            })
            .map(|_| ())
            .map_err(|_| error::task_queue())
    }

    /// Known registry ids at spawn (keymap reload).
    #[must_use]
    pub fn command_ids(&self) -> &BTreeSet<String> {
        &self.shared.command_ids
    }

    pub(crate) fn ipc_command_policy(&self, id: &str) -> Result<(CommandKind, bool), CoreError> {
        let Some((kind, eligible, exposure)) = self.shared.command_policy.get(id).copied() else {
            return Err(error::unknown(id));
        };
        if exposure == Exposure::Internal {
            return Err(error::internal(id));
        }
        Ok((kind, eligible))
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
            .reader
            .clone()
    }

    /// Queue resolution of conditional formats for up to four visible-pane ranges.
    ///
    /// Requests are coalesced and evaluated on a dedicated reader worker. The
    /// result is published only if `snapshot` is still the current committed
    /// reader view.
    pub fn request_conditional_formats(
        &self,
        snapshot: &Arc<ReaderSnapshot>,
        sheet: SheetId,
        ranges: &[RangeRef],
    ) -> Result<(), CoreError> {
        if self.shared.shutdown.load(Ordering::SeqCst) {
            return Err(error::task_shutdown());
        }
        validate_cf_ranges(ranges)?;
        let published = self
            .shared
            .snapshot
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if !Arc::ptr_eq(snapshot, &published.reader) {
            return Ok(());
        }
        let Some(sheet_ref) = snapshot.workbook.sheet(sheet) else {
            return Err(CoreError::sheet_id(format!(
                "unknown sheet {}",
                sheet.index()
            )));
        };
        let request = CfRequest {
            reader: Arc::clone(snapshot),
            registry: Arc::clone(&published.registry),
            sheet,
            ranges: ranges.to_vec(),
        };
        if sheet_ref.cond_formats.is_empty() || ranges.is_empty() {
            if self.conditional_formats(snapshot, sheet).is_some() {
                return Ok(());
            }
            *self
                .shared
                .conditional_formats
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(ConditionalFormatSnapshot {
                reader: Arc::clone(snapshot),
                sheet,
                overlays: Vec::new(),
                error: None,
            }));
            return Ok(());
        }
        drop(published);

        let should_wake = {
            let mut state = self
                .shared
                .cf_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if state
                .last
                .as_ref()
                .is_some_and(|last| same_cf_request(last, &request))
            {
                return Ok(());
            }
            state.pending = Some(request.clone());
            state.last = Some(request);
            if state.wake_queued {
                false
            } else {
                state.wake_queued = true;
                true
            }
        };
        if should_wake && self.cf_submit.send(CfWorkerMsg::Resolve).is_err() {
            let mut state = self
                .shared
                .cf_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            state.pending = None;
            state.last = None;
            state.wake_queued = false;
            return Err(error::task_shutdown());
        }
        Ok(())
    }

    /// Latest worker-resolved conditional formats for `snapshot` and `sheet`.
    #[must_use]
    pub fn conditional_formats(
        &self,
        snapshot: &Arc<ReaderSnapshot>,
        sheet: SheetId,
    ) -> Option<Arc<ConditionalFormatSnapshot>> {
        let resolved = self
            .shared
            .conditional_formats
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()?;
        (resolved.sheet == sheet && Arc::ptr_eq(&resolved.reader, snapshot)).then_some(resolved)
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
        self.shared.writer_busy.load(Ordering::SeqCst) || self.tracked_tasks() != 0
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
                .map(|flag| self.cancel_handle(state.id, flag))
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
        &self.shared.cancels
    }

    fn cancel_handle(&self, id: TaskId, flag: Arc<AtomicBool>) -> CancelHandle {
        let shared = Arc::downgrade(&self.shared);
        CancelHandle::new(
            id,
            flag,
            Arc::new(move |task| {
                if let Some(shared) = Weak::upgrade(&shared) {
                    mark_cancelling(&shared, task);
                }
            }),
        )
    }

    /// Number of queued/running task records retained by the runner.
    #[must_use]
    pub fn tracked_tasks(&self) -> usize {
        self.shared
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
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

    /// Drain committed command-bus events for retained scripting hosts.
    pub fn drain_bus_events(&self) -> Vec<Event> {
        let mut q = self
            .shared
            .bus_events
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        q.drain(..).collect()
    }

    pub(crate) fn subscribe_ipc(
        &self,
        cap: usize,
        byte_cap: usize,
        filter: &[String],
    ) -> SubscriberId {
        self.shared
            .ipc_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .subscribe_filtered(cap, byte_cap, filter)
    }

    pub(crate) fn unsubscribe_ipc(&self, id: SubscriberId) {
        self.shared
            .ipc_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .unsubscribe(id);
    }

    pub(crate) fn drain_ipc_events(&self, id: SubscriberId) -> (u64, Vec<Event>) {
        let mut events = self
            .shared
            .ipc_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (events.dropped(id), events.drain(id))
    }

    /// Wake a frontend whenever a task event is queued.
    ///
    /// This lets GUI event loops sleep while idle and still repaint for IPC or
    /// worker-thread activity.
    pub fn set_event_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self
            .shared
            .event_waker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(wake));
    }

    /// Wake the registered frontend after out-of-band composition work.
    ///
    /// AI settlement uses this when a background provider request fails before
    /// it can enqueue the ordinary second-wave recalculation task.
    pub fn wake_frontend(&self) {
        wake_event_consumer(&self.shared);
    }

    /// Queue a command. Does not wait. Long TUI work uses this.
    pub fn submit(
        &self,
        origin: Origin,
        cmd: &str,
        args: serde_json::Value,
    ) -> Result<(TaskId, CancelHandle), CoreError> {
        if self.shared.shutdown.load(Ordering::SeqCst) {
            return Err(error::task_shutdown());
        }
        self.reserve_task()?;
        let id = self.alloc();
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = self.cancel_handle(id, Arc::clone(&cancel));
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
                self.fail_submit(id, "task.queue", "command queue is full");
                Err(error::task_queue())
            }
            Err(TrySendError::Disconnected(_)) => {
                self.fail_submit(id, "task.shutdown", "command worker stopped");
                Err(error::task_shutdown())
            }
        }
    }

    fn fail_submit(&self, id: TaskId, code: &str, message: &str) {
        let mut tasks = self.shared.tasks.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(mut state) = tasks.remove(&id) {
            state.status = TaskStatus::Failed;
            drop(tasks);
            self.shared
                .cancels
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&id);
            self.push_event(TaskEvent::Failed {
                state,
                code: code.into(),
                message: message.into(),
            });
            self.shared.task_slots.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Queue and wait for the outcome (short commands and IPC).
    pub fn submit_wait(&self, origin: Origin, cmd: &str, args: serde_json::Value) -> Outcome {
        if self.shared.shutdown.load(Ordering::SeqCst) {
            return Outcome::failure(error::task_shutdown());
        }
        if let Err(err) = self.reserve_task() {
            return Outcome::failure(err);
        }
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
        match self.submit.try_send(msg) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.fail_submit(id, "task.queue", "command queue is full");
                return Outcome::failure(error::task_queue());
            }
            Err(TrySendError::Disconnected(_)) => {
                self.fail_submit(id, "task.shutdown", "command worker stopped");
                return Outcome::failure(error::task_shutdown());
            }
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

    /// Replace the accepted command subset of a proposal through the writer.
    pub fn revise_proposal(
        &self,
        origin: Origin,
        id: &ChangesetId,
        forward: Vec<CommandCall>,
    ) -> Result<Changeset, CoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit
            .send(WorkerMsg::ReviseProposal {
                origin,
                id: id.clone(),
                forward,
                reply: tx,
            })
            .map_err(|_| error::task_shutdown())?;
        rx.recv().map_err(|_| error::task_shutdown())?
    }

    /// Reject and remove a proposal through the writer.
    pub fn discard_proposal(
        &self,
        origin: Origin,
        id: &ChangesetId,
    ) -> Result<Changeset, CoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit
            .send(WorkerMsg::DiscardProposal {
                origin,
                id: id.clone(),
                reply: tx,
            })
            .map_err(|_| error::task_shutdown())?;
        rx.recv().map_err(|_| error::task_shutdown())?
    }

    /// Build command-local before/after data for a proposal through the writer.
    pub fn preview_changeset(&self, id: &ChangesetId) -> Result<ChangePreview, CoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit
            .send(WorkerMsg::PreviewChangeset {
                id: id.clone(),
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

    /// Revert a changeset through the writer.
    pub fn revert(&self, origin: Origin, id: &ChangesetId) -> Result<Changeset, CoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit
            .send(WorkerMsg::Revert {
                origin,
                id: id.clone(),
                reply: tx,
            })
            .map_err(|_| error::task_shutdown())?;
        rx.recv().map_err(|_| error::task_shutdown())?
    }

    /// List changesets through the writer.
    pub fn list_changesets(&self) -> Result<Vec<Changeset>, CoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit
            .send(WorkerMsg::ListChangesets { reply: tx })
            .map_err(|_| error::task_shutdown())?;
        rx.recv().map_err(|_| error::task_shutdown())?
    }

    /// Fetch a changeset through the writer.
    pub fn get_changeset(&self, id: &ChangesetId) -> Result<Changeset, CoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit
            .send(WorkerMsg::GetChangeset {
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

    /// Register a Lua worksheet function on the writer-owned calculation engine.
    pub fn register_function(&self, def: DynamicFn) -> Result<(), CoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit
            .send(WorkerMsg::RegisterFunction { def, reply: tx })
            .map_err(|_| error::task_shutdown())?;
        rx.recv().map_err(|_| error::task_shutdown())?
    }

    /// Rebuild and recalculate after a retained host registers worksheet functions.
    pub fn refresh_functions(&self) -> Result<(), CoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit
            .send(WorkerMsg::RefreshFunctions { reply: tx })
            .map_err(|_| error::task_shutdown())?;
        rx.recv().map_err(|_| error::task_shutdown())?
    }

    /// Replace one retained scripting host's worksheet functions and recalculate.
    pub fn replace_functions(
        &self,
        previous: BTreeSet<String>,
        current: Vec<DynamicFn>,
    ) -> Result<(), CoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit
            .send(WorkerMsg::ReplaceFunctions {
                previous,
                current,
                reply: tx,
            })
            .map_err(|_| error::task_shutdown())?;
        rx.recv().map_err(|_| error::task_shutdown())?
    }

    /// Request worker shutdown. In-flight work finishes or cancels atomically.
    pub fn shutdown(&self) {
        if self.shared.shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        let pending = self
            .shared
            .cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(id, flag)| (*id, Arc::clone(flag)))
            .collect::<Vec<_>>();
        for (id, flag) in pending {
            self.cancel_handle(id, flag).cancel();
        }
        let _ = self.submit.try_send(WorkerMsg::Shutdown);
        let _ = self.cf_submit.send(CfWorkerMsg::Shutdown);
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
    drop(q);
    wake_event_consumer(shared);
}

fn wake_event_consumer(shared: &Shared) {
    let wake = shared
        .event_waker
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(wake) = wake {
        wake();
    }
}

fn worker_loop(
    mut bus: Bus,
    bus_event_subscriber: crate::event::SubscriberId,
    rx: Receiver<WorkerMsg>,
    shared: Arc<Shared>,
) {
    while let Ok(msg) = rx.recv() {
        shared.writer_busy.store(true, Ordering::SeqCst);
        match msg {
            WorkerMsg::Shutdown => {
                shared.writer_busy.store(false, Ordering::SeqCst);
                fail_pending(&rx, &shared);
                break;
            }
            WorkerMsg::Execute {
                id,
                origin,
                cmd,
                args,
                cancel,
                reply,
            } => {
                if cancel.load(Ordering::SeqCst) {
                    mark_cancelling(&shared, id);
                    let outcome = Outcome::failure(error::task_cancelled());
                    finish_execute(&shared, id, &outcome);
                    if let Some(reply) = reply {
                        let _ = reply.send(outcome);
                    }
                    shared.writer_busy.store(false, Ordering::SeqCst);
                    if shared.shutdown.load(Ordering::SeqCst) {
                        fail_pending(&rx, &shared);
                        break;
                    }
                    continue;
                }
                let running_event = {
                    let mut tasks = shared.tasks.lock().unwrap_or_else(|p| p.into_inner());
                    tasks.get_mut(&id).map(|state| {
                        state.status = TaskStatus::Running;
                        TaskEvent::Running(state.clone())
                    })
                };
                *shared.running.lock().unwrap_or_else(|p| p.into_inner()) = Some(id);
                if let Some(event) = running_event {
                    push_event(&shared, event);
                }
                let progress_shared = Arc::clone(&shared);
                let progress_id = id;
                let ctl = TaskCtl {
                    cancel: Some(Arc::clone(&cancel)),
                    progress: Some(Arc::new(move |done, total, label: &str| {
                        coalesce_progress(&progress_shared, progress_id, done, total, label);
                    })),
                };
                let mutating = bus
                    .registry()
                    .get_str(&cmd)
                    .is_ok_and(|command| command.kind == CommandKind::Mutating);
                let outcome = bus.execute_with_task(origin, &cmd, args, ctl);
                if outcome.ok && mutating {
                    publish_snapshot(&shared, &bus);
                }
                publish_bus_events(&shared, &mut bus, bus_event_subscriber);
                finish_execute(&shared, id, &outcome);
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
                publish_bus_events(&shared, &mut bus, bus_event_subscriber);
                let _ = reply.send(result);
            }
            WorkerMsg::ReviseProposal {
                origin,
                id,
                forward,
                reply,
            } => {
                let result = bus.revise_proposal(origin, &id, forward);
                publish_bus_events(&shared, &mut bus, bus_event_subscriber);
                let _ = reply.send(result);
            }
            WorkerMsg::DiscardProposal { origin, id, reply } => {
                let result = bus.discard_proposal(origin, &id);
                let _ = reply.send(result);
            }
            WorkerMsg::PreviewChangeset { id, reply } => {
                let _ = reply.send(bus.preview_changeset(&id));
            }
            WorkerMsg::Apply { origin, id, reply } => {
                let result = bus.apply(origin, &id);
                if result.is_ok() {
                    publish_snapshot(&shared, &bus);
                }
                publish_bus_events(&shared, &mut bus, bus_event_subscriber);
                let _ = reply.send(result);
            }
            WorkerMsg::Revert { origin, id, reply } => {
                let result = bus.revert(origin, &id);
                if result.is_ok() {
                    publish_snapshot(&shared, &bus);
                }
                publish_bus_events(&shared, &mut bus, bus_event_subscriber);
                let _ = reply.send(result);
            }
            WorkerMsg::ListChangesets { reply } => {
                let _ = reply.send(Ok(bus.list_changesets()));
            }
            WorkerMsg::GetChangeset { id, reply } => {
                let _ = reply.send(bus.get_changeset(&id).cloned());
            }
            WorkerMsg::DryRun {
                origin,
                cmd,
                args,
                reply,
            } => {
                let _ = reply.send(bus.dry_run(origin, &cmd, args));
            }
            WorkerMsg::RegisterFunction { def, reply } => {
                bus.engine_mut().registry_mut().register_dynamic(def);
                let _ = reply.send(Ok(()));
            }
            WorkerMsg::RefreshFunctions { reply } => {
                bus.recalc_after_registry_change();
                publish_snapshot(&shared, &bus);
                publish_bus_events(&shared, &mut bus, bus_event_subscriber);
                let _ = reply.send(Ok(()));
            }
            WorkerMsg::ReplaceFunctions {
                previous,
                current,
                reply,
            } => {
                replace_functions(&mut bus, &previous, current);
                publish_snapshot(&shared, &bus);
                publish_bus_events(&shared, &mut bus, bus_event_subscriber);
                let _ = reply.send(Ok(()));
            }
        }
        shared.writer_busy.store(false, Ordering::SeqCst);
        if shared.shutdown.load(Ordering::SeqCst) {
            fail_pending(&rx, &shared);
            break;
        }
    }
    shared.writer_busy.store(false, Ordering::SeqCst);
}

fn cf_worker_loop(rx: Receiver<CfWorkerMsg>, shared: Arc<Shared>) {
    while let Ok(message) = rx.recv() {
        match message {
            CfWorkerMsg::Shutdown => break,
            CfWorkerMsg::Resolve => loop {
                if shared.shutdown.load(Ordering::SeqCst) {
                    let mut state = shared.cf_requests.lock().unwrap_or_else(|p| p.into_inner());
                    state.pending = None;
                    state.wake_queued = false;
                    break;
                }
                let request = {
                    let mut state = shared.cf_requests.lock().unwrap_or_else(|p| p.into_inner());
                    let Some(request) = state.pending.take() else {
                        state.wake_queued = false;
                        break;
                    };
                    request
                };
                let resolved = request
                    .ranges
                    .iter()
                    .copied()
                    .map(|range| {
                        resolve_overlay_with_registry(
                            &request.reader.workbook,
                            request.sheet,
                            range,
                            &request.registry,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>();
                let (overlays, error) = match resolved {
                    Ok(overlays) => (overlays, None),
                    Err(error) => (Vec::new(), Some(error)),
                };
                let current = {
                    let published = shared.snapshot.lock().unwrap_or_else(|p| p.into_inner());
                    Arc::ptr_eq(&published.reader, &request.reader)
                };
                let latest = shared
                    .cf_requests
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .last
                    .as_ref()
                    .is_some_and(|last| same_cf_request(last, &request));
                if current && latest {
                    *shared
                        .conditional_formats
                        .lock()
                        .unwrap_or_else(|p| p.into_inner()) =
                        Some(Arc::new(ConditionalFormatSnapshot {
                            reader: Arc::clone(&request.reader),
                            sheet: request.sheet,
                            overlays,
                            error,
                        }));
                    wake_event_consumer(&shared);
                }
            },
        }
    }
}

fn same_cf_request(left: &CfRequest, right: &CfRequest) -> bool {
    Arc::ptr_eq(&left.reader, &right.reader)
        && left.sheet == right.sheet
        && left.ranges == right.ranges
}

fn validate_cf_ranges(ranges: &[RangeRef]) -> Result<(), CoreError> {
    if ranges.len() > MAX_CF_VIEWPORT_RANGES {
        return Err(CoreError::new(
            "condfmt.limit",
            format!(
                "conditional-format viewport has {} ranges; maximum is {MAX_CF_VIEWPORT_RANGES}",
                ranges.len()
            ),
        ));
    }
    let mut total = 0u64;
    for range in ranges {
        let rows = u64::from(range.start.row.abs_diff(range.end.row)) + 1;
        let cols = u64::from(range.start.col.abs_diff(range.end.col)) + 1;
        total = total
            .checked_add(rows.saturating_mul(cols))
            .ok_or_else(|| CoreError::new("condfmt.limit", "viewport size overflow"))?;
    }
    if total > MAX_CF_OVERLAY_CELLS {
        return Err(CoreError::new(
            "condfmt.limit",
            format!(
                "conditional-format viewport has {total} cells; maximum is {MAX_CF_OVERLAY_CELLS}"
            ),
        ));
    }
    Ok(())
}

fn mark_cancelling(shared: &Shared, id: TaskId) {
    let mut tasks = shared.tasks.lock().unwrap_or_else(|p| p.into_inner());
    let Some(state) = tasks.get_mut(&id) else {
        return;
    };
    if !matches!(state.status, TaskStatus::Queued | TaskStatus::Running) {
        return;
    }
    state.status = TaskStatus::Cancelling;
    let state = state.clone();
    drop(tasks);
    push_event(shared, TaskEvent::Cancelling(state));
}

fn finish_execute(shared: &Shared, id: TaskId, outcome: &Outcome) {
    *shared.running.lock().unwrap_or_else(|p| p.into_inner()) = None;
    let event = {
        let mut tasks = shared.tasks.lock().unwrap_or_else(|p| p.into_inner());
        let Some(state) = tasks.get_mut(&id) else {
            return;
        };
        if outcome.ok {
            state.status = TaskStatus::Completed;
            TaskEvent::Completed {
                state: state.clone(),
                outcome: outcome.clone(),
            }
        } else {
            state.status = TaskStatus::Failed;
            let (code, message) = outcome
                .error
                .as_ref()
                .map(|e| (e.code.clone(), e.message.clone()))
                .unwrap_or_else(|| ("task.failed".into(), "command failed".into()));
            TaskEvent::Failed {
                state: state.clone(),
                code,
                message,
            }
        }
    };
    // Publish the terminal event before making `is_busy()` false, while still
    // invoking the external waker without any task-map lock held.
    push_event(shared, event);
    shared
        .tasks
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&id);
    shared
        .cancels
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&id);
    shared.task_slots.fetch_sub(1, Ordering::SeqCst);
}

fn fail_pending(rx: &Receiver<WorkerMsg>, shared: &Shared) {
    while let Ok(msg) = rx.try_recv() {
        match msg {
            WorkerMsg::Execute { id, reply, .. } => {
                let outcome = Outcome::failure(error::task_shutdown());
                finish_execute(shared, id, &outcome);
                if let Some(reply) = reply {
                    let _ = reply.send(outcome);
                }
            }
            WorkerMsg::Propose { reply, .. }
            | WorkerMsg::ReviseProposal { reply, .. }
            | WorkerMsg::DiscardProposal { reply, .. }
            | WorkerMsg::Apply { reply, .. }
            | WorkerMsg::Revert { reply, .. }
            | WorkerMsg::GetChangeset { reply, .. } => {
                let _ = reply.send(Err(error::task_shutdown()));
            }
            WorkerMsg::ListChangesets { reply } => {
                let _ = reply.send(Err(error::task_shutdown()));
            }
            WorkerMsg::PreviewChangeset { reply, .. } => {
                let _ = reply.send(Err(error::task_shutdown()));
            }
            WorkerMsg::DryRun { reply, .. } => {
                let _ = reply.send(Err(error::task_shutdown()));
            }
            WorkerMsg::RegisterFunction { reply, .. } => {
                let _ = reply.send(Err(error::task_shutdown()));
            }
            WorkerMsg::RefreshFunctions { reply } => {
                let _ = reply.send(Err(error::task_shutdown()));
            }
            WorkerMsg::ReplaceFunctions { reply, .. } => {
                let _ = reply.send(Err(error::task_shutdown()));
            }
            WorkerMsg::Shutdown => {}
        }
    }
}

fn publish_snapshot(shared: &Shared, bus: &Bus) {
    let next = Arc::new(ReaderSnapshot {
        workbook: bus.workbook().clone(),
        spill: bus.engine().spill().clone(),
    });
    *shared.snapshot.lock().unwrap_or_else(|p| p.into_inner()) = PublishedReader {
        reader: next,
        registry: Arc::new(bus.engine().registry().clone()),
    };
    *shared
        .conditional_formats
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = None;
    let mut requests = shared.cf_requests.lock().unwrap_or_else(|p| p.into_inner());
    requests.pending = None;
    requests.last = None;
    drop(requests);
    wake_event_consumer(shared);
}

fn replace_functions(bus: &mut Bus, previous: &BTreeSet<String>, current: Vec<DynamicFn>) {
    let previous = previous
        .iter()
        .map(|name| name.to_ascii_uppercase())
        .collect::<BTreeSet<_>>();
    let mut registry = FnRegistry::new();
    for def in bus.engine().registry().iter() {
        registry.register(*def);
    }
    for def in bus.engine().registry().iter_dynamic() {
        if !previous.contains(&def.name.to_ascii_uppercase()) {
            registry.register_dynamic(def.clone());
        }
    }
    for def in current {
        registry.register_dynamic(def);
    }
    *bus.engine_mut().registry_mut() = registry;
    bus.recalc_after_registry_change();
}

fn publish_bus_events(shared: &Shared, bus: &mut Bus, subscriber: crate::event::SubscriberId) {
    let events = bus.drain(subscriber);
    if events.is_empty() {
        return;
    }
    {
        let mut ipc_events = shared
            .ipc_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for event in &events {
            ipc_events.emit(event.clone());
        }
    }
    let mut queue = shared
        .bus_events
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for event in events {
        if queue.len() > crate::changeset::MAX_EFFECT_RECORDS {
            queue.pop_front();
            shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
        queue.push_back(event);
    }
    drop(queue);
    wake_event_consumer(shared);
}

fn coalesce_progress(shared: &Shared, id: TaskId, done: u64, total: Option<u64>, label: &str) {
    let mut tasks = shared.tasks.lock().unwrap_or_else(|p| p.into_inner());
    let Some(state) = tasks.get_mut(&id) else {
        return;
    };
    state.progress = Some(TaskProgress {
        done,
        total,
        label: label.chars().take(64).collect(),
    });
    let snapshot = state.clone();
    drop(tasks);
    let mut q = shared.events.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(TaskEvent::Progress(existing)) = q.back_mut()
        && existing.id == id
    {
        *existing = snapshot;
        drop(q);
        wake_event_consumer(shared);
        return;
    }
    if q.len() >= MAX_TASK_EVENTS {
        q.pop_front();
        shared.dropped.fetch_add(1, Ordering::Relaxed);
    }
    q.push_back(TaskEvent::Progress(snapshot));
    drop(q);
    wake_event_consumer(shared);
}

/// Test helper: register `test.hold` which waits on `start` then spins until
/// `release` or cancel. Synchronization uses a barrier and atomics, not sleeps.
#[cfg(feature = "test-util")]
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
