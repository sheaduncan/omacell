//! TUI session over the WP-13 composition objects.

use std::io::{self, IsTerminal, stdout};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::ExecutableCommand;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyEventKind,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use omacell_bus::ipc::{IpcHandle, default_runtime_dir, serve};
use omacell_bus::{Bus, CommandsEnvelope};
use omacell_conf::{ConfigStore, Paths, ReloadEvent};
use omacell_core::command::{Origin, Outcome};
use omacell_core::error::CoreError;
use omacell_ui::{Area, KeyCode, KeyEvent, KeyOutcome, KeymapRoots, UiSession};
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
}

/// Running TUI. Tests drive [`Self::draw`] / [`Self::step_key`].
pub struct Tui {
    paths: Paths,
    store: ConfigStore,
    bus: Arc<Mutex<Bus>>,
    ui: UiSession,
    roots: KeymapRoots,
    truecolor: bool,
    message: Option<String>,
    palette_index: usize,
    _ipc: Option<IpcHandle>,
}

impl Tui {
    /// Wrap a launch. `ipc` starts the in-process socket used by the theme hook.
    pub fn new(launch: Launch, ipc: bool) -> Result<Self, CoreError> {
        let loaded = launch.store.snapshot();
        let truecolor = truecolor_enabled(&loaded.config.tui.truecolor);
        let (bus, ipc_handle) = if ipc {
            let handle = serve(default_runtime_dir(), launch.bus)?;
            let bus = handle.bus().clone();
            (bus, Some(handle))
        } else {
            (Arc::new(Mutex::new(launch.bus)), None)
        };
        Ok(Self {
            paths: launch.paths,
            store: launch.store,
            bus,
            ui: launch.ui,
            roots: launch.roots,
            truecolor,
            message: None,
            palette_index: 0,
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

    /// Apply pending filesystem/theme reloads without resetting the session.
    pub fn poll_reload(&mut self) -> Result<(), CoreError> {
        let events = self.store.drain_events();
        if events.is_empty() {
            return Ok(());
        }
        let snapshot = self.store.snapshot();
        for ev in &events {
            match ev {
                ReloadEvent::Invalid { message, .. } => {
                    self.message = Some(message.clone());
                }
                ReloadEvent::Applied { .. } | ReloadEvent::ThemeChanged { .. } => {
                    let bus = self.bus.lock().unwrap_or_else(|p| p.into_inner());
                    self.ui
                        .apply_config(&snapshot, &self.roots, bus.registry())?;
                    self.truecolor = truecolor_enabled(&snapshot.config.tui.truecolor);
                    if let ReloadEvent::ThemeChanged { name } = ev {
                        self.message = Some(format!("theme {name}"));
                    }
                }
            }
        }
        Ok(())
    }

    /// Draw using the current workbook snapshot.
    pub fn draw<B: Backend>(&self, terminal: &mut Terminal<B>) -> io::Result<()> {
        let bus = self.bus.lock().unwrap_or_else(|p| p.into_inner());
        let loaded = self.store.snapshot();
        let unicode = loaded.config.tui.unicode_borders;
        terminal.draw(|frame| {
            render::draw(
                frame,
                render::FrameInput {
                    wb: bus.workbook(),
                    engine: bus.engine(),
                    ui: &self.ui,
                    theme_name: &loaded.theme.name,
                    truecolor: self.truecolor,
                    unicode_borders: unicode,
                    message: self.message.as_deref(),
                },
            );
        })?;
        Ok(())
    }

    /// Handle one toolkit-neutral key (tests and the live loop).
    pub fn step_key(&mut self, event: KeyEvent) -> Result<KeyOutcome, CoreError> {
        self.poll_reload()?;
        if self.ui.palette().open {
            return self.step_palette(event);
        }
        if let Some(id) = self.ui.panel().visible.clone()
            && matches!(id.as_str(), "find" | "goto" | "command")
        {
            return self.step_panel(event, &id);
        }
        let outcome = self.ui.handle_key(event);
        if let KeyOutcome::Command {
            cmd,
            mut args,
            count,
        } = outcome.clone()
        {
            if count > 1 {
                match &mut args {
                    serde_json::Value::Object(map) => {
                        map.entry("count").or_insert(serde_json::json!(count));
                    }
                    serde_json::Value::Null => {
                        args = serde_json::json!({"count": count});
                    }
                    _ => {}
                }
            }
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
        let result = {
            let mut bus = self.bus.lock().unwrap_or_else(|p| p.into_inner());
            bus.execute(Origin::User, cmd, args)
        };
        if !result.ok {
            if let Some(err) = &result.error {
                self.message = Some(err.message.clone());
            }
        } else {
            self.ui.remember_command(cmd);
            self.message = None;
            if cmd == "palette.open" {
                self.refresh_palette("");
            }
        }
        Ok(result)
    }

    fn step_palette(&mut self, event: KeyEvent) -> Result<KeyOutcome, CoreError> {
        match (event.code, event.ctrl, event.alt) {
            (KeyCode::Esc, false, false) => {
                let mut palette = self.ui.palette();
                palette.close();
                self.ui.set_palette(palette);
                self.palette_index = 0;
            }
            (KeyCode::Enter, false, false) => {
                let palette = self.ui.palette();
                if let Some(hit) = palette.hits.get(self.palette_index) {
                    let id = hit.id.clone();
                    let mut palette = palette;
                    palette.close();
                    self.ui.set_palette(palette);
                    self.execute_cmd(&id, serde_json::json!({}))?;
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
                self.refresh_palette(&q);
            }
            (KeyCode::Char(c), false, false) => {
                let mut palette = self.ui.palette();
                palette.query.push(c);
                let q = palette.query.clone();
                self.ui.set_palette(palette);
                self.refresh_palette(&q);
            }
            (KeyCode::Space, false, false) => {
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

    fn refresh_palette(&mut self, query: &str) {
        let commands = {
            let bus = self.bus.lock().unwrap_or_else(|p| p.into_inner());
            bus.commands_json()
                .ok()
                .and_then(|text| serde_json::from_str::<CommandsEnvelope>(&text).ok())
                .map(|env| env.commands)
                .unwrap_or_default()
        };
        self.ui.rank_palette(&commands, query, None);
        let n = self.ui.palette().hits.len();
        if n == 0 {
            self.palette_index = 0;
        } else {
            self.palette_index = self.palette_index.min(n - 1);
        }
    }

    fn step_panel(&mut self, event: KeyEvent, id: &str) -> Result<KeyOutcome, CoreError> {
        match (event.code, event.ctrl) {
            (KeyCode::Esc, false) => {
                let mut panel = self.ui.panel();
                panel.dismiss();
                self.ui.set_panel(panel);
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
            _ => {}
        }
        Ok(KeyOutcome::Pending)
    }

    /// Mouse click in terminal cells → selection (when `[tui] mouse`).
    pub fn step_mouse(&mut self, col: u16, row: u16) {
        let loaded = self.store.snapshot();
        if !loaded.config.tui.mouse {
            return;
        }
        let cfg = self.ui.config();
        let compact =
            u32::from(col.saturating_add(1)).saturating_mul(8) < cfg.layout.compact_below_width;
        let mut chrome = 0u16;
        if cfg.appearance.show_sheet_tabs && !compact {
            chrome = chrome.saturating_add(1);
        }
        if cfg.appearance.show_formula_bar {
            chrome = chrome.saturating_add(1);
        }
        if row < chrome {
            return;
        }
        let mut vp = self.ui.viewport();
        let header_w = 4u16;
        let col_chars = loaded
            .config
            .appearance
            .column_width
            .round()
            .clamp(4.0, 24.0) as u16;
        let cell_w = col_chars
            .saturating_add(u16::from(cfg.appearance.grid_lines))
            .max(1);
        let grid_row = row.saturating_sub(chrome).saturating_sub(1);
        let grid_col = col.saturating_sub(header_w) / cell_w;
        let hit_row =
            vp.hit_row(u64::from(grid_row) * u64::from(omacell_core::geometry::DEFAULT_ROW_PX));
        let hit_col =
            vp.hit_col(u64::from(grid_col) * u64::from(omacell_core::geometry::DEFAULT_COL_PX));
        let mut sel = self.ui.selection();
        sel.cursor.row = hit_row;
        sel.cursor.col = hit_col;
        sel.replace(Area::cell(sel.cursor));
        vp.ensure_row_visible(hit_row);
        vp.ensure_col_visible(hit_col);
        self.ui.set_viewport(vp);
        self.ui.set_selection(sel);
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
        let mouse = self.store.snapshot().config.tui.mouse;
        stdout()
            .execute(EnterAlternateScreen)
            .map_err(|e| CoreError::new("tui.tty", e.to_string()))?;
        if mouse {
            stdout()
                .execute(EnableMouseCapture)
                .map_err(|e| CoreError::new("tui.tty", e.to_string()))?;
        }
        let backend = CrosstermBackend::new(stdout());
        let mut terminal =
            Terminal::new(backend).map_err(|e| CoreError::new("tui.tty", e.to_string()))?;
        let result = self.event_loop(&mut terminal);
        if mouse {
            let _ = stdout().execute(DisableMouseCapture);
        }
        disable_raw_mode().ok();
        stdout().execute(LeaveAlternateScreen).ok();
        result
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
                        let quit = matches!(mapped.code, KeyCode::Char('q') | KeyCode::Char('c'))
                            && mapped.ctrl
                            && self.ui.edit().is_idle()
                            && !self.ui.palette().open;
                        if quit {
                            break;
                        }
                        self.step_key(mapped)?;
                    }
                }
                CEvent::Mouse(mouse) => {
                    if let Some((c, r, _)) = map_mouse(mouse) {
                        self.step_mouse(c, r);
                    }
                }
                CEvent::Resize(_, _) => {}
                _ => {}
            }
        }
        Ok(())
    }
}

/// Run the TUI on the current terminal.
pub fn run(launch: Launch) -> Result<(), CoreError> {
    Tui::new(launch, true)?.run_crossterm()
}
