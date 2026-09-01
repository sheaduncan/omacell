//! TUI session over the WP-13 composition objects.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{IsTerminal, stdout};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::ExecutableCommand;
use crossterm::cursor::Show;
use crossterm::event::{
    self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
    Event as CEvent, KeyEventKind,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use omacell_ai::{AutopilotPolicy, AutopilotScope, Plan, to_calls};
use omacell_bus::ipc::{IpcHandle, default_runtime_dir};
use omacell_bus::{
    Bus, CancelHandle, CommandJson, CommandsEnvelope, LongOps, TaskEvent, TaskId, TaskRunner,
    TaskRunnerHandle,
};
use omacell_conf::{ConfigStore, Paths, ReloadEvent};
use omacell_core::addr::{SheetId, quote_sheet_name};
use omacell_core::changeset::{ChangesetId, ChangesetStatus};
use omacell_core::command::{Origin, Outcome};
use omacell_core::error::CoreError;
use omacell_core::workbook::Workbook;
use omacell_lua::{InteractiveRuntime, InteractiveUi};
use omacell_ui::{
    AgentRole, Area, ChangesetReview, ExtendMode, FormulaAssist, KeyCode, KeyEvent, KeyOutcome,
    KeymapRoots, UiSession, apply_local_command, apply_search_result,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};

use crate::input::{map_key, map_mouse};
use crate::render;
use crate::theme::truecolor_enabled;

/// Objects the CLI composition root hands to the TUI. No second config load.
pub struct Launch {
    /// XDG paths already resolved by the CLI.
    pub paths: Paths,
    /// Live config store (watcher enabled).
    pub store: ConfigStore,
    /// Command bus with file, theme, and UI commands registered.
    pub bus: Bus,
    /// WP-14 session.
    pub ui: UiSession,
    /// Keymap search roots used on reload.
    pub roots: KeymapRoots,
    /// Long-operation classifier (composition layer).
    pub long_ops: LongOps,
    /// Workbook path from `omacell --tui [file]`, if any.
    pub file: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct FormulaTask {
    command: String,
    target: String,
}

#[derive(Default)]
struct InlineCompletion {
    observed: Option<String>,
    due: Option<(Instant, String)>,
    tasks: BTreeMap<TaskId, String>,
    active: Option<CancelHandle>,
}

/// Running TUI. Tests drive [`Self::draw`] / [`Self::step_key`].
pub struct Tui {
    paths: Paths,
    store: ConfigStore,
    runner: TaskRunner,
    scripts: InteractiveRuntime,
    ui: UiSession,
    roots: KeymapRoots,
    truecolor: bool,
    message: Option<String>,
    script_status: Option<String>,
    palette_index: usize,
    palette_command: Option<String>,
    palette_plan_task: Option<TaskId>,
    autopilot: Option<AutopilotPolicy>,
    formula_tasks: BTreeMap<TaskId, FormulaTask>,
    completion: InlineCompletion,
    last_grid: Mutex<Option<render::GridHitMap>>,
    catalog: Vec<CommandJson>,
    active_sheet: SheetId,
    dirty: bool,
    discard_armed: Option<String>,
    quit_armed: bool,
    quit_requested: bool,
    last_queued: Option<TaskId>,
    focused_cancel: Option<CancelHandle>,
    quit_after: Option<TaskId>,
    file: Option<PathBuf>,
    window_focused: Option<bool>,
    ipc: Option<IpcHandle>,
}

impl Tui {
    /// Wrap a launch. `ipc` starts the in-process socket used by the theme hook.
    pub fn new(launch: Launch, ipc: bool) -> Result<Self, CoreError> {
        let loaded = launch.store.snapshot();
        let truecolor = truecolor_enabled(&loaded.config.tui.truecolor);
        let catalog = {
            let text = launch
                .bus
                .commands_json()
                .map_err(|err| CoreError::new("tui.palette", err.to_string()))?;
            serde_json::from_str::<CommandsEnvelope>(&text)
                .map(|envelope| envelope.commands)
                .map_err(|err| CoreError::new("tui.palette", err.to_string()))?
        };
        let runner = TaskRunner::spawn(launch.bus, launch.long_ops)?;
        let script_ui = Arc::new(FrontendScriptUi {
            ui: launch.ui.clone(),
            known: runner.handle().command_ids().clone(),
        });
        let scripts = InteractiveRuntime::new(
            runner.handle(),
            script_ui,
            launch.paths.user_config.clone(),
            &loaded,
        )?;
        let mut message = scripts.take_messages().into_iter().last();
        let script_status = message.clone();
        if launch.file.is_some()
            && let Err(error) = scripts.emit_open()
        {
            message = Some(format!("{}: {}", error.code, error.message));
        }
        let ipc_handle = if ipc {
            Some(omacell_bus::ipc::serve_runner(
                default_runtime_dir(),
                runner.handle(),
            )?)
        } else {
            None
        };
        let snapshot = runner.handle().snapshot();
        let active_sheet = snapshot.workbook.active_sheet();
        apply_sheet_view(&launch.ui, &snapshot.workbook, active_sheet);
        Ok(Self {
            paths: launch.paths,
            store: launch.store,
            runner,
            scripts,
            ui: launch.ui,
            roots: launch.roots,
            truecolor,
            message,
            script_status,
            palette_index: 0,
            palette_command: None,
            palette_plan_task: None,
            autopilot: None,
            formula_tasks: BTreeMap::new(),
            completion: InlineCompletion::default(),
            last_grid: Mutex::new(None),
            catalog,
            active_sheet,
            dirty: false,
            discard_armed: None,
            quit_armed: false,
            quit_requested: false,
            last_queued: None,
            focused_cancel: None,
            quit_after: None,
            file: launch.file,
            window_focused: None,
            ipc: ipc_handle,
        })
    }

    /// XDG paths from the composition root.
    #[must_use]
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// WP-14 session.
    #[must_use]
    pub fn ui(&self) -> &UiSession {
        &self.ui
    }

    /// Config store (reload tests).
    #[must_use]
    pub fn store(&self) -> &ConfigStore {
        &self.store
    }

    /// Last status-line message (reload errors, failed commands).
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Whether workbook changes have not been saved in this TUI session.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Whether the event loop should exit after the current input step.
    #[must_use]
    pub fn quit_requested(&self) -> bool {
        self.quit_requested
    }

    /// Whether commands are queued or currently using the writer.
    #[must_use]
    pub fn has_pending_tasks(&self) -> bool {
        self.runner.handle().tracked_tasks() != 0
    }

    /// Single-writer task handle used by integrations and black-box tests.
    #[must_use]
    pub fn runner(&self) -> TaskRunnerHandle {
        self.runner.handle()
    }

    /// Apply pending filesystem/theme reloads without resetting the session.
    pub fn poll_reload(&mut self) -> Result<(), CoreError> {
        self.poll_tasks();
        self.sync_inline_completion();
        let events = self.store.drain_events();
        if events.is_empty() {
            self.sync_active_sheet();
            return Ok(());
        }
        let snapshot = self.store.snapshot();
        for ev in &events {
            match ev {
                ReloadEvent::Invalid { message, .. } => {
                    self.message = Some(message.clone());
                }
                ReloadEvent::Applied { .. } | ReloadEvent::ThemeChanged { .. } => {
                    if let Err(err) = self.ui.apply_config_ids(
                        &snapshot,
                        &self.roots,
                        self.runner.handle().command_ids(),
                    ) {
                        self.message = Some(format!("{}: {}", err.code, err.message));
                        continue;
                    }
                    if let Err(error) = self.scripts.tighten(&snapshot) {
                        self.message = Some(format!("{}: {}", error.code, error.message));
                    }
                    if matches!(ev, ReloadEvent::Applied { .. }) {
                        self.reset_autopilot("Autopilot reset after configuration reload.");
                    }
                    self.truecolor = truecolor_enabled(&snapshot.config.tui.truecolor);
                    if let ReloadEvent::ThemeChanged { name } = ev {
                        self.message = Some(format!("theme {name}"));
                    }
                }
            }
        }
        self.sync_active_sheet();
        Ok(())
    }

    /// Draw using the current workbook snapshot.
    pub fn draw<B: Backend>(&self, terminal: &mut Terminal<B>) -> Result<(), B::Error> {
        let loaded = self.store.snapshot();
        let unicode = loaded.config.tui.unicode_borders;
        let snapshot = self.runner.handle().snapshot();
        let busy = self.runner.handle().is_busy();
        let mut progress_msg = self.message.clone();
        if let Some(task) = self.runner.handle().running()
            && let Some(progress) = task.progress
        {
            let label = match progress.total {
                Some(total) => format!("{} {}/{}", progress.label, progress.done, total),
                None => format!("{} {}", progress.label, progress.done),
            };
            progress_msg = Some(label);
        }
        let mut hit_map = None;
        terminal.draw(|frame| {
            hit_map = Some(render::draw(
                frame,
                render::FrameInput {
                    wb: &snapshot.workbook,
                    spill: &snapshot.spill,
                    ui: &self.ui,
                    theme_name: &loaded.theme.name,
                    truecolor: self.truecolor,
                    unicode_borders: unicode,
                    message: progress_msg.as_deref(),
                    palette_index: self.palette_index,
                    dirty: self.dirty,
                    busy,
                },
            ));
        })?;
        *self.last_grid.lock().unwrap_or_else(|p| p.into_inner()) = hit_map;
        Ok(())
    }

    /// Handle one toolkit-neutral key (tests and the live loop).
    pub fn step_key(&mut self, event: KeyEvent) -> Result<KeyOutcome, CoreError> {
        self.poll_reload()?;
        if event.code == KeyCode::Char('q') && event.ctrl && !event.alt && self.ui.edit().is_idle()
        {
            self.request_quit(false);
            return Ok(KeyOutcome::Pending);
        }
        self.quit_armed = false;
        if event.code == KeyCode::Esc
            && let Some(handle) = self
                .focused_cancel
                .clone()
                .or_else(|| self.runner.handle().running_cancel())
        {
            handle.cancel();
            self.message = Some("cancelling…".into());
            return Ok(KeyOutcome::Pending);
        }
        if self.ui.palette().open {
            if event.code != KeyCode::Enter {
                self.discard_armed = None;
            }
            return self.step_palette(event);
        }
        if let Some(id) = self.ui.panel().visible.clone()
            && matches!(
                id.as_str(),
                "find" | "goto" | "command" | "changeset" | "agent" | "formula"
            )
        {
            if event.code != KeyCode::Enter {
                self.discard_armed = None;
            }
            return self.step_panel(event, &id);
        }
        let outcome = self.ui.handle_key(event);
        if let KeyOutcome::Command { cmd, args, .. } = outcome.clone() {
            self.execute_cmd(&cmd, args)?;
        } else {
            self.discard_armed = None;
        }
        Ok(outcome)
    }

    /// Run a registry command as the interactive user.
    pub fn execute_cmd(
        &mut self,
        cmd: &str,
        args: serde_json::Value,
    ) -> Result<Outcome, CoreError> {
        let args = inject_selection_context(&self.ui, cmd, args);
        if self.prompt_command_args(cmd, &args) {
            return Ok(Outcome::success(serde_json::json!({"prompt": true})));
        }
        if cmd == "file.close" {
            if !self.confirm_discard(cmd) {
                return Ok(discard_confirmation(cmd));
            }
            self.ui.remember_command(cmd);
            self.message = None;
            self.request_quit(true);
            return Ok(Outcome::success(serde_json::json!({"close": true})));
        }
        if matches!(cmd, "file.new" | "file.open") && !self.confirm_discard(cmd) {
            return Ok(discard_confirmation(cmd));
        }
        if !matches!(cmd, "file.new" | "file.open") {
            self.discard_armed = None;
        }
        let handle = self.runner.handle();
        self.last_queued = None;
        let formula_task = formula_task(cmd, &args);
        if let Some(local) = apply_local_command(&self.ui, &handle.snapshot().workbook, cmd, &args)
        {
            if let Err(err) = local {
                self.message = Some(err.message.clone());
                return Ok(Outcome::failure(err));
            }
            self.ui.remember_command(cmd);
            self.message = None;
            if matches!(cmd, "palette.open" | "ai.assist") {
                let query = if cmd == "ai.assist" {
                    "ai.formula."
                } else {
                    ""
                };
                self.refresh_palette(query)?;
                if cmd == "ai.assist" {
                    let mut palette = self.ui.palette();
                    palette.prompt =
                        Some("AI assist — choose generate, explain, fix, or refactor".into());
                    self.ui.set_palette(palette);
                }
            }
            if cmd == "ai.agent" {
                self.dispatch_agent();
            }
            if cmd == "changeset.review"
                && let Err(error) = self.open_latest_review()
            {
                self.message = Some(format!("{}: {}", error.code, error.message));
                return Ok(Outcome::failure(error));
            }
            return Ok(Outcome::success(serde_json::json!({"ok": true})));
        }
        if let Err(error) = self.scripts.before_command(cmd) {
            self.message = Some(format!("{}: {}", error.code, error.message));
            return Ok(Outcome::failure(error));
        }
        let (id, cancel) = match handle.submit(Origin::User, cmd, args) {
            Ok(task) => task,
            Err(err) => {
                self.message = Some(err.message.clone());
                return Ok(Outcome::failure(err));
            }
        };
        if let Some(task) = formula_task {
            self.formula_tasks.insert(id, task);
        }
        self.last_queued = Some(id);
        self.focused_cancel = Some(cancel);
        self.message = Some(if handle.long_ops().contains(cmd) {
            "working…".into()
        } else {
            "queued…".into()
        });
        Ok(Outcome::success(serde_json::json!({
            "queued": true,
            "task": id.get(),
        })))
    }

    fn dispatch_agent(&mut self) {
        let Some(handoff) = self.ui.take_agent_handoff() else {
            return;
        };
        let handle = self.runner.handle();
        let snapshot = handle.snapshot();
        let selection = selection_a1(&self.ui, &snapshot.workbook);
        let diagnose = if handoff.diagnose {
            let diagnostic_ref = cursor_a1(&self.ui, &snapshot.workbook);
            let outcome = handle.submit_wait(
                Origin::User,
                "audit.diagnose",
                serde_json::json!({"ref": diagnostic_ref}),
            );
            if !outcome.ok {
                self.message = outcome.error.map(|error| error.message);
                return;
            }
            let mut bundle = serde_json::json!({
                "schema": 1,
                "workbook": self.file.as_ref().map(|path| path.display().to_string()),
                "selection": &selection,
                "diagnostic": outcome.result,
            });
            let _ = omacell_ai::redact_json(&mut bundle);
            match omacell_conf::write_diagnostic_bundle(&self.paths.state_dir, &bundle) {
                Ok(path) => Some(path),
                Err(error) => {
                    self.message = Some(error.message);
                    return;
                }
            }
        } else {
            None
        };
        let prompt = if handoff.diagnose {
            "Diagnose this Omacell workbook".into()
        } else if handoff.prompt.is_empty() {
            "Help with this workbook".into()
        } else {
            handoff.prompt
        };
        match omacell_conf::hand_off(omacell_conf::HandOffRequest {
            prompt,
            workbook: self.file.clone(),
            selection: Some(selection),
            diagnose,
            state_dir: self.paths.state_dir.clone(),
        }) {
            Ok(result) if result.hidden => {
                self.message = Some(format!(
                    "no default agent; run: {}",
                    omacell_conf::shell_command(&result.argv)
                ));
            }
            Ok(_) => self.message = Some("handed to omarchy agent".into()),
            Err(err) => self.message = Some(err.message),
        }
    }

    fn adopt_snapshot(&mut self) {
        let snapshot = self.runner.handle().snapshot();
        let sheet = snapshot.workbook.active_sheet();
        if sheet != self.active_sheet {
            apply_sheet_view(&self.ui, &snapshot.workbook, sheet);
            self.active_sheet = sheet;
        }
    }

    fn poll_tasks(&mut self) {
        for event in self.runner.handle().drain_events() {
            match event {
                TaskEvent::Completed { state, outcome } => {
                    if self
                        .focused_cancel
                        .as_ref()
                        .is_some_and(|cancel| cancel.id() == state.id)
                    {
                        self.focused_cancel = None;
                    }
                    if self.finish_inline_completion(state.id, outcome.result.as_ref()) {
                        continue;
                    }
                    self.message = None;
                    if let Some(task) = self.formula_tasks.remove(&state.id)
                        && let Err(error) =
                            self.handle_formula_result(&task, outcome.result.as_ref())
                    {
                        self.message = Some(format!("{}: {}", error.code, error.message));
                    }
                    if self.palette_plan_task == Some(state.id) {
                        self.palette_plan_task = None;
                        match outcome
                            .result
                            .as_ref()
                            .ok_or_else(|| CoreError::new("ai.plan", "AI returned an empty plan"))
                            .and_then(|value| self.open_plan_review(value))
                        {
                            Ok(()) => {}
                            Err(error) => {
                                self.message = Some(format!("{}: {}", error.code, error.message));
                            }
                        }
                    }
                    if state.command == "ai.agent.turn" {
                        let result = outcome
                            .result
                            .as_ref()
                            .ok_or_else(|| {
                                CoreError::new("ai.agent", "agent returned an empty result")
                            })
                            .and_then(|value| self.handle_agent_result(value));
                        if let Err(error) = result {
                            self.agent_failed(&error.message);
                            self.message = Some(format!("{}: {}", error.code, error.message));
                        }
                    }
                    if matches!(
                        state.command.as_str(),
                        "file.open" | "file.save" | "file.saveas" | "file.new"
                    ) {
                        self.dirty = false;
                        self.file = outcome
                            .result
                            .as_ref()
                            .and_then(|value| value.get("path"))
                            .and_then(|value| value.as_str())
                            .map(PathBuf::from);
                    } else {
                        let mutating = self
                            .catalog
                            .iter()
                            .find(|command| command.id == state.command)
                            .is_some_and(|command| command.mutating);
                        if command_changes_workbook(&state.command, &outcome, mutating) {
                            self.dirty = true;
                        }
                    }
                    self.ui.remember_command(&state.command);
                    if state.command == "script.source"
                        && let Err(error) = self.scripts.source()
                    {
                        self.message = Some(format!("{}: {}", error.code, error.message));
                    }
                    if state.command == "file.close" {
                        self.request_quit(false);
                    } else if matches!(state.command.as_str(), "file.open" | "file.new") {
                        self.adopt_file_snapshot();
                    } else if matches!(
                        state.command.as_str(),
                        "edit.searchnext" | "edit.searchprev"
                    ) {
                        self.adopt_snapshot();
                        if !outcome
                            .result
                            .as_ref()
                            .is_some_and(|result| apply_search_result(&self.ui, result))
                        {
                            self.message = Some("no matches".into());
                        }
                    } else if state.command == "changeset.review"
                        && let Err(error) = self.open_latest_review()
                    {
                        self.message = Some(format!("{}: {}", error.code, error.message));
                    } else {
                        self.adopt_snapshot();
                    }
                    if self.quit_after == Some(state.id) {
                        self.quit_after = None;
                        self.request_quit(true);
                    }
                }
                TaskEvent::Failed { state, message, .. } => {
                    if self
                        .focused_cancel
                        .as_ref()
                        .is_some_and(|cancel| cancel.id() == state.id)
                    {
                        self.focused_cancel = None;
                    }
                    if self.drop_inline_completion(state.id) {
                        continue;
                    }
                    if self.palette_plan_task == Some(state.id) {
                        self.palette_plan_task = None;
                        let mut palette = self.ui.palette();
                        palette.prompt = Some("AI plan failed".into());
                        palette.preview = Some(message);
                        self.ui.set_palette(palette);
                    } else {
                        self.message = Some(message);
                    }
                    if let Some(task) = self.formula_tasks.remove(&state.id) {
                        self.ui.set_formula_assist(Some(FormulaAssist {
                            task: task.command,
                            target: task.target,
                            explanation: Some(
                                self.message
                                    .clone()
                                    .unwrap_or_else(|| "formula assist failed".into()),
                            ),
                            ..FormulaAssist::default()
                        }));
                        self.open_formula_panel();
                    }
                    if state.command == "ai.agent.turn" {
                        self.agent_failed(self.message.as_deref().unwrap_or("agent turn failed"));
                    }
                    self.adopt_snapshot();
                    if self.quit_after == Some(state.id) {
                        self.quit_after = None;
                    }
                }
                TaskEvent::Progress(state) => {
                    if let Some(progress) = state.progress {
                        self.message = Some(match progress.total {
                            Some(total) => {
                                format!("{} {}/{}", progress.label, progress.done, total)
                            }
                            None => format!("{} {}", progress.label, progress.done),
                        });
                    }
                }
                TaskEvent::Cancelling(_) | TaskEvent::Running(_) | TaskEvent::Queued(_) => {}
            }
        }
        if let Err(error) = self.scripts.poll_events() {
            self.message = Some(format!("{}: {}", error.code, error.message));
        }
        if let Some(message) = self.scripts.take_messages().into_iter().last() {
            self.script_status = Some(message.clone());
            self.message = Some(message);
        } else if self.message.is_none() {
            self.message.clone_from(&self.script_status);
        }
    }

    fn step_palette(&mut self, event: KeyEvent) -> Result<KeyOutcome, CoreError> {
        if self.palette_command.is_some() {
            return self.step_palette_args(event);
        }
        match (event.code, event.ctrl, event.alt) {
            (KeyCode::Esc, false, false) => {
                self.close_palette();
            }
            (KeyCode::Enter, false, false) => {
                let palette = self.ui.palette();
                if let Some(prompt) = palette.query.strip_prefix('?').map(str::trim)
                    && !prompt.is_empty()
                {
                    self.submit_palette_plan(prompt.to_string())?;
                    return Ok(KeyOutcome::Pending);
                }
                if let Some(hit) = palette.hits.get(self.palette_index) {
                    let id = hit.id.clone();
                    let args = inject_selection_context(&self.ui, &id, serde_json::json!({}));
                    if !self.prompt_command_args(&id, &args) {
                        let result = self.execute_cmd(&id, args)?;
                        if result.ok {
                            self.close_palette();
                        }
                    }
                }
            }
            (KeyCode::Up, false, false) => {
                self.palette_index = self.palette_index.saturating_sub(1);
            }
            (KeyCode::Down, false, false) => {
                let n = self.ui.palette().hits.len();
                if n > 0 {
                    self.palette_index = (self.palette_index + 1).min(n - 1);
                }
            }
            (KeyCode::Backspace, false, false) => {
                let mut palette = self.ui.palette();
                palette.query.pop();
                let q = palette.query.clone();
                self.ui.set_palette(palette);
                self.refresh_palette(&q)?;
            }
            (KeyCode::Char(c), false, false) => {
                let mut palette = self.ui.palette();
                palette.query.push(c);
                let q = palette.query.clone();
                self.ui.set_palette(palette);
                self.refresh_palette(&q)?;
            }
            (KeyCode::Space, false, false) => {
                let mut palette = self.ui.palette();
                palette.query.push(' ');
                let q = palette.query.clone();
                self.ui.set_palette(palette);
                self.refresh_palette(&q)?;
            }
            _ => {}
        }
        Ok(KeyOutcome::Pending)
    }

    fn step_palette_args(&mut self, event: KeyEvent) -> Result<KeyOutcome, CoreError> {
        match (event.code, event.ctrl, event.alt) {
            (KeyCode::Esc, false, false) => self.close_palette(),
            (KeyCode::Enter, false, false) => {
                let text = self.ui.palette().query;
                let args = match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(serde_json::Value::Object(map)) => serde_json::Value::Object(map),
                    Ok(_) => {
                        self.message = Some("palette arguments must be a JSON object".into());
                        return Ok(KeyOutcome::Pending);
                    }
                    Err(err) => {
                        self.message = Some(format!("invalid JSON arguments: {err}"));
                        return Ok(KeyOutcome::Pending);
                    }
                };
                if let Some(id) = self.palette_command.clone() {
                    let result = self.execute_cmd(&id, args)?;
                    if result.ok {
                        self.close_palette();
                    }
                }
            }
            (KeyCode::Backspace, false, false) => {
                let mut palette = self.ui.palette();
                palette.query.pop();
                self.ui.set_palette(palette);
            }
            (KeyCode::Char(c), false, false) => {
                let mut palette = self.ui.palette();
                palette.query.push(c);
                self.ui.set_palette(palette);
            }
            (KeyCode::Space, false, false) => {
                let mut palette = self.ui.palette();
                palette.query.push(' ');
                self.ui.set_palette(palette);
            }
            _ => {}
        }
        Ok(KeyOutcome::Pending)
    }

    fn close_palette(&mut self) {
        let mut palette = self.ui.palette();
        palette.close();
        self.ui.set_palette(palette);
        self.palette_index = 0;
        self.palette_command = None;
    }

    fn prompt_command_args(&mut self, id: &str, args: &serde_json::Value) -> bool {
        let Some(command) = self
            .catalog
            .iter()
            .find(|command| command.id == id)
            .cloned()
            .filter(|command| has_missing_required_args(command, args))
        else {
            return false;
        };
        let mut palette = self.ui.palette();
        palette.open();
        if args.as_object().is_some_and(|fields| !fields.is_empty()) {
            palette.query = serde_json::to_string(args).unwrap_or_default();
        }
        palette.prompt_for(&command);
        if let Some(fields) = palette.prompt.take() {
            palette.prompt = Some(format!("{id} — {fields}; enter JSON object"));
        }
        self.ui.set_palette(palette);
        self.palette_command = Some(id.to_string());
        self.palette_index = 0;
        true
    }

    fn confirm_discard(&mut self, command: &str) -> bool {
        if !self.dirty || self.discard_armed.as_deref() == Some(command) {
            self.discard_armed = None;
            return true;
        }
        self.discard_armed = Some(command.to_string());
        self.message = Some(format!(
            "unsaved changes; run {command} again to discard them"
        ));
        false
    }

    fn submit_palette_plan(&mut self, prompt: String) -> Result<(), CoreError> {
        let (id, cancel) = self.runner.handle().submit(
            Origin::User,
            "ai.plan",
            serde_json::json!({"prompt": prompt, "apply": false}),
        )?;
        self.palette_plan_task = Some(id);
        self.focused_cancel = Some(cancel);
        let mut palette = self.ui.palette();
        palette.prompt = Some("Planning…".into());
        palette.preview = None;
        self.ui.set_palette(palette);
        Ok(())
    }

    fn command_catalog(&self) -> Result<Vec<CommandJson>, CoreError> {
        Ok(self.catalog.clone())
    }

    fn refresh_palette(&mut self, query: &str) -> Result<(), CoreError> {
        let commands = self.command_catalog()?;
        self.ui.rank_palette(&commands, query, None);
        self.palette_index = 0;
        Ok(())
    }

    fn step_panel(&mut self, event: KeyEvent, id: &str) -> Result<KeyOutcome, CoreError> {
        if id == "changeset" {
            return self.step_changeset_review(event);
        }
        if id == "formula" {
            return self.step_formula_panel(event);
        }
        if id == "agent" {
            return self.step_agent_panel(event);
        }
        match (event.code, event.ctrl) {
            (KeyCode::Esc, false) => {
                if id == "command" {
                    self.dismiss_command_line()?;
                } else {
                    let mut panel = self.ui.panel();
                    panel.dismiss();
                    self.ui.set_panel(panel);
                }
            }
            (KeyCode::Backspace, false) => match id {
                "find" => {
                    let mut find = self.ui.find_replace();
                    find.find.pop();
                    self.ui.set_find_replace(find);
                }
                "goto" | "command" => {
                    let mut goto = self.ui.goto();
                    goto.target.pop();
                    self.ui.set_goto(goto);
                }
                _ => {}
            },
            (KeyCode::Char(c), false) => match id {
                "find" => {
                    let mut find = self.ui.find_replace();
                    find.find.push(c);
                    self.ui.set_find_replace(find);
                }
                "goto" | "command" => {
                    let mut goto = self.ui.goto();
                    goto.target.push(c);
                    self.ui.set_goto(goto);
                }
                _ => {}
            },
            (KeyCode::Space, false) => match id {
                "find" => {
                    let mut find = self.ui.find_replace();
                    find.find.push(' ');
                    self.ui.set_find_replace(find);
                }
                "goto" | "command" => {
                    let mut goto = self.ui.goto();
                    goto.target.push(' ');
                    self.ui.set_goto(goto);
                }
                _ => {}
            },
            (KeyCode::Enter, false) if id == "goto" => {
                let target = self.ui.goto().target;
                if !target.is_empty() {
                    let result = self.execute_cmd(
                        "view.select",
                        serde_json::json!({
                            "range": target,
                        }),
                    )?;
                    if result.ok {
                        let mut panel = self.ui.panel();
                        panel.dismiss();
                        self.ui.set_panel(panel);
                    }
                }
            }
            (KeyCode::Enter, false) if id == "find" && !self.ui.find_replace().find.is_empty() => {
                let result = self.execute_cmd("edit.searchnext", serde_json::json!({}))?;
                if result.ok {
                    let mut panel = self.ui.panel();
                    panel.dismiss();
                    self.ui.set_panel(panel);
                }
            }
            (KeyCode::Enter, false) if id == "command" => {
                self.execute_command_line()?;
            }
            _ => {}
        }
        Ok(KeyOutcome::Pending)
    }

    fn open_plan_review(&mut self, value: &serde_json::Value) -> Result<(), CoreError> {
        let plan: Plan = serde_json::from_value(value.clone())
            .map_err(|error| CoreError::new("ai.payload", format!("plan JSON: {error}")))?;
        let calls = to_calls(&plan).map_err(CoreError::from)?;
        if calls.is_empty() {
            return Err(CoreError::new("ai.plan", "AI plan contains no commands"));
        }
        let proposal = self.runner.handle().propose(Origin::PalettePlan, calls)?;
        self.close_palette();
        self.open_review(&proposal.id)
    }

    fn open_latest_review(&mut self) -> Result<(), CoreError> {
        let proposal = self
            .runner
            .handle()
            .list_changesets()?
            .into_iter()
            .rev()
            .find(|changeset| changeset.status == ChangesetStatus::Proposed)
            .ok_or_else(|| CoreError::new("changeset.review", "no proposed changesets"))?;
        self.open_review(&proposal.id)
    }

    fn open_review(&mut self, id: &ChangesetId) -> Result<(), CoreError> {
        let review = ChangesetReview::from(self.runner.handle().preview_changeset(id)?);
        self.ui.set_changeset_review(Some(review));
        let mut panel = self.ui.panel();
        panel.open("changeset");
        self.ui.set_panel(panel);
        Ok(())
    }

    fn step_changeset_review(&mut self, event: KeyEvent) -> Result<KeyOutcome, CoreError> {
        let Some(mut review) = self.ui.changeset_review() else {
            let mut panel = self.ui.panel();
            panel.dismiss();
            self.ui.set_panel(panel);
            return Ok(KeyOutcome::Pending);
        };
        match (event.code, event.ctrl, event.alt) {
            (KeyCode::Esc, false, false) => {
                let mut panel = self.ui.panel();
                panel.dismiss();
                self.ui.set_panel(panel);
            }
            (KeyCode::Up, false, false) => {
                review.move_selection(-1);
                self.ui.set_changeset_review(Some(review));
            }
            (KeyCode::Down, false, false) => {
                review.move_selection(1);
                self.ui.set_changeset_review(Some(review));
            }
            (KeyCode::Space, false, false) => {
                review.toggle_selected();
                self.ui.set_changeset_review(Some(review));
            }
            (KeyCode::Char('a' | 'A'), false, false) => {
                review.accept_all();
                self.ui.set_changeset_review(Some(review));
            }
            (KeyCode::Char('r' | 'R'), false, false) => {
                self.runner
                    .handle()
                    .discard_proposal(Origin::User, &review.id)?;
                self.ui.set_changeset_review(None);
                let mut panel = self.ui.panel();
                if review.origin == Origin::InAppAgent {
                    panel.open("agent");
                } else {
                    panel.dismiss();
                }
                self.ui.set_panel(panel);
                self.message = Some("proposal rejected".into());
            }
            (KeyCode::Enter, false, false) => {
                let accepted = review.accepted_calls();
                if accepted.is_empty() {
                    self.runner
                        .handle()
                        .discard_proposal(Origin::User, &review.id)?;
                    self.message = Some("proposal rejected".into());
                } else {
                    self.runner
                        .handle()
                        .revise_proposal(Origin::User, &review.id, accepted)?;
                    self.runner.handle().apply(Origin::User, &review.id)?;
                    self.dirty = true;
                    self.message = Some("proposal applied as one changeset".into());
                    self.adopt_snapshot();
                }
                self.ui.set_changeset_review(None);
                let mut panel = self.ui.panel();
                if review.origin == Origin::InAppAgent {
                    panel.open("agent");
                } else {
                    panel.dismiss();
                }
                self.ui.set_panel(panel);
            }
            _ => {}
        }
        Ok(KeyOutcome::Pending)
    }

    fn step_formula_panel(&mut self, event: KeyEvent) -> Result<KeyOutcome, CoreError> {
        if self.ui.changeset_review().is_some() {
            return self.step_changeset_review(event);
        }
        if matches!(
            (event.code, event.ctrl, event.alt),
            (KeyCode::Esc, false, false)
        ) {
            let mut panel = self.ui.panel();
            panel.dismiss();
            self.ui.set_panel(panel);
        }
        Ok(KeyOutcome::Pending)
    }

    fn handle_agent_result(&mut self, value: &serde_json::Value) -> Result<(), CoreError> {
        let mut panel_state = self.ui.agent_panel();
        panel_state.busy = false;
        if let Some(prompt) = value.get("prompt").and_then(serde_json::Value::as_str)
            && panel_state
                .turns
                .last()
                .is_none_or(|turn| turn.role != AgentRole::User || turn.text != prompt)
        {
            panel_state.push_turn(AgentRole::User, prompt);
        }
        let proposed = value
            .get("proposed")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));
        let plan: Plan = serde_json::from_value(serde_json::json!({"commands": proposed}))
            .map_err(|error| CoreError::new("ai.payload", format!("agent plan JSON: {error}")))?;
        let calls = to_calls(&plan).map_err(CoreError::from)?;
        if calls.is_empty() {
            panel_state.push_turn(AgentRole::Assistant, "No workbook changes proposed.");
            self.ui.set_agent_panel(panel_state);
            self.open_agent_panel();
            return Ok(());
        }
        let proposal = self
            .runner
            .handle()
            .propose(Origin::InAppAgent, calls.clone())?;
        if let Some(policy) = &self.autopilot {
            let mut authorized = policy.clone();
            let snapshot = self.runner.handle().snapshot();
            match authorized.authorize_and_record(&calls, &snapshot.workbook) {
                Ok(()) => {
                    self.runner.handle().apply(Origin::User, &proposal.id)?;
                    self.autopilot = Some(authorized.clone());
                    panel_state.set_autopilot(
                        true,
                        autopilot_scope_label(authorized.scope(), &snapshot.workbook),
                        authorized.used_ops(),
                        authorized.max_ops(),
                    );
                    panel_state.push_turn(
                        AgentRole::Assistant,
                        format!("Applied {} commands as one changeset.", calls.len()),
                    );
                    self.dirty = true;
                    self.adopt_snapshot();
                    self.ui.set_agent_panel(panel_state);
                    self.open_agent_panel();
                    return Ok(());
                }
                Err(error) => {
                    panel_state.push_turn(
                        AgentRole::System,
                        format!("Autopilot stopped: {}. Review required.", error.message),
                    );
                }
            }
        } else {
            panel_state.push_turn(
                AgentRole::Assistant,
                format!("Proposed {} commands for review.", calls.len()),
            );
        }
        self.ui.set_agent_panel(panel_state);
        self.open_review(&proposal.id)
    }

    fn step_agent_panel(&mut self, event: KeyEvent) -> Result<KeyOutcome, CoreError> {
        let mut agent = self.ui.agent_panel();
        match (event.code, event.ctrl, event.alt) {
            (KeyCode::Esc, false, false) => {
                let mut panel = self.ui.panel();
                panel.dismiss();
                self.ui.set_panel(panel);
            }
            (KeyCode::F(8), false, false) => self.toggle_autopilot(),
            (KeyCode::Backspace, false, false) if !agent.busy => {
                agent.draft.pop();
                self.ui.set_agent_panel(agent);
            }
            (KeyCode::Space, false, false) if !agent.busy => {
                agent.draft.push(' ');
                self.ui.set_agent_panel(agent);
            }
            (KeyCode::Char(character), false, false) if !agent.busy => {
                if agent.draft.len() < 8_192 {
                    agent.draft.push(character);
                }
                self.ui.set_agent_panel(agent);
            }
            (KeyCode::Enter, false, false) if !agent.busy => {
                let prompt = agent.draft.trim().to_string();
                if !prompt.is_empty() {
                    agent.draft.clear();
                    agent.busy = true;
                    agent.push_turn(AgentRole::User, &prompt);
                    self.ui.set_agent_panel(agent);
                    let outcome = self.execute_cmd(
                        "ai.agent.turn",
                        serde_json::json!({"prompt": prompt, "apply": false}),
                    )?;
                    if !outcome.ok {
                        self.agent_failed("agent turn could not be queued");
                    }
                }
            }
            _ => {}
        }
        Ok(KeyOutcome::Pending)
    }

    fn toggle_autopilot(&mut self) {
        let config = self.ui.config().ai.agent;
        let mut agent = self.ui.agent_panel();
        if self.autopilot.take().is_some() {
            agent.set_autopilot(
                false,
                "review required",
                0,
                config.autopilot_max_ops as usize,
            );
            agent.push_turn(AgentRole::System, "Autopilot disabled for this session.");
            self.ui.set_agent_panel(agent);
            return;
        }
        if config.review != "autopilot_opt_in" {
            agent.push_turn(
                AgentRole::System,
                "Autopilot is disabled by ai.agent.review = \"always\".",
            );
            self.ui.set_agent_panel(agent);
            return;
        }
        let selection = self.ui.selection();
        let (min_row, min_col, max_row, max_col) = selection.active().normalized();
        let scope = match config.autopilot_scope.as_str() {
            "workbook" => AutopilotScope::Workbook,
            "range" => AutopilotScope::Range {
                sheet: selection.sheet,
                min_row,
                min_col,
                max_row,
                max_col,
            },
            _ => AutopilotScope::Sheet(selection.sheet),
        };
        let policy = AutopilotPolicy::new(scope, config.autopilot_max_ops);
        let snapshot = self.runner.handle().snapshot();
        agent.set_autopilot(
            true,
            autopilot_scope_label(policy.scope(), &snapshot.workbook),
            policy.used_ops(),
            policy.max_ops(),
        );
        agent.push_turn(
            AgentRole::System,
            "Autopilot enabled explicitly for this session and scope.",
        );
        self.autopilot = Some(policy);
        self.ui.set_agent_panel(agent);
    }

    fn open_agent_panel(&self) {
        if !self.ui.config().ai.agent.panel {
            return;
        }
        let mut panel = self.ui.panel();
        panel.open("agent");
        self.ui.set_panel(panel);
    }

    fn agent_failed(&self, message: &str) {
        let mut agent = self.ui.agent_panel();
        agent.busy = false;
        agent.push_turn(AgentRole::System, message);
        self.ui.set_agent_panel(agent);
        self.open_agent_panel();
    }

    fn handle_formula_result(
        &mut self,
        task: &FormulaTask,
        result: Option<&serde_json::Value>,
    ) -> Result<(), CoreError> {
        let result = result
            .ok_or_else(|| CoreError::new("ai.formula", "formula assistant returned no result"))?;
        if task.command == "ai.formula.explain" {
            let explanation = result
                .get("explanation")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| CoreError::new("ai.payload", "formula explanation is missing"))?;
            self.ui
                .set_formula_assist(Some(FormulaAssist::explained(&task.target, explanation)));
            self.ui.set_changeset_review(None);
            self.open_formula_panel();
            return Ok(());
        }
        let formula = result
            .get("formula")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| CoreError::new("ai.payload", "generated formula is missing"))?;
        let scratch = result
            .get("scratch")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("validated");
        self.ui.set_formula_assist(Some(FormulaAssist::generated(
            &task.command,
            &task.target,
            formula,
            scratch,
        )));
        let call = omacell_core::changeset::CommandCall {
            id: omacell_core::command::CommandId::new("cell.set")?,
            args: serde_json::json!({"ref": task.target, "input": formula}),
        };
        let proposal = self
            .runner
            .handle()
            .propose(Origin::PalettePlan, vec![call])?;
        let review = ChangesetReview::from(self.runner.handle().preview_changeset(&proposal.id)?);
        self.ui.set_changeset_review(Some(review));
        self.open_formula_panel();
        Ok(())
    }

    fn open_formula_panel(&self) {
        let mut panel = self.ui.panel();
        panel.open("formula");
        self.ui.set_panel(panel);
    }

    fn sync_inline_completion(&mut self) {
        let edit = self.ui.edit();
        let enabled = self.ui.config().ai.completion.mode != "off"
            && self
                .catalog
                .iter()
                .any(|command| command.id == "ai.complete");
        let prefix = (enabled
            && !edit.is_idle()
            && edit.cursor == edit.buffer.len()
            && edit.buffer.starts_with('='))
        .then(|| edit.buffer.clone());
        if self.completion.observed != prefix {
            if let Some(cancel) = self.completion.active.take() {
                cancel.cancel();
            }
            self.completion.observed.clone_from(&prefix);
            self.completion.due = prefix.map(|prefix| {
                (
                    Instant::now() + omacell_ai::runtime::debounce_ms(&self.ui.config()),
                    prefix,
                )
            });
            if edit.ghost.is_some() {
                let mut cleared = edit;
                cleared.ghost = None;
                self.ui.set_edit(cleared);
            }
        }
        let Some((due, prefix)) = self.completion.due.clone() else {
            return;
        };
        if Instant::now() < due {
            return;
        }
        self.completion.due = None;
        if let Ok((id, cancel)) = self.runner.handle().submit(
            Origin::User,
            "ai.complete",
            serde_json::json!({"prefix": prefix}),
        ) {
            self.completion.tasks.insert(id, prefix);
            self.completion.active = Some(cancel);
        }
    }

    fn finish_inline_completion(&mut self, id: TaskId, result: Option<&serde_json::Value>) -> bool {
        let Some(prefix) = self.completion.tasks.remove(&id) else {
            return false;
        };
        if self
            .completion
            .active
            .as_ref()
            .is_some_and(|cancel| cancel.id() == id)
        {
            self.completion.active = None;
        }
        let Some(result) = result else {
            return true;
        };
        if result.get("prefix").and_then(serde_json::Value::as_str) != Some(prefix.as_str()) {
            return true;
        }
        let Some(text) = result.get("text").and_then(serde_json::Value::as_str) else {
            return true;
        };
        let mut edit = self.ui.edit();
        if edit.set_ghost(&prefix, text) {
            self.ui.set_edit(edit);
        }
        true
    }

    fn drop_inline_completion(&mut self, id: TaskId) -> bool {
        if self.completion.tasks.remove(&id).is_none() {
            return false;
        }
        if self
            .completion
            .active
            .as_ref()
            .is_some_and(|cancel| cancel.id() == id)
        {
            self.completion.active = None;
        }
        true
    }

    fn execute_command_line(&mut self) -> Result<(), CoreError> {
        let input = self.ui.goto().target.trim().to_string();
        if input.is_empty() {
            return Ok(());
        }
        match input.as_str() {
            "q" | "quit" => self.request_quit(false),
            "q!" | "quit!" => self.request_quit(true),
            "w" | "write" => {
                if self.execute_cmd("file.save", serde_json::json!({}))?.ok {
                    self.dismiss_command_line()?;
                }
            }
            "wq" | "x" => {
                if self.execute_cmd("file.save", serde_json::json!({}))?.ok {
                    self.quit_after = self.last_queued;
                }
            }
            _ => {
                let (command, args) = match parse_command_line(&input) {
                    Ok(parsed) => parsed,
                    Err(err) => {
                        self.message = Some(err.message);
                        return Ok(());
                    }
                };
                if self.execute_cmd(&command, args)?.ok {
                    self.dismiss_command_line()?;
                }
            }
        }
        Ok(())
    }

    fn dismiss_command_line(&mut self) -> Result<(), CoreError> {
        self.execute_cmd("mode.normal", serde_json::json!({}))?;
        let mut goto = self.ui.goto();
        goto.target.clear();
        self.ui.set_goto(goto);
        Ok(())
    }

    /// Mouse click in terminal cells → selection (when `[tui] mouse`).
    pub fn step_mouse(&mut self, col: u16, row: u16) {
        self.step_mouse_with_modifiers(col, row, false, false);
    }

    /// Mouse selection with Ctrl-add and drag-extension semantics.
    pub fn step_mouse_with_modifiers(&mut self, col: u16, row: u16, ctrl: bool, drag: bool) {
        if !self.store.snapshot().config.tui.mouse {
            return;
        }
        let hit = self
            .last_grid
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .and_then(|layout| layout.hit(col, row));
        let Some((hit_row, hit_col)) = hit else {
            return;
        };
        let mut vp = self.ui.viewport();
        let mut sel = self.ui.selection();
        sel.cursor.row = hit_row;
        sel.cursor.col = hit_col;
        if drag {
            if let Some(active) = sel.areas.last_mut() {
                active.end = sel.cursor;
            }
            sel.extend = ExtendMode::Extend;
        } else if ctrl {
            if sel.areas.len() < 1_024 {
                sel.areas.push(Area::cell(sel.cursor));
            }
            sel.extend = ExtendMode::Add;
        } else {
            sel.replace(Area::cell(sel.cursor));
        }
        vp.ensure_row_visible(hit_row);
        vp.ensure_col_visible(hit_col);
        self.ui.set_viewport(vp);
        self.ui.set_selection(sel);
    }

    /// Scroll the grid, or zoom it when Ctrl accompanies a vertical wheel event.
    pub fn step_scroll(
        &mut self,
        horizontal: i32,
        vertical: i32,
        ctrl: bool,
    ) -> Result<(), CoreError> {
        if !self.store.snapshot().config.tui.mouse {
            return Ok(());
        }
        if ctrl && vertical != 0 {
            let delta = if vertical < 0 { 0.1 } else { -0.1 };
            self.execute_cmd("view.zoom", serde_json::json!({"delta": delta}))?;
            return Ok(());
        }
        let mut viewport = self.ui.viewport();
        let row = i64::from(viewport.first_row)
            .saturating_add(i64::from(vertical).saturating_mul(3))
            .clamp(
                i64::from(viewport.freeze.rows),
                i64::from(omacell_core::limits::MAX_ROWS - 1),
            );
        let col = i64::from(viewport.first_col)
            .saturating_add(i64::from(horizontal).saturating_mul(3))
            .clamp(
                i64::from(viewport.freeze.cols),
                i64::from(omacell_core::limits::MAX_COLS - 1),
            );
        viewport.first_row = row as u32;
        viewport.first_col = col as u16;
        self.ui.set_viewport(viewport);
        Ok(())
    }

    /// Alternate-screen event loop. Requires a TTY.
    pub fn run_crossterm(mut self) -> Result<(), CoreError> {
        if !stdout().is_terminal() {
            return Err(
                CoreError::new("tui.tty", "omacell --tui requires a terminal")
                    .with_hint("run from a TTY or omit --tui"),
            );
        }
        enable_raw_mode().map_err(|e| CoreError::new("tui.tty", e.to_string()))?;
        let mut restore = TerminalRestore {
            raw: true,
            alternate: false,
            mouse: false,
            focus: false,
        };
        let mouse = self.store.snapshot().config.tui.mouse;
        stdout()
            .execute(EnterAlternateScreen)
            .map_err(|e| CoreError::new("tui.tty", e.to_string()))?;
        restore.alternate = true;
        if mouse {
            stdout()
                .execute(EnableMouseCapture)
                .map_err(|e| CoreError::new("tui.tty", e.to_string()))?;
            restore.mouse = true;
        }
        stdout()
            .execute(EnableFocusChange)
            .map_err(|e| CoreError::new("tui.tty", e.to_string()))?;
        restore.focus = true;
        self.sync_ipc_focus(true)?;
        let backend = CrosstermBackend::new(stdout());
        let mut terminal =
            Terminal::new(backend).map_err(|e| CoreError::new("tui.tty", e.to_string()))?;
        self.event_loop(&mut terminal)
    }

    fn event_loop<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<(), CoreError> {
        loop {
            self.poll_reload()?;
            self.draw(terminal)
                .map_err(|e| CoreError::new("tui.draw", e.to_string()))?;
            if !event::poll(Duration::from_millis(50))
                .map_err(|e| CoreError::new("tui.input", e.to_string()))?
            {
                continue;
            }
            match event::read().map_err(|e| CoreError::new("tui.input", e.to_string()))? {
                CEvent::Key(key) => {
                    if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
                        continue;
                    }
                    if let Some(mapped) = map_key(key) {
                        self.step_key(mapped)?;
                        if self.quit_requested {
                            break;
                        }
                    }
                }
                CEvent::Mouse(mouse) => {
                    use crossterm::event::{KeyModifiers, MouseEventKind};
                    let ctrl = mouse.modifiers.contains(KeyModifiers::CONTROL);
                    let shift = mouse.modifiers.contains(KeyModifiers::SHIFT);
                    match mouse.kind {
                        MouseEventKind::ScrollUp if shift => self.step_scroll(-1, 0, false)?,
                        MouseEventKind::ScrollDown if shift => self.step_scroll(1, 0, false)?,
                        MouseEventKind::ScrollUp => self.step_scroll(0, -1, ctrl)?,
                        MouseEventKind::ScrollDown => self.step_scroll(0, 1, ctrl)?,
                        MouseEventKind::ScrollLeft => self.step_scroll(-1, 0, false)?,
                        MouseEventKind::ScrollRight => self.step_scroll(1, 0, false)?,
                        _ => {
                            if let Some((c, r, ctrl)) = map_mouse(mouse) {
                                let drag = matches!(mouse.kind, MouseEventKind::Drag(_));
                                self.step_mouse_with_modifiers(c, r, ctrl, drag);
                            }
                        }
                    }
                }
                CEvent::Resize(_, _) => {}
                CEvent::FocusGained => self.sync_ipc_focus(true)?,
                CEvent::FocusLost => self.sync_ipc_focus(false)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn sync_ipc_focus(&mut self, focused: bool) -> Result<(), CoreError> {
        if self.window_focused == Some(focused) {
            return Ok(());
        }
        self.window_focused = Some(focused);
        if let Some(ipc) = &self.ipc {
            ipc.set_focused(focused)?;
        }
        Ok(())
    }

    fn request_quit(&mut self, force: bool) {
        if force || !self.dirty || self.quit_armed {
            self.quit_requested = true;
            return;
        }
        self.quit_armed = true;
        self.message = Some("unsaved changes; press Ctrl+Q again to quit".into());
    }

    fn adopt_file_snapshot(&mut self) {
        self.reset_ai_workbook_state();
        let snapshot = self.runner.handle().snapshot();
        let sheet = snapshot.workbook.active_sheet();
        apply_sheet_view(&self.ui, &snapshot.workbook, sheet);
        self.active_sheet = sheet;
    }

    fn reset_ai_workbook_state(&mut self) {
        self.reset_autopilot("Autopilot reset for the new workbook.");
        if let Some(cancel) = self.completion.active.take() {
            cancel.cancel();
        }
        self.completion = InlineCompletion::default();
        self.ui.set_changeset_review(None);
        self.ui.set_formula_assist(None);
    }

    fn reset_autopilot(&mut self, message: &str) {
        if self.autopilot.take().is_some() {
            let mut agent = self.ui.agent_panel();
            agent.set_autopilot(
                false,
                "review required",
                0,
                self.ui.config().ai.agent.autopilot_max_ops as usize,
            );
            agent.push_turn(AgentRole::System, message);
            self.ui.set_agent_panel(agent);
        }
    }

    fn sync_active_sheet(&mut self) {
        self.adopt_snapshot();
    }
}

fn selection_a1(ui: &UiSession, wb: &Workbook) -> String {
    let selection = ui.selection();
    let sheet = wb
        .sheet(selection.sheet)
        .map(|sheet| sheet.name.as_str())
        .unwrap_or("Sheet1");
    format!(
        "{}!{}",
        quote_sheet_name(sheet),
        selection.active().to_range().to_a1()
    )
}

fn cursor_a1(ui: &UiSession, wb: &Workbook) -> String {
    let selection = ui.selection();
    let sheet = wb
        .sheet(selection.sheet)
        .map(|sheet| sheet.name.as_str())
        .unwrap_or("Sheet1");
    format!("{}!{}", quote_sheet_name(sheet), selection.cursor.to_a1())
}

struct TerminalRestore {
    raw: bool,
    alternate: bool,
    mouse: bool,
    focus: bool,
}

impl Drop for TerminalRestore {
    fn drop(&mut self) {
        if self.focus {
            let _ = stdout().execute(DisableFocusChange);
        }
        if self.mouse {
            let _ = stdout().execute(DisableMouseCapture);
        }
        if self.alternate {
            let _ = stdout().execute(LeaveAlternateScreen);
        }
        if self.raw {
            let _ = disable_raw_mode();
        }
        let _ = stdout().execute(Show);
    }
}

fn has_missing_required_args(command: &CommandJson, args: &serde_json::Value) -> bool {
    command
        .arg_schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|required| {
            required
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|name| args.get(name).is_none_or(serde_json::Value::is_null))
        })
}

