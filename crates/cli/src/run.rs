//! Subcommand dispatch.

use std::ffi::{OsStr, OsString};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;
use omacell_bus::ipc::{
    ControlOp, IpcClient, Mode, default_runtime_dir, discovered_socket, list_live_instances,
};
use omacell_conf::schema::package_defaults;
use omacell_conf::{
    HYPRLAND_SNIPPET, Layer, LoadOptions, LoadedConfig, Paths, SetupOptions, keys,
    load_with_options, reset_user_file, reset_user_rel, setup_omarchy, show_all_json,
    validate_user_rel,
};
use omacell_core::addr::{RefKind, parse_a1};
use omacell_core::command::Outcome;
use omacell_core::eval::{eval_formula_in, format_runtime};
use omacell_core::formula::parse;
use omacell_core::graph::CellCoord;
use omacell_core::spill::SpillTable;
use omacell_core::value::Value;
use omacell_core::{PRODUCT_NAME, error::CoreError};
use omacell_fn::{all_specs, functions_json};
use omacell_io::csv::ImportPlan;
use omacell_io::xlsx;

use omacell_gui::Launch as GuiLaunch;
use omacell_tui::Launch;

use crate::app::App;
use crate::cli::{
    ChangesetCmd, Cli, Commands, ConfigCmd, FnCmd, KeysCmd, QueryFormat, SetupCmd, ThemeCmd,
    TrustCmd,
};
use crate::error::{CliError, EXIT_OK, EXIT_USAGE};
use crate::log;
use crate::output::Output;

/// Parse args and run one command. Returns a process exit code.
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    let json = args.iter().any(|arg| arg == OsStr::new("--json"));
    match run_inner(args, json) {
        Ok(code) => code,
        Err(err) => {
            let output = Output { json, quiet: false };
            let _ = output.error(&err);
            err.exit
        }
    }
}

fn run_inner(args: Vec<OsString>, json_requested: bool) -> Result<i32, CliError> {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => {
            if err.use_stderr() && json_requested {
                let err = CliError::from(err);
                let _ = (Output {
                    json: true,
                    quiet: false,
                })
                .error(&err);
                return Ok(err.exit);
            }
            let _ = err.print();
            return Ok(if err.use_stderr() {
                EXIT_USAGE
            } else {
                EXIT_OK
            });
        }
    };
    let output = Output {
        json: cli.json,
        quiet: cli.quiet,
    };
    match dispatch(&cli, output) {
        Ok(()) => Ok(EXIT_OK),
        Err(err) => {
            let _ = output.error(&err);
            Ok(err.exit)
        }
    }
}

fn dispatch(cli: &Cli, output: Output) -> Result<(), CliError> {
    if cli.tui {
        let _ = output;
        return cmd_tui(cli);
    }
    match &cli.command {
        None => cmd_gui(cli),
        Some(Commands::Ai { .. }) => Err(CliError::nyi("omacell ai", "WP-22")),
        Some(Commands::Agent { .. }) => Err(CliError::nyi("omacell agent", "WP-21")),
        Some(Commands::Mcp { .. }) => Err(CliError::nyi("omacell mcp", "WP-21")),
        Some(cmd) => {
            let app_needed = !matches!(
                cmd,
                Commands::Fn { .. }
                    | Commands::Ipc { .. }
                    | Commands::Changeset { .. }
                    | Commands::Diff { .. }
            );
            if app_needed {
                // Logging needs HOME; initialize after we know we will run.
            }
            run_command(cli, cmd, output)
        }
    }
}

fn run_command(cli: &Cli, cmd: &Commands, output: Output) -> Result<(), CliError> {
    match cmd {
        Commands::Fn { cmd } => cmd_fn(cmd, output),
        Commands::Ipc {
            command,
            payload,
            all,
            quiet,
            socket,
        } => cmd_ipc(
            command,
            payload.as_deref(),
            *all,
            *quiet || cli.quiet,
            socket.as_ref(),
            cli.dry_run,
            output,
        ),
        Commands::Changeset { cmd } => cmd_changeset(cmd, cli.dry_run, output),
        Commands::Diff { a, b } => cmd_diff(a, b, output),
        Commands::Convert {
            input,
            output: dest,
            sheet,
            range,
            plan,
        } => cmd_convert(
            cli,
            input,
            dest,
            sheet.as_deref(),
            range.as_deref(),
            plan.as_deref(),
            output,
        ),
        Commands::Query {
            book,
            range,
            format,
            formulas,
        } => cmd_query(cli, book, range, *format, *formulas, output),
        Commands::Set { book, range, value } => cmd_set(cli, book, range, value, output),
        Commands::Eval { book, formula } => cmd_eval(cli, book, formula, output),
        Commands::Recalc { book, write } => cmd_recalc(cli, book, *write, output),
        Commands::Config { cmd } => cmd_config(cli, cmd, output),
        Commands::Theme { cmd } => cmd_theme(cli, cmd, output),
        Commands::Keys { cmd } => cmd_keys(cli, cmd, output),
        Commands::Setup { cmd } => cmd_setup(cli, cmd, output),
        Commands::Catalog => cmd_commands(cli, output),
        Commands::Audit { book } => cmd_audit(cli, book, output),
        Commands::Run {
            script,
            book,
            embedded,
            python,
        } => cmd_run(cli, script, book.as_deref(), *embedded, *python, output),
        Commands::Trust { cmd } => cmd_trust(cli, cmd, output),
        Commands::Ai { .. } | Commands::Agent { .. } | Commands::Mcp { .. } => {
            unreachable!("stubs handled earlier")
        }
    }
}

fn display_available() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some()
}

