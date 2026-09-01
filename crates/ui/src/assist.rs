//! Formula-assistant workflow launcher.

use omacell_bus::args::EmptyArgs;
use omacell_bus::{CommandKind, CommandRegistry, CommandSpec, Effect, Exposure};
use omacell_core::error::CoreError;

use crate::palette::Palette;
use crate::session::UiSession;

pub(crate) const PROMPT: &str = "AI assist — choose generate, explain, fix, or refactor";

pub(crate) fn register_ai_assist(
    registry: &mut CommandRegistry,
    session: &UiSession,
) -> Result<(), CoreError> {
    let session = session.clone();
    registry.register::<EmptyArgs, _>(
        CommandSpec {
            id: "ai.assist",
            doc: "Choose a formula generate, explain, fix, or refactor workflow",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Shift+X"],
        },
        move |ctx, _args| {
            if !ctx.is_preflight() {
                open(
                    &mut session
                        .inner
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .palette,
                );
            }
            Ok(Effect::query(serde_json::json!({"open": true})))
        },
    )
}

pub(crate) fn open(palette: &mut Palette) {
    palette.open();
    palette.query = "ai.formula.".into();
    palette.prompt = Some(PROMPT.into());
}
