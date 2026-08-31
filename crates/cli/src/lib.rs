//! Command-line adapter over the command bus, I/O, and configuration.
//!
//! The binary is a thin composition root. File and theme commands are registered
//! here because `omacell-io` cannot depend on `omacell-bus`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod agent;
mod app;
mod cli;
mod error;
mod files;
mod log;
mod mcp;
mod output;
mod reload;
mod run;

pub use cli::command;
pub use error::{EXIT_ERROR, EXIT_NYI, EXIT_OK, EXIT_USAGE};
pub use files::{FileSession, register_file_commands};
pub use reload::{register_theme_reload, spawn_sigusr1_reloader};
pub use run::{run, write_dist};

/// Parse CLI arguments without executing a command.
pub fn try_parse<I, T>(args: I) -> Result<clap::ArgMatches, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    command().try_get_matches_from(args)
}
