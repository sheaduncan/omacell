//! Shared UI logic: modes, keymaps, selection, editing, palette, viewport.
//!
//! No `egui`, `ratatui`, or `winit` types. Front-ends map toolkit events into
//! [`event::KeyEvent`] and render [`session::UiSession`].
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod clipboard;
mod complete;
mod deferred;
mod edit;
mod error;
mod event;
mod fill;
mod find;
mod keymap;
mod local;
mod mode;
mod palette;
mod panel;
mod persist;
mod selection;
mod session;
mod status;
mod undo;
mod view;
mod viewport;

pub use clipboard::{ClipboardPayload, INTERNAL_MIME};
pub use complete::{Completion, CompletionSource, complete_functions};
pub use deferred::{DEFERRED_COMMANDS, DeferredCommand, owner as deferred_owner};
pub use edit::{EditState, EditSurface, canonicalize_entry};
pub use event::{KeyCode, KeyEvent};
pub use fill::{FillKind, detect_series, extend_series};
pub use find::{FindReplace, FindScope, GoTo};
pub use keymap::{Binding, KeyOutcome, Keymap, KeymapRoots, command_is_known, resolve_keymap_path};
pub use local::{apply_local_command, is_local_command};
pub use mode::{KeyModel, Mode};
pub use palette::{AiPlanProvider, Palette, PaletteHit};
pub use panel::PanelState;
pub use persist::SessionState;
pub use selection::{Area, ExtendMode, Selection, SelectionStats, SelectionStatsProvider};
pub use session::UiSession;
pub use status::{StatusLine, StatusSegment};
pub use undo::{UndoEntry, UndoHistory};
pub use view::register_ui_commands;
pub use viewport::Viewport;
