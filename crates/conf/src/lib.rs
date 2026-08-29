//! Layered TOML configuration, Omarchy theme/font resolution, and watchers.
//!
//! ```
//! use omacell_conf::schema::package_defaults;
//! let cfg = package_defaults().unwrap();
//! assert_eq!(cfg.schema, 1);
//! assert!(cfg.appearance.grid_lines);
//! ```
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod error;
pub mod font;
pub mod keys;
pub mod layer;
pub mod paths;
pub mod schema;
pub mod setup;
pub mod theme;
pub mod watch;

pub use layer::{
    Explain, Layer, LoadedConfig, Provenance, load, load_with_env, reset_user_file, show_all_json,
};
pub use paths::Paths;
pub use schema::Config;
pub use setup::{HYPRLAND_SNIPPET, SetupOptions, SetupReport, setup_omarchy};
pub use theme::{ColorsToml, Rgb, ThemeRoles, mix, resolve_roles};
pub use watch::{ConfigStore, ReloadEvent};
