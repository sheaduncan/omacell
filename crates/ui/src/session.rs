//! `UiSession` and `apply_config`.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use omacell_bus::{CommandJson, CommandRegistry};
use omacell_conf::{Config, LoadedConfig};
use omacell_core::addr::SheetId;
use omacell_core::error::CoreError;

use crate::agent_panel::AgentPanel;
use crate::clipboard::ClipboardPayload;
use crate::edit::{EditState, EditSurface};
use crate::error;
use crate::find::{FindReplace, GoTo};
use crate::formula_assist::FormulaAssist;
use crate::import_review::ImportPlanReview;
use crate::keymap::{Keymap, KeymapRoots};
use crate::mode::{KeyModel, Mode};
use crate::palette::{AiPlanProvider, Palette};
use crate::panel::PanelState;
use crate::persist::SessionState;
use crate::review::ChangesetReview;
use crate::selection::Selection;
use crate::status::StatusLine;
use crate::undo::UndoHistory;
use crate::viewport::Viewport;

/// Shared mutable UI state (view commands lock this).
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct UiInner {
    pub mode: Mode,
    pub model: KeyModel,
    pub keymap: Keymap,
    pub base_keymap: Keymap,
    pub script_bindings: Vec<(String, String, String)>,
    pub selection: Selection,
    pub edit: EditState,
    pub viewport: Viewport,
    pub palette: Palette,
    pub panel: PanelState,
    pub changeset_review: Option<ChangesetReview>,
    pub agent_panel: AgentPanel,
    pub formula_assist: Option<FormulaAssist>,
    pub import_review: Option<ImportPlanReview>,
    pub status: StatusLine,
    pub undo: UndoHistory,
    pub session: SessionState,
    pub find: FindReplace,
    pub goto: GoTo,
    pub clipboard: Option<ClipboardPayload>,
    pub reference_colors: [String; 8],
    pub enter_moves: String,
    pub status_ids: Vec<String>,
    pub formula_bar_expanded: bool,
    pub show_formulas: bool,
    pub config: Config,
    pub agent_visible: bool,
    pub pending_agent: Option<AgentHandoff>,
}

/// Pending *Hand to agent* request from the palette or status line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentHandoff {
    /// Prompt text.
    pub prompt: String,
    /// `omacell agent diagnose` when true.
    pub diagnose: bool,
}

/// Toolkit-free UI session.
#[derive(Clone)]
pub struct UiSession {
    pub(crate) inner: Arc<Mutex<UiInner>>,
}

impl UiSession {
    /// Build from a config snapshot and keymap search roots.
    pub fn new(config: &LoadedConfig, roots: &KeymapRoots) -> Result<Self, CoreError> {
        let inner = load_inner(config, roots)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, UiInner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Apply a new config snapshot without resetting live interaction state.
    pub fn apply_config(
        &self,
        config: &LoadedConfig,
        roots: &KeymapRoots,
        registry: &CommandRegistry,
    ) -> Result<(), CoreError> {
        let mut next = load_inner(config, roots)?;
        next.keymap.validate_commands(registry)?;
        let script_bindings = self.lock().script_bindings.clone();
        for (mode, keys, cmd) in &script_bindings {
            if registry.get_str(cmd).is_err() {
                return Err(error::keymap(format!("unowned script command {cmd}")));
            }
            next.keymap.set_script_binding(mode, keys, cmd)?;
        }
        next.script_bindings = script_bindings;
        let mut g = self.lock();
        if next.model != g.model {
            g.mode = if !g.edit.is_idle() && next.model == KeyModel::Modal {
                Mode::Insert
            } else {
                next.mode
            };
        }
        g.model = next.model;
        g.keymap = next.keymap;
        g.base_keymap = next.base_keymap;
        g.script_bindings = next.script_bindings;
        g.reference_colors = next.reference_colors;
        g.enter_moves = next.enter_moves;
        g.status_ids = next.status_ids;
        g.panel.side = next.panel.side;
        g.panel.width = next.panel.width;
        g.config = next.config;
        Ok(())
    }

    /// Apply config using a captured command-id set (task runner).
    pub fn apply_config_ids(
        &self,
        config: &LoadedConfig,
        roots: &KeymapRoots,
        known: &BTreeSet<String>,
    ) -> Result<(), CoreError> {
        let mut next = load_inner(config, roots)?;
        for (_mode, chord, binding) in next.keymap.iter() {
            if !known.contains(&binding.cmd)
                && crate::deferred::owner(&binding.cmd).is_none()
                && !crate::deferred::is_composition_command(&binding.cmd)
            {
                return Err(error::keymap(format!(
                    "unowned command {} for chord {chord}",
                    binding.cmd
                )));
            }
        }
        let script_bindings = self.lock().script_bindings.clone();
        for (mode, keys, cmd) in &script_bindings {
            if !known.contains(cmd) {
                return Err(error::keymap(format!("unowned script command {cmd}")));
            }
            next.keymap.set_script_binding(mode, keys, cmd)?;
        }
        next.script_bindings = script_bindings;
        let mut g = self.lock();
        if next.model != g.model {
            g.mode = if !g.edit.is_idle() && next.model == KeyModel::Modal {
                Mode::Insert
            } else {
                next.mode
            };
        }
        g.model = next.model;
        g.keymap = next.keymap;
        g.base_keymap = next.base_keymap;
        g.script_bindings = next.script_bindings;
        g.reference_colors = next.reference_colors;
        g.enter_moves = next.enter_moves;
        g.status_ids = next.status_ids;
        g.panel.side = next.panel.side;
        g.panel.width = next.panel.width;
        g.config = next.config;
        Ok(())
    }

    /// Current mode.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.lock().mode
    }

