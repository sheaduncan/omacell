//! Bus commands for `:source` and the macro recorder.

use std::sync::{Arc, Mutex};

use omacell_core::changeset::ChangeSummary;
use omacell_core::error::CoreError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::recorder::Recorder;
use omacell_bus::{CommandContext, CommandKind, CommandRegistry, CommandSpec, Effect, Exposure};

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

/// Register `macro.*` and `script.source` (source is a no-op hint; CLI reloads).
pub fn register_script_commands(
    registry: &mut CommandRegistry,
    gate: ScriptGate,
) -> Result<(), CoreError> {
    let rec = Arc::clone(&gate.recorder);
    registry.register(
        CommandSpec {
            id: "macro.record",
            doc: "Start recording commands as Lua",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        {
            let rec = Arc::clone(&rec);
            move |_ctx: &mut CommandContext<'_>, _args: EmptyArgs| {
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
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        {
            let rec = Arc::clone(&rec);
            move |_ctx: &mut CommandContext<'_>, _args: EmptyArgs| {
                rec.lock()
                    .map_err(|_| CoreError::new("macro.lock", "recorder lock poisoned"))?
                    .stop();
                Ok(Effect::query(serde_json::json!({"recording": false})))
            }
        },
    )?;
    registry.register(
        CommandSpec {
            id: "macro.save",
            doc: "Write the recorded Lua to a path",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        {
            let rec = Arc::clone(&rec);
            move |_ctx: &mut CommandContext<'_>, args: MacroSaveArgs| {
                let lua = rec
                    .lock()
                    .map_err(|_| CoreError::new("macro.lock", "recorder lock poisoned"))?
                    .to_lua();
                std::fs::write(&args.path, lua)
                    .map_err(|e| CoreError::new("macro.io", e.to_string()))?;
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
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        |_ctx: &mut CommandContext<'_>, _args: EmptyArgs| {
            Ok(Effect::query(serde_json::json!({
                "ok": true,
                "hint": "CLI/UI host reloads ~/.config/omacell/init.lua"
            })))
        },
    )?;
    Ok(())
}
