//! Shared GUI test harness (no second config load).
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use omacell_bus::{Bus, CommandKind, CommandSpec, Effect, Exposure, LongOps};
use omacell_conf::{ConfigStore, LoadOptions, Paths};
use omacell_core::eval::FnRegistry;
use omacell_core::event::Event;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_gui::{Gui, Launch};
use omacell_ui::{KeymapRoots, UiSession, register_ui_commands};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub struct HarnessParts {
    pub _dir: tempfile::TempDir,
    pub launch: Launch,
    pub open_count: Arc<AtomicUsize>,
}

pub fn fixture_theme(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/omarchy-themes")
        .join(name)
        .join("colors.toml")
}

pub fn fixed_gpu_setup() -> Option<eframe::egui_wgpu::WgpuSetup> {
    if std::env::var_os("OMACELL_FIXED_GPU").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return None;
    }
    let setup = eframe::egui_wgpu::WgpuSetupCreateNew {
        native_adapter_selector: Some(Arc::new(|adapters, _surface| {
            adapters
                .iter()
                .find(|adapter| {
                    adapter.get_info().device_type == eframe::wgpu::DeviceType::IntegratedGpu
                })
                .cloned()
                .ok_or_else(|| {
                    "fixed-host gate requires an integrated-GPU wgpu adapter".to_string()
                })
        })),
        ..eframe::egui_wgpu::WgpuSetupCreateNew::default()
    };
    Some(eframe::egui_wgpu::WgpuSetup::CreateNew(setup))
}

