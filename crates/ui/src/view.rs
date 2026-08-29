//! WP-14-owned commands: view.*, nav.*, sel.*, mode.*, edit.*, palette.open.

use std::sync::{Arc, Mutex};

use omacell_bus::{CommandContext, CommandKind, CommandRegistry, CommandSpec, Effect, Exposure};
use omacell_core::error::CoreError;
use omacell_core::sheet::{FreezePanes, SplitView};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::edit::EditSurface;
use crate::mode::{KeyModel, Mode};
use crate::selection::{Area, ExtendMode};
use crate::session::UiInner;

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CountArgs {
    #[serde(default)]
    count: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ZoomArgs {
    #[serde(default)]
    factor: Option<f64>,
    #[serde(default)]
    delta: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SelectArgs {
    #[serde(default)]
    range: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

fn count(args: &CountArgs) -> i64 {
    i64::from(if args.count == 0 { 1 } else { args.count })
}

/// Register UI-session commands. Composition roots call this after `register_core`.
pub fn register_ui_commands(
    registry: &mut CommandRegistry,
    session: &crate::session::UiSession,
) -> Result<(), CoreError> {
    let inner = session.inner.clone();
    let specs: &[(&str, &str, HandlerKind)] = &[
        (
            "view.freeze",
            "Freeze panes at the cursor",
            HandlerKind::Freeze,
        ),
        (
            "view.split",
            "Split the view at the cursor",
            HandlerKind::Split,
        ),
        ("view.zoom", "Set or nudge grid zoom", HandlerKind::Zoom),
        (
            "view.select",
            "Select a range in the UI session",
            HandlerKind::Select,
        ),
        (
            "view.center",
            "Center the cursor in the viewport",
            HandlerKind::Center,
        ),
        ("nav.left", "Move left", HandlerKind::Move(0, -1)),
        ("nav.right", "Move right", HandlerKind::Move(0, 1)),
        ("nav.up", "Move up", HandlerKind::Move(-1, 0)),
        ("nav.down", "Move down", HandlerKind::Move(1, 0)),
        ("nav.top", "Go to the first row", HandlerKind::Top),
        ("nav.bottom", "Go to the last used row", HandlerKind::Bottom),
        ("nav.firstcol", "Go to column A", HandlerKind::FirstCol),
        (
            "nav.lastcol",
            "Go to the last used column",
            HandlerKind::LastCol,
        ),
        ("nav.pagedown", "Page down", HandlerKind::Move(20, 0)),
        ("nav.pageup", "Page up", HandlerKind::Move(-20, 0)),
        ("nav.enter", "Commit and move down", HandlerKind::Enter),
        ("nav.enterup", "Commit and move up", HandlerKind::EnterUp),
        ("nav.a1", "Go to A1", HandlerKind::A1),
        ("nav.goto", "Open Go To", HandlerKind::Goto),
        (
            "nav.nextedge",
            "Jump to the next data edge",
            HandlerKind::Move(0, 8),
        ),
        (
            "nav.prevedge",
            "Jump to the previous data edge",
            HandlerKind::Move(0, -8),
        ),
        ("sel.visual", "Visual selection", HandlerKind::Visual),
        ("sel.visualrow", "Visual row", HandlerKind::VisualRow),
        ("sel.visualcol", "Visual column", HandlerKind::VisualCol),
        ("sel.extendleft", "Extend left", HandlerKind::Extend(0, -1)),
        ("sel.extendright", "Extend right", HandlerKind::Extend(0, 1)),
        ("sel.extendup", "Extend up", HandlerKind::Extend(-1, 0)),
        ("sel.extenddown", "Extend down", HandlerKind::Extend(1, 0)),
        ("sel.row", "Select the cursor row", HandlerKind::SelRow),
        ("sel.col", "Select the cursor column", HandlerKind::SelCol),
        ("sel.extendmode", "Toggle extend mode (F8)", HandlerKind::F8),
        (
            "sel.addmode",
            "Add to selection (Shift+F8)",
            HandlerKind::ShiftF8,
        ),
        ("mode.normal", "Return to Normal", HandlerKind::Normal),
        ("edit.cell", "Edit in cell", HandlerKind::Edit),
        ("edit.append", "Append in cell", HandlerKind::Edit),
        ("edit.formula", "Start a formula", HandlerKind::Formula),
        ("edit.cancel", "Cancel edit", HandlerKind::Cancel),
        ("edit.commit", "Commit edit", HandlerKind::Commit),
        (
            "edit.cycleanchor",
            "Cycle reference anchors (F4)",
            HandlerKind::F4,
        ),
        ("edit.cut", "Cut", HandlerKind::Clip),
        ("edit.copy", "Copy", HandlerKind::Clip),
        ("edit.paste", "Paste", HandlerKind::Clip),
        ("edit.yank", "Yank", HandlerKind::Clip),
        ("edit.delete", "Delete selection", HandlerKind::Clip),
        ("edit.change", "Change selection", HandlerKind::Edit),
        ("edit.clearrow", "Clear the cursor row", HandlerKind::Clip),
        ("edit.repeat", "Repeat last action", HandlerKind::Clip),
        ("edit.find", "Find", HandlerKind::Find),
        ("edit.replace", "Replace", HandlerKind::Find),
        ("edit.search", "Search", HandlerKind::Find),
        ("edit.searchnext", "Search next", HandlerKind::Find),
        ("edit.searchprev", "Search previous", HandlerKind::Find),
        ("edit.fillselection", "Fill selection", HandlerKind::Clip),
        ("edit.filldown", "Fill down", HandlerKind::Clip),
        ("edit.fillright", "Fill right", HandlerKind::Clip),
        ("edit.autosum", "AutoSum", HandlerKind::Clip),
        (
            "palette.open",
            "Open the command palette",
            HandlerKind::Palette,
        ),
        (
            "command.line",
            "Open the modal command line",
            HandlerKind::CommandLine,
        ),
        ("help.keys", "Keys overlay", HandlerKind::Help),
        ("sheet.next", "Next sheet", HandlerKind::Clip),
        ("sheet.prev", "Previous sheet", HandlerKind::Clip),
        (
            "changeset.review",
            "Changeset review panel",
            HandlerKind::Changeset,
        ),
    ];
    for (id, doc, kind) in specs {
        let inner = inner.clone();
        let kind = *kind;
        registry.register::<CountArgs, _>(
            CommandSpec {
                id,
                doc,
                kind: CommandKind::Mutating,
                changeset_eligible: false,
                exposure: Exposure::Public,
                default_keys: &[],
            },
            move |ctx, args| handle(ctx, &inner, kind, args),
        )?;
    }
    let _ = (
        ZoomArgs::default(),
        SelectArgs::default(),
        EmptyArgs::default(),
    );
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum HandlerKind {
    Freeze,
    Split,
    Zoom,
    Select,
    Center,
    Move(i64, i64),
    Extend(i64, i64),
    Top,
    Bottom,
    FirstCol,
    LastCol,
    Enter,
    EnterUp,
    A1,
    Goto,
    Visual,
    VisualRow,
    VisualCol,
    SelRow,
    SelCol,
    F8,
    ShiftF8,
    Normal,
    Edit,
    Formula,
    Cancel,
    Commit,
    F4,
    Clip,
    Find,
    Palette,
    CommandLine,
    Help,
    Changeset,
}

fn handle(
    ctx: &mut CommandContext<'_>,
    inner: &Arc<Mutex<UiInner>>,
    kind: HandlerKind,
    args: CountArgs,
) -> Result<Effect, CoreError> {
    if ctx.is_dry_run() {
        return Ok(Effect::query(serde_json::json!({"dry_run": true})));
    }
    let mut g = inner.lock().unwrap_or_else(|p| p.into_inner());
    let n = count(&args);
    match kind {
        HandlerKind::Move(dr, dc) => g.selection.move_by(dr * n, dc * n),
        HandlerKind::Extend(dr, dc) => {
            g.selection.extend = ExtendMode::Extend;
            g.selection.move_by(dr * n, dc * n);
        }
        HandlerKind::Freeze => {
            g.viewport.freeze = FreezePanes {
                rows: g.selection.cursor.row,
                cols: g.selection.cursor.col,
            };
        }
        HandlerKind::Split => {
            g.viewport.split = Some(SplitView {
                x_px: 200,
                y_px: 200,
            });
        }
        HandlerKind::Zoom => {
            let factor = if n == 1 { 1.1 } else { 1.0 + 0.1 * n as f64 };
            let zoom = g.viewport.zoom * factor;
            g.viewport.set_zoom(zoom);
        }
        HandlerKind::Select => {}
        HandlerKind::Center => {
            let row = g.selection.cursor.row;
            g.viewport.ensure_row_visible(row);
        }
        HandlerKind::Top => {
            g.selection.cursor.row = 0;
            let cursor = g.selection.cursor;
            g.selection.replace(Area::cell(cursor));
        }
        HandlerKind::Bottom => {
            g.selection.cursor.row = g.selection.cursor.row.saturating_add(100);
            let cursor = g.selection.cursor;
            g.selection.replace(Area::cell(cursor));
        }
        HandlerKind::FirstCol => {
            g.selection.cursor.col = 0;
            let cursor = g.selection.cursor;
            g.selection.replace(Area::cell(cursor));
        }
        HandlerKind::LastCol => {
            g.selection.cursor.col = g.selection.cursor.col.saturating_add(16);
            let cursor = g.selection.cursor;
            g.selection.replace(Area::cell(cursor));
        }
        HandlerKind::Enter => {
            let _ = g.edit.commit();
            let dir = g.enter_moves.clone();
            match dir.as_str() {
                "right" => g.selection.move_by(0, 1),
                "none" => {}
                _ => g.selection.move_by(1, 0),
            }
        }
        HandlerKind::EnterUp => {
            let _ = g.edit.commit();
            g.selection.move_by(-1, 0);
        }
        HandlerKind::A1 => g.selection = crate::selection::Selection::a1(g.selection.sheet),
        HandlerKind::Goto => g.panel.open("goto"),
        HandlerKind::Visual => {
            g.mode = Mode::Visual;
            g.selection.extend = ExtendMode::Extend;
        }
        HandlerKind::VisualRow => {
            g.mode = Mode::VisualRow;
            g.selection.select_row();
        }
        HandlerKind::VisualCol => {
            g.mode = Mode::VisualCol;
            g.selection.select_col();
        }
        HandlerKind::SelRow => g.selection.select_row(),
        HandlerKind::SelCol => g.selection.select_col(),
        HandlerKind::F8 => {
            g.selection.extend = match g.selection.extend {
                ExtendMode::Extend => ExtendMode::Replace,
                _ => ExtendMode::Extend,
            };
        }
        HandlerKind::ShiftF8 => g.selection.extend = ExtendMode::Add,
        HandlerKind::Normal => {
            g.mode = Mode::Normal;
            g.edit.cancel();
            g.panel.dismiss();
        }
        HandlerKind::Edit => {
            let origin = g.selection.cursor;
            g.edit.begin(EditSurface::InCell, origin, "");
            if g.model == KeyModel::Modal {
                g.mode = Mode::Insert;
            }
        }
        HandlerKind::Formula => {
            let origin = g.selection.cursor;
            g.edit.begin(EditSurface::InCell, origin, "=");
            if g.model == KeyModel::Modal {
                g.mode = Mode::Insert;
            }
        }
        HandlerKind::Cancel => {
            g.edit.cancel();
            g.panel.dismiss();
        }
        HandlerKind::Commit => {
            let _ = g.edit.commit();
        }
        HandlerKind::F4 => {
            let _ = g.edit.cycle_anchor();
        }
        HandlerKind::Clip => {}
        HandlerKind::Find => g.panel.open("find"),
        HandlerKind::Palette => g.palette.open(),
        HandlerKind::CommandLine => {
            g.mode = Mode::Command;
            g.panel.open("command");
        }
        HandlerKind::Help => g.panel.open("keys"),
        HandlerKind::Changeset => g.panel.open("changeset"),
    }
    let row = g.selection.cursor.row;
    let col = g.selection.cursor.col;
    g.viewport.ensure_row_visible(row);
    g.viewport.ensure_col_visible(col);
    Ok(Effect {
        result: serde_json::json!({"ok": true}),
        auto_recalc: false,
        ..Effect::default()
    })
}