fn cmd_gui(cli: &Cli) -> Result<(), CliError> {
    if cli.files.len() > 1 {
        return Err(
            CliError::new("cli.usage", "omacell GUI accepts at most one workbook")
                .hint("run a separate Omacell instance for each workbook")
                .exit(EXIT_USAGE),
        );
    }
    if cli.dry_run {
        return Err(
            CliError::new("cli.usage", "--dry-run is not valid for the GUI")
                .hint("use --dry-run with a CLI or IPC command")
                .exit(EXIT_USAGE),
        );
    }
    if !display_available() {
        return Err(CliError::new(
            "gui.display",
            "omacell GUI requires a Wayland or X11 display",
        )
        .hint("set WAYLAND_DISPLAY or DISPLAY, or use omacell --tui"));
    }
    let book = cli.files.first().map(PathBuf::as_path);
    // File loading belongs to the GUI task runner so first paint, progress, and
    // cancellation remain available for large workbooks.
    let mut app = App::bootstrap_live(cli, None)?;
    log::init(&app.paths, cli.verbose, cli.quiet, !cli.dry_run);
    let _sig = crate::reload::spawn_sigusr1_reloader(app.reload_handle())?;
    let (ui, roots) = app.attach_session(cli.config.as_deref())?;
    let launch = GuiLaunch {
        paths: app.paths,
        store: app.store,
        bus: app.bus,
        ui,
        roots,
        long_ops: omacell_bus::LongOps::production(),
        file: book.map(Path::to_path_buf),
        use_shell_font: true,
    };
    omacell_gui::run(launch)?;
    Ok(())
}

fn cmd_tui(cli: &Cli) -> Result<(), CliError> {
    if cli.command.is_some() {
        return Err(
            CliError::new("cli.usage", "--tui cannot be combined with a subcommand")
                .hint("use omacell --tui [FILE] or run the subcommand without --tui")
                .exit(EXIT_USAGE),
        );
    }
    if cli.files.len() > 1 {
        return Err(
            CliError::new("cli.usage", "--tui accepts at most one workbook")
                .hint("run a separate Omacell instance for each workbook")
                .exit(EXIT_USAGE),
        );
    }
    if cli.dry_run {
        return Err(
            CliError::new("cli.usage", "--dry-run is not valid for an interactive TUI")
                .hint("use --dry-run with a CLI or IPC command")
                .exit(EXIT_USAGE),
        );
    }
    if !io::stdout().is_terminal() {
        return Err(
            CliError::new("tui.tty", "omacell --tui requires a terminal")
                .hint("run from a TTY or omit --tui"),
        );
    }
    let book = cli.files.first().map(PathBuf::as_path);
    let mut app = App::bootstrap_live(cli, book)?;
    log::init(&app.paths, cli.verbose, cli.quiet, !cli.dry_run);
    let _sig = crate::reload::spawn_sigusr1_reloader(app.reload_handle())?;
    let (ui, roots) = app.attach_session(cli.config.as_deref())?;
    let launch = Launch {
        paths: app.paths,
        store: app.store,
        bus: app.bus,
        ui,
        roots,
        long_ops: omacell_bus::LongOps::production(),
    };
    omacell_tui::run(launch)?;
    Ok(())
}

fn init_app(cli: &Cli) -> Result<App, CliError> {
    let app = App::bootstrap(cli)?;
    log::init(&app.paths, cli.verbose, cli.quiet, !cli.dry_run);
    Ok(app)
}

fn init_app_book(cli: &Cli, book: &Path) -> Result<App, CliError> {
    init_app_book_plan(cli, book, None)
}

fn init_app_book_plan(cli: &Cli, book: &Path, plan: Option<&ImportPlan>) -> Result<App, CliError> {
    let app = App::with_workbook_plan(cli, book, plan)?;
    log::init(&app.paths, cli.verbose, cli.quiet, !cli.dry_run);
    Ok(app)
}

fn init_paths(cli: &Cli) -> Result<Paths, CliError> {
    let paths = Paths::from_env()?;
    log::init(&paths, cli.verbose, cli.quiet, !cli.dry_run);
    Ok(paths)
}

fn cmd_fn(cmd: &FnCmd, output: Output) -> Result<(), CliError> {
    match cmd {
        FnCmd::List => {
            let json: serde_json::Value = serde_json::from_str(
                &functions_json().map_err(|e| CliError::new("fn.catalog", e.to_string()))?,
            )
            .map_err(|e| CliError::new("fn.catalog", e.to_string()))?;
            let human = json
                .get("functions")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|function| function.get("name").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            output.success(json, &human)?;
            Ok(())
        }
        FnCmd::Doc { name } => {
            let wanted = name.to_ascii_uppercase();
            let spec = all_specs().into_iter().find(|spec| {
                spec.name.eq_ignore_ascii_case(&wanted)
                    || spec
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(&wanted))
            });
            let Some(spec) = spec else {
                return Err(
                    CliError::new("fn.unknown", format!("unknown function {name}"))
                        .hint("omacell fn list --json"),
                );
            };
            let json = serde_json::to_value(spec.to_json())
                .map_err(|e| CliError::new("fn.catalog", e.to_string()))?;
            let human = format!("{} — {}\n{}", spec.name, spec.signature, spec.doc);
            output.success(json, &human)?;
            Ok(())
        }
    }
}

fn cmd_commands(cli: &Cli, output: Output) -> Result<(), CliError> {
    let app = init_app(cli)?;
    let text = app
        .bus
        .commands_json()
        .map_err(|e| CliError::new("command.catalog", e.to_string()))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| CliError::new("command.catalog", e.to_string()))?;
    output.success(json, &text)?;
    Ok(())
}