fn parse_command_line(input: &str) -> Result<(String, serde_json::Value), CoreError> {
    if let Some(path) = input
        .strip_prefix("e ")
        .or_else(|| input.strip_prefix("open "))
    {
        return Ok(("file.open".into(), serde_json::json!({"path": path.trim()})));
    }
    if let Some(range) = input.strip_prefix("goto ") {
        return Ok((
            "view.select".into(),
            serde_json::json!({"range": range.trim()}),
        ));
    }
    let (command, raw_args) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    let args = if raw_args.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(raw_args.trim()).map_err(|err| {
            CoreError::new("tui.command", format!("invalid JSON arguments: {err}"))
        })?
    };
    if !args.is_object() {
        return Err(CoreError::new(
            "tui.command",
            "command arguments must be a JSON object",
        ));
    }
    Ok((command.to_string(), args))
}

fn command_changes_workbook(command: &str, outcome: &Outcome, registered_mutating: bool) -> bool {
    if outcome
        .result
        .as_ref()
        .and_then(|result| result.get("changed"))
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|changed| changed > 0)
    {
        return true;
    }
    if matches!(command, "edit.undo" | "edit.redo") {
        return true;
    }
    if !registered_mutating
        || matches!(
            command.split_once('.').map(|(prefix, _)| prefix),
            Some("nav" | "sel" | "view" | "mode" | "palette" | "help" | "edit")
        )
        || matches!(
            command,
            "sheet.next"
                | "sheet.prev"
                | "changeset.review"
                | "file.open"
                | "file.save"
                | "file.saveas"
                | "file.new"
                | "file.close"
                | "file.export"
                | "theme.reload"
        )
    {
        return false;
    }
    true
}

