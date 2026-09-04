//! Shared UI logic: modes, keymaps, selection, editing, palette, viewport.
//!
//! No `egui`, `ratatui`, or `winit` types. Front-ends map toolkit events into
//! [`event::KeyEvent`] and render [`session::UiSession`].
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod agent_panel;
mod assist;
mod clipboard;
mod command_args;
mod complete;
mod deferred;
mod edit;
mod error;
mod event;
mod fill;
mod find;
mod formula_assist;
mod import_review;
mod keymap;
mod local;
mod mode;
mod name;
mod palette;
mod panel;
mod persist;
mod review;
mod selection;
mod session;
mod status;
mod undo;
mod view;
mod viewport;

pub use agent_panel::{AgentPanel, AgentRole, AgentTurn};
pub use clipboard::{ClipboardPayload, INTERNAL_MIME};
pub use command_args::inject_selection_args;
pub use complete::{Completion, CompletionSource, complete_functions};
pub use deferred::{
    COMPOSITION_COMMANDS, DEFERRED_COMMANDS, DeferredCommand, is_composition_command,
    owner as deferred_owner,
};
pub use edit::{EditState, EditSurface, canonicalize_entry};
pub use event::{KeyCode, KeyEvent};
pub use fill::{FillKind, detect_series, extend_series};
pub use find::{FindReplace, FindScope, GoTo, apply_search_result};
pub use formula_assist::{FormulaAssist, FormulaReference};
pub use import_review::ImportPlanReview;
pub use keymap::{Binding, KeyOutcome, Keymap, KeymapRoots, command_is_known, resolve_keymap_path};
pub use local::{apply_local_command, command_changes_workbook, is_local_command};
pub use mode::{KeyModel, Mode};
pub use omacell_io::csv::MAX_CLIPBOARD_BYTES;
pub use palette::{AiPlanProvider, Palette, PaletteHit};
pub use panel::{PanelState, apply_command_panel};
pub use persist::SessionState;
pub use review::{ChangesetReview, ReviewCellMark, ReviewItem};
pub use selection::{Area, ExtendMode, Selection, SelectionStats, SelectionStatsProvider};
pub use session::{AgentHandoff, UiSession};
pub use status::{StatusLine, StatusSegment, ai_status_text, diagnose_offer};
pub use undo::{UndoEntry, UndoHistory};
pub use view::register_ui_commands;
pub use viewport::{Viewport, conditional_format_ranges};