fn install_omarchy_theme(paths: &Paths, name: &str) {
    let theme_dir = paths.omarchy_state.join("current/theme");
    std::fs::create_dir_all(&theme_dir).unwrap();
    std::fs::copy(fixture_theme(name), theme_dir.join("colors.toml")).unwrap();
    std::fs::write(paths.omarchy_state.join("current/theme.name"), name).unwrap();
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ThemeReloadArgs {}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FileOpenArgs {
    path: String,
    #[serde(default)]
    plan: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FileSaveAsArgs {
    path: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FilePrintArgs {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    printer: Option<String>,
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

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportArgs {
    plan: serde_json::Value,
    preview: serde_json::Value,
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
                Ok(Effect::query(json!({
                    "prompt": args.prompt,
                    "proposed": [{
                        "id": "cell.set",
                        "args": {"ref": "D6", "input": "agent"}
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

fn register_test_import_assist(bus: &mut Bus) {
    bus.registry_mut()
        .register::<ImportArgs, _>(
            CommandSpec {
                id: "ai.import.assist",
                doc: "Recorded import-plan proposal",
                kind: CommandKind::Query,
                changeset_eligible: false,
                exposure: Exposure::Public,
                default_keys: &[],
            },
            |_ctx, args| {
                let mut proposed = args.plan.clone();
                proposed["columns"][0]["name"] = json!("Pressure");
                let _preview = args.preview;
                Ok(Effect::query(json!({
                    "current": args.plan,
                    "proposed": proposed,
                    "applied": false,
                })))
            },
        )
        .unwrap();
}

fn register_theme_reload(
    bus: &mut Bus,
    handle: omacell_conf::ReloadHandle,
) -> Result<(), omacell_core::error::CoreError> {
    bus.registry_mut().register::<ThemeReloadArgs, _>(
        CommandSpec {
            id: "theme.reload",
            doc: "Reload configuration and the active Omarchy theme",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, _args: ThemeReloadArgs| {
            if ctx.is_preflight() {
                return Ok(Effect::query(json!({"dry_run": ctx.is_dry_run()})));
            }
            handle.reload()?;
            let name = handle.snapshot().theme.name.clone();
            Ok(Effect {
                events: vec![Event::ThemeChanged { name: name.clone() }],
                result: json!({"name": name}),
                auto_recalc: false,
                ..Effect::default()
            })
        },
    )
}

fn register_file_open(
    bus: &mut Bus,
    open_count: Arc<AtomicUsize>,
) -> Result<(), omacell_core::error::CoreError> {
    bus.registry_mut().register::<FileOpenArgs, _>(
        CommandSpec {
            id: "file.open",
            doc: "Open a workbook from disk",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args: FileOpenArgs| {
            if ctx.is_preflight() {
                return Ok(Effect::query(json!({"path": args.path})));
            }
            open_count.fetch_add(1, Ordering::SeqCst);
            if args.path.ends_with(".csv") {
                let current = args.plan.unwrap_or_else(|| {
                    json!({
                        "delimiter": ",",
                        "has_header": true,
                        "columns": [{
                            "name": "Pressure (psi)",
                            "ty": {"kind": "auto"}
                        }]
                    })
                });
                return Ok(Effect {
                    result: json!({
                        "path": args.path,
                        "import": {
                            "current": current,
                            "preview": {
                                "header": ["Pressure (psi)"],
                                "rows": [[{
                                    "raw": "14.7",
                                    "would_become": "14.7",
                                    "kind": "number",
                                    "changed": true
                                }]]
                            }
                        }
                    }),
                    ..Effect::default()
                });
            }
            Ok(Effect {
                result: json!({"path": args.path}),
                ..Effect::default()
            })
        },
    )
}

fn register_file_lifecycle(bus: &mut Bus) -> Result<(), omacell_core::error::CoreError> {
    bus.registry_mut().register::<EmptyArgs, _>(
        CommandSpec {
            id: "file.new",
            doc: "Create a new workbook",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+N"],
        },
        |ctx, _args| {
            if ctx.is_preflight() {
                return Ok(Effect::query(json!({"path": null})));
            }
            *ctx.workbook() = Workbook::new();
            Ok(Effect {
                events: vec![Event::WorkbookOpened { path: None }],
                result: json!({"path": null}),
                auto_recalc: false,
                ..Effect::default()
            })
        },
    )?;
    bus.registry_mut().register::<EmptyArgs, _>(
        CommandSpec {
            id: "file.close",
            doc: "Close the current workbook window",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+W"],
        },
        |_ctx, _args| Ok(Effect::query(json!({"close": true}))),
    )?;
    bus.registry_mut().register::<FileSaveAsArgs, _>(
        CommandSpec {
            id: "file.saveas",
            doc: "Save the workbook to a new path",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["F12"],
        },
        |_ctx, args| Ok(Effect::query(json!({"path": args.path}))),
    )?;
    bus.registry_mut().register::<FilePrintArgs, _>(
        CommandSpec {
            id: "file.print",
            doc: "Print-preview page boxes, export PDF, or send a PDF to CUPS",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+P"],
        },
        |_ctx, args| {
            Ok(Effect::query(json!({
                "pages": [],
                "printers": ["office", "lab"],
                "last_printer": "lab",
                "printed": args.printer,
                "path": args.path,
            })))
        },
    )?;
    Ok(())
}

pub fn launch_theme(theme: Option<&str>) -> HarnessParts {
    launch_opts(theme, Workbook::new(), false)
}

pub fn launch_watched(theme: Option<&str>) -> HarnessParts {
    launch_opts(theme, Workbook::new(), true)
}

pub fn launch_opts(theme: Option<&str>, workbook: Workbook, watch: bool) -> HarnessParts {
    launch_opts_with_script(theme, workbook, watch, None)
}

pub fn launch_script(source: &str) -> HarnessParts {
    launch_opts_with_script(None, Workbook::new(), false, Some(source))
}

fn launch_opts_with_script(
    theme: Option<&str>,
    workbook: Workbook,
    watch: bool,
    script: Option<&str>,
) -> HarnessParts {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    if let Some(script) = script {
        std::fs::write(
            paths.user_config.join("config.toml"),
            format!(
                "[scripting]\ntrusted_dirs = [{:?}]\n",
                paths.user_config.display().to_string()
            ),
        )
        .unwrap();
        std::fs::write(paths.user_config.join("init.lua"), script).unwrap();
    }
    if let Some(theme) = theme {
        install_omarchy_theme(&paths, theme);
    }
    let store = if watch {
        ConfigStore::load_and_watch_with(paths.clone(), LoadOptions::default()).unwrap()
    } else {
        ConfigStore::load_with(paths.clone(), LoadOptions::default()).unwrap()
    };
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
    register_test_import_assist(&mut bus);
    register_ui_commands(bus.registry_mut(), &ui).unwrap();
    register_theme_reload(&mut bus, store.handle()).unwrap();
    let open_count = Arc::new(AtomicUsize::new(0));
    register_file_open(&mut bus, Arc::clone(&open_count)).unwrap();
    register_file_lifecycle(&mut bus).unwrap();
    for (cell, input) in [
        ("A1", "Hello"),
        ("B1", "1234.5"),
        ("C1", "TRUE"),
        ("A2", "=B1*2"),
        ("A3", "=1/0"),
    ] {
        let result = bus.execute(
            omacell_core::command::Origin::User,
            "cell.set",
            json!({"ref": cell, "input": input}),
        );
        assert!(result.ok, "{cell}: {:?}", result.error);
    }
    HarnessParts {
        _dir: dir,
        open_count,
        launch: Launch {
            paths,
            store,
            bus,
            ui,
            roots,
            long_ops: LongOps::production(),
            ai: None,
            file: None,
            use_shell_font: false,
        },
    }
}

pub fn gui_from(parts: HarnessParts, ctx: &egui::Context) -> Gui {
    Gui::new(parts.launch, false, ctx).unwrap()
}
