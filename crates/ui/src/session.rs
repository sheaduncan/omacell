//! `UiSession` and `apply_config`.

use std::sync::{Arc, Mutex};

use omacell_conf::LoadedConfig;
use omacell_core::addr::SheetId;
use omacell_core::error::CoreError;

use crate::clipboard::ClipboardPayload;
use crate::edit::{EditState, EditSurface};
use crate::error;
use crate::find::{FindReplace, GoTo};
use crate::keymap::{Keymap, KeymapRoots, resolve_keymap_path};
use crate::mode::{KeyModel, Mode};
use crate::palette::Palette;
use crate::panel::PanelState;
use crate::persist::SessionState;
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
    pub selection: Selection,
    pub edit: EditState,
    pub viewport: Viewport,
    pub palette: Palette,
    pub panel: PanelState,
    pub status: StatusLine,
    pub undo: UndoHistory,
    pub session: SessionState,
    pub find: FindReplace,
    pub goto: GoTo,
    pub clipboard: Option<ClipboardPayload>,
    pub reference_colors: [String; 8],
    pub enter_moves: String,
    pub status_ids: Vec<String>,
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
    ) -> Result<(), CoreError> {
        let mut g = self.lock();
        let mode = g.mode;
        let edit = g.edit.clone();
        let selection = g.selection.clone();
        let viewport = g.viewport.clone();
        let undo = g.undo.clone();
        let session = g.session.clone();
        let palette_recents = g.palette.recents.clone();
        let find = g.find.clone();
        let clipboard = g.clipboard.clone();

        let mut next = load_inner(config, roots)?;
        if next.model == g.model {
            next.mode = mode;
        }
        next.edit = edit;
        next.selection = selection;
        next.viewport.freeze = viewport.freeze;
        next.viewport.split = viewport.split;
        next.viewport.first_row = viewport.first_row;
        next.viewport.first_col = viewport.first_col;
        next.viewport.width_px = viewport.width_px;
        next.viewport.height_px = viewport.height_px;
        next.viewport.rows = viewport.rows;
        next.viewport.cols = viewport.cols;
        next.viewport.zoom = viewport.zoom;
        next.undo = undo;
        next.session = session;
        next.palette.recents = palette_recents;
        next.find = find;
        next.clipboard = clipboard;
        *g = next;
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

    /// Current edit state.
    #[must_use]
    pub fn edit(&self) -> EditState {
        self.lock().edit.clone()
    }

    /// Current viewport.
    #[must_use]
    pub fn viewport(&self) -> Viewport {
        self.lock().viewport.clone()
    }

    /// Session persistence blob.
    #[must_use]
    pub fn session_state(&self) -> SessionState {
        self.lock().session.clone()
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
            match event.code {
                crate::event::KeyCode::Esc => {
                    g.edit.cancel();
                    if g.model == KeyModel::Modal {
                        g.mode = Mode::Normal;
                    }
                    g.panel.dismiss();
                    return crate::keymap::KeyOutcome::Command {
                        cmd: "edit.cancel".into(),
                        args: serde_json::Value::Null,
                        count: 1,
                    };
                }
                crate::event::KeyCode::F(4) => {
                    let _ = g.edit.cycle_anchor();
                    return crate::keymap::KeyOutcome::Command {
                        cmd: "edit.cycleanchor".into(),
                        args: serde_json::Value::Null,
                        count: 1,
                    };
                }
                crate::event::KeyCode::Enter => {
                    let _ = g.edit.commit();
                    return crate::keymap::KeyOutcome::Command {
                        cmd: "edit.commit".into(),
                        args: serde_json::Value::Null,
                        count: 1,
                    };
                }
                crate::event::KeyCode::Char(c) if !event.ctrl && !event.alt => {
                    g.edit.insert_char(c);
                    return crate::keymap::KeyOutcome::Pending;
                }
                _ => {}
            }
        }
        let mode = g.mode;
        g.keymap.dispatch(mode, event)
    }
}

fn load_inner(config: &LoadedConfig, roots: &KeymapRoots) -> Result<UiInner, CoreError> {
    let path = resolve_keymap_path(&config.config.keys.file, roots)?;
    let user_overlay = roots.user_config.join(&config.config.keys.file);
    let overlay = if user_overlay != path && user_overlay.is_file() {
        Some(user_overlay.as_path())
    } else {
        None
    };
    let keymap = Keymap::load(&path, overlay)?;
    let model = keymap.model;
    let mut colors: [String; 8] = std::array::from_fn(|_| String::new());
    for (i, slot) in colors.iter_mut().enumerate() {
        if let Some(c) = config.theme.roles.get(&format!("references.{i}")) {
            *slot = c.clone();
        }
    }
    if colors.iter().all(|c| c.is_empty()) {
        return Err(error::keymap(
            "LoadedConfig.theme.roles missing references.0–7",
        ));
    }
    Ok(UiInner {
        mode: Mode::for_model(model),
        model,
        keymap,
        selection: Selection::a1(SheetId::new(0)),
        edit: EditState::default(),
        viewport: Viewport::default(),
        palette: Palette::default(),
        panel: PanelState {
            visible: None,
            side: config.config.layout.panel_side.clone(),
            width: config.config.layout.panel_width,
            grid_focused: true,
        },
        status: StatusLine::default(),
        undo: UndoHistory::default(),
        session: SessionState::default(),
        find: FindReplace::default(),
        goto: GoTo::default(),
        clipboard: None,
        reference_colors: colors,
        enter_moves: config.config.behavior.enter_moves.clone(),
        status_ids: config.config.layout.status_line.clone(),
    })
}
