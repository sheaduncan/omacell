//! eframe application over the WP-13 composition root and WP-15a runner.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use omacell_ai::{AutopilotPolicy, AutopilotScope, Plan, to_calls};
use omacell_bus::ipc::{IpcHandle, default_runtime_dir, serve_runner};
use omacell_bus::{
    Bus, CancelHandle, CommandJson, CommandsEnvelope, LongOps, TaskEvent, TaskId, TaskRunner,
    TaskRunnerHandle,
};
use omacell_conf::{ConfigStore, LoadedConfig, Paths, ReloadEvent};
use omacell_core::addr::{SheetId, col_to_letters, parse_a1_cell, quote_sheet_name};
use omacell_core::changeset::{ChangesetId, ChangesetStatus};
use omacell_core::command::{Origin, Outcome};
use omacell_core::error::CoreError;
use omacell_core::print::paginate;
use omacell_core::{PRODUCT_DISPLAY_NAME, PRODUCT_NAME};
use omacell_lua::{InteractiveRuntime, InteractiveUi};
use omacell_ui::{
    AgentRole, Area, ChangesetReview, EditSurface, FormulaAssist, KeyCode, KeyEvent, KeyOutcome,
    KeymapRoots, SessionState, UiSession, apply_local_command, apply_search_result,
};
use serde_json::json;

use crate::chrome;
use crate::grid::{self, GridLayout};
use crate::input;
use crate::theme::{GuiTheme, install_font};

/// Objects the CLI composition root hands to the GUI. No second config load.
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
    /// Long-operation classifier.
    pub long_ops: LongOps,
    /// Workbook path from `omacell [file]`, if any.
    pub file: Option<PathBuf>,
    /// Load `LoadedConfig.shell.ui_font_path` into egui. Tests keep bundled fonts.
    pub use_shell_font: bool,
}

enum PointerDrag {
    Select { start: (u32, u16) },
    Move { press: (u32, u16) },
    Fill { start: (u32, u16) },
    ColResize { col: u16, start_x: f32, orig: u32 },
    RowResize { row: u32, start_y: f32, orig: u32 },
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

/// Running GUI. Tests drive [`Self::ui_frame`].
pub struct Gui {
    paths: Paths,
    store: ConfigStore,
    runner: TaskRunner,
    scripts: InteractiveRuntime,
    ui: UiSession,
    roots: KeymapRoots,
    theme: GuiTheme,
    catalog: Vec<CommandJson>,
    message: Option<String>,
    script_status: Option<String>,
    dirty: bool,
    discard_armed: Option<String>,
    close_requested: bool,
    active_sheet: SheetId,
    grid: GridLayout,
    palette_index: usize,
    palette_command: Option<String>,
    palette_plan_task: Option<TaskId>,
    autopilot: Option<AutopilotPolicy>,
    formula_tasks: BTreeMap<TaskId, FormulaTask>,
    completion: InlineCompletion,
    context_menu: Option<egui::Pos2>,
    file: Option<PathBuf>,
    use_shell_font: bool,
    drag: Option<PointerDrag>,
    focused_cancel: Option<CancelHandle>,
    last_title: String,
    print_preview: bool,
    _ipc: Option<IpcHandle>,
}

impl Gui {
    /// Wrap a launch. `ipc` starts the in-process socket used by the theme hook.
    pub fn new(launch: Launch, ipc: bool, ctx: &egui::Context) -> Result<Self, CoreError> {
        let requested_file = launch.file.clone();
        let loaded = launch.store.snapshot();
        let catalog = catalog_from_bus(&launch.bus)?;
        let runner = TaskRunner::spawn(launch.bus, launch.long_ops)?;
        let config_repaint = ctx.clone();
        launch
            .store
            .set_event_waker(move || config_repaint.request_repaint());
        let task_repaint = ctx.clone();
        runner
            .handle()
            .set_event_waker(move || task_repaint.request_repaint());
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
        let startup_message = scripts.take_messages().into_iter().last();
        let script_status = startup_message.clone();
        let (message, focused_cancel) = if let Some(path) = requested_file {
            let (_, cancel) = runner.handle().submit(
                Origin::User,
                "file.open",
                json!({"path": path.display().to_string()}),
            )?;
            (
                startup_message.or_else(|| Some("opening…".into())),
                Some(cancel),
            )
        } else {
            (startup_message, None)
        };
        let ipc_handle = if ipc {
            Some(serve_runner(default_runtime_dir(), runner.handle())?)
        } else {
            None
        };
        let snapshot = runner.handle().snapshot();
        let mut active_sheet = snapshot.workbook.active_sheet();
        apply_sheet_view(&launch.ui, &snapshot.workbook, active_sheet);
        if let Ok(state) = SessionState::load(&launch.paths.state_dir) {
            apply_restored_session(&launch.ui, &snapshot.workbook, &state);
            active_sheet = launch.ui.selection().sheet;
            launch.ui.set_session_state(state);
        }
        let theme = GuiTheme::from_loaded(&loaded.theme, &loaded.shell);
        if launch.use_shell_font {
            install_font(ctx, loaded.shell.ui_font_path.as_deref());
        }
        theme.apply_visuals(ctx);
        ctx.tessellation_options_mut(|opts| {
            opts.feathering = false;
        });
        Ok(Self {
            paths: launch.paths,
            store: launch.store,
            runner,
            scripts,
            ui: launch.ui,
            roots: launch.roots,
            theme,
            catalog,
            message,
            script_status,
            dirty: false,
            discard_armed: None,
            close_requested: false,
            active_sheet,
            grid: GridLayout::default(),
            palette_index: 0,
            palette_command: None,
            palette_plan_task: None,
            autopilot: None,
            formula_tasks: BTreeMap::new(),
            completion: InlineCompletion::default(),
            context_menu: None,
            file: None,
            use_shell_font: launch.use_shell_font,
            drag: None,
            focused_cancel,
            last_title: String::new(),
            print_preview: false,
            _ipc: ipc_handle,
        })
    }