fn cmd_config(cli: &Cli, cmd: &ConfigCmd, output: Output) -> Result<(), CliError> {
    match cmd {
        ConfigCmd::Check => {
            let app = init_app(cli)?;
            let loaded = app.loaded();
            let json = serde_json::json!({
                "ok": true,
                "schema": loaded.config.schema,
                "migrations": loaded.migrations.iter().map(|m| serde_json::json!({
                    "from": m.from,
                    "to": m.to,
                    "backup": m.backup,
                })).collect::<Vec<_>>(),
            });
            output.success(json, "ok")?;
            Ok(())
        }
        ConfigCmd::Edit => {
            let paths = init_paths(cli)?;
            let path = cli
                .config
                .clone()
                .unwrap_or_else(|| paths.user_config_toml());
            if cli.dry_run {
                output.success(
                    serde_json::json!({"path": path.display().to_string(), "dry_run": true}),
                    &path.display().to_string(),
                )?;
                return Ok(());
            }
            std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))
                .map_err(|e| CliError::new("config.io", e.to_string()))?;
            if !path.is_file() {
                std::fs::write(&path, "").map_err(|e| CliError::new("config.io", e.to_string()))?;
            }
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
            let status = std::process::Command::new(&editor)
                .arg(&path)
                .status()
                .map_err(|e| CliError::new("config.edit", e.to_string()))?;
            if !status.success() {
                return Err(CliError::new("config.edit", format!("{editor} failed")));
            }
            output.success(
                serde_json::json!({"path": path.display().to_string()}),
                &path.display().to_string(),
            )?;
            Ok(())
        }
        ConfigCmd::Reset { file } => {
            let paths = init_paths(cli)?;
            if cli.dry_run {
                if let Some(relative) = file {
                    let _ = validate_user_rel(relative)?;
                }
                output.success(serde_json::json!({"dry_run": true}), "dry-run")?;
                return Ok(());
            }
            let stamp = backup_stamp()?;
            let dest = match file.as_deref() {
                None => reset_user_file(&paths, &stamp)?,
                Some(rel) => reset_user_rel(&paths, &stamp, rel)?,
            };
            output.success(
                serde_json::json!({"backup": dest.as_ref().map(|p| p.display().to_string())}),
                dest.map(|p| p.display().to_string())
                    .unwrap_or_else(|| "nothing to reset".into())
                    .as_str(),
            )?;
            Ok(())
        }
        ConfigCmd::Show { key, all, explain } => {
            let app = init_app(cli)?;
            let loaded = app.loaded();
            if *all {
                let json = show_all_json(&loaded);
                let human = serde_json::to_string_pretty(&json)
                    .map_err(|err| CliError::new("cli.json", err.to_string()))?;
                output.success(json, &human)?;
                return Ok(());
            }
            let Some(key) = key else {
                return Err(CliError::new("config.key", "pass a key or --all")
                    .hint("omacell config show appearance.grid_lines --explain"));
            };
            if *explain {
                let Some(exp) = loaded.explain(key) else {
                    return Err(CliError::new("config.key", format!("unknown key {key}")));
                };
                let json = serde_json::json!({
                    "key": exp.key,
                    "value": exp.value,
                    "layer": exp.layer.as_str(),
                    "source": exp.source,
                });
                let human = format!(
                    "{key} = {}\nlayer: {} ({})",
                    exp.value,
                    exp.layer.as_str(),
                    exp.source
                );
                output.success(json, &human)?;
            } else {
                let Some(value) = loaded.get_json(key) else {
                    return Err(CliError::new("config.key", format!("unknown key {key}")));
                };
                output.success(value.clone(), &value.to_string())?;
            }
            Ok(())
        }
        ConfigCmd::Diff => {
            let paths = init_paths(cli)?;
            let user = cli
                .config
                .clone()
                .unwrap_or_else(|| paths.user_config_toml());
            let loaded = load_with_options(
                &paths,
                &LoadOptions {
                    config_file: cli.config.clone(),
                    ..LoadOptions::default()
                },
            )?;
            let defaults = serde_json::to_value(package_defaults()?)
                .map_err(|err| CliError::new("config.json", err.to_string()))?;
            let changes = loaded
                .provenance
                .iter()
                .filter(|(_, provenance)| provenance.layer == Layer::User)
                .filter_map(|(key, _)| {
                    let user = loaded.get_json(key)?;
                    let package = json_at_dotted(&defaults, key);
                    (package.as_ref() != Some(&user))
                        .then(|| serde_json::json!({"key": key, "package": package, "user": user}))
                })
                .collect::<Vec<_>>();
            let human = changes
                .iter()
                .map(|change| {
                    format!(
                        "{}: {} -> {}",
                        change["key"].as_str().unwrap_or("?"),
                        change["package"],
                        change["user"]
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let json = serde_json::json!({
                "path": user.display().to_string(),
                "changes": changes,
            });
            output.success(json, &human)?;
            Ok(())
        }
    }
}

fn json_at_dotted(value: &serde_json::Value, dotted: &str) -> Option<serde_json::Value> {
    let mut current = value;
    for part in dotted.split('.') {
        current = current.get(part)?;
    }
    Some(current.clone())
}

fn cmd_theme(cli: &Cli, cmd: &ThemeCmd, output: Output) -> Result<(), CliError> {
    let mut app = init_app(cli)?;
    match cmd {
        ThemeCmd::Show => {
            let loaded = app.loaded();
            theme_show(&loaded, output)
        }
        ThemeCmd::Reload => {
            if cli.dry_run {
                output.success(serde_json::json!({"dry_run": true}), "dry-run")?;
                return Ok(());
            }
            let outcome = app.execute("theme.reload", serde_json::json!({}));
            finish_outcome(outcome, output)
        }
    }
}

fn theme_show(loaded: &LoadedConfig, output: Output) -> Result<(), CliError> {
    let json = serde_json::json!({
        "theme": loaded.theme,
        "shell": loaded.shell,
    });
    let human = format!(
        "theme {} ({})\nui font {} {}pt",
        loaded.theme.name,
        loaded.theme.mode,
        loaded.shell.ui_font_family,
        loaded.shell.ui_font_size_pt
    );
    output.success(json, &human)?;
    Ok(())
}

fn cmd_keys(cli: &Cli, cmd: &KeysCmd, output: Output) -> Result<(), CliError> {
    let paths = init_paths(cli)?;
    match cmd {
        KeysCmd::Check { hyprland } => {
            let path = hyprland
                .clone()
                .unwrap_or_else(|| paths.home.join(".config/hypr/bindings.lua"));
            let conflicts = keys::check_hyprland(&path, keys::CLASSIC_CHORDS)?;
            let json = serde_json::json!({
                "path": path.display().to_string(),
                "conflicts": conflicts.iter().map(|c| serde_json::json!({
                    "chord": c.chord,
                    "omacell": c.omacell,
                })).collect::<Vec<_>>(),
            });
            let human = if conflicts.is_empty() {
                "ok".into()
            } else {
                conflicts
                    .iter()
                    .map(|c| format!("{} -> {}", c.chord, c.omacell))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            output.success(json, &human)?;
            Ok(())
        }
    }
}

fn cmd_setup(cli: &Cli, cmd: &SetupCmd, output: Output) -> Result<(), CliError> {
    match cmd {
        SetupCmd::Omarchy {
            show_hyprland,
            menu,
        } => {
            if *show_hyprland {
                output.success(
                    serde_json::json!({"snippet": HYPRLAND_SNIPPET}),
                    HYPRLAND_SNIPPET,
                )?;
                return Ok(());
            }
            let paths = init_paths(cli)?;
            if cli.dry_run {
                output.success(serde_json::json!({"dry_run": true}), "dry-run")?;
                return Ok(());
            }
            let confirm_menu = *menu || prompt_menu();
            let report = setup_omarchy(
                &paths,
                SetupOptions {
                    confirm_menu,
                    link_skill: true,
                },
            )?;
            let _ = keys::check_hyprland(
                &paths.home.join(".config/hypr/bindings.lua"),
                keys::CLASSIC_CHORDS,
            )?;
            let json = serde_json::json!({
                "written": report.written.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                "skipped": report.skipped,
            });
            output.success(json, "ok")?;
            Ok(())
        }
    }
}

fn prompt_menu() -> bool {
    if !io::stdin().is_terminal() {
        return false;
    }
    eprint!("Add Omarchy menu entries? [y/N] ");
    let _ = io::stderr().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim(), "y" | "Y" | "yes")
}

fn cmd_convert(
    cli: &Cli,
    input: &Path,
    dest: &Path,
    sheet: Option<&str>,
    range: Option<&str>,
    plan: Option<&Path>,
    output: Output,
) -> Result<(), CliError> {
    if dest
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("png"))
    {
        return cmd_export_chart(cli, input, dest, sheet, output);
    }
    let plan = plan.map(read_import_plan).transpose()?;
    let mut app = init_app_book_plan(cli, input, plan.as_ref())?;
    let args = serde_json::json!({
        "path": dest.display().to_string(),
        "sheet": sheet,
        "range": range,
    });
    let outcome = if cli.dry_run {
        let dry = app.dry_run("file.export", args)?;
        return finish_outcome(dry.outcome, output);
    } else {
        app.execute("file.export", args)
    };
    finish_outcome(outcome, output)
}

