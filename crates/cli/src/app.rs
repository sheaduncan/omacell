//! Composition root: one `Paths`, one `LoadOptions`, one bus.

use std::path::Path;

use omacell_bus::Bus;
use omacell_conf::layer::LoadOptions;
use omacell_conf::{ConfigStore, LoadedConfig, Paths, ReloadHandle, workbook_settings_overlay};
use omacell_core::command::{Origin, Outcome};
use omacell_core::error::CoreError;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_io::csv::ImportPlan;

use crate::cli::Cli;
use crate::files::{self, FileSession};
use crate::reload;

/// Process-wide CLI session.
pub struct App {
    /// Resolved XDG paths.
    pub paths: Paths,
    /// Last-good configuration.
    pub store: ConfigStore,
    /// Command bus.
    pub bus: Bus,
    /// File sidecar (retained so save/export keep package bytes).
    #[allow(dead_code)]
    pub files: FileSession,
}

impl App {
    /// Build paths + config without a workbook overlay.
    pub fn bootstrap(cli: &Cli) -> Result<Self, CoreError> {
        let paths = Paths::from_env()?;
        let mut options = LoadOptions::from_process();
        options.config_file = cli.config.clone();
        options.theme_override = cli.theme.clone();
        options.cli_sets = cli.sets.clone();
        if let Some(path) = &cli.from_workbook {
            let opened = files::open_any(path)?;
            options.workbook = Some(workbook_settings_overlay(opened.workbook.settings()));
        }
        Self::from_parts(paths, options, Workbook::new(), FileSession::new())
    }

    /// Bootstrap with an optional shared CSV import plan.
    pub fn with_workbook_plan(
        cli: &Cli,
        book: &Path,
        plan: Option<&ImportPlan>,
    ) -> Result<Self, CoreError> {
        let paths = Paths::from_env()?;
        let opened = files::open_any_with_plan(book, plan)?;
        let mut options = LoadOptions::from_process();
        options.config_file = cli.config.clone();
        options.theme_override = cli.theme.clone();
        options.cli_sets = cli.sets.clone();
        options.workbook = Some(workbook_settings_overlay(opened.workbook.settings()));
        let file_session = FileSession::new();
        file_session.attach(book, &opened);
        Self::from_parts(paths, options, opened.workbook, file_session)
    }

    fn from_parts(
        paths: Paths,
        options: LoadOptions,
        mut workbook: Workbook,
        file_session: FileSession,
    ) -> Result<Self, CoreError> {
        let store = ConfigStore::load_with(paths.clone(), options.clone())?;
        file_session.attach_config(store.handle());
        let mut registry = FnRegistry::new();
        register_all(&mut registry);
        let mut engine = RecalcEngine::new(registry);
        engine.recalc_rebuild(&mut workbook);
        let mut bus = Bus::new(workbook, engine)?;
        files::register_file_commands(&mut bus, file_session.clone())?;
        reload::register_theme_reload(&mut bus, store.handle())?;
        Ok(Self {
            paths,
            store,
            bus,
            files: file_session,
        })
    }

    /// Effective configuration snapshot.
    #[must_use]
    pub fn loaded(&self) -> LoadedConfig {
        self.store.snapshot()
    }

    /// Shared reload target for SIGUSR1 / WP-15.
    #[must_use]
    #[allow(dead_code)]
    pub fn reload_handle(&self) -> ReloadHandle {
        self.store.handle()
    }

    /// Execute a registry command as the CLI user.
    pub fn execute(&mut self, id: &str, args: serde_json::Value) -> Outcome {
        self.bus.execute(Origin::User, id, args)
    }

    /// Dry-run a registry command.
    pub fn dry_run(
        &mut self,
        id: &str,
        args: serde_json::Value,
    ) -> Result<omacell_bus::DryRun, CoreError> {
        self.bus.dry_run(Origin::User, id, args)
    }
}
