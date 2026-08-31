//! eframe application over the WP-13 composition root and WP-15a runner.

use std::path::PathBuf;

use eframe::egui;
use omacell_bus::ipc::{IpcHandle, default_runtime_dir, serve_runner};
use omacell_bus::{
    Bus, CancelHandle, CommandJson, CommandsEnvelope, LongOps, TaskEvent, TaskRunner,
    TaskRunnerHandle,
};
use omacell_conf::{ConfigStore, LoadedConfig, Paths, ReloadEvent};
use omacell_core::addr::{SheetId, col_to_letters, parse_a1_cell, quote_sheet_name};
use omacell_core::command::{Origin, Outcome};
use omacell_core::error::CoreError;
use omacell_core::print::paginate;
use omacell_core::{PRODUCT_DISPLAY_NAME, PRODUCT_NAME};
use omacell_ui::{
    Area, EditSurface, KeyCode, KeyEvent, KeyOutcome, KeymapRoots, SessionState, UiSession,
    apply_local_command,
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

/// Running GUI. Tests drive [`Self::ui_frame`].
pub struct Gui {
    paths: Paths,
    store: ConfigStore,
    runner: TaskRunner,
    ui: UiSession,
    roots: KeymapRoots,
    theme: GuiTheme,
    catalog: Vec<CommandJson>,
    message: Option<String>,
    dirty: bool,
    active_sheet: SheetId,
    grid: GridLayout,
    palette_index: usize,
    palette_command: Option<String>,
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
        let (message, focused_cancel) = if let Some(path) = requested_file {
            let (_, cancel) = runner.handle().submit(
                Origin::User,
                "file.open",
                json!({"path": path.display().to_string()}),
            )?;
            (Some("opening…".into()), Some(cancel))
        } else {
            (None, None)
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
            ui: launch.ui,
            roots: launch.roots,
            theme,
            catalog,
            message,
            dirty: false,
            active_sheet,
            grid: GridLayout::default(),
            palette_index: 0,
            palette_command: None,
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
                    self.message = None;
                    if state.command == "file.open" || state.command == "file.save" {
                        self.dirty = false;
                        if let Some(path) = outcome
                            .result
                            .as_ref()
                            .and_then(|value| value.get("path"))
                            .and_then(|value| value.as_str())
                        {
                            self.file = Some(PathBuf::from(path));
                        }
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
                    if state.command == "file.open" {
                        self.adopt_opened_snapshot();
                    } else if matches!(state.command.as_str(), "sheet.next" | "sheet.prev") {
                        self.adopt_snapshot();
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
                    self.message = Some(message);
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
            return self.step_palette(event);
        }
        let outcome = self.ui.handle_key(event);
        if let KeyOutcome::Command { cmd, args, .. } = outcome.clone() {
            self.execute_cmd(&cmd, args)?;
        }
        Ok(outcome)
    }

    /// Run a registry command as the interactive user.
    pub fn execute_cmd(
        &mut self,
        cmd: &str,
        args: serde_json::Value,
    ) -> Result<Outcome, CoreError> {
        let handle = self.runner.handle();
        if let Some(local) = apply_local_command(&self.ui, &handle.snapshot().workbook, cmd, &args)
        {
            if let Err(err) = local {
                self.message = Some(err.message.clone());
                return Ok(Outcome::failure(err));
            }
            self.ui.remember_command(cmd);
            self.message = None;
            if cmd == "palette.open" {
                self.refresh_palette("");
                self.palette_command = None;
            }
            if cmd == "ai.agent" {
                self.dispatch_agent();
            }
            return Ok(Outcome::success(json!({"ok": true})));
        }
        let args = inject_chart_range(&self.ui, cmd, args);
        if cmd == "file.print" {
            self.toggle_print_preview();
        }
        match handle.submit(Origin::User, cmd, args) {
            Ok((id, cancel)) => {
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
            let bundle = json!({
                "schema": 1,
                "workbook": self.file.as_ref().map(|path| path.display().to_string()),
                "selection": &selection,
                "diagnostic": outcome.result,
            });
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
        if let Some(command) = self
            .catalog
            .iter()
            .find(|command| command.id == id)
            .cloned()
            && has_required_args(&command)
        {
            let mut palette = self.ui.palette();
            palette.prompt_for(&command);
            if let Some(fields) = palette.prompt.take() {
                palette.prompt = Some(format!("{id} — {fields}; enter JSON object"));
            }
            palette.query.clear();
            self.ui.set_palette(palette);
            self.palette_command = Some(id.to_string());
            return Ok(());
        }
        self.close_palette();
        let _ = self.execute_cmd(id, json!({}))?;
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
        let input = ctx.input(|i| i.clone());
        for key in input::pressed_keys(&input.events) {
            if toolkit_owns_key(&edit, &key) {
                continue;
            }
            let _ = self.step_key(key);
        }
        if !self.ui.edit().is_idle() && self.ui.edit().surface == EditSurface::InCell {
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
                    edit.buffer = text;
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

fn inject_chart_range(ui: &UiSession, cmd: &str, mut args: serde_json::Value) -> serde_json::Value {
    if cmd == "chart.fromselection"
        && args
            .get("range")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .is_empty()
    {
        let sel = ui.selection();
        args["range"] = json!(sel.active().to_range().to_a1());
    }
    args
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

fn has_required_args(command: &CommandJson) -> bool {
    command
        .arg_schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|required| !required.is_empty())
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
                | "file.export"
                | "file.print"
                | "theme.reload"
        )
    {
        return false;
    }
    true
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