    /// Current selection.
    #[must_use]
    pub fn selection(&self) -> Selection {
        self.lock().selection.clone()
    }

    /// Replace selection state after toolkit-neutral mouse or accessibility input.
    pub fn set_selection(&self, selection: Selection) {
        self.lock().selection = selection;
    }

    /// Current edit state.
    #[must_use]
    pub fn edit(&self) -> EditState {
        self.lock().edit.clone()
    }

    /// Replace the shared edit buffer after a formula-bar caret operation.
    pub fn set_edit(&self, edit: EditState) {
        self.lock().edit = edit;
    }

    /// Current viewport.
    #[must_use]
    pub fn viewport(&self) -> Viewport {
        self.lock().viewport.clone()
    }

    /// Replace viewport geometry or scroll state supplied by a frontend.
    pub fn set_viewport(&self, viewport: Viewport) {
        self.lock().viewport = viewport;
    }

    /// Atomically snapshot the retained fields that make up a persisted sheet view.
    #[must_use]
    pub fn sheet_view_snapshot(&self) -> (Selection, Viewport, bool) {
        let state = self.lock();
        (
            state.selection.clone(),
            state.viewport.clone(),
            state.show_formulas,
        )
    }

    /// Session persistence blob.
    #[must_use]
    pub fn session_state(&self) -> SessionState {
        self.lock().session.clone()
    }

    /// Current command-palette model.
    #[must_use]
    pub fn palette(&self) -> Palette {
        self.lock().palette.clone()
    }

    /// Replace the command-palette model after frontend typing or dismissal.
    pub fn set_palette(&self, palette: Palette) {
        self.lock().palette = palette;
    }

    /// Current panel state.
    #[must_use]
    pub fn panel(&self) -> PanelState {
        self.lock().panel.clone()
    }

    /// Replace docked-panel presentation after a frontend resize or focus change.
    pub fn set_panel(&self, panel: PanelState) {
        self.lock().panel = panel;
    }

    /// Active proposed-changeset review, if any.
    #[must_use]
    pub fn changeset_review(&self) -> Option<ChangesetReview> {
        self.lock().changeset_review.clone()
    }

    /// Replace or clear the active proposed-changeset review.
    pub fn set_changeset_review(&self, review: Option<ChangesetReview>) {
        self.lock().changeset_review = review;
    }

    /// Retained in-app agent transcript, entry, and policy presentation.
    #[must_use]
    pub fn agent_panel(&self) -> AgentPanel {
        self.lock().agent_panel.clone()
    }

    /// Replace the retained in-app agent panel state.
    pub fn set_agent_panel(&self, panel: AgentPanel) {
        self.lock().agent_panel = panel;
    }

    /// Latest formula-assist result.
    #[must_use]
    pub fn formula_assist(&self) -> Option<FormulaAssist> {
        self.lock().formula_assist.clone()
    }

