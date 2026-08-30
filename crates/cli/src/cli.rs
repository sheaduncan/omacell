//! Clap command tree (spec F-10.5).

use std::path::PathBuf;

use clap::{ArgAction, CommandFactory, Parser, Subcommand, ValueEnum};
use omacell_core::PRODUCT_NAME;

/// F-10.5 command tree. Tests snapshot `--help` from [`command`].
#[derive(Debug, Parser)]
#[command(
    name = PRODUCT_NAME,
    version,
    about = "A spreadsheet for Omarchy Linux",
    long_about = None,
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Machine-readable stdout (and stderr errors).
    #[arg(long, global = true)]
    pub json: bool,
    /// Validate writes without changing files.
    #[arg(long, global = true)]
    pub dry_run: bool,
    /// Overlay a dotted config key (`appearance.grid_lines=false`). Repeatable.
    #[arg(long = "set", global = true, value_name = "KEY=VALUE", action = ArgAction::Append)]
    pub sets: Vec<String>,
    /// Explicit config file; replaces `~/.config/omacell/config.toml`.
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Explicit theme `colors.toml`; wins over `OMACELL_THEME`.
    #[arg(long, global = true, value_name = "FILE")]
    pub theme: Option<PathBuf>,
    /// Suppress non-error human output.
    #[arg(short, long, global = true)]
    pub quiet: bool,
    /// Increase log verbosity.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,
    /// Open the terminal UI.
    #[arg(long)]
    pub tui: bool,
    /// Workbook whose frozen settings overlay configuration.
    #[arg(long = "from-workbook", global = true, value_name = "FILE")]
    pub from_workbook: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Commands>,
    /// Workbook paths. With no subcommand, opens the graphical UI.
    #[arg(value_name = "FILE")]
    pub files: Vec<PathBuf>,
}

