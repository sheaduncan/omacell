//! Subcommand dispatch.

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;
use omacell_bus::ipc::{
    ControlOp, IpcClient, default_runtime_dir, discovered_socket, list_live_instances,
};
use omacell_conf::{
    HYPRLAND_SNIPPET, LoadedConfig, SetupOptions, keys, reset_user_file, reset_user_rel,
    setup_omarchy, show_all_json,
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
use omacell_io::xlsx;

use crate::app::App;
use crate::cli::{
    ChangesetCmd, Cli, Commands, ConfigCmd, FnCmd, KeysCmd, QueryFormat, SetupCmd, ThemeCmd,
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
    match run_inner(args) {
        Ok(code) => code,
        Err(err) => {
            let output = Output {
                json: std::env::args_os().any(|a| a == "--json"),
                quiet: false,
            };
            let _ = output.error(&err);
            err.exit
        }
    }
}

fn run_inner<I, T>(args: I) -> Result<i32, CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => {
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
        return Err(CliError::nyi("omacell --tui", "WP-15"));
    }
    match &cli.command {
        None => {
            if cli.files.is_empty() {
                return Err(CliError::nyi("omacell GUI", "WP-16"));
            }
            Err(CliError::nyi("omacell GUI", "WP-16"))
        }
        Some(Commands::Run { .. }) => Err(CliError::nyi("omacell run", "WP-20")),
        Some(Commands::Audit { .. }) => Err(CliError::nyi("omacell audit", "WP-19")),
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
            output,
        ),
        Commands::Changeset { cmd } => cmd_changeset(cmd, cli.dry_run, output),
        Commands::Diff { a, b } => cmd_diff(a, b, output),
        Commands::Convert {
            input,
            output: dest,
            sheet,
            range,
        } => cmd_convert(cli, input, dest, sheet.as_deref(), range.as_deref(), output),
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
        Commands::Run { .. }
        | Commands::Audit { .. }
        | Commands::Ai { .. }
        | Commands::Agent { .. }
        | Commands::Mcp { .. } => unreachable!("stubs handled earlier"),
    }
}

fn init_app(cli: &Cli) -> Result<App, CliError> {
    let app = App::bootstrap(cli)?;
    log::init(&app.paths, cli.verbose, cli.quiet);
    Ok(app)
}

fn init_app_book(cli: &Cli, book: &Path) -> Result<App, CliError> {
    let app = App::with_workbook(cli, book)?;
    log::init(&app.paths, cli.verbose, cli.quiet);
    Ok(app)
}