    /// Replace or clear the formula-assist result.
    pub fn set_formula_assist(&self, assist: Option<FormulaAssist>) {
        self.lock().formula_assist = assist;
    }

    /// Active CSV import preview and optional AI plan proposal.
    #[must_use]
    pub fn import_review(&self) -> Option<ImportPlanReview> {
        self.lock().import_review.clone()
    }

    /// Replace or clear the active CSV import preview.
    pub fn set_import_review(&self, review: Option<ImportPlanReview>) {
        self.lock().import_review = review;
    }

    /// Current status-line presentation.
    #[must_use]
    pub fn status(&self) -> StatusLine {
        self.lock().status.clone()
    }

    /// Replace rendered status-line segments.
    pub fn set_status(&self, status: StatusLine) {
        self.lock().status = status;
    }

    /// Current undo-history presentation.
    #[must_use]
    pub fn undo_history(&self) -> UndoHistory {
        self.lock().undo.clone()
    }

    /// Replace the presentation-only undo history from bus events.
    pub fn set_undo_history(&self, undo: UndoHistory) {
        self.lock().undo = undo;
    }

    /// Current find/replace panel model.
    #[must_use]
    pub fn find_replace(&self) -> FindReplace {
        self.lock().find.clone()
    }

    /// Replace find/replace panel inputs.
    pub fn set_find_replace(&self, find: FindReplace) {
        self.lock().find = find;
    }

    /// Current Go To input.
    #[must_use]
    pub fn goto(&self) -> GoTo {
        self.lock().goto.clone()
    }

    /// Replace the Go To input.
    pub fn set_goto(&self, goto: GoTo) {
        self.lock().goto = goto;
    }

    /// Current clipboard snapshot.
    #[must_use]
    pub fn clipboard(&self) -> Option<ClipboardPayload> {
        self.lock().clipboard.clone()
    }

    /// Whether the formula bar is expanded beyond its configured base height.
    #[must_use]
    pub fn formula_bar_expanded(&self) -> bool {
        self.lock().formula_bar_expanded
    }

    /// Whether the grid is showing formula source instead of calculated values.
    #[must_use]
    pub fn show_formulas(&self) -> bool {
        self.lock().show_formulas
    }

    /// Replace formula-source display after a frontend activates a saved sheet view.
    pub fn set_show_formulas(&self, show: bool) {
        self.lock().show_formulas = show;
    }

    /// Replace the toolkit-independent clipboard snapshot.
    pub fn set_clipboard(&self, clipboard: Option<ClipboardPayload>) {
        self.lock().clipboard = clipboard;
    }

    /// Rank the retained command palette for a new query.
    pub fn rank_palette(
        &self,
        commands: &[CommandJson],
        query: &str,
        ai: Option<&dyn AiPlanProvider>,
    ) {
        let visible = self.lock().agent_visible;
        let filtered: Vec<CommandJson> = commands
            .iter()
            .filter(|c| visible || c.id != "ai.agent")
            .cloned()
            .collect();
        let mut palette = self.palette();
        palette.rank_with_ai(&filtered, query, ai);
        self.lock().palette = palette;
    }

    /// Whether a default Omarchy agent is available (palette/status).
    #[must_use]
    pub fn agent_visible(&self) -> bool {
        self.lock().agent_visible
    }

    /// Set from the composition root after [`omacell_conf::detect_default_agent`].
    pub fn set_agent_visible(&self, visible: bool) {
        self.lock().agent_visible = visible;
    }

    /// Take a pending hand-off request, if any.
    pub fn take_agent_handoff(&self) -> Option<AgentHandoff> {
        self.lock().pending_agent.take()
    }

    /// Populate the retained palette's inline argument prompt.
    pub fn prompt_palette_for(&self, command: &CommandJson) {
        self.lock().palette.prompt_for(command);
    }

    /// Record a successfully executed command in palette recents.
    pub fn remember_command(&self, id: &str) {
        self.lock().palette.remember(id);
    }

    /// Replace restored session state supplied by the composition root.
    pub fn set_session_state(&self, state: SessionState) {
        self.lock().session = state;
    }

    /// Reference color for index 0–7.
    #[must_use]
    pub fn reference_color(&self, index: usize) -> String {
        self.lock().reference_colors[index % 8].clone()
    }

    /// Borrow the keymap (cloned).
    #[must_use]
    pub fn keymap(&self) -> Keymap {
        self.lock().keymap.clone()
    }

