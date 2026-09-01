//! Bus commands for `:source` and the macro recorder.

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use omacell_core::changeset::ChangeSummary;
use omacell_core::error::CoreError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::recorder::Recorder;
use omacell_bus::{
    Bus, CommandContext, CommandKind, CommandRegistry, CommandSpec, Effect, Exposure,
};

static MACRO_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Shared recorder for UI/CLI.
#[derive(Clone, Default)]
pub struct ScriptGate {
    /// Recorder.
    pub recorder: Arc<Mutex<Recorder>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MacroSaveArgs {
    path: String,
}

/// Register `macro.*` and the interactive-host `script.source` signal.
pub fn register_script_commands(
    registry: &mut CommandRegistry,
    gate: ScriptGate,
) -> Result<(), CoreError> {
    let rec = Arc::clone(&gate.recorder);
    registry.register(
        CommandSpec {
            id: "macro.record",
            doc: "Start recording commands as Lua",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        {
            let rec = Arc::clone(&rec);
            move |ctx: &mut CommandContext<'_>, _args: EmptyArgs| {
                if ctx.is_preflight() {
                    return Ok(Effect::query(serde_json::json!({
                        "recording": true,
                        "dry_run": ctx.is_dry_run(),
                    })));
                }
                rec.lock()
                    .map_err(|_| CoreError::new("macro.lock", "recorder lock poisoned"))?
                    .start();
                Ok(Effect::query(serde_json::json!({"recording": true})))
            }
        },
    )?;
    registry.register(
        CommandSpec {
            id: "macro.stop",
            doc: "Stop recording commands",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        {
            let rec = Arc::clone(&rec);
            move |ctx: &mut CommandContext<'_>, _args: EmptyArgs| {
                if ctx.is_preflight() {
                    return Ok(Effect::query(serde_json::json!({
                        "recording": false,
                        "dry_run": ctx.is_dry_run(),
                    })));
                }
                let mut recorder = rec
                    .lock()
                    .map_err(|_| CoreError::new("macro.lock", "recorder lock poisoned"))?;
                recorder.stop();
                Ok(Effect::query(serde_json::json!({
                    "recording": false,
                    "overflowed": recorder.overflowed(),
                })))
            }
        },
    )?;
    registry.register(
        CommandSpec {
            id: "macro.save",
            doc: "Write the recorded Lua to a path",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        {
            let rec = Arc::clone(&rec);
            move |ctx: &mut CommandContext<'_>, args: MacroSaveArgs| {
                let recorder = rec
                    .lock()
                    .map_err(|_| CoreError::new("macro.lock", "recorder lock poisoned"))?;
                if recorder.overflowed() {
                    return Err(CoreError::new(
                        "macro.limit",
                        "recording exceeded its bounded retention limit",
                    )
                    .with_hint("start a new, shorter recording"));
                }
                let lua = recorder.to_lua();
                drop(recorder);
                if ctx.is_preflight() {
                    return Ok(Effect::query(serde_json::json!({
                        "path": args.path,
                        "dry_run": ctx.is_dry_run(),
                    })));
                }
                write_macro(Path::new(&args.path), lua.as_bytes())?;
                Ok(Effect {
                    summary: ChangeSummary {
                        text: format!("save macro {}", args.path),
                        ..ChangeSummary::default()
                    },
                    result: serde_json::json!({"path": args.path}),
                    auto_recalc: false,
                    ..Effect::default()
                })
            }
        },
    )?;
    registry.register(
        CommandSpec {
            id: "script.source",
            doc: "Reload user Lua scripts (init.lua and plugins)",
            // Sourcing executes user-controlled code with the full user
            // profile. Its mutating classification prevents model origins from
            // triggering that host action.
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        |_ctx: &mut CommandContext<'_>, _args: EmptyArgs| {
            Ok(Effect::query(serde_json::json!({
                "ok": true,
                "source": true
            })))
        },
    )?;
    Ok(())
}

fn write_macro(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| CoreError::new("macro.io", "destination has no file name"))?;
    let (mut file, temp) = loop {
        let sequence = MACRO_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = OsString::from(".");
        temp_name.push(name);
        temp_name.push(format!(".omacell-{}-{sequence}.tmp", std::process::id()));
        let temp = directory.join(temp_name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        match options.open(&temp) {
            Ok(file) => break (file, temp),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(CoreError::new("macro.io", error.to_string())),
        }
    };
    let write_result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let write_result = write_result
        .and_then(|()| std::fs::rename(&temp, path))
        .and_then(|()| std::fs::File::open(directory)?.sync_all());
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp);
        return Err(CoreError::new("macro.io", error.to_string()));
    }
    Ok(())
}

/// Attach `gate` to successful commands committed by `bus`.
pub fn attach_recorder(bus: &mut Bus, gate: &ScriptGate) {
    let recorder = Arc::clone(&gate.recorder);
    bus.observe_commands(Arc::new(move |_origin, call| {
        let id = call.id.as_str();
        if id.starts_with("macro.") || id == "script.source" {
            return;
        }
        recorder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(id, call.args.clone());
    }));
}