fn autopilot_scope_label(scope: &AutopilotScope, workbook: &Workbook) -> String {
    match scope {
        AutopilotScope::Workbook => "workbook".into(),
        AutopilotScope::Sheet(sheet) => workbook.sheet(*sheet).map_or_else(
            || format!("sheet {}", sheet.index()),
            |sheet| sheet.name.clone(),
        ),
        AutopilotScope::Range {
            sheet,
            min_row,
            min_col,
            max_row,
            max_col,
        } => {
            let name = workbook.sheet(*sheet).map_or_else(
                || format!("Sheet{}", sheet.index()),
                |sheet| sheet.name.clone(),
            );
            let start = omacell_core::addr::col_to_letters(*min_col)
                .map_or_else(|_| "?".into(), |col| format!("{col}{}", min_row + 1));
            let end = omacell_core::addr::col_to_letters(*max_col)
                .map_or_else(|_| "?".into(), |col| format!("{col}{}", max_row + 1));
            format!("range {name}!{start}:{end}")
        }
    }
}

fn discard_confirmation(command: &str) -> Outcome {
    Outcome::failure(CoreError::new(
        "file.unsaved",
        format!("unsaved changes; run {command} again to discard them"),
    ))
}

struct FrontendScriptUi {
    ui: UiSession,
    known: BTreeSet<String>,
}