    /// Add or replace a user-Lua key binding after registry validation.
    pub fn set_script_binding(
        &self,
        mode: &str,
        keys: &str,
        cmd: &str,
        registry: &CommandRegistry,
    ) -> Result<(), CoreError> {
        if registry.get_str(cmd).is_err() {
            return Err(error::keymap(format!("unowned script command {cmd}")));
        }
        self.set_script_binding_validated(mode, keys, cmd)
    }

    /// Add or replace a user-Lua key binding using a task-runner catalog.
    pub fn set_script_binding_ids(
        &self,
        mode: &str,
        keys: &str,
        cmd: &str,
        known: &BTreeSet<String>,
    ) -> Result<(), CoreError> {
        if !known.contains(cmd) {
            return Err(error::keymap(format!("unowned script command {cmd}")));
        }
        self.set_script_binding_validated(mode, keys, cmd)
    }

    fn set_script_binding_validated(
        &self,
        mode: &str,
        keys: &str,
        cmd: &str,
    ) -> Result<(), CoreError> {
        const MAX_SCRIPT_BINDINGS: usize = 1024;
        let mut g = self.lock();
        let mut keymap = g.keymap.clone();
        let chord = keymap.set_script_binding(mode, keys, cmd)?;
        if let Some(existing) = g
            .script_bindings
            .iter_mut()
            .find(|(bound_mode, bound_keys, _)| bound_mode == mode && bound_keys == &chord)
        {
            existing.2 = cmd.to_string();
            g.keymap = keymap;
            return Ok(());
        }
        if g.script_bindings.len() >= MAX_SCRIPT_BINDINGS {
            return Err(error::keymap(format!(
                "script keymap exceeds {MAX_SCRIPT_BINDINGS} bindings"
            )));
        }
        g.keymap = keymap;
        g.script_bindings
            .push((mode.to_string(), chord, cmd.to_string()));
        Ok(())
    }

    /// Remove all user-Lua bindings while retaining config-file bindings.
    pub fn clear_script_bindings(&self) {
        let mut g = self.lock();
        let base = g.base_keymap.clone();
        g.keymap = base;
        g.script_bindings.clear();
    }

    /// Effective UI configuration retained for thin frontends.
    #[must_use]
    pub fn config(&self) -> Config {
        self.lock().config.clone()
    }

    /// Begin an in-cell edit (tests / command handlers).
    pub fn begin_edit(&self, surface: EditSurface, initial: &str) {
        let mut g = self.lock();
        let origin = g.selection.cursor;
        g.edit.begin(surface, origin, initial);
        if g.model == KeyModel::Modal {
            g.mode = Mode::Insert;
        }
    }

