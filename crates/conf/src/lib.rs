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

pub mod agent;
pub mod error;
pub mod font;
pub mod keys;
pub mod layer;
pub mod notify;
pub mod paths;
pub mod schema;
pub mod setup;
pub mod theme;
mod validate;
pub mod watch;

pub use agent::{DefaultAgent, HandOff, HandOffRequest, detect_default_agent, hand_off, on_path};
pub use layer::{
    Explain, Layer, LoadOptions, LoadedConfig, Migration, Provenance, load, load_with_env,
    load_with_options, reset_user_file, reset_user_rel, show_all_json, validate_user_rel,
    workbook_settings_overlay,
};
pub use notify::{NotifyKind, allowed as notify_allowed, send as notify_send};
pub use paths::Paths;
pub use schema::Config;
pub use setup::{HYPRLAND_SNIPPET, SetupOptions, SetupReport, setup_omarchy};
pub use theme::{ColorsToml, Rgb, ThemeRoles, mix, resolve_roles, resolve_roles_with_override};
pub use watch::{ConfigStore, ReloadEvent, ReloadHandle};
