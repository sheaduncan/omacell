//! TUI session over the WP-13 composition objects.

use std::io::{self, IsTerminal, stdout};
use std::sync::Mutex;
use std::time::Duration;

use crossterm::ExecutableCommand;
use crossterm::cursor::Show;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyEventKind,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use omacell_bus::ipc::{IpcHandle, default_runtime_dir};
use omacell_bus::{
    Bus, CancelHandle, CommandJson, CommandsEnvelope, LongOps, TaskEvent, TaskId, TaskRunner,
};
use omacell_conf::{ConfigStore, Paths, ReloadEvent};
use omacell_core::addr::SheetId;
use omacell_core::command::{Origin, Outcome};
use omacell_core::error::CoreError;
use omacell_core::workbook::Workbook;
use omacell_ui::{
    Area, ExtendMode, KeyCode, KeyEvent, KeyOutcome, KeymapRoots, UiSession, apply_local_command,
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
}

/// Running TUI. Tests drive [`Self::draw`] / [`Self::step_key`].
pub struct Tui {
    paths: Paths,
    store: ConfigStore,
    runner: TaskRunner,
    ui: UiSession,
    roots: KeymapRoots,
    truecolor: bool,
    message: Option<String>,
    palette_index: usize,
    palette_command: Option<String>,
    last_grid: Mutex<Option<render::GridHitMap>>,
    catalog: Vec<CommandJson>,
    active_sheet: SheetId,
    dirty: bool,
    quit_armed: bool,
    quit_requested: bool,
    last_queued: Option<TaskId>,
    focused_cancel: Option<CancelHandle>,
    quit_after: Option<TaskId>,
    _ipc: Option<IpcHandle>,
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
            ui: launch.ui,
            roots: launch.roots,
            truecolor,
            message: None,
            palette_index: 0,
            palette_command: None,
            last_grid: Mutex::new(None),
            catalog,
            active_sheet,
            dirty: false,
            quit_armed: false,
            quit_requested: false,
            last_queued: None,
            focused_cancel: None,
            quit_after: None,
            _ipc: ipc_handle,
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

    /// Apply pending filesystem/theme reloads without resetting the session.
    pub fn poll_reload(&mut self) -> Result<(), CoreError> {
        self.poll_tasks();
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
    pub fn draw<B: Backend>(&self, terminal: &mut Terminal<B>) -> io::Result<()> {
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
            return self.step_palette(event);
        }
        if let Some(id) = self.ui.panel().visible.clone()
            && matches!(id.as_str(), "find" | "goto" | "command")
        {
            return self.step_panel(event, &id);
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
        self.last_queued = None;
        if let Some(local) = apply_local_command(&self.ui, &handle.snapshot().workbook, cmd, &args)
        {
            if let Err(err) = local {
                self.message = Some(err.message.clone());
                return Ok(Outcome::failure(err));
            }
            self.ui.remember_command(cmd);
            self.message = None;
            if cmd == "palette.open" {
                self.refresh_palette("")?;
            }
            return Ok(Outcome::success(serde_json::json!({"ok": true})));
        }
        let args = inject_chart_range(&self.ui, cmd, args);
        let (id, cancel) = match handle.submit(Origin::User, cmd, args) {
            Ok(task) => task,
            Err(err) => {
                self.message = Some(err.message.clone());
                return Ok(Outcome::failure(err));
            }
        };
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
                    self.message = None;
                    if state.command == "file.open" || state.command == "file.save" {
                        self.dirty = false;
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
                    self.adopt_snapshot();
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
                    self.message = Some(message);
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
                if let Some(hit) = palette.hits.get(self.palette_index) {
                    let id = hit.id.clone();
                    if let Some(command) = self
                        .command_catalog()?
                        .into_iter()
                        .find(|command| command.id == id)
                        && has_required_args(&command)
                    {
                        let mut palette = palette;
                        palette.prompt_for(&command);
                        if let Some(fields) = palette.prompt.take() {
                            palette.prompt = Some(format!("{id} — {fields}; enter JSON object"));
                        }
                        palette.query.clear();
                        self.ui.set_palette(palette);
                        self.palette_command = Some(id);
                    } else {
                        let result = self.execute_cmd(&id, serde_json::json!({}))?;
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
            (KeyCode::Enter, false) if id == "command" => {
                self.execute_command_line()?;
            }
            _ => {}
        }
        Ok(KeyOutcome::Pending)
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
                _ => {}
            }
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

    fn sync_active_sheet(&mut self) {
        self.adopt_snapshot();
    }
}

struct TerminalRestore {
    raw: bool,
    alternate: bool,
    mouse: bool,
}

impl Drop for TerminalRestore {
    fn drop(&mut self) {
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

fn has_required_args(command: &CommandJson) -> bool {
    command
        .arg_schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|required| !required.is_empty())
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
                | "file.export"
                | "theme.reload"
        )
    {
        return false;
    }
    true
}

fn inject_chart_range(ui: &UiSession, cmd: &str, mut args: serde_json::Value) -> serde_json::Value {
    if cmd == "chart.fromselection"
        && args
            .get("range")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .is_empty()
    {
        args["range"] = serde_json::json!(ui.selection().active().to_range().to_a1());
    }
    args
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
