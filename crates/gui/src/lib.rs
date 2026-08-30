//! Graphical UI for Omacell (eframe/egui on wgpu).
//!
//! Thin renderer over `omacell-ui` and `omacell-bus`. The CLI composition root
//! supplies one `ConfigStore` and `TaskRunner`; this crate does not load config
//! itself and does not lock `Bus` on the UI thread.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod app;
mod chrome;
mod grid;
mod input;
mod theme;

pub use app::{Gui, Launch, run};
pub use theme::{GuiTheme, hex_color};