/// Top-level subcommands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Convert a workbook between `.xlsx`, CSV, and `.omc`.
    Convert {
        /// Input path.
        input: PathBuf,
        /// Output path (extension selects format).
        output: PathBuf,
        /// Sheet name.
        #[arg(long)]
        sheet: Option<String>,
        /// A1 range to export.
        #[arg(long)]
        range: Option<String>,
        /// Shared WP-08 CSV import plan JSON.
        #[arg(long, value_name = "FILE")]
        plan: Option<PathBuf>,
    },
    /// Print a range as json, csv, or markdown.
    Query {
        /// Workbook path.
        book: PathBuf,
        /// A1 range (`Sheet!A1:D20`).
        range: String,
        /// Output table format.
        #[arg(long, value_enum, default_value_t = QueryFormat::Json)]
        format: QueryFormat,
        /// Emit formula-bar text instead of values.
        #[arg(long)]
        formulas: bool,
    },
    /// Set a cell or range from formula-bar text.
    Set {
        /// Workbook path.
        book: PathBuf,
        /// A1 cell or range.
        range: String,
        /// Formula-bar text.
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    /// Evaluate a formula in a workbook.
    Eval {
        /// Workbook path.
        book: PathBuf,
        /// Formula (`=SUM(A1:A10)`).
        formula: String,
    },
    /// Recalculate a workbook.
    Recalc {
        /// Workbook path.
        book: PathBuf,
        /// Write the recalculated workbook back.
        #[arg(long)]
        write: bool,
    },
    /// Run a Lua script (WP-20).
    Run {
        /// Script path.
        script: PathBuf,
        /// Workbook path.
        book: PathBuf,
    },
    /// Function catalog.
    Fn {
        #[command(subcommand)]
        cmd: FnCmd,
    },
    /// Configuration.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Theme inspection and reload.
    Theme {
        #[command(subcommand)]
        cmd: ThemeCmd,
    },
    /// Keymap / Hyprland conflict check.
    Keys {
        #[command(subcommand)]
        cmd: KeysCmd,
    },
    /// Omarchy integration.
    Setup {
        #[command(subcommand)]
        cmd: SetupCmd,
    },
    /// List registered commands and JSON schemas.
    #[command(name = "commands")]
    Catalog,
    /// Talk to a running instance.
    Ipc {
        /// Registry command id or control op (`ping`, `theme.reload`).
        command: String,
        /// JSON object arguments.
        payload: Option<String>,
        /// Send to every live owned instance.
        #[arg(long, conflicts_with = "socket")]
        all: bool,
        /// Suppress per-instance human output.
        #[arg(long)]
        quiet: bool,
        /// Socket path (default: newest live instance).
        #[arg(long, conflicts_with = "all")]
        socket: Option<PathBuf>,
    },
    /// Changesets on a running instance.
    Changeset {
        #[command(subcommand)]
        cmd: ChangesetCmd,
    },
    /// Semantic `.xlsx` diff.
    Diff {
        /// First workbook.
        a: PathBuf,
        /// Second workbook.
        b: PathBuf,
    },
    /// Deterministic audit (WP-19).
    Audit {
        /// Workbook path.
        book: PathBuf,
    },
    /// AI provider commands (WP-22).
    Ai {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Hand off to the Omarchy default agent (WP-21).
    Agent {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// MCP server (WP-21).
    Mcp {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

/// `omacell query --format`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum QueryFormat {
    /// JSON array of rows.
    #[default]
    Json,
    /// CSV.
    Csv,
    /// GitHub-flavored markdown table.
    Md,
}

/// `omacell fn`.
#[derive(Debug, Subcommand)]
pub enum FnCmd {
    /// List the function catalog.
    List,
    /// Print one function's documentation.
    Doc {
        /// Canonical name or alias.
        name: String,
    },
}

/// `omacell config`.
#[derive(Debug, Subcommand)]
pub enum ConfigCmd {
    /// Validate the effective configuration.
    Check,
    /// Open the user config in `$EDITOR`.
    Edit,
    /// Restore package defaults (backup first).
    Reset {
        /// Relative file under `~/.config/omacell` (default `config.toml`).
        file: Option<String>,
    },
    /// Show one key or the full effective config.
    Show {
        /// Dotted key (`appearance.grid_lines`).
        key: Option<String>,
        /// Dump every key, theme, shell, and provenance.
        #[arg(long)]
        all: bool,
        /// Include winning layer and source.
        #[arg(long)]
        explain: bool,
    },
    /// Diff the user file against package defaults.
    Diff,
}

/// `omacell theme`.
#[derive(Debug, Subcommand)]
pub enum ThemeCmd {
    /// Print resolved roles and shell tokens.
    Show,
    /// Re-read theme and config files.
    Reload,
}

/// `omacell keys`.
#[derive(Debug, Subcommand)]
pub enum KeysCmd {
    /// Compare classic chords to Hyprland `bindings.lua`.
    Check {
        /// Path to `bindings.lua` (default `~/.config/hypr/bindings.lua`).
        #[arg(long)]
        hyprland: Option<PathBuf>,
    },
}

/// `omacell setup`.
#[derive(Debug, Subcommand)]
pub enum SetupCmd {
    /// Install the Omarchy theme template, hook, and skill links.
    Omarchy {
        /// Print the Hyprland snippet and exit.
        #[arg(long)]
        show_hyprland: bool,
        /// Write Omarchy menu rows (otherwise skipped unless confirmed on a TTY).
        #[arg(long)]
        menu: bool,
    },
}

/// `omacell changeset`.
#[derive(Debug, Subcommand)]
pub enum ChangesetCmd {
    /// List stored changesets.
    List,
    /// Show one changeset.
    Show {
        /// Changeset id.
        id: String,
    },
    /// Apply a proposed changeset.
    Apply {
        /// Changeset id.
        id: String,
    },
    /// Revert an applied changeset.
    Revert {
        /// Changeset id.
        id: String,
    },
    /// Export a changeset as `.omc`.
    Export {
        /// Changeset id.
        id: String,
        /// Destination `.omc` path.
        #[arg(long = "omc", value_name = "FILE")]
        omc: PathBuf,
    },
}

/// Clap command used by help snapshots, completions, and the arg fuzzer.
#[must_use]
pub fn command() -> clap::Command {
    Cli::command()
}