impl InteractiveUi for FrontendScriptUi {
    fn keymap_set(&self, mode: &str, keys: &str, cmd: &str) -> Result<(), CoreError> {
        self.ui.set_script_binding_ids(mode, keys, cmd, &self.known)
    }

    fn clear_keymap(&self) {
        self.ui.clear_script_bindings();
    }
}

fn inject_selection_context(
    ui: &UiSession,
    cmd: &str,
    mut args: serde_json::Value,
) -> serde_json::Value {
    if matches!(cmd, "chart.fromselection" | "name.createfrom")
        && args
            .get("range")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .is_empty()
    {
        args["range"] = serde_json::json!(ui.selection().active().to_range().to_a1());
    }
    if cmd.starts_with("ai.formula.")
        && args
            .get("ref")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .is_empty()
    {
        args["ref"] = serde_json::json!(ui.selection().cursor.to_a1());
    }
    args
}

fn formula_task(command: &str, args: &serde_json::Value) -> Option<FormulaTask> {
    command.starts_with("ai.formula.").then(|| FormulaTask {
        command: command.to_string(),
        target: args
            .get("ref")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("A1")
            .to_string(),
    })
}

fn apply_sheet_view(ui: &UiSession, workbook: &Workbook, id: SheetId) {
    let Some(sheet) = workbook.sheet(id) else {
        return;
    };
    let mut viewport = ui.viewport();
    viewport.first_row = sheet.view.scroll_row;
    viewport.first_col = sheet.view.scroll_col;
    viewport.set_zoom(sheet.view.zoom);
    viewport.freeze = sheet.view.freeze;
    viewport.split = sheet.view.split;
    viewport.rows = sheet.geometry.rows.clone();
    viewport.cols = sheet.geometry.cols.clone();
    ui.set_viewport(viewport);

    let mut start = sheet.view.selection.start;
    let mut end = sheet.view.selection.end;
    start.sheet = Some(id);
    end.sheet = Some(id);
    let mut selection = ui.selection();
    selection.sheet = id;
    selection.replace(Area { start, end });
    ui.set_selection(selection);
    ui.set_show_formulas(sheet.view.show_formulas);
}

/// Run the TUI on the current terminal.
pub fn run(launch: Launch) -> Result<(), CoreError> {
    Tui::new(launch, true)?.run_crossterm()
}