fn cmd_export_chart(
    cli: &Cli,
    input: &Path,
    dest: &Path,
    sheet: Option<&str>,
    output: Output,
) -> Result<(), CliError> {
    let app = init_app_book_plan(cli, input, None)?;
    let loaded = app.store.snapshot();
    let theme = chart_theme_from_roles(&loaded.theme.roles);
    let wb = app.bus.workbook().clone();
    let sheet_id = match sheet {
        Some(name) => wb
            .sheet_by_name(name)
            .map(|s| s.id)
            .ok_or_else(|| CliError::new("chart.export", format!("unknown sheet {name}")))?,
        None => wb.active_sheet(),
    };
    let chart = wb
        .sheet(sheet_id)
        .and_then(|s| s.charts.first())
        .cloned()
        .ok_or_else(|| CliError::new("chart.export", "workbook has no chart on that sheet"))?;
    if cli.dry_run {
        return finish_outcome(
            omacell_core::command::Outcome::success(serde_json::json!({
                "path": dest.display().to_string(),
                "dry_run": true,
            })),
            output,
        );
    }
    let png = dest
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    let bytes = if png {
        omacell_io::chart_export::chart_png(&wb, &chart, &theme, 800, 480)?
    } else {
        omacell_io::chart_export::chart_svg(&wb, &chart, &theme, 800.0, 480.0)?.into_bytes()
    };
    std::fs::write(dest, bytes).map_err(|err| CliError::new("chart.export", err.to_string()))?;
    finish_outcome(
        omacell_core::command::Outcome::success(serde_json::json!({
            "path": dest.display().to_string(),
        })),
        output,
    )
}