fn cmd_fn(cmd: &FnCmd, output: Output) -> Result<(), CliError> {
    match cmd {
        FnCmd::List => {
            let json: serde_json::Value = serde_json::from_str(
                &functions_json().map_err(|e| CliError::new("fn.catalog", e.to_string()))?,
            )
            .map_err(|e| CliError::new("fn.catalog", e.to_string()))?;
            output.success(json, "ok")?;
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
    let app = init_app(cli)?;
    match cmd {
        ConfigCmd::Check => {
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
            let path = app
                .options
                .config_file
                .clone()
                .unwrap_or_else(|| app.paths.user_config_toml());
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
            if cli.dry_run {
                output.success(serde_json::json!({"dry_run": true}), "dry-run")?;
                return Ok(());
            }
            let stamp = backup_stamp()?;
            let dest = match file.as_deref() {
                None => reset_user_file(&app.paths, &stamp)?,
                Some(rel) => reset_user_rel(&app.paths, &stamp, rel)?,
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
            let loaded = app.loaded();
            if *all {
                let json = show_all_json(&loaded);
                output.success(json, "ok")?;
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
            let user = app.paths.user_config_toml();
            let text = if user.is_file() {
                std::fs::read_to_string(&user)
                    .map_err(|e| CliError::new("config.io", e.to_string()))?
            } else {
                String::new()
            };
            output.success(
                serde_json::json!({"user": text, "path": user.display().to_string()}),
                &text,
            )?;
            Ok(())
        }
    }
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
    let app = init_app(cli)?;
    match cmd {
        KeysCmd::Check { hyprland } => {
            let path = hyprland
                .clone()
                .unwrap_or_else(|| app.paths.home.join(".config/hypr/bindings.lua"));
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
    let app = init_app(cli)?;
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
            if cli.dry_run {
                output.success(serde_json::json!({"dry_run": true}), "dry-run")?;
                return Ok(());
            }
            let confirm_menu = *menu || prompt_menu();
            let report = setup_omarchy(
                &app.paths,
                SetupOptions {
                    confirm_menu,
                    link_skill: true,
                },
            )?;
            let _ = keys::check_hyprland(
                &app.paths.home.join(".config/hypr/bindings.lua"),
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
    output: Output,
) -> Result<(), CliError> {
    let mut app = init_app_book(cli, input)?;
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
            output.success(serde_json::json!({"rows": rows}), "ok")?;
        }
        QueryFormat::Csv => {
            let mut buf = String::new();
            for (i, row) in rows.iter().enumerate() {
                if i > 0 {
                    buf.push('\n');
                }
                buf.push_str(&row.join(","));
            }
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
                buf.push_str(&row.join("|"));
                buf.push_str("|\n");
            }
            output.success(serde_json::json!({"markdown": buf}), &buf)?;
        }
    }
    Ok(())
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
        let dry = app.dry_run(id, args)?;
        return finish_outcome(dry.outcome, output);
    } else {
        app.execute(id, args)
    };
    if !cli.dry_run && outcome.ok {
        let save = app.execute(
            "file.save",
            serde_json::json!({"path": book.display().to_string()}),
        );
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

fn cmd_recalc(cli: &Cli, book: &Path, write: bool, output: Output) -> Result<(), CliError> {
    let mut app = init_app_book(cli, book)?;
    let args = serde_json::json!({"mode": "rebuild"});
    let outcome = if cli.dry_run {
        let dry = app.dry_run("calc.recalc", args)?;
        return finish_outcome(dry.outcome, output);
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
    output: Output,
) -> Result<(), CliError> {
    let args = match json {
        Some(text) => serde_json::from_str(text)
            .map_err(|e| CliError::new("cli.json", e.to_string()).exit(EXIT_USAGE))?,
        None => serde_json::json!({}),
    };
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
        for inst in &instances {
            let path = discovered_socket(&dir, inst);
            let reply = ipc_one(&path, command, &args)?;
            results.push(serde_json::json!({
                "pid": inst.pid,
                "ok": reply.ok,
                "result": reply.result,
                "error": reply.error,
            }));
            if !quiet && !output.json {
                println!("{}: ok={}", inst.pid, reply.ok);
            }
        }
        output.success(serde_json::json!({"instances": results}), "ok")?;
        return Ok(());
    }
    let mut client = if let Some(socket) = socket {
        IpcClient::connect(socket)?
    } else {
        IpcClient::connect_default()?
    };
    let reply = ipc_command(&mut client, command, args)?;
    if !reply.ok {
        let err = reply
            .error
            .clone()
            .unwrap_or_else(|| CoreError::new("ipc.command", "command failed"));
        return Err(err.into());
    }
    output.success(reply.result.clone().unwrap_or(serde_json::json!({})), "ok")?;
    Ok(())
}

fn ipc_one(
    path: &Path,
    command: &str,
    args: &serde_json::Value,
) -> Result<omacell_bus::ipc::Reply, CliError> {
    let mut client = IpcClient::connect(path)?;
    ipc_command(&mut client, command, args.clone())
}

fn ipc_command(
    client: &mut IpcClient,
    command: &str,
    args: serde_json::Value,
) -> Result<omacell_bus::ipc::Reply, CliError> {
    if let Some(op) = control_op(command) {
        let changeset = args.get("id").and_then(|v| v.as_str());
        Ok(client.control(op, &[], changeset)?)
    } else {
        Ok(client.command(command, args, None)?)
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
        ChangesetCmd::List => cmd_ipc("changeset.list", None, false, true, None, output),
        ChangesetCmd::Show { id } => cmd_ipc(
            "changeset.get",
            Some(&serde_json::json!({"id": id}).to_string()),
            false,
            true,
            None,
            output,
        ),
        ChangesetCmd::Apply { id } => cmd_ipc(
            "changeset.apply",
            Some(&serde_json::json!({"id": id}).to_string()),
            false,
            true,
            None,
            output,
        ),
        ChangesetCmd::Revert { id } => cmd_ipc(
            "changeset.revert",
            Some(&serde_json::json!({"id": id}).to_string()),
            false,
            true,
            None,
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
