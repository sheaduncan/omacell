//! Terminal UI for Omacell, built on ratatui.
//!
//! Thin renderer over `omacell-ui` and `omacell-bus`. The CLI composition root
//! supplies one `ConfigStore` and bus; this crate does not load config itself.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod app;
mod graphics;
mod input;
mod render;
mod theme;

pub use app::{Launch, Tui, run};
pub use input::{map_key, map_mouse};
pub use render::{FrameInput, buffer_text, prepare_viewport};
pub use theme::{AnsiRoles, file_color, graphics_protocol, truecolor_enabled};