    /// Dispatch a toolkit-neutral key through the keymap.
    pub fn handle_key(&self, event: crate::event::KeyEvent) -> crate::keymap::KeyOutcome {
        let mut g = self.lock();
        if !g.edit.is_idle() {
            if matches!(
                (event.code, event.ctrl, event.alt, event.shift),
                (crate::event::KeyCode::Tab, false, false, false)
            ) && g.edit.accept_ghost()
            {
                return crate::keymap::KeyOutcome::Pending;
            }
            match (event.code, event.ctrl, event.alt) {
                (crate::event::KeyCode::Enter, false, true) => {
                    g.edit.insert_char('\n');
                    return crate::keymap::KeyOutcome::Pending;
                }
                (crate::event::KeyCode::Backspace, false, false) => {
                    g.edit.backspace();
                    return crate::keymap::KeyOutcome::Pending;
                }
                (crate::event::KeyCode::Delete, false, false) => {
                    g.edit.delete_forward();
                    return crate::keymap::KeyOutcome::Pending;
                }
                (crate::event::KeyCode::Left, false, false) if !g.edit.point => {
                    g.edit.move_left();
                    return crate::keymap::KeyOutcome::Pending;
                }
                (crate::event::KeyCode::Right, false, false) if !g.edit.point => {
                    g.edit.move_right();
                    return crate::keymap::KeyOutcome::Pending;
                }
                (crate::event::KeyCode::Up, false, false) if !g.edit.point => {
                    g.edit.move_up();
                    return crate::keymap::KeyOutcome::Pending;
                }
                (crate::event::KeyCode::Down, false, false) if !g.edit.point => {
                    g.edit.move_down();
                    return crate::keymap::KeyOutcome::Pending;
                }
                (crate::event::KeyCode::Home, false, false) => {
                    g.edit.move_home();
                    return crate::keymap::KeyOutcome::Pending;
                }
                (crate::event::KeyCode::End, false, false) => {
                    g.edit.move_end();
                    return crate::keymap::KeyOutcome::Pending;
                }
                (
                    crate::event::KeyCode::Left
                    | crate::event::KeyCode::Right
                    | crate::event::KeyCode::Up
                    | crate::event::KeyCode::Down,
                    false,
                    false,
                ) if g.edit.point => {
                    let (dr, dc) = match event.code {
                        crate::event::KeyCode::Left => (0, -1),
                        crate::event::KeyCode::Right => (0, 1),
                        crate::event::KeyCode::Up => (-1, 0),
                        crate::event::KeyCode::Down => (1, 0),
                        _ => (0, 0),
                    };
                    g.selection.move_by(dr, dc);
                    let cell = g.selection.cursor;
                    let _ = g.edit.insert_ref(cell);
                    return crate::keymap::KeyOutcome::Pending;
                }
                (crate::event::KeyCode::Char(c), false, false) => {
                    g.edit.insert_char(c);
                    return crate::keymap::KeyOutcome::Pending;
                }
                _ => {}
            }
            let cmd = match (event.code, event.ctrl, event.alt, event.shift) {
                (crate::event::KeyCode::Esc, false, false, _) => {
                    if g.model == KeyModel::Modal {
                        "mode.normal"
                    } else {
                        "edit.cancel"
                    }
                }
                (crate::event::KeyCode::F(4), false, false, false) => "edit.cycleanchor",
                (crate::event::KeyCode::Tab, false, false, false) => "nav.tab",
                (crate::event::KeyCode::Tab, false, false, true) => "nav.tableft",
                (crate::event::KeyCode::Enter, false, false, true)
                    if g.model == KeyModel::Classic =>
                {
                    "nav.enterup"
                }
                (crate::event::KeyCode::Enter, false, false, false) => {
                    if g.model == KeyModel::Modal {
                        "edit.commit"
                    } else {
                        "nav.enter"
                    }
                }
                _ => "",
            };
            if !cmd.is_empty() {
                return crate::keymap::KeyOutcome::Command {
                    cmd: cmd.into(),
                    args: serde_json::Value::Null,
                    count: 1,
                };
            }
        }
        let mode = g.mode;
        g.keymap.dispatch(mode, event)
    }
}

fn load_inner(config: &LoadedConfig, roots: &KeymapRoots) -> Result<UiInner, CoreError> {
    let keymap = Keymap::load_from_roots(&config.config.keys.file, roots)?;
    let base_keymap = keymap.clone();
    let model = keymap.model;
    let mut colors: [String; 8] = std::array::from_fn(|_| String::new());
    for (i, slot) in colors.iter_mut().enumerate() {
        if let Some(c) = config.theme.roles.get(&format!("references.{i}")) {
            *slot = c.clone();
        }
    }
    if colors.iter().any(String::is_empty) {
        return Err(error::keymap(
            "LoadedConfig.theme.roles must define every reference role from references.0–7",
        ));
    }
    Ok(UiInner {
        mode: Mode::for_model(model),
        model,
        keymap,
        base_keymap,
        script_bindings: Vec::new(),
        selection: Selection::a1(SheetId::new(0)),
        edit: EditState::default(),
        viewport: Viewport::default(),
        palette: Palette::default(),
        panel: PanelState {
            visible: None,
            body: None,
            side: config.config.layout.panel_side.clone(),
            width: config.config.layout.panel_width,
            grid_focused: true,
        },
        changeset_review: None,
        agent_panel: AgentPanel::default(),
        formula_assist: None,
        import_review: None,
        status: StatusLine::default(),
        undo: UndoHistory::default(),
        session: SessionState::default(),
        find: FindReplace::default(),
        goto: GoTo::default(),
        clipboard: None,
        reference_colors: colors,
        enter_moves: config.config.behavior.enter_moves.clone(),
        status_ids: config.config.layout.status_line.clone(),
        formula_bar_expanded: false,
        show_formulas: false,
        config: config.config.clone(),
        agent_visible: false,
        pending_agent: None,
    })
}
