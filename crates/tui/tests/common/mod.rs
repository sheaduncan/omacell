//! Shared TUI test harness (no second config load; no watcher).
#![allow(dead_code)]

use omacell_bus::{Bus, CommandKind, CommandSpec, Effect, Exposure, LongOps};
use omacell_conf::{ConfigStore, LoadOptions, Paths};
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_tui::{Launch, Tui};
use omacell_ui::{KeymapRoots, UiSession, register_ui_commands};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FileSaveAsArgs {
    path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentTurnArgs {
    prompt: String,
    #[serde(default)]
    apply: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FormulaArgs {
    #[serde(default)]
    prompt: String,
    #[serde(default, rename = "ref")]
    reference: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CompleteArgs {
    prefix: String,
}

fn register_test_agent(bus: &mut Bus) {
    bus.registry_mut()
        .register::<AgentTurnArgs, _>(
            CommandSpec {
                id: "ai.agent.turn",
                doc: "Recorded in-app agent turn",
                kind: CommandKind::Query,
                changeset_eligible: false,
                exposure: Exposure::Public,
                default_keys: &[],
            },
            |_ctx, args| {
                let target = if args.prompt.contains("B1") {
                    "B1"
                } else {
                    "A1"
                };
                Ok(Effect::query(json!({
                    "prompt": args.prompt,
                    "proposed": [{
                        "id": "cell.set",
                        "args": {"ref": target, "input": "agent"}
                    }],
                    "applied": false,
                    "autopilot": false,
                })))
            },
        )
        .unwrap();
}

fn register_test_formula_assist(bus: &mut Bus) {
    for id in [
        "ai.formula.generate",
        "ai.formula.explain",
        "ai.formula.fix",
        "ai.formula.refactor",
    ] {
        bus.registry_mut()
            .register::<FormulaArgs, _>(
                CommandSpec {
                    id,
                    doc: "Recorded formula-assist result",
                    kind: CommandKind::Query,
                    changeset_eligible: false,
                    exposure: Exposure::Public,
                    default_keys: &[],
                },
                move |_ctx, args| {
                    let _request = (args.prompt, args.reference);
                    if id == "ai.formula.explain" {
                        Ok(Effect::query(json!({
                            "explanation": "Adds the selected inputs."
                        })))
                    } else {
                        Ok(Effect::query(json!({
                            "formula": "=SUM(B1:C1)+D2",
                            "scratch": "Number(6)"
                        })))
                    }
                },
            )
            .unwrap();
    }
}

fn register_test_completion(bus: &mut Bus) {
    bus.registry_mut()
        .register::<CompleteArgs, _>(
            CommandSpec {
                id: "ai.complete",
                doc: "Recorded inline completion",
                kind: CommandKind::Query,
                changeset_eligible: false,
                exposure: Exposure::Public,
                default_keys: &[],
            },
            |_ctx, args| {
                Ok(Effect::query(json!({
                    "prefix": args.prefix,
                    "text": "=SUM(A1:A3)"
                })))
            },
        )
        .unwrap();
}

fn register_file_lifecycle(bus: &mut Bus) {
    let register_empty =
        |bus: &mut Bus, id: &'static str, doc: &'static str, keys: &'static [&'static str]| {
            bus.registry_mut()
                .register::<EmptyArgs, _>(
                    CommandSpec {
                        id,
                        doc,
                        kind: CommandKind::Mutating,
                        changeset_eligible: false,
                        exposure: Exposure::Public,
                        default_keys: keys,
                    },
                    move |ctx, _args| {
                        if ctx.is_preflight() {
                            return Ok(Effect::query(json!({})));
                        }
                        match id {
                            "file.new" => {
                                *ctx.workbook() = Workbook::new();
                                Ok(Effect {
                                    events: vec![omacell_core::event::Event::WorkbookOpened {
                                        path: None,
                                    }],
                                    result: json!({"path": null}),
                                    auto_recalc: false,
                                    ..Effect::default()
                                })
                            }
                            "file.close" => Ok(Effect::query(json!({"close": true}))),
                            _ => unreachable!(),
                        }
                    },
                )
                .unwrap();
        };
    register_empty(bus, "file.new", "Create a new workbook", &["Ctrl+N"]);
    register_empty(
        bus,
        "file.close",
        "Close the current workbook window",
        &["Ctrl+W"],
    );
    bus.registry_mut()
        .register::<FileSaveAsArgs, _>(
            CommandSpec {
                id: "file.saveas",
                doc: "Save the workbook to a new path",
                kind: CommandKind::Mutating,
                changeset_eligible: false,
                exposure: Exposure::Public,
                default_keys: &["F12"],
            },
            |_ctx, args| Ok(Effect::query(json!({"path": args.path}))),
        )
        .unwrap();
}

pub struct Harness {
    pub _dir: tempfile::TempDir,
    pub tui: Tui,
}

pub fn fixture_theme(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/omarchy-themes")
        .join(name)
        .join("colors.toml")
}

fn install_omarchy_theme(paths: &Paths, name: &str) {
    let theme_dir = paths.omarchy_state.join("current/theme");
    std::fs::create_dir_all(&theme_dir).unwrap();
    std::fs::copy(fixture_theme(name), theme_dir.join("colors.toml")).unwrap();
    std::fs::write(paths.omarchy_state.join("current/theme.name"), name).unwrap();
}

pub fn harness() -> Harness {
    harness_opts(None, "keys/classic.toml", "off")
}

pub fn harness_theme(theme: &str) -> Harness {
    harness_opts(Some(theme), "keys/classic.toml", "off")
}

pub fn harness_modal() -> Harness {
    harness_opts(None, "keys/modal.toml", "off")
}

pub fn harness_opts(theme: Option<&str>, keymap: &str, truecolor: &str) -> Harness {
    harness_opts_with_workbook(theme, keymap, truecolor, Workbook::new(), &[])
}

pub fn harness_workbook(workbook: Workbook) -> Harness {
    harness_opts_with_workbook(None, "keys/classic.toml", "off", workbook, &[])
}

pub fn harness_sets(sets: &[&str]) -> Harness {
    harness_opts_with_workbook(None, "keys/classic.toml", "off", Workbook::new(), sets)
}

pub fn harness_script(source: &str) -> Harness {
    harness_opts_with_script(
        None,
        "keys/classic.toml",
        "off",
        Workbook::new(),
        &[],
        Some(source),
    )
}

fn harness_opts_with_workbook(
    theme: Option<&str>,
    keymap: &str,
    truecolor: &str,
    workbook: Workbook,
    extra_sets: &[&str],
) -> Harness {
    harness_opts_with_script(theme, keymap, truecolor, workbook, extra_sets, None)
}

fn harness_opts_with_script(
    theme: Option<&str>,
    keymap: &str,
    truecolor: &str,
    workbook: Workbook,
    extra_sets: &[&str],
    script: Option<&str>,
) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    let scripting = script
        .map(|_| {
            format!(
                "[scripting]\ntrusted_dirs = [{:?}]\n",
                paths.user_config.display().to_string()
            )
        })
        .unwrap_or_default();
    std::fs::write(
        paths.user_config.join("config.toml"),
        format!("[keys]\nfile = {keymap:?}\n[tui]\ntruecolor = {truecolor:?}\n{scripting}"),
    )
    .unwrap();
    if let Some(script) = script {
        std::fs::write(paths.user_config.join("init.lua"), script).unwrap();
    }
    if let Some(theme) = theme {
        install_omarchy_theme(&paths, theme);
    }
    let mut cli_sets = vec![
        format!("keys.file={keymap}"),
        format!("tui.truecolor={truecolor}"),
    ];
    cli_sets.extend(extra_sets.iter().map(|value| (*value).to_string()));
    let options = LoadOptions {
        cli_sets,
        ..LoadOptions::default()
    };
    let store = ConfigStore::load_with(paths.clone(), options).unwrap();
    let loaded = store.snapshot();
    let roots = KeymapRoots::new(paths.user_config.clone(), paths.default_dir.clone(), None);
    let ui = UiSession::new(&loaded, &roots).unwrap();

    let mut functions = FnRegistry::new();
    register_all(&mut functions);
    let mut bus = Bus::new(workbook, RecalcEngine::new(functions)).unwrap();
    omacell_bus::register_chart_commands(bus.registry_mut()).unwrap();
    omacell_bus::register_edit_commands(bus.registry_mut()).unwrap();
    omacell_bus::register_data_commands(bus.registry_mut()).unwrap();
    omacell_bus::register_audit_commands(bus.registry_mut()).unwrap();
    omacell_bus::register_analysis_commands(bus.registry_mut()).unwrap();
    omacell_lua::register_script_commands(bus.registry_mut(), omacell_lua::ScriptGate::default())
        .unwrap();
    register_test_agent(&mut bus);
    register_test_formula_assist(&mut bus);
    register_test_completion(&mut bus);
    register_file_lifecycle(&mut bus);
    register_ui_commands(bus.registry_mut(), &ui).unwrap();

    let tui = Tui::new(
        Launch {
            paths,
            store,
            bus,
            ui,
            roots,
            long_ops: LongOps::production().with("test.hold"),
            ai: None,
            file: None,
        },
        false,
    )
    .unwrap();
    Harness { _dir: dir, tui }
}

pub fn seed_demo(tui: &mut Tui) {
    for (cell, input) in [
        ("A1", "Hello"),
        ("B1", "1234.5"),
        ("C1", "TRUE"),
        ("A2", "=B1*2"),
        ("D1", "overflows into next"),
        ("A3", "=1/0"),
    ] {
        let result = tui
            .execute_cmd("cell.set", json!({"ref": cell, "input": input}))
            .unwrap();
        assert!(result.ok, "{cell}: {:?}", result.error);
    }
    wait_tasks(tui);
}

pub fn wait_tasks(tui: &mut Tui) {
    let started = std::time::Instant::now();
    while tui.has_pending_tasks() {
        tui.poll_reload().unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "TUI command task did not finish"
        );
        std::thread::yield_now();
    }
    tui.poll_reload().unwrap();
}

pub fn draw_text(tui: &Tui, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    tui.draw(&mut terminal).unwrap();
    omacell_tui::buffer_text(&terminal)
}