fn chart_theme_from_roles(
    roles: &std::collections::BTreeMap<String, String>,
) -> omacell_core::chart::ChartTheme {
    fn grab(
        roles: &std::collections::BTreeMap<String, String>,
        key: &str,
        fallback: &str,
    ) -> String {
        roles.get(key).cloned().unwrap_or_else(|| fallback.into())
    }
    omacell_core::chart::ChartTheme {
        background: grab(roles, "surfaces.background", "#1a1b26"),
        foreground: grab(roles, "text.foreground", "#c0caf5"),
        axis: grab(roles, "charts.axis", "#a9b1d6"),
        gridline: grab(roles, "charts.gridline", "#3b4261"),
        palette: std::array::from_fn(|i| grab(roles, &format!("charts.palette.{i}"), "#7aa2f7")),
    }
}

fn read_import_plan(path: &Path) -> Result<ImportPlan, CliError> {
    const MAX_PLAN_BYTES: u64 = 1024 * 1024;

    let file = std::fs::File::open(path)
        .map_err(|err| CliError::new("csv.plan", format!("{}: {err}", path.display())))?;
    let mut bytes = Vec::new();
    file.take(MAX_PLAN_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| CliError::new("csv.plan", format!("{}: {err}", path.display())))?;
    if bytes.len() as u64 > MAX_PLAN_BYTES {
        return Err(CliError::new(
            "csv.plan",
            format!("import plan exceeds {MAX_PLAN_BYTES} bytes"),
        ));
    }
    let plan: ImportPlan = serde_json::from_slice(&bytes)
        .map_err(|err| CliError::new("csv.plan", format!("{}: {err}", path.display())))?;
    plan.validate()?;
    Ok(plan)
}

fn cmd_query(
    cli: &Cli,
    book: &Path,
    range: &str,
    format: QueryFormat,
    formulas: bool,
    output: Output,
) -> Result<(), CliError> {
    let app = init_app_book(cli, book)?;
    let wb = app.bus.workbook();
    let parsed = parse_a1(range)?;
    let kind = wb.resolve_parsed(parsed)?;
    let (sheet, min_row, min_col, max_row, max_col) = match kind {
        RefKind::Cell(cell) => {
            let sheet = cell.sheet.unwrap_or_else(|| wb.active_sheet());
            (sheet, cell.row, cell.col, cell.row, cell.col)
        }
        RefKind::Range(r) => {
            let sheet = r.start.sheet.unwrap_or_else(|| wb.active_sheet());
            (
                sheet,
                r.start.row.min(r.end.row),
                r.start.col.min(r.end.col),
                r.start.row.max(r.end.row),
                r.start.col.max(r.end.col),
            )
        }
    };
    if u64::from(max_row - min_row + 1) * u64::from(max_col - min_col + 1)
        > omacell_bus::MAX_RANGE_CELLS
    {
        return Err(CliError::new(
            "range.size",
            "query range exceeds the command limit",
        ));
    }
    let mut rows: Vec<Vec<String>> = Vec::new();
    for row in min_row..=max_row {
        let mut line = Vec::new();
        for col in min_col..=max_col {
            let slot = wb.get(sheet, row, col)?;
            let text = match slot {
                None => String::new(),
                Some(slot) if formulas => {
                    if let Some(fid) = slot.formula {
                        wb.intern().formulas.get(fid).unwrap_or("").to_string()
                    } else {
                        format_value(wb, &slot.value)
                    }
                }
                Some(slot) => format_value(wb, &slot.value),
            };
            line.push(text);
        }
        rows.push(line);
    }
    match format {
        QueryFormat::Json => {
            let human = serde_json::to_string_pretty(&rows)
                .map_err(|err| CliError::new("cli.json", err.to_string()))?;
            output.success(serde_json::json!({"rows": rows}), &human)?;
        }
        QueryFormat::Csv => {
            let buf = encode_csv_rows(&rows);
            output.success(serde_json::json!({"csv": buf}), &buf)?;
        }
        QueryFormat::Md => {
            if rows.is_empty() {
                output.success(serde_json::json!({"rows": rows}), "")?;
                return Ok(());
            }
            let cols = rows[0].len();
            let mut buf = String::from("|");
            buf.push_str(&vec![" col "; cols].join("|"));
            buf.push_str("|\n|");
            buf.push_str(&vec!["---"; cols].join("|"));
            buf.push_str("|\n");
            for row in &rows {
                buf.push('|');
                buf.push_str(
                    &row.iter()
                        .map(|value| escape_markdown_cell(value))
                        .collect::<Vec<_>>()
                        .join("|"),
                );
                buf.push_str("|\n");
            }
            output.success(serde_json::json!({"markdown": buf}), &buf)?;
        }
    }
    Ok(())
}