    /// WP-14 session.
    #[must_use]
    pub fn ui_session(&self) -> &UiSession {
        &self.ui
    }

    /// Config store (reload tests).
    #[must_use]
    pub fn store(&self) -> &ConfigStore {
        &self.store
    }

    /// XDG paths used for session restore and theme fixtures.
    #[must_use]
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// Task runner handle.
    #[must_use]
    pub fn runner(&self) -> TaskRunnerHandle {
        self.runner.handle()
    }

    /// Last status message.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Whether the current workbook has unsaved frontend mutations.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Whether a file-close command asked the native window to close.
    #[must_use]
    pub fn close_requested(&self) -> bool {
        self.close_requested
    }

    /// Resolved theme name (status / reload tests).
    #[must_use]
    pub fn theme_name(&self) -> &str {
        &self.theme.name
    }

    /// Window title.
    #[must_use]
    pub fn title(&self) -> String {
        let file = self
            .file
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("untitled");
        if self.dirty {
            format!("• {file} — {PRODUCT_DISPLAY_NAME}")
        } else {
            format!("{file} — {PRODUCT_DISPLAY_NAME}")
        }
    }

    /// Apply pending config/task events without resetting the session.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.poll_tasks();
        let events = self.store.drain_events();
        if events.is_empty() {
            return;
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
                    self.rebuild_theme(&snapshot, ctx);
                    if let ReloadEvent::ThemeChanged { name } = ev {
                        self.message = Some(format!("theme {name}"));
                    }
                }
            }
        }
    }

    fn rebuild_theme(&mut self, loaded: &LoadedConfig, ctx: &egui::Context) {
        if self.use_shell_font {
            install_font(ctx, loaded.shell.ui_font_path.as_deref());
        }
        self.theme = GuiTheme::from_loaded(&loaded.theme, &loaded.shell);
        self.theme.apply_visuals(ctx);
        ctx.tessellation_options_mut(|opts| {
            opts.feathering = false;
        });
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
                        self.request_close();
                    } else if state.command == "file.open" {
                        self.adopt_opened_snapshot();
                    } else if state.command == "file.new" {
                        self.adopt_new_snapshot();
                    } else if matches!(state.command.as_str(), "sheet.next" | "sheet.prev") {
                        self.adopt_snapshot();
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
                        let message = self
                            .message
                            .clone()
                            .unwrap_or_else(|| "agent turn failed".into());
                        self.agent_failed(&message);
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
                TaskEvent::Running(_) | TaskEvent::Queued(_) | TaskEvent::Cancelling(_) => {}
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

    fn adopt_snapshot(&mut self) {
        let snapshot = self.runner.handle().snapshot();
        let sheet = snapshot.workbook.active_sheet();
        if sheet != self.active_sheet {
            apply_sheet_view(&self.ui, &snapshot.workbook, sheet);
            self.active_sheet = sheet;
        }
    }

    fn adopt_opened_snapshot(&mut self) {
        self.reset_ai_workbook_state();
        let snapshot = self.runner.handle().snapshot();
        let state = self.ui.session_state();
        apply_sheet_view(
            &self.ui,
            &snapshot.workbook,
            snapshot.workbook.active_sheet(),
        );
        apply_restored_session(&self.ui, &snapshot.workbook, &state);
        self.active_sheet = self.ui.selection().sheet;
    }

    fn adopt_new_snapshot(&mut self) {
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

    /// Handle one toolkit-neutral key.
    pub fn step_key(&mut self, event: KeyEvent) -> Result<KeyOutcome, CoreError> {
        if event.code == KeyCode::Esc
            && let Some(handle) = self
                .runner
                .handle()
                .running_cancel()
                .or_else(|| self.focused_cancel.clone())
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
        if self.ui.panel().visible.as_deref() == Some("find") {
            return self.step_find_panel(event);
        }
        if self.ui.panel().visible.as_deref() == Some("changeset") {
            return self.step_changeset_review(event);
        }
        if self.ui.panel().visible.as_deref() == Some("formula") {
            return self.step_formula_panel(event);
        }
        if self.ui.panel().visible.as_deref() == Some("agent") {
            return self.step_agent_panel(event);
        }
        let outcome = self.ui.handle_key(event);
        if let KeyOutcome::Command { cmd, args, .. } = outcome.clone() {
            self.execute_cmd(&cmd, args)?;
        } else {
            self.discard_armed = None;
        }
        Ok(outcome)
    }

    fn step_find_panel(&mut self, event: KeyEvent) -> Result<KeyOutcome, CoreError> {
        match (event.code, event.ctrl, event.alt) {
            (KeyCode::Esc, false, false) => {
                let mut panel = self.ui.panel();
                panel.dismiss();
                self.ui.set_panel(panel);
            }
            (KeyCode::Backspace, false, false) => {
                let mut find = self.ui.find_replace();
                find.find.pop();
                self.ui.set_find_replace(find);
            }
            (KeyCode::Char(c), false, false) => {
                let mut find = self.ui.find_replace();
                find.find.push(c);
                self.ui.set_find_replace(find);
            }
            (KeyCode::Space, false, false) => {
                let mut find = self.ui.find_replace();
                find.find.push(' ');
                self.ui.set_find_replace(find);
            }
            (KeyCode::Enter, false, false) if !self.ui.find_replace().find.is_empty() => {
                let outcome = self.execute_cmd("edit.searchnext", json!({}))?;
                if outcome.ok {
                    let mut panel = self.ui.panel();
                    panel.dismiss();
                    self.ui.set_panel(panel);
                }
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
        let proposed = value.get("proposed").cloned().unwrap_or_else(|| json!([]));
        let plan: Plan = serde_json::from_value(json!({"commands": proposed}))
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
                    let outcome = self
                        .execute_cmd("ai.agent.turn", json!({"prompt": prompt, "apply": false}))?;
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
            args: json!({"ref": task.target, "input": formula}),
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

    fn sync_inline_completion(&mut self) -> Option<Duration> {
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
        let mut schedule_repaint = false;
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
            schedule_repaint = self.completion.due.is_some();
            if edit.ghost.is_some() {
                let mut cleared = edit;
                cleared.ghost = None;
                self.ui.set_edit(cleared);
            }
        }
        let (due, prefix) = self.completion.due.clone()?;
        let now = Instant::now();
        if now < due {
            return schedule_repaint.then(|| due.saturating_duration_since(now));
        }
        self.completion.due = None;
        if let Ok((id, cancel)) =
            self.runner
                .handle()
                .submit(Origin::User, "ai.complete", json!({"prefix": prefix}))
        {
            self.completion.tasks.insert(id, prefix);
            self.completion.active = Some(cancel);
        }
        None
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

    /// Run a registry command as the interactive user.
    pub fn execute_cmd(
        &mut self,
        cmd: &str,
        args: serde_json::Value,
    ) -> Result<Outcome, CoreError> {
        let args = inject_selection_context(&self.ui, cmd, args);
        if self.prompt_command_args(cmd, &args) {
            return Ok(Outcome::success(json!({"prompt": true})));
        }
        if cmd == "file.close" {
            if !self.confirm_discard(cmd) {
                return Ok(discard_confirmation(cmd));
            }
            self.ui.remember_command(cmd);
            self.message = None;
            self.close_requested = true;
            return Ok(Outcome::success(json!({"close": true})));
        }
        if matches!(cmd, "file.new" | "file.open") && !self.confirm_discard(cmd) {
            return Ok(discard_confirmation(cmd));
        }
        if !matches!(cmd, "file.new" | "file.open") {
            self.discard_armed = None;
        }
        let handle = self.runner.handle();
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
                self.refresh_palette(query);
                self.palette_command = None;
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
            return Ok(Outcome::success(json!({"ok": true})));
        }
        if cmd == "file.print" {
            self.toggle_print_preview();
        }
        if let Err(error) = self.scripts.before_command(cmd) {
            self.message = Some(format!("{}: {}", error.code, error.message));
            return Ok(Outcome::failure(error));
        }
        match handle.submit(Origin::User, cmd, args) {
            Ok((id, cancel)) => {
                if let Some(task) = formula_task {
                    self.formula_tasks.insert(id, task);
                }
                self.focused_cancel = Some(cancel);
                self.message = Some(if handle.long_ops().contains(cmd) {
                    "working…".into()
                } else {
                    "queued…".into()
                });
                Ok(Outcome::success(json!({"queued": true, "task": id.get()})))
            }
            Err(err) => {
                self.message = Some(err.message.clone());
                Ok(Outcome::failure(err))
            }
        }
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
                json!({"ref": diagnostic_ref}),
            );
            if !outcome.ok {
                self.message = outcome.error.map(|error| error.message);
                return;
            }
            let mut bundle = json!({
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

    fn step_palette(&mut self, event: KeyEvent) -> Result<KeyOutcome, CoreError> {
        if self.palette_command.is_some() {
            return self.step_palette_args(event);
        }
        match event.code {
            KeyCode::Esc => self.close_palette(),
            KeyCode::Enter => {
                let palette = self.ui.palette();
                if let Some(prompt) = palette.query.strip_prefix('?').map(str::trim)
                    && !prompt.is_empty()
                {
                    self.submit_palette_plan(prompt.to_string())?;
                    return Ok(KeyOutcome::Pending);
                }
                if let Some(hit) = palette.hits.get(self.palette_index) {
                    let id = hit.id.clone();
                    self.choose_palette(&id)?;
                }
            }
            KeyCode::Down => {
                let n = self.ui.palette().hits.len();
                if n > 0 {
                    self.palette_index = (self.palette_index + 1).min(n - 1);
                }
            }
            KeyCode::Up => self.palette_index = self.palette_index.saturating_sub(1),
            KeyCode::Backspace => {
                let mut palette = self.ui.palette();
                palette.query.pop();
                let q = palette.query.clone();
                self.ui.set_palette(palette);
                self.refresh_palette(&q);
            }
            KeyCode::Char(c) => {
                let mut palette = self.ui.palette();
                palette.query.push(c);
                let q = palette.query.clone();
                self.ui.set_palette(palette);
                self.refresh_palette(&q);
            }
            KeyCode::Space => {
                let mut palette = self.ui.palette();
                palette.query.push(' ');
                let q = palette.query.clone();
                self.ui.set_palette(palette);
                self.refresh_palette(&q);
            }
            _ => {}
        }
        Ok(KeyOutcome::Pending)
    }

    fn step_palette_args(&mut self, event: KeyEvent) -> Result<KeyOutcome, CoreError> {
        match event.code {
            KeyCode::Esc => self.close_palette(),
            KeyCode::Enter => {
                let text = self.ui.palette().query;
                let args = match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(serde_json::Value::Object(fields)) => serde_json::Value::Object(fields),
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
                    let outcome = self.execute_cmd(&id, args)?;
                    if outcome.ok {
                        self.close_palette();
                    }
                }
            }
            KeyCode::Backspace => {
                let mut palette = self.ui.palette();
                palette.query.pop();
                self.ui.set_palette(palette);
            }
            KeyCode::Char(c) => {
                let mut palette = self.ui.palette();
                palette.query.push(c);
                self.ui.set_palette(palette);
            }
            KeyCode::Space => {
                let mut palette = self.ui.palette();
                palette.query.push(' ');
                self.ui.set_palette(palette);
            }
            _ => {}
        }
        Ok(KeyOutcome::Pending)
    }

    fn choose_palette(&mut self, id: &str) -> Result<(), CoreError> {
        let args = inject_selection_context(&self.ui, id, json!({}));
        if self.prompt_command_args(id, &args) {
            return Ok(());
        }
        self.close_palette();
        let _ = self.execute_cmd(id, args)?;
        Ok(())
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

    fn request_close(&mut self) {
        if self.confirm_discard("file.close") {
            self.close_requested = true;
        }
    }

    fn submit_palette_plan(&mut self, prompt: String) -> Result<(), CoreError> {
        let (id, cancel) = self.runner.handle().submit(
            Origin::User,
            "ai.plan",
            json!({"prompt": prompt, "apply": false}),
        )?;
        self.palette_plan_task = Some(id);
        self.focused_cancel = Some(cancel);
        let mut palette = self.ui.palette();
        palette.prompt = Some("Planning…".into());
        palette.preview = None;
        self.ui.set_palette(palette);
        Ok(())
    }

    fn close_palette(&mut self) {
        let mut palette = self.ui.palette();
        palette.close();
        self.ui.set_palette(palette);
        self.palette_index = 0;
        self.palette_command = None;
    }

    fn refresh_palette(&mut self, query: &str) {
        self.ui.rank_palette(&self.catalog, query, None);
        self.palette_index = 0;
    }

    /// One egui frame (eframe + kittest).
    pub fn ui_frame(&mut self, ctx: &egui::Context) {
        self.poll(ctx);
        if !self.close_requested
            && ctx.input(|input| input.viewport().close_requested())
            && self.dirty
        {
            if self.confirm_discard("file.close") {
                self.close_requested = true;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
        let title = self.title();
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        let snapshot = self.runner.handle().snapshot();
        let busy = self.runner.handle().is_busy();
        let cfg = self.ui.config();
        let compact = ctx.screen_rect().width() < cfg.layout.compact_below_width as f32;
        let edit = self.ui.edit();
        let find_panel_open = self.ui.panel().visible.as_deref() == Some("find");
        let input = ctx.input(|i| i.clone());
        for key in input::pressed_keys(&input.events) {
            if toolkit_owns_key(&edit, &key)
                || find_panel_open
                    && !key.ctrl
                    && !key.alt
                    && matches!(key.code, KeyCode::Char(_) | KeyCode::Space)
            {
                continue;
            }
            let _ = self.step_key(key);
        }
        if (!self.ui.edit().is_idle() && self.ui.edit().surface == EditSurface::InCell)
            || find_panel_open
        {
            for text in input::text_events(&input.events) {
                for c in text.chars() {
                    let _ = self.step_key(KeyEvent::new(KeyCode::Char(c)));
                }
            }
        }
        if input.modifiers.ctrl && input.raw_scroll_delta.y.abs() > 0.0 {
            let delta = if input.raw_scroll_delta.y > 0.0 {
                0.1
            } else {
                -0.1
            };
            let _ = self.execute_cmd("view.zoom", json!({"delta": delta}));
        } else if input.raw_scroll_delta.x.abs() > 0.0
            || (input.modifiers.shift && input.raw_scroll_delta.y.abs() > 0.0)
        {
            let delta = if input.raw_scroll_delta.x.abs() > 0.0 {
                input.raw_scroll_delta.x
            } else {
                input.raw_scroll_delta.y
            };
            let mut vp = self.ui.viewport();
            let cols: i16 = if delta > 0.0 { -1 } else { 1 };
            vp.first_col = vp.first_col.saturating_add_signed(cols);
            self.ui.set_viewport(vp);
        } else if input.raw_scroll_delta.y.abs() > 0.0 {
            let mut vp = self.ui.viewport();
            let rows = if input.raw_scroll_delta.y > 0.0 {
                -3
            } else {
                3
            };
            vp.first_row = vp.first_row.saturating_add_signed(rows);
            self.ui.set_viewport(vp);
        }

        if cfg.layout.menu_bar && !compact {
            let mut picked = None;
            egui::TopBottomPanel::top("omacell-menu").show(ctx, |ui| {
                picked = chrome::menu_bar(ui);
            });
            if let Some(cmd) = picked {
                if cmd == "edit.copy" {
                    self.execute_if_available(cmd, "WP-17");
                } else {
                    let _ = self.execute_cmd(cmd, json!({}));
                }
            }
        }

        if cfg.appearance.show_sheet_tabs && !compact {
            let mut selected = None;
            egui::TopBottomPanel::top("omacell-tabs").show(ctx, |ui| {
                selected = chrome::tabs(ui, &snapshot.workbook, &self.ui, &self.theme);
            });
            if let Some(sheet) = selected {
                self.activate_sheet(&snapshot.workbook, sheet);
            }
        }
        if cfg.appearance.show_formula_bar {
            egui::TopBottomPanel::top("omacell-fx").show(ctx, |ui| {
                if let Some(text) =
                    chrome::formula_bar(ui, &snapshot.workbook, &self.ui, &self.theme)
                    && !self.ui.edit().is_idle()
                {
                    let mut edit = self.ui.edit();
                    edit.replace_from_toolkit(text);
                    self.ui.set_edit(edit);
                }
            });
        }
        if cfg.appearance.show_status_line {
            egui::TopBottomPanel::bottom("omacell-status").show(ctx, |ui| {
                chrome::status(
                    ui,
                    &snapshot.workbook,
                    &self.ui,
                    &self.theme,
                    self.dirty,
                    self.message.as_deref(),
                    busy,
                );
            });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            chrome::panel(ui, &self.ui.panel(), &self.ui, &self.theme);
            let a11y = grid::cell_a11y(&snapshot.workbook, &self.ui);
            self.grid = grid::paint(
                ui,
                &snapshot.workbook,
                &snapshot.spill,
                &self.ui,
                &self.theme,
                ctx.pixels_per_point(),
                &a11y,
            );
            let sheet = self.ui.selection().sheet;
            if let Some(ws) = snapshot.workbook.sheet(sheet) {
                let theme = self.theme.chart_theme();
                for chart in &ws.charts {
                    let from = self
                        .grid
                        .cell_rect(chart.anchor.from_row, chart.anchor.from_col);
                    let to = self
                        .grid
                        .cell_rect(chart.anchor.to_row, chart.anchor.to_col);
                    if let (Some(a), Some(b)) = (from, to) {
                        let rect = egui::Rect::from_two_pos(a.min, b.max);
                        if let Ok(scene) = omacell_core::chart::layout_chart(
                            &snapshot.workbook,
                            chart,
                            &theme,
                            480.0,
                            280.0,
                        ) {
                            crate::chart::paint(ui, &scene, rect);
                        }
                    }
                }
            }
            if self.print_preview
                && let Some(ws) = snapshot.workbook.sheet(sheet)
                && let Ok(pages) = paginate(ws, &ws.page_setup)
            {
                let painter = ui.painter();
                for page in pages {
                    if page.row1 >= page.row0 && page.col1 >= page.col0 {
                        let from = self.grid.cell_rect(page.row0, page.col0);
                        let to = self.grid.cell_rect(page.row1, page.col1);
                        if let (Some(a), Some(b)) = (from, to) {
                            painter.rect_stroke(
                                egui::Rect::from_two_pos(a.min, b.max),
                                0.0,
                                egui::Stroke::new(1.5_f32, self.theme.warning),
                                egui::StrokeKind::Inside,
                            );
                        }
                    }
                }
            }
        });

        let double = input
            .pointer
            .button_double_clicked(egui::PointerButton::Primary);
        if let Some((pos, ctrl, shift)) = input::pointer_press(&input.events) {
            self.handle_press(pos, ctrl, shift, double);
        }
        if let Some(pos) = input::pointer_moved(&input.events) {
            self.handle_drag(pos);
        }
        if let Some((pos, ctrl)) = input::pointer_release(&input.events) {
            self.handle_release(pos, ctrl);
        }
        if let Some(pos) = input::pointer_secondary(&input.events) {
            self.context_menu = Some(pos);
        }
        if let Some(pos) = self.context_menu {
            let mut close = false;
            egui::Area::new(egui::Id::new("omacell-context"))
                .fixed_pos(pos)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        if ui.button("Copy").clicked() {
                            self.execute_if_available("edit.copy", "WP-17");
                            close = true;
                        }
                        if ui.button("Paste").clicked() {
                            self.execute_if_available("edit.paste", "WP-17");
                            close = true;
                        }
                    });
                });
            if close {
                self.context_menu = None;
            }
        }

        if self.ui.palette().open
            && let Some(id) =
                chrome::palette(ctx, &self.ui.palette(), self.palette_index, &self.theme)
        {
            let _ = self.choose_palette(&id);
        }
        if self.close_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if let Some(wait) = self.sync_inline_completion() {
            ctx.request_repaint_after(wait);
        }
    }

    fn toggle_print_preview(&mut self) {
        if self.print_preview {
            self.print_preview = false;
            self.message = Some("print preview off".into());
            return;
        }
        let snap = self.runner.handle().snapshot();
        let Some(sheet) = snap.workbook.sheet(self.active_sheet) else {
            return;
        };
        match paginate(sheet, &sheet.page_setup) {
            Ok(pages) => {
                self.message = Some(format!("print preview · {} page(s)", pages.len()));
                self.print_preview = true;
            }
            Err(err) => self.message = Some(err.message),
        }
    }

    fn execute_if_available(&mut self, command: &str, owner: &str) {
        if self.catalog.iter().any(|entry| entry.id == command) {
            let _ = self.execute_cmd(command, json!({}));
        } else {
            self.message = Some(format!("{command} arrives in {owner}"));
        }
    }

    fn activate_sheet(&mut self, workbook: &omacell_core::workbook::Workbook, id: SheetId) {
        let Some(sheet) = workbook.sheet(id) else {
            return;
        };
        apply_sheet_view(&self.ui, workbook, id);
        self.active_sheet = id;
        let range = format!(
            "{}!{}",
            quote_sheet_name(&sheet.name),
            sheet.view.selection.to_a1()
        );
        match self
            .runner
            .handle()
            .submit(Origin::User, "view.select", json!({"range": range}))
        {
            Ok((_, cancel)) => self.focused_cancel = Some(cancel),
            Err(err) => self.message = Some(err.message),
        }
    }

    fn handle_press(&mut self, pos: egui::Pos2, ctrl: bool, shift: bool, double: bool) {
        let vp = self.ui.viewport();
        if self.grid.in_fill_handle(pos) {
            let sel = self.ui.selection();
            self.drag = Some(PointerDrag::Fill {
                start: (sel.cursor.row, sel.cursor.col),
            });
            return;
        }
        if let Some(col) = self.grid.col_edge(pos) {
            if double {
                let width =
                    grid::autofit_col_px(&self.runner.handle().snapshot().workbook, &self.ui, col);
                let mut vp = self.ui.viewport();
                let _ = vp.cols.set_size(u32::from(col), width);
                self.ui.set_viewport(vp);
                return;
            }
            let orig = vp.col_px(col);
            self.drag = Some(PointerDrag::ColResize {
                col,
                start_x: pos.x,
                orig,
            });
            return;
        }
        if let Some(row) = self.grid.row_edge(pos) {
            if double {
                let mut vp = self.ui.viewport();
                let _ = vp.rows.set_size(row, DEFAULT_FIT_ROW);
                self.ui.set_viewport(vp);
                return;
            }
            let orig = vp.row_px(row);
            self.drag = Some(PointerDrag::RowResize {
                row,
                start_y: pos.y,
                orig,
            });
            return;
        }
        if self.grid.in_row_header(pos) {
            if let Some((row, _)) = self.grid.hit(pos, &vp) {
                let mut sel = self.ui.selection();
                sel.cursor.row = row;
                sel.select_row();
                self.ui.set_selection(sel);
            }
            return;
        }
        if self.grid.in_col_header(pos) {
            if let Some((_, col)) = self.grid.hit(pos, &vp) {
                let mut sel = self.ui.selection();
                sel.cursor.col = col;
                sel.select_col();
                self.ui.set_selection(sel);
            }
            return;
        }
        let Some((row, col)) = self.grid.hit(pos, &vp) else {
            return;
        };
        let sel = self.ui.selection();
        let (r0, c0, r1, c1) = sel.active().normalized();
        let inside = row >= r0 && row <= r1 && col >= c0 && col <= c1 && (r1 > r0 || c1 > c0);
        if inside && !shift {
            self.drag = Some(PointerDrag::Move { press: (row, col) });
            return;
        }
        let mut sel = sel;
        sel.cursor.row = row;
        sel.cursor.col = col;
        if ctrl {
            sel.extend = omacell_ui::ExtendMode::Add;
        } else if shift {
            sel.extend = omacell_ui::ExtendMode::Extend;
        } else {
            sel.extend = omacell_ui::ExtendMode::Replace;
        }
        if sel.extend == omacell_ui::ExtendMode::Extend {
            if let Some(active) = sel.areas.last_mut() {
                active.end = sel.cursor;
            }
        } else if sel.extend == omacell_ui::ExtendMode::Add {
            sel.areas.push(Area::cell(sel.cursor));
            sel.extend = omacell_ui::ExtendMode::Extend;
        } else {
            sel.replace(Area::cell(sel.cursor));
        }
        self.ui.set_selection(sel);
        self.drag = Some(PointerDrag::Select { start: (row, col) });
    }

    fn handle_drag(&mut self, pos: egui::Pos2) {
        let Some(drag) = &self.drag else {
            return;
        };
        match *drag {
            PointerDrag::Select { start } | PointerDrag::Fill { start } => {
                let vp = self.ui.viewport();
                let Some((row, col)) = self.grid.hit(pos, &vp) else {
                    return;
                };
                let mut sel = self.ui.selection();
                sel.cursor.row = start.0;
                sel.cursor.col = start.1;
                sel.extend = omacell_ui::ExtendMode::Extend;
                if let Some(active) = sel.areas.last_mut() {
                    active.start.row = start.0;
                    active.start.col = start.1;
                    active.end.row = row;
                    active.end.col = col;
                }
                sel.cursor.row = row;
                sel.cursor.col = col;
                self.ui.set_selection(sel);
            }
            PointerDrag::Move { .. } => {}
            PointerDrag::ColResize { col, start_x, orig } => {
                let delta = ((pos.x - start_x) / self.ui.viewport().zoom.max(0.25) as f32).round();
                let width = (orig as i32 + delta as i32).clamp(16, 800) as u32;
                let mut vp = self.ui.viewport();
                let _ = vp.cols.set_size(u32::from(col), width);
                self.ui.set_viewport(vp);
            }
            PointerDrag::RowResize { row, start_y, orig } => {
                let delta = ((pos.y - start_y) / self.ui.viewport().zoom.max(0.25) as f32).round();
                let height = (orig as i32 + delta as i32).clamp(12, 200) as u32;
                let mut vp = self.ui.viewport();
                let _ = vp.rows.set_size(row, height);
                self.ui.set_viewport(vp);
            }
        }
    }

    fn handle_release(&mut self, pos: egui::Pos2, ctrl: bool) {
        let drag = self.drag.take();
        match drag {
            Some(PointerDrag::Fill { .. }) => {
                self.execute_if_available("edit.fillselection", "WP-17");
            }
            Some(PointerDrag::Move { press }) => {
                let vp = self.ui.viewport();
                let Some((row, col)) = self.grid.hit(pos, &vp) else {
                    return;
                };
                if (row, col) == press {
                    let mut selection = self.ui.selection();
                    selection.cursor.row = row;
                    selection.cursor.col = col;
                    selection.replace(Area::cell(selection.cursor));
                    self.ui.set_selection(selection);
                    return;
                }
                let operation = if ctrl { "copy" } else { "move" };
                self.message = Some(format!("drag {operation} arrives in WP-17"));
            }
            _ => {}
        }
    }

    fn persist_session(&self) {
        let snapshot = self.runner.handle().snapshot();
        let mut state = self.ui.session_state();
        state.zoom = self.ui.viewport().zoom;
        state.panel = self.ui.panel().visible.clone();
        if let Some(sheet) = snapshot.workbook.sheet(self.ui.selection().sheet) {
            state.sheet = Some(sheet.name.clone());
        }
        let sel = self.ui.selection();
        if let Ok(letters) = col_to_letters(sel.cursor.col) {
            state.cursor = Some(format!("{}{}", letters, sel.cursor.row + 1));
        }
        if let Some(file) = &self.file {
            state.touch_file(&file.display().to_string());
        }
        let _ = state.save(&self.paths.state_dir);
    }
}

fn selection_a1(ui: &UiSession, wb: &omacell_core::workbook::Workbook) -> String {
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

fn cursor_a1(ui: &UiSession, wb: &omacell_core::workbook::Workbook) -> String {
    let selection = ui.selection();
    let sheet = wb
        .sheet(selection.sheet)
        .map(|sheet| sheet.name.as_str())
        .unwrap_or("Sheet1");
    format!("{}!{}", quote_sheet_name(sheet), selection.cursor.to_a1())
}

impl eframe::App for Gui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui_frame(ctx);
        if self.runner.handle().is_busy() {
            ctx.request_repaint();
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.persist_session();
    }
}

impl Drop for Gui {
    fn drop(&mut self) {
        self.persist_session();
    }
}

const DEFAULT_FIT_ROW: u32 = 20;

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
        let sel = ui.selection();
        args["range"] = json!(sel.active().to_range().to_a1());
    }
    if cmd.starts_with("ai.formula.")
        && args
            .get("ref")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .is_empty()
    {
        args["ref"] = json!(ui.selection().cursor.to_a1());
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

fn toolkit_owns_key(edit: &omacell_ui::EditState, event: &KeyEvent) -> bool {
    if edit.is_idle() {
        return false;
    }
    match edit.surface {
        EditSurface::Idle => false,
        EditSurface::FormulaBar => !matches!(
            event.code,
            KeyCode::Esc | KeyCode::Enter | KeyCode::Tab | KeyCode::F(4)
        ),
        EditSurface::InCell => {
            !event.ctrl && !event.alt && matches!(event.code, KeyCode::Char(_) | KeyCode::Space)
        }
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

fn catalog_from_bus(bus: &Bus) -> Result<Vec<CommandJson>, CoreError> {
    let text = bus
        .commands_json()
        .map_err(|err| CoreError::new("gui.palette", err.to_string()))?;
    serde_json::from_str::<CommandsEnvelope>(&text)
        .map(|envelope| envelope.commands)
        .map_err(|err| CoreError::new("gui.palette", err.to_string()))
}

fn apply_sheet_view(ui: &UiSession, wb: &omacell_core::workbook::Workbook, id: SheetId) {
    let Some(sheet) = wb.sheet(id) else {
        return;
    };
    let mut vp = ui.viewport();
    vp.first_row = sheet.view.scroll_row;
    vp.first_col = sheet.view.scroll_col;
    vp.set_zoom(sheet.view.zoom);
    vp.freeze = sheet.view.freeze;
    vp.split = sheet.view.split;
    vp.rows = sheet.geometry.rows.clone();
    vp.cols = sheet.geometry.cols.clone();
    ui.set_viewport(vp);

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

fn apply_restored_session(
    ui: &UiSession,
    wb: &omacell_core::workbook::Workbook,
    state: &SessionState,
) {
    let mut vp = ui.viewport();
    vp.set_zoom(state.zoom);
    ui.set_viewport(vp);
    if let Some(name) = &state.sheet
        && let Some(sheet) = wb.sheet_by_name(name)
    {
        apply_sheet_view(ui, wb, sheet.id);
    }
    if let Some(cursor) = &state.cursor
        && let Ok(cell) = parse_a1_cell(cursor)
    {
        let mut sel = ui.selection();
        sel.cursor.row = cell.row;
        sel.cursor.col = cell.col;
        sel.replace(Area::cell(sel.cursor));
        ui.set_selection(sel);
    }
    if let Some(id) = &state.panel {
        let mut panel = ui.panel();
        panel.open(id);
        ui.set_panel(panel);
    }
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
                | "file.print"
                | "theme.reload"
        )
    {
        return false;
    }
    true
}

fn autopilot_scope_label(
    scope: &AutopilotScope,
    workbook: &omacell_core::workbook::Workbook,
) -> String {
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
            let start = col_to_letters(*min_col)
                .map_or_else(|_| "?".into(), |col| format!("{col}{}", min_row + 1));
            let end = col_to_letters(*max_col)
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

/// Native event loop.
pub fn run(launch: Launch) -> Result<(), CoreError> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(PRODUCT_NAME)
            .with_decorations(false)
            .with_title(PRODUCT_DISPLAY_NAME)
            .with_min_inner_size([48.0, 36.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        PRODUCT_NAME,
        options,
        Box::new(move |cc| {
            Gui::new(launch, true, &cc.egui_ctx)
                .map(|gui| Box::new(gui) as Box<dyn eframe::App>)
                .map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync>)
        }),
    )
    .map_err(|err| CoreError::new("gui.eframe", err.to_string()))
}
