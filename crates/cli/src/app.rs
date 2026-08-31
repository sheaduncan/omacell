//! Composition root: one `Paths`, one `LoadOptions`, one bus.

use std::path::Path;
use std::sync::Arc;

use omacell_ai::cache::AiCache;
use omacell_ai::http::{ReqwestTransport, SharedTransport};
use omacell_ai::runtime::AiRuntime;
use omacell_ai::{PromptSet, register_ai_functions};
use omacell_bus::Bus;
use omacell_conf::layer::LoadOptions;
use omacell_conf::{
    ConfigStore, LoadedConfig, Paths, ReloadHandle, merge_overlays, workbook_settings_overlay,
};
use omacell_core::command::{Origin, Outcome};
use omacell_core::error::CoreError;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_io::csv::ImportPlan;
use omacell_ui::{KeymapRoots, UiSession, register_ui_commands};
use toml::Value as TomlValue;

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
    /// AI runtime (async cells + `ai.*` commands).
    pub ai: Option<Arc<AiRuntime>>,
    /// Tokio runtime backing [`Self::ai`].
    #[allow(dead_code)]
    pub ai_tokio: Option<tokio::runtime::Runtime>,
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
            options.workbook = Some(workbook_overlay(&opened.workbook));
        }
        Self::from_parts(paths, options, Workbook::new(), FileSession::new(), false)
    }

    /// Bootstrap from an already-opened workbook (JSON `--jq`, etc.).
    pub fn with_opened(
        cli: &Cli,
        book: &Path,
        opened: crate::files::Opened,
    ) -> Result<Self, CoreError> {
        let paths = Paths::from_env()?;
        let mut options = LoadOptions::from_process();
        options.config_file = cli.config.clone();
        options.theme_override = cli.theme.clone();
        options.cli_sets = cli.sets.clone();
        options.workbook = Some(workbook_overlay(&opened.workbook));
        let file_session = FileSession::new();
        file_session.attach(book, &opened);
        Self::from_parts(paths, options, opened.workbook, file_session, false)
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
        options.workbook = Some(workbook_overlay(&opened.workbook));
        let file_session = FileSession::new();
        file_session.attach(book, &opened);
        Self::from_parts(paths, options, opened.workbook, file_session, false)
    }

    /// Bootstrap from workbook bytes that were read once for an embedded-script
    /// trust decision.
    pub fn with_scriptable_workbook_bytes(
        cli: &Cli,
        book: &Path,
        bytes: &[u8],
    ) -> Result<Self, CoreError> {
        let paths = Paths::from_env()?;
        let opened = files::open_scriptable_bytes(book, bytes)?;
        let mut options = LoadOptions::from_process();
        options.config_file = cli.config.clone();
        options.theme_override = cli.theme.clone();
        options.cli_sets = cli.sets.clone();
        options.workbook = Some(workbook_overlay(&opened.workbook));
        let file_session = FileSession::new();
        file_session.attach(book, &opened);
        Self::from_parts(paths, options, opened.workbook, file_session, false)
    }

    /// Live TUI/GUI composition: watcher-enabled [`ConfigStore`].
    pub fn bootstrap_live(cli: &Cli, book: Option<&Path>) -> Result<Self, CoreError> {
        match book {
            Some(path) => {
                let paths = Paths::from_env()?;
                let opened = files::open_any(path)?;
                let mut options = LoadOptions::from_process();
                options.config_file = cli.config.clone();
                options.theme_override = cli.theme.clone();
                options.cli_sets = cli.sets.clone();
                options.workbook = if let Some(settings_path) = &cli.from_workbook {
                    let settings_book = files::open_any(settings_path)?;
                    Some(workbook_ai_overlay(
                        workbook_settings_overlay(settings_book.workbook.settings()),
                        &opened.workbook,
                    ))
                } else {
                    Some(workbook_overlay(&opened.workbook))
                };
                let file_session = FileSession::new();
                file_session.attach(path, &opened);
                Self::from_parts(paths, options, opened.workbook, file_session, true)
            }
            None => {
                let paths = Paths::from_env()?;
                let mut options = LoadOptions::from_process();
                options.config_file = cli.config.clone();
                options.theme_override = cli.theme.clone();
                options.cli_sets = cli.sets.clone();
                if let Some(settings_path) = &cli.from_workbook {
                    let settings_book = files::open_any(settings_path)?;
                    options.workbook = Some(workbook_overlay(&settings_book.workbook));
                }
                Self::from_parts(paths, options, Workbook::new(), FileSession::new(), true)
            }
        }
    }

    /// Bind a WP-14 session onto this composition root (call once, after core commands).
    pub fn attach_session(
        &mut self,
        config_file: Option<&Path>,
    ) -> Result<(UiSession, KeymapRoots), CoreError> {
        let loaded = self.store.snapshot();
        let roots = KeymapRoots::new(
            self.paths.user_config.clone(),
            self.paths.default_dir.clone(),
            config_file,
        );
        let ui = UiSession::new(&loaded, &roots)?;
        ui.set_agent_visible(omacell_conf::detect_default_agent().is_some());
        register_ui_commands(self.bus.registry_mut(), &ui)?;
        Ok((ui, roots))
    }

    fn from_parts(
        paths: Paths,
        options: LoadOptions,
        mut workbook: Workbook,
        file_session: FileSession,
        watch: bool,
    ) -> Result<Self, CoreError> {
        let store = if watch {
            ConfigStore::load_and_watch_with(paths.clone(), options.clone())?
        } else {
            ConfigStore::load_with(paths.clone(), options.clone())?
        };
        file_session.attach_config(store.handle());
        let mut registry = FnRegistry::new();
        register_all(&mut registry);
        register_ai_functions(&mut registry);
        let mut engine = RecalcEngine::new(registry);
        let loaded = store.snapshot();
        let (ai, ai_tokio) = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(rt) => {
                let handle = rt.handle().clone();
                let transport: SharedTransport = match ReqwestTransport::new() {
                    Ok(t) => Arc::new(t),
                    Err(_) => {
                        engine.recalc_rebuild(&mut workbook);
                        let mut bus = Bus::new(workbook, engine)?;
                        files::register_file_commands(&mut bus, file_session.clone())?;
                        reload::register_theme_reload(&mut bus, store.handle())?;
                        omacell_bus::register_chart_commands(bus.registry_mut())?;
                        omacell_bus::register_edit_commands(bus.registry_mut())?;
                        omacell_bus::register_data_commands(bus.registry_mut())?;
                        omacell_bus::register_audit_commands(bus.registry_mut())?;
                        omacell_bus::register_analysis_commands(bus.registry_mut())?;
                        let script_gate = omacell_lua::ScriptGate::default();
                        omacell_lua::register_script_commands(
                            bus.registry_mut(),
                            script_gate.clone(),
                        )?;
                        omacell_lua::attach_recorder(&mut bus, &script_gate);
                        return Ok(Self {
                            paths,
                            store,
                            bus,
                            files: file_session,
                            ai: None,
                            ai_tokio: None,
                        });
                    }
                };
                let prompts = PromptSet::load(&paths.default_dir, Some(&paths.user_config))
                    .unwrap_or_else(|_| PromptSet::builtin());
                let cache = workbook
                    .custom_parts
                    .get(omacell_ai::cache::AICACHE_PART)
                    .map(|b| AiCache::from_bytes(b))
                    .unwrap_or_default();
                let runtime = AiRuntime::new(
                    handle,
                    loaded.config.clone(),
                    transport,
                    prompts,
                    paths.home.join(".cache/omacell"),
                    paths.state_dir.clone(),
                    cache,
                );
                engine.set_async_provider(runtime.clone());
                (Some(runtime), Some(rt))
            }
            Err(_) => (None, None),
        };
        engine.recalc_rebuild(&mut workbook);
        let mut bus = Bus::new(workbook, engine)?;
        files::register_file_commands(&mut bus, file_session.clone())?;
        reload::register_theme_reload(&mut bus, store.handle())?;
        omacell_bus::register_chart_commands(bus.registry_mut())?;
        omacell_bus::register_edit_commands(bus.registry_mut())?;
        omacell_bus::register_data_commands(bus.registry_mut())?;
        omacell_bus::register_audit_commands(bus.registry_mut())?;
        omacell_bus::register_analysis_commands(bus.registry_mut())?;
        if let Some(runtime) = ai.clone() {
            crate::ai_cmd::register_ai_commands(
                &mut bus,
                crate::ai_cmd::AiSession {
                    runtime: Arc::clone(&runtime),
                },
            )?;
            file_session.attach_ai(Arc::clone(&runtime));
        }
        if let Some(runtime) = &ai {
            let catalog = bus
                .registry()
                .iter()
                .filter(|(_, cmd)| {
                    cmd.exposure == omacell_bus::Exposure::Public && cmd.changeset_eligible
                })
                .map(|(id, cmd)| {
                    let args = serde_json::to_value(&cmd.descriptor.arg_schema).map_err(|err| {
                        CoreError::new(
                            "ai.catalog",
                            format!("cannot serialize schema for {id}: {err}"),
                        )
                    })?;
                    Ok((
                        id.to_string(),
                        serde_json::json!({
                            "id": id,
                            "doc": cmd.descriptor.doc,
                            "args": args,
                        }),
                    ))
                })
                .collect::<Result<Vec<_>, CoreError>>()?;
            runtime.set_catalog(catalog);
        }
        let script_gate = omacell_lua::ScriptGate::default();
        omacell_lua::register_script_commands(bus.registry_mut(), script_gate.clone())?;
        omacell_lua::attach_recorder(&mut bus, &script_gate);
        Ok(Self {
            paths,
            store,
            bus,
            files: file_session,
            ai,
            ai_tokio,
        })
    }

    /// Effective configuration snapshot.
    #[must_use]
    pub fn loaded(&self) -> LoadedConfig {
        self.store.snapshot()
    }

    /// Shared reload target for SIGUSR1 / WP-15.
    #[must_use]
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

fn workbook_overlay(workbook: &Workbook) -> TomlValue {
    workbook_ai_overlay(workbook_settings_overlay(workbook.settings()), workbook)
}

fn workbook_ai_overlay(base: TomlValue, workbook: &Workbook) -> TomlValue {
    match omacell_ai::workbook_config_overlay(workbook) {
        Some(extra) => merge_overlays(base, extra),
        None => base,
    }
}
