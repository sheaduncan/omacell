//! Defined-name UI workflows.

use omacell_bus::args::EmptyArgs;
use omacell_bus::{CommandKind, CommandRegistry, CommandSpec, Effect, Exposure};
use omacell_core::error::CoreError;
use omacell_core::names::{NameReferent, NameScope};
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::edit::EditSurface;
use crate::mode::{KeyModel, Mode};
use crate::session::UiSession;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NamePasteArgs {
    name: String,
}

pub(crate) fn register_name_commands(
    registry: &mut CommandRegistry,
    session: &UiSession,
) -> Result<(), CoreError> {
    let manager = session.clone();
    registry.register::<EmptyArgs, _>(
        CommandSpec {
            id: "name.manager",
            doc: "Open the defined-name manager",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+F3"],
        },
        move |ctx, _args| {
            let (body, count) = manager_body(ctx.workbook_ref());
            if !ctx.is_preflight() {
                manager
                    .inner
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .panel
                    .open_with_body("names", body);
            }
            Ok(Effect::query(serde_json::json!({"count": count})))
        },
    )?;

    let paste = session.clone();
    registry.register::<NamePasteArgs, _>(
        CommandSpec {
            id: "name.paste",
            doc: "Insert a defined name into the current formula",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["F3"],
        },
        move |ctx, args| {
            let selection = paste.selection();
            let defined = ctx
                .workbook_ref()
                .names()
                .resolve(selection.sheet, &args.name)
                .ok_or_else(|| {
                    CoreError::name_defined(format!(
                        "defined name {:?} does not exist for the active sheet",
                        args.name
                    ))
                })?;
            let name = defined.name.clone();
            if !ctx.is_preflight() {
                let mut inner = paste
                    .inner
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if inner.edit.is_idle() {
                    let origin = inner.selection.cursor;
                    inner
                        .edit
                        .begin(EditSurface::InCell, origin, &format!("={name}"));
                } else {
                    inner.edit.insert_text(&name);
                }
                if inner.model == KeyModel::Modal {
                    inner.mode = Mode::Insert;
                }
            }
            Ok(Effect::query(serde_json::json!({"name": name})))
        },
    )?;
    Ok(())
}

fn manager_body(workbook: &Workbook) -> (String, usize) {
    let mut lines = Vec::new();
    for name in workbook.names().iter() {
        let scope = match name.scope {
            NameScope::Workbook => "workbook".to_string(),
            NameScope::Sheet(sheet) => workbook.sheet(sheet).map_or_else(
                || format!("sheet {}", sheet.index()),
                |sheet| sheet.name.clone(),
            ),
        };
        lines.push(format!(
            "{}  [{}]  {}",
            name.name,
            scope,
            referent_text(workbook, &name.referent)
        ));
    }
    let count = lines.len();
    if lines.is_empty() {
        lines.push("No defined names.".into());
    }
    lines.push(String::new());
    lines.push("Use name.define or name.remove from the palette to make changes.".into());
    (lines.join("\n"), count)
}

fn referent_text(workbook: &Workbook, referent: &NameReferent) -> String {
    match referent {
        NameReferent::Range(range) => {
            let prefix = range
                .start
                .sheet
                .and_then(|sheet| workbook.sheet(sheet))
                .map(|sheet| format!("{}!", omacell_core::addr::quote_sheet_name(&sheet.name)))
                .unwrap_or_default();
            format!("{prefix}{}", range.to_a1())
        }
        NameReferent::Formula(formula) => formula.clone(),
        NameReferent::Constant(value) => match value {
            Value::Empty => String::new(),
            Value::Number(number) => number.to_string(),
            Value::Bool(boolean) => boolean.to_string().to_ascii_uppercase(),
            Value::Text(id) => workbook.intern().strings.get(*id).unwrap_or("").to_string(),
            Value::Error(error) => error.as_str().to_string(),
            Value::Array(_) => "{array}".into(),
        },
    }
}