fn encode_csv_rows(rows: &[Vec<String>]) -> String {
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|value| {
                    if value
                        .chars()
                        .any(|ch| matches!(ch, ',' | '"' | '\r' | '\n'))
                    {
                        format!("\"{}\"", value.replace('"', "\"\""))
                    } else {
                        value.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn escape_markdown_cell(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace("\r\n", "<br>")
        .replace(['\r', '\n'], "<br>")
}

fn format_value(wb: &omacell_core::workbook::Workbook, value: &Value) -> String {
    match value {
        Value::Empty => String::new(),
        Value::Number(n) => {
            if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{n:.0}")
            } else {
                n.to_string()
            }
        }
        Value::Bool(true) => "TRUE".into(),
        Value::Bool(false) => "FALSE".into(),
        Value::Text(id) => wb.intern().strings.get(*id).unwrap_or("").to_string(),
        Value::Error(kind) => kind.as_str().to_string(),
        Value::Array(_) => String::new(),
    }
}

fn cmd_set(
    cli: &Cli,
    book: &Path,
    range: &str,
    value: &str,
    output: Output,
) -> Result<(), CliError> {
    let mut app = init_app_book(cli, book)?;
    let parsed = parse_a1(range)?;
    let id = match parsed.kind {
        RefKind::Cell(_) => "cell.set",
        RefKind::Range(_) => "range.set",
    };
    let args = if id == "cell.set" {
        serde_json::json!({"ref": range, "input": value})
    } else {
        serde_json::json!({"range": range, "input": value})
    };
    let outcome = if cli.dry_run {
        app.dry_run(id, args)?.outcome
    } else {
        app.execute(id, args)
    };
    if outcome.ok {
        let save_args = serde_json::json!({"path": book.display().to_string()});
        let save = if cli.dry_run {
            app.dry_run("file.save", save_args)?.outcome
        } else {
            app.execute("file.save", save_args)
        };
        return finish_outcome(save, output);
    }
    finish_outcome(outcome, output)
}

fn cmd_eval(cli: &Cli, book: &Path, formula: &str, output: Output) -> Result<(), CliError> {
    let app = init_app_book(cli, book)?;
    let parsed = parse(formula).map_err(|e| CliError::new("formula.parse", e.to_string()))?;
    let wb = app.bus.workbook();
    let cell = CellCoord::new(wb.active_sheet(), 0, 0);
    let (value, _) = eval_formula_in(
        wb,
        app.bus.engine().registry(),
        &SpillTable::new(),
        cell,
        &parsed.ast,
        0,
        omacell_core::eval::PassEnv::default(),
    );
    let text = format_runtime(&value);
    output.success(serde_json::json!({"value": text}), &text)?;
    Ok(())
}

fn cmd_audit(cli: &Cli, book: &Path, output: Output) -> Result<(), CliError> {
    let mut app = init_app_book(cli, book)?;
    let outcome = app.execute("audit.run", serde_json::json!({}));
    if !outcome.ok {
        return finish_outcome(outcome, output);
    }
    let json = outcome.result.ok_or_else(|| {
        CliError::new(
            "audit.output",
            "audit.run succeeded without returning an audit report",
        )
    })?;
    let n = json
        .get("findings")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    output.success(json, &format!("{n} findings"))?;
    Ok(())
}

fn cmd_recalc(cli: &Cli, book: &Path, write: bool, output: Output) -> Result<(), CliError> {
    let mut app = init_app_book(cli, book)?;
    let args = serde_json::json!({"mode": "rebuild"});
    let outcome = if cli.dry_run {
        app.dry_run("calc.recalc", args)?.outcome
    } else {
        app.execute("calc.recalc", args)
    };
    if write && outcome.ok {
        let save_args = serde_json::json!({"path": book.display().to_string()});
        let save = if cli.dry_run {
            app.dry_run("file.save", save_args)?.outcome
        } else {
            app.execute("file.save", save_args)
        };
        return finish_outcome(save, output);
    }
    finish_outcome(outcome, output)
}

fn cmd_diff(a: &Path, b: &Path, output: Output) -> Result<(), CliError> {
    let left = xlsx::open(a)?;
    let right = xlsx::open(b)?;
    let report = xlsx::diff(&left, &right);
    let json =
        serde_json::to_value(&report).map_err(|e| CliError::new("xlsx.diff", e.to_string()))?;
    let human = if report.empty {
        "empty".into()
    } else {
        serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string())
    };
    output.success(json, &human)?;
    Ok(())
}

fn cmd_ipc(
    command: &str,
    json: Option<&str>,
    all: bool,
    quiet: bool,
    socket: Option<&PathBuf>,
    dry_run: bool,
    output: Output,
) -> Result<(), CliError> {
    let args = match json {
        Some(text) => serde_json::from_str(text)
            .map_err(|e| CliError::new("cli.json", e.to_string()).exit(EXIT_USAGE))?,
        None => serde_json::json!({}),
    };
    if dry_run
        && matches!(
            control_op(command),
            Some(ControlOp::ChangesetApply | ControlOp::ChangesetRevert)
        )
    {
        output.success(
            serde_json::json!({"command": command, "dry_run": true}),
            "dry-run",
        )?;
        return Ok(());
    }
    if all {
        let dir = default_runtime_dir();
        let instances = list_live_instances(&dir)?;
        if instances.is_empty() {
            return Err(CliError::new(
                "ipc.socket",
                format!("no live Omacell instance in {}", dir.display()),
            ));
        }
        let mut results = Vec::new();
        let mut failed = 0_usize;
        let mut first_error = None;
        for inst in &instances {
            let path = discovered_socket(&dir, inst);
            let reply = ipc_one(&path, command, &args, dry_run)?;
            if !reply.ok {
                failed += 1;
                if first_error.is_none() {
                    first_error = reply.error.clone();
                }
            }
            results.push(serde_json::json!({
                "pid": inst.pid,
                "ok": reply.ok,
                "result": reply.result,
                "error": reply.error,
            }));
        }
        if failed > 0 {
            let first = first_error
                .map(|err| format!("{}: {}", err.code, err.message))
                .unwrap_or_else(|| "remote command failed without an error payload".into());
            return Err(CliError::new(
                "ipc.command",
                format!("{failed} of {} instances failed; {first}", instances.len()),
            ));
        }
        if !quiet && !output.json {
            for inst in &instances {
                println!("{}: ok=true", inst.pid);
            }
        }
        let json = serde_json::json!({"instances": results});
        let human = serde_json::to_string_pretty(&json)
            .map_err(|err| CliError::new("cli.json", err.to_string()))?;
        output.success(json, &human)?;
        return Ok(());
    }
    let mut client = if let Some(socket) = socket {
        IpcClient::connect(socket)?
    } else {
        IpcClient::connect_default()?
    };
    let reply = ipc_command(&mut client, command, args, dry_run)?;
    if !reply.ok {
        let err = reply
            .error
            .clone()
            .unwrap_or_else(|| CoreError::new("ipc.command", "command failed"));
        return Err(err.into());
    }
    let result = reply.result.clone().unwrap_or(serde_json::json!({}));
    let human = serde_json::to_string_pretty(&result)
        .map_err(|err| CliError::new("cli.json", err.to_string()))?;
    output.success(result, &human)?;
    Ok(())
}

fn ipc_one(
    path: &Path,
    command: &str,
    args: &serde_json::Value,
    dry_run: bool,
) -> Result<omacell_bus::ipc::Reply, CliError> {
    let mut client = IpcClient::connect(path)?;
    ipc_command(&mut client, command, args.clone(), dry_run)
}

fn ipc_command(
    client: &mut IpcClient,
    command: &str,
    args: serde_json::Value,
    dry_run: bool,
) -> Result<omacell_bus::ipc::Reply, CliError> {
    if let Some(op) = control_op(command) {
        let changeset = args.get("id").and_then(|v| v.as_str());
        Ok(client.control(op, &[], changeset)?)
    } else {
        let mode = dry_run.then_some(Mode::DryRun);
        Ok(client.command(command, args, mode)?)
    }
}

fn control_op(command: &str) -> Option<ControlOp> {
    match command {
        "ping" => Some(ControlOp::Ping),
        "changeset.list" => Some(ControlOp::ChangesetList),
        "changeset.get" => Some(ControlOp::ChangesetGet),
        "changeset.apply" => Some(ControlOp::ChangesetApply),
        "changeset.revert" => Some(ControlOp::ChangesetRevert),
        _ => None,
    }
}

fn cmd_changeset(cmd: &ChangesetCmd, dry_run: bool, output: Output) -> Result<(), CliError> {
    if dry_run
        && matches!(
            cmd,
            ChangesetCmd::Apply { .. } | ChangesetCmd::Revert { .. } | ChangesetCmd::Export { .. }
        )
    {
        output.success(serde_json::json!({"dry_run": true}), "dry-run")?;
        return Ok(());
    }
    match cmd {
        ChangesetCmd::List => cmd_ipc("changeset.list", None, false, true, None, false, output),
        ChangesetCmd::Show { id } => cmd_ipc(
            "changeset.get",
            Some(&serde_json::json!({"id": id}).to_string()),
            false,
            true,
            None,
            false,
            output,
        ),
        ChangesetCmd::Apply { id } => cmd_ipc(
            "changeset.apply",
            Some(&serde_json::json!({"id": id}).to_string()),
            false,
            true,
            None,
            false,
            output,
        ),
        ChangesetCmd::Revert { id } => cmd_ipc(
            "changeset.revert",
            Some(&serde_json::json!({"id": id}).to_string()),
            false,
            true,
            None,
            false,
            output,
        ),
        ChangesetCmd::Export { id, omc } => {
            let mut client = IpcClient::connect_default()?;
            let reply = client.control(ControlOp::ChangesetGet, &[], Some(id))?;
            if !reply.ok {
                let err = reply
                    .error
                    .unwrap_or_else(|| CoreError::new("changeset.not_found", "missing changeset"));
                return Err(err.into());
            }
            let value = reply.result.unwrap_or(serde_json::json!({}));
            let cs: omacell_core::changeset::Changeset = serde_json::from_value(value)
                .map_err(|e| CliError::new("changeset.export", e.to_string()))?;
            let text = omacell_io::omc::changeset_to_omc(&cs)?;
            std::fs::write(omc, text)
                .map_err(|e| CliError::new("changeset.export", e.to_string()))?;
            output.success(
                serde_json::json!({"path": omc.display().to_string()}),
                &omc.display().to_string(),
            )?;
            Ok(())
        }
    }
}

fn cmd_run(
    cli: &Cli,
    script: &Path,
    book: Option<&Path>,
    embedded: bool,
    python: bool,
    output: Output,
) -> Result<(), CliError> {
    let book = if embedded {
        script
    } else {
        book.ok_or_else(|| {
            CliError::new("cli.usage", "omacell run requires a workbook path")
                .hint("omacell run script.lua book.xlsx")
                .exit(EXIT_USAGE)
        })?
    };
    if python {
        return cmd_run_python(cli, script, book, output);
    }
    let mut app = init_app_book(cli, book)?;
    let loaded = app.loaded();
    let policy = omacell_lua::ScriptPolicy::from_config(
        loaded.config.scripting.enabled,
        &loaded.config.scripting.embedded_scripts,
        &loaded.config.scripting.trusted_dirs,
    );
    if !policy.enabled {
        return Err(CliError::new("lua.disabled", "scripting is disabled"));
    }
    let bus = std::mem::replace(
        &mut app.bus,
        omacell_bus::Bus::new(
            omacell_core::workbook::Workbook::new(),
            omacell_core::recalc::RecalcEngine::new(omacell_core::eval::FnRegistry::new()),
        )
        .expect("empty bus"),
    );
    let host = omacell_lua::BusHost::new(bus);
    if embedded {
        let bytes = std::fs::read(book).map_err(|e| CliError::new("lua.io", e.to_string()))?;
        let store = omacell_lua::load_trust(&app.paths.state_dir)?;
        omacell_lua::allow_embedded(&policy, &store, book, &bytes)?;
        let source = host
            .bus
            .workbook()
            .custom_parts
            .get(omacell_lua::EMBEDDED_PART)
            .and_then(|b| std::str::from_utf8(b).ok())
            .ok_or_else(|| {
                CliError::new(
                    "lua.embedded",
                    "workbook has no xl/omacell/scripts/main.lua part",
                )
            })?
            .to_string();
        let rt = omacell_lua::Runtime::new(omacell_lua::Profile::Embedded, Box::new(host))?;
        rt.exec(&source, &book.display().to_string())?;
        if !cli.dry_run {
            rt.execute_cmd(
                "file.save",
                serde_json::json!({"path": book.display().to_string()}),
            )?;
        }
        output.success(serde_json::json!({"ok": true, "embedded": true}), "ok")?;
        return Ok(());
    }
    let source =
        std::fs::read_to_string(script).map_err(|e| CliError::new("lua.io", e.to_string()))?;
    let rt = omacell_lua::Runtime::new(omacell_lua::Profile::User, Box::new(host))?;
    rt.exec(&source, &script.display().to_string())?;
    if !cli.dry_run {
        rt.execute_cmd(
            "file.save",
            serde_json::json!({"path": book.display().to_string()}),
        )?;
    }
    output.success(serde_json::json!({"ok": true}), "ok")?;
    Ok(())
}

fn cmd_run_python(cli: &Cli, script: &Path, book: &Path, output: Output) -> Result<(), CliError> {
    let mut app = init_app_book(cli, book)?;
    let mut child = std::process::Command::new("python3")
        .arg("-u")
        .arg(script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            CliError::new("lua.python", e.to_string()).hint("install python3 or omit --python")
        })?;
    let mut child_in = child
        .stdin
        .take()
        .ok_or_else(|| CliError::new("lua.python", "python stdin is missing"))?;
    let mut child_out = std::io::BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| CliError::new("lua.python", "python stdout is missing"))?,
    );
    let mut line = String::new();
    loop {
        line.clear();
        let n = std::io::BufRead::read_line(&mut child_out, &mut line)
            .map_err(|e| CliError::new("lua.python", e.to_string()))?;
        if n == 0 {
            break;
        }
        let req: serde_json::Value = serde_json::from_str(line.trim()).map_err(|e| {
            CliError::new("lua.python", e.to_string()).hint("write JSON-lines {cmd, args}")
        })?;
        let id = req
            .get("cmd")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| CliError::new("lua.python", "missing cmd"))?;
        let args = req
            .get("args")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let outcome = app.execute(id, args);
        let reply = if outcome.ok {
            serde_json::json!({"ok": true, "result": outcome.result})
        } else {
            serde_json::json!({"ok": false, "error": outcome.error.map(|e| e.to_string())})
        };
        writeln!(child_in, "{reply}").map_err(|e| CliError::new("lua.python", e.to_string()))?;
        child_in
            .flush()
            .map_err(|e| CliError::new("lua.python", e.to_string()))?;
    }
    let status = child
        .wait()
        .map_err(|e| CliError::new("lua.python", e.to_string()))?;
    if !status.success() {
        return Err(CliError::new(
            "lua.python",
            format!("python exited {status}"),
        ));
    }
    if !cli.dry_run {
        let _ = app.execute(
            "file.save",
            serde_json::json!({"path": book.display().to_string()}),
        );
    }
    output.success(serde_json::json!({"ok": true, "python": true}), "ok")?;
    Ok(())
}

fn cmd_trust(cli: &Cli, cmd: &TrustCmd, output: Output) -> Result<(), CliError> {
    let paths = init_paths(cli)?;
    let path = omacell_lua::trust_path(&paths.state_dir);
    let mut store = omacell_lua::TrustStore::load(&path)?;
    match cmd {
        TrustCmd::Add { file } => {
            let hash = omacell_lua::hash_path(file)?;
            if !cli.dry_run {
                store.add(hash.clone(), Some(file.display().to_string()));
                store.save(&path)?;
            }
            output.success(
                serde_json::json!({"sha256": hash, "path": file.display().to_string()}),
                &hash,
            )?;
        }
        TrustCmd::Remove { file } => {
            let hash = if Path::new(file).is_file() {
                omacell_lua::hash_path(Path::new(file))?
            } else {
                file.to_ascii_lowercase()
            };
            let removed = store.remove(&hash);
            if !cli.dry_run {
                store.save(&path)?;
            }
            output.success(
                serde_json::json!({"removed": removed, "sha256": hash}),
                "ok",
            )?;
        }
        TrustCmd::List => {
            let json = serde_json::to_value(&store.files)
                .map_err(|e| CliError::new("trust.serialize", e.to_string()))?;
            let human = store
                .files
                .iter()
                .map(|e| e.sha256.clone())
                .collect::<Vec<_>>()
                .join("\n");
            output.success(json, &human)?;
        }
    }
    Ok(())
}

fn finish_outcome(outcome: Outcome, output: Output) -> Result<(), CliError> {
    if outcome.ok {
        output.success(outcome.result.unwrap_or(serde_json::json!({})), "ok")?;
        Ok(())
    } else {
        Err(outcome
            .error
            .map(CliError::from)
            .unwrap_or_else(|| CliError::new("command.failed", "command failed")))
    }
}

fn backup_stamp() -> Result<String, CliError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| CliError::new("config.reset", e.to_string()))?;
    Ok(format!(
        "{}-{:09}-{}",
        elapsed.as_secs(),
        elapsed.subsec_nanos(),
        std::process::id()
    ))
}

/// Write completions and the man page under `dir`.
pub fn write_dist(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut cmd = crate::cli::command();
    for shell in [
        clap_complete::Shell::Bash,
        clap_complete::Shell::Zsh,
        clap_complete::Shell::Fish,
    ] {
        clap_complete::generate_to(shell, &mut cmd, PRODUCT_NAME, dir)?;
    }
    let man = clap_mangen::Man::new(cmd);
    let mut buf = Vec::new();
    man.render(&mut buf)?;
    std::fs::write(dir.join("omacell.1"), buf)?;
    Ok(())
}
