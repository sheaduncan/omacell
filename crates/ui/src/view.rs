//! WP-14-owned commands: view.*, nav.*, sel.*, mode.*, edit.*, palette.open.

use std::sync::{Arc, Mutex};

use omacell_bus::{CommandContext, CommandKind, CommandRegistry, CommandSpec, Effect, Exposure};
use omacell_core::addr::{RefKind, parse_a1};
use omacell_core::changeset::ChangeSummary;
use omacell_core::error::CoreError;
use omacell_core::event::Event;
use omacell_core::find::{FindHit, FindSpec, find_cells};
use omacell_core::graph::CellCoord;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::sheet::{FreezePanes, SheetVisibility, SplitView};
use omacell_core::storage::CellSlot;
use omacell_core::value::Value;
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
    range: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    #[serde(default)]
    diagnose: bool,
}

fn count(args: &CountArgs) -> i64 {
    i64::from(if args.count == 0 { 1 } else { args.count })
}

/// Register UI-session commands. Composition roots call this after `register_core`.
pub fn register_ui_commands(
    registry: &mut CommandRegistry,
    session: &crate::session::UiSession,
) -> Result<(), CoreError> {
    crate::assist::register_ai_assist(registry, session)?;
    crate::name::register_name_commands(registry, session)?;
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
        ("nav.pagedown", "Page down", HandlerKind::PageRows(1, 1)),
        ("nav.pageup", "Page up", HandlerKind::PageRows(-1, 1)),
        (
            "nav.halfpagedown",
            "Move down half a page",
            HandlerKind::PageRows(1, 2),
        ),
        (
            "nav.halfpageup",
            "Move up half a page",
            HandlerKind::PageRows(-1, 2),
        ),
        ("nav.pageleft", "Page left", HandlerKind::PageCols(-1)),
        ("nav.pageright", "Page right", HandlerKind::PageCols(1)),
        (
            "nav.screentop",
            "Move to the top row on screen",
            HandlerKind::ScreenRow(ScreenRow::Top),
        ),
        (
            "nav.screenmiddle",
            "Move to the middle row on screen",
            HandlerKind::ScreenRow(ScreenRow::Middle),
        ),
        (
            "nav.screenbottom",
            "Move to the bottom row on screen",
            HandlerKind::ScreenRow(ScreenRow::Bottom),
        ),
        ("nav.enter", "Commit and move down", HandlerKind::Enter),
        ("nav.enterup", "Commit and move up", HandlerKind::EnterUp),
        ("nav.tab", "Commit and move right", HandlerKind::Tab(1)),
        ("nav.tableft", "Commit and move left", HandlerKind::Tab(-1)),
        ("nav.a1", "Go to A1", HandlerKind::A1),
        ("nav.goto", "Open Go To", HandlerKind::Goto),
        (
            "nav.nextedge",
            "Jump to the next data edge",
            HandlerKind::Edge(0, 1),
        ),
        (
            "nav.prevedge",
            "Jump to the previous data edge",
            HandlerKind::Edge(0, -1),
        ),
        (
            "nav.edgeup",
            "Jump to the upper data edge",
            HandlerKind::Edge(-1, 0),
        ),
        (
            "nav.edgedown",
            "Jump to the lower data edge",
            HandlerKind::Edge(1, 0),
        ),
        ("sel.visual", "Visual selection", HandlerKind::Visual),
        ("sel.visualrow", "Visual row", HandlerKind::VisualRow),
        ("sel.visualcol", "Visual column", HandlerKind::VisualCol),
        ("sel.extendleft", "Extend left", HandlerKind::Extend(0, -1)),
        ("sel.extendright", "Extend right", HandlerKind::Extend(0, 1)),
        ("sel.extendup", "Extend up", HandlerKind::Extend(-1, 0)),
        ("sel.extenddown", "Extend down", HandlerKind::Extend(1, 0)),
        (
            "sel.edgeleft",
            "Extend to the left data edge",
            HandlerKind::ExtendEdge(0, -1),
        ),
        (
            "sel.edgeright",
            "Extend to the right data edge",
            HandlerKind::ExtendEdge(0, 1),
        ),
        (
            "sel.edgeup",
            "Extend to the upper data edge",
            HandlerKind::ExtendEdge(-1, 0),
        ),
        (
            "sel.edgedown",
            "Extend to the lower data edge",
            HandlerKind::ExtendEdge(1, 0),
        ),
        ("sel.row", "Select the cursor row", HandlerKind::SelRow),
        ("sel.col", "Select the cursor column", HandlerKind::SelCol),
        (
            "sel.regionall",
            "Select the current region, then the whole sheet",
            HandlerKind::RegionAll,
        ),
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
        ("edit.find", "Find", HandlerKind::Find),
        ("edit.replace", "Replace", HandlerKind::Find),
        ("edit.search", "Search", HandlerKind::Find),
        (
            "edit.searchnext",
            "Select the next search result",
            HandlerKind::Search(true),
        ),
        (
            "edit.searchprev",
            "Select the previous search result",
            HandlerKind::Search(false),
        ),
        (
            "edit.explainerror",
            "Explain the selected cell error",
            HandlerKind::ExplainError,
        ),
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
        ("sheet.next", "Next sheet", HandlerKind::Sheet(1)),
        ("sheet.prev", "Previous sheet", HandlerKind::Sheet(-1)),
        (
            "view.formulabar",
            "Expand or collapse the formula bar",
            HandlerKind::FormulaBar,
        ),
        (
            "view.formulas",
            "Toggle formula-source display",
            HandlerKind::ShowFormulas,
        ),
        (
            "changeset.review",
            "Changeset review panel",
            HandlerKind::Changeset,
        ),
    ];
    let agent_inner = session.inner.clone();
    registry.register::<AgentArgs, _>(
        CommandSpec {
            id: "ai.agent",
            doc: "Hand the workbook to the Omarchy default agent",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |_ctx, args| {
            let mut g = agent_inner.lock().unwrap_or_else(|p| p.into_inner());
            g.pending_agent = Some(crate::session::AgentHandoff {
                prompt: args.prompt.unwrap_or_default(),
                diagnose: args.diagnose,
            });
            Ok(Effect::query(serde_json::json!({"queued": true})))
        },
    )?;
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
    let panel_specs = [
        (
            "comments.panel",
            "List notes and comments on the active sheet",
            crate::panel::WorkbookPanel::Comments,
        ),
        (
            "sort.panel",
            "Open sort controls for the selection",
            crate::panel::WorkbookPanel::Sort,
        ),
        (
            "filter.panel",
            "Show the active filter and filter controls",
            crate::panel::WorkbookPanel::Filter,
        ),
    ];
    for (id, doc, kind) in panel_specs {
        let inner = inner.clone();
        registry.register::<omacell_bus::args::EmptyArgs, _>(
            CommandSpec {
                id,
                doc,
                kind: CommandKind::Mutating,
                changeset_eligible: false,
                exposure: Exposure::Public,
                default_keys: &[],
            },
            move |ctx, _args| {
                if ctx.is_preflight() {
                    return Ok(Effect::query(
                        serde_json::json!({"dry_run": ctx.is_dry_run()}),
                    ));
                }
                let mut session = inner.lock().unwrap_or_else(|p| p.into_inner());
                let selection = session.selection.clone();
                crate::panel::open_workbook_panel(
                    &mut session.panel,
                    &selection,
                    ctx.workbook_ref(),
                    kind,
                );
                Ok(Effect::query(serde_json::json!({"panel": kind.id()})))
            },
        )?;
    }
    let zoom_inner = session.inner.clone();
    registry.register::<ZoomArgs, _>(
        CommandSpec {
            id: "view.zoom",
            doc: "Set or nudge grid zoom",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args| handle_zoom(ctx, &zoom_inner, args),
    )?;
    let select_inner = session.inner.clone();
    registry.register::<SelectArgs, _>(
        CommandSpec {
            id: "view.select",
            doc: "Select an A1 range in the UI session",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args| handle_select(ctx, &select_inner, args),
    )?;
    session.keymap().validate_commands(registry)?;
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum HandlerKind {
    Freeze,
    Split,
    Center,
    Move(i64, i64),
    PageRows(i64, u32),
    PageCols(i64),
    ScreenRow(ScreenRow),
    Edge(i64, i64),
    Extend(i64, i64),
    ExtendEdge(i64, i64),
    Top,
    Bottom,
    FirstCol,
    LastCol,
    Enter,
    EnterUp,
    Tab(i64),
    A1,
    Goto,
    Visual,
    VisualRow,
    VisualCol,
    SelRow,
    SelCol,
    RegionAll,
    F8,
    ShiftF8,
    Normal,
    Edit,
    Formula,
    Cancel,
    Commit,
    F4,
    Sheet(i32),
    FormulaBar,
    ShowFormulas,
    Find,
    Search(bool),
    ExplainError,
    Palette,
    CommandLine,
    Help,
    Changeset,
}

#[derive(Clone, Copy, Debug)]
enum ScreenRow {
    Top,
    Middle,
    Bottom,
}

fn handle(
    ctx: &mut CommandContext<'_>,
    inner: &Arc<Mutex<UiInner>>,
    kind: HandlerKind,
    args: CountArgs,
) -> Result<Effect, CoreError> {
    if ctx.is_preflight() {
        match kind {
            HandlerKind::Enter
            | HandlerKind::EnterUp
            | HandlerKind::Tab(_)
            | HandlerKind::Commit => {
                let g = inner
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                return apply_edit(ctx, &g);
            }
            HandlerKind::F4 => {
                let mut edit = inner
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .edit
                    .clone();
                edit.cycle_anchor()?;
            }
            HandlerKind::Edit => {
                let cursor = inner
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .selection
                    .cursor;
                let _ = cell_input(ctx, cursor)?;
            }
            _ => {}
        }
        return Ok(Effect::query(
            serde_json::json!({"dry_run": ctx.is_dry_run()}),
        ));
    }
    let mut g = inner.lock().unwrap_or_else(|p| p.into_inner());
    let n = count(&args);
    let mut effect = Effect::query(serde_json::json!({"ok": true}));
    match kind {
        HandlerKind::Move(dr, dc) => g.selection.move_by(dr * n, dc * n),
        HandlerKind::PageRows(direction, divisor) => {
            let rows = i64::from((g.viewport.page_rows() / divisor).max(1));
            g.selection.move_by(direction * rows * n, 0);
        }
        HandlerKind::PageCols(direction) => {
            let cols = i64::from(g.viewport.page_cols().max(1));
            g.selection.move_by(0, direction * cols * n);
        }
        HandlerKind::ScreenRow(position) => {
            let (top, middle, bottom) = g.viewport.screen_rows();
            let target = match position {
                ScreenRow::Top => top,
                ScreenRow::Middle => middle,
                ScreenRow::Bottom => bottom,
            };
            let current = g.selection.cursor;
            g.selection
                .move_by(i64::from(target) - i64::from(current.row), 0);
        }
        HandlerKind::Edge(dr, dc) => {
            if let Some(cursor) = data_edge(ctx, g.selection.cursor, dr, dc)? {
                g.selection.replace(Area::cell(cursor));
            }
        }
        HandlerKind::Extend(dr, dc) => {
            g.selection.extend = ExtendMode::Extend;
            g.selection.move_by(dr * n, dc * n);
        }
        HandlerKind::ExtendEdge(dr, dc) => {
            if let Some(target) = data_edge(ctx, g.selection.cursor, dr, dc)? {
                let current = g.selection.cursor;
                g.selection.extend = ExtendMode::Extend;
                g.selection.move_by(
                    i64::from(target.row) - i64::from(current.row),
                    i64::from(target.col) - i64::from(current.col),
                );
            }
        }
        HandlerKind::Freeze => {
            g.viewport.freeze = FreezePanes {
                rows: g.selection.cursor.row,
                cols: g.selection.cursor.col,
            };
            g.viewport.split = None;
        }
        HandlerKind::Split => {
            let cursor = g.selection.cursor;
            let x = g.viewport.cols.index_to_pixel(u32::from(cursor.col));
            let y = g.viewport.rows.index_to_pixel(cursor.row);
            g.viewport.split = Some(SplitView {
                x_px: scaled_coordinate(x, g.viewport.zoom),
                y_px: scaled_coordinate(y, g.viewport.zoom),
            });
            g.viewport.freeze = FreezePanes::default();
            g.viewport.first_row = cursor.row;
            g.viewport.first_col = cursor.col;
        }
        HandlerKind::Center => {
            let row = g.selection.cursor.row;
            let col = g.selection.cursor.col;
            g.viewport.center_on(row, col);
        }
        HandlerKind::Top => {
            g.selection.cursor.row = 0;
            let cursor = g.selection.cursor;
            g.selection.replace(Area::cell(cursor));
        }
        HandlerKind::Bottom => {
            g.selection.cursor.row = ctx
                .workbook_ref()
                .sheet(g.selection.sheet)
                .and_then(|sheet| sheet.used_range())
                .map_or(0, |used| used.max_row);
            let cursor = g.selection.cursor;
            g.selection.replace(Area::cell(cursor));
        }
        HandlerKind::FirstCol => {
            g.selection.cursor.col = 0;
            let cursor = g.selection.cursor;
            g.selection.replace(Area::cell(cursor));
        }
        HandlerKind::LastCol => {
            g.selection.cursor.col = ctx
                .workbook_ref()
                .sheet(g.selection.sheet)
                .and_then(|sheet| sheet.used_range())
                .map_or(0, |used| used.max_col);
            let cursor = g.selection.cursor;
            g.selection.replace(Area::cell(cursor));
        }
        HandlerKind::Enter => {
            effect = apply_edit(ctx, &g)?;
            let _ = g.edit.commit();
            let dir = g.enter_moves.clone();
            match dir.as_str() {
                "right" => g.selection.move_by(0, 1),
                "none" => {}
                _ => g.selection.move_by(1, 0),
            }
        }
        HandlerKind::EnterUp => {
            effect = apply_edit(ctx, &g)?;
            let _ = g.edit.commit();
            g.selection.move_by(-1, 0);
        }
        HandlerKind::Tab(direction) => {
            effect = apply_edit(ctx, &g)?;
            let _ = g.edit.commit();
            g.selection.move_by(0, direction);
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
        HandlerKind::RegionAll => {
            let cursor = g.selection.cursor;
            let whole = Area {
                start: omacell_core::addr::CellRef {
                    row: 0,
                    col: 0,
                    ..cursor
                },
                end: omacell_core::addr::CellRef {
                    row: omacell_core::limits::MAX_ROWS - 1,
                    col: omacell_core::limits::MAX_COLS - 1,
                    ..cursor
                },
            };
            let region = current_region(ctx, cursor)?;
            if g.selection.areas.len() == 1 && g.selection.active() == region {
                g.selection.replace(whole);
            } else {
                g.selection.replace(region);
            }
        }
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
            let initial = cell_input(ctx, origin)?;
            g.edit.begin(EditSurface::InCell, origin, &initial);
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
            effect = apply_edit(ctx, &g)?;
            let _ = g.edit.commit();
            if g.model == KeyModel::Modal {
                g.mode = Mode::Normal;
            }
        }
        HandlerKind::F4 => {
            g.edit.cycle_anchor()?;
        }
        HandlerKind::Sheet(delta) => {
            let sheets = ctx
                .workbook_ref()
                .sheets()
                .filter(|sheet| {
                    sheet.visibility == SheetVisibility::Visible || sheet.id == g.selection.sheet
                })
                .map(|sheet| sheet.id)
                .collect::<Vec<_>>();
            if let Some(index) = sheets.iter().position(|id| *id == g.selection.sheet) {
                let next = if delta > 0 && args.count > 0 {
                    usize::try_from(args.count.saturating_sub(1))
                        .unwrap_or(usize::MAX)
                        .min(sheets.len() - 1)
                } else if delta < 0 {
                    index.saturating_sub(
                        delta.unsigned_abs() as usize * usize::try_from(n).unwrap_or(usize::MAX),
                    )
                } else {
                    index.saturating_add(delta as usize).min(sheets.len() - 1)
                };
                let sheet = sheets[next];
                ctx.workbook().set_active_sheet(sheet)?;
                g.selection = crate::selection::Selection::a1(sheet);
            }
        }
        HandlerKind::FormulaBar => g.formula_bar_expanded = !g.formula_bar_expanded,
        HandlerKind::ShowFormulas => g.show_formulas = !g.show_formulas,
        HandlerKind::Find => g.panel.open("find"),
        HandlerKind::Search(forward) => {
            effect = search(ctx, &mut g, forward, args.count.max(1))?;
        }
        HandlerKind::ExplainError => {
            let cursor = g.selection.cursor;
            let cell = CellCoord::new(g.selection.sheet, cursor.row, cursor.col);
            let explanation =
                omacell_core::audit::explain_error(ctx.workbook_ref(), ctx.engine_ref(), cell);
            let body = explanation.as_ref().map_or_else(
                || format!("{} does not contain an error.", g.selection.cursor.to_a1()),
                |explanation| {
                    format!(
                        "{}!{}\n{}",
                        explanation.sheet, explanation.cell_ref, explanation.message
                    )
                },
            );
            g.panel.open_with_body("explainerror", body);
            effect = Effect::query(serde_json::to_value(explanation).map_err(|err| {
                CoreError::new("ui.explain", format!("serialize error explanation: {err}"))
            })?);
        }
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
    Ok(effect)
}

fn search(
    ctx: &mut CommandContext<'_>,
    inner: &mut UiInner,
    forward: bool,
    steps: u32,
) -> Result<Effect, CoreError> {
    let options = &inner.find;
    let spec = FindSpec {
        query: options.find.clone(),
        formulas: options.in_formulas,
        whole: options.whole_cell,
        case: options.case,
        regex: options.regex,
        workbook: matches!(options.scope, crate::find::FindScope::Workbook),
    };
    let hits = find_cells(ctx.workbook_ref(), inner.selection.sheet, &spec)?;
    let Some(hit) = search_hit(&hits, &inner.selection, forward, steps) else {
        return Ok(Effect::query(serde_json::json!({"count": 0})));
    };
    ctx.workbook().set_active_sheet(hit.sheet)?;
    let cell = omacell_core::addr::CellRef {
        sheet: Some(hit.sheet),
        row: hit.row,
        col: hit.col,
        row_abs: false,
        col_abs: false,
    };
    inner.selection.replace(Area::cell(cell));
    Ok(Effect::query(serde_json::json!({
        "count": hits.len(),
        "sheet": hit.sheet.index(),
        "row": hit.row,
        "col": hit.col,
    })))
}

fn search_hit<'a>(
    hits: &'a [FindHit],
    selection: &crate::selection::Selection,
    forward: bool,
    steps: u32,
) -> Option<&'a FindHit> {
    if hits.is_empty() {
        return None;
    }
    let current = (
        selection.sheet.index(),
        selection.cursor.row,
        selection.cursor.col,
    );
    let insertion = hits.partition_point(|hit| (hit.sheet.index(), hit.row, hit.col) < current);
    let len = hits.len();
    let offset = usize::try_from(steps.saturating_sub(1)).unwrap_or(usize::MAX) % len;
    let index = if forward {
        let first_after =
            hits.partition_point(|hit| (hit.sheet.index(), hit.row, hit.col) <= current);
        (first_after % len + offset) % len
    } else {
        let first_before = if insertion == 0 {
            len - 1
        } else {
            insertion - 1
        };
        (first_before + len - offset) % len
    };
    hits.get(index)
}

fn data_edge(
    ctx: &CommandContext<'_>,
    cursor: omacell_core::addr::CellRef,
    drow: i64,
    dcol: i64,
) -> Result<Option<omacell_core::addr::CellRef>, CoreError> {
    let sheet_id = cursor
        .sheet
        .unwrap_or_else(|| ctx.workbook_ref().active_sheet());
    let Some(sheet) = ctx.workbook_ref().sheet(sheet_id) else {
        return Ok(None);
    };
    let (current, maximum, occupied) = if drow != 0 {
        (
            cursor.row,
            MAX_ROWS - 1,
            sheet
                .store
                .iter_col(cursor.col)
                .filter(|(_, slot)| cell_has_contents(*slot))
                .map(|(row, _)| row)
                .collect::<Vec<_>>(),
        )
    } else if dcol != 0 {
        (
            u32::from(cursor.col),
            u32::from(MAX_COLS - 1),
            sheet
                .store
                .iter_row(cursor.row)
                .filter(|(_, slot)| cell_has_contents(*slot))
                .map(|(col, _)| u32::from(col))
                .collect::<Vec<_>>(),
        )
    } else {
        return Ok(Some(cursor));
    };
    let target = edge_index(&occupied, current, maximum, drow > 0 || dcol > 0);
    let mut target_cell = cursor;
    if drow != 0 {
        target_cell.row = target;
    } else {
        target_cell.col = u16::try_from(target).unwrap_or(MAX_COLS - 1);
    }
    Ok(Some(target_cell))
}

fn edge_index(occupied: &[u32], current: u32, maximum: u32, forward: bool) -> u32 {
    if forward {
        let adjacent = current.saturating_add(1).min(maximum);
        if occupied.binary_search(&current).is_ok()
            && let Ok(mut index) = occupied.binary_search(&adjacent)
        {
            let mut target = adjacent;
            while let Some(next) = occupied.get(index + 1)
                && *next == target.saturating_add(1)
            {
                target = *next;
                index += 1;
            }
            return target;
        }
        let index = occupied.partition_point(|position| *position <= current);
        occupied.get(index).copied().unwrap_or(maximum)
    } else {
        let adjacent = current.saturating_sub(1);
        if occupied.binary_search(&current).is_ok()
            && current > 0
            && let Ok(mut index) = occupied.binary_search(&adjacent)
        {
            let mut target = adjacent;
            while index > 0 && occupied[index - 1].saturating_add(1) == target {
                index -= 1;
                target = occupied[index];
            }
            return target;
        }
        let index = occupied.partition_point(|position| *position < current);
        index
            .checked_sub(1)
            .and_then(|previous| occupied.get(previous))
            .copied()
            .unwrap_or(0)
    }
}

fn current_region(
    ctx: &CommandContext<'_>,
    cursor: omacell_core::addr::CellRef,
) -> Result<Area, CoreError> {
    let sheet_id = cursor
        .sheet
        .unwrap_or_else(|| ctx.workbook_ref().active_sheet());
    let Some(sheet) = ctx.workbook_ref().sheet(sheet_id) else {
        return Ok(Area::cell(cursor));
    };
    if !sheet
        .store
        .get(cursor.row, cursor.col)?
        .copied()
        .is_some_and(cell_has_contents)
    {
        return Ok(Area::cell(cursor));
    }

    let mut row_spans = vec![None::<(u16, u16)>; MAX_ROWS as usize];
    let mut col_spans = vec![None::<(u32, u32)>; usize::from(MAX_COLS)];
    for (row, col, _slot) in sheet
        .store
        .iter()
        .filter(|(_, _, slot)| cell_has_contents(*slot))
    {
        let row_span = &mut row_spans[row as usize];
        *row_span = Some(row_span.map_or((col, col), |(min, max)| (min.min(col), max.max(col))));
        let col_span = &mut col_spans[usize::from(col)];
        *col_span = Some(col_span.map_or((row, row), |(min, max)| (min.min(row), max.max(row))));
    }
    let (mut min_row, mut max_row) = (cursor.row, cursor.row);
    let (mut min_col, mut max_col) = (cursor.col, cursor.col);
    loop {
        let before = (min_row, min_col, max_row, max_col);
        while min_row > 0
            && row_spans[(min_row - 1) as usize]
                .is_some_and(|span| overlaps_u16(span, (min_col, max_col)))
        {
            min_row -= 1;
        }
        while max_row + 1 < MAX_ROWS
            && row_spans[(max_row + 1) as usize]
                .is_some_and(|span| overlaps_u16(span, (min_col, max_col)))
        {
            max_row += 1;
        }
        while min_col > 0
            && col_spans[usize::from(min_col - 1)]
                .is_some_and(|span| overlaps_u32(span, (min_row, max_row)))
        {
            min_col -= 1;
        }
        while max_col + 1 < MAX_COLS
            && col_spans[usize::from(max_col + 1)]
                .is_some_and(|span| overlaps_u32(span, (min_row, max_row)))
        {
            max_col += 1;
        }
        if before == (min_row, min_col, max_row, max_col) {
            break;
        }
    }
    Ok(Area {
        start: omacell_core::addr::CellRef {
            row: min_row,
            col: min_col,
            ..cursor
        },
        end: omacell_core::addr::CellRef {
            row: max_row,
            col: max_col,
            ..cursor
        },
    })
}

fn overlaps_u16(left: (u16, u16), right: (u16, u16)) -> bool {
    left.0 <= right.1 && right.0 <= left.1
}

fn overlaps_u32(left: (u32, u32), right: (u32, u32)) -> bool {
    left.0 <= right.1 && right.0 <= left.1
}

fn cell_has_contents(slot: CellSlot) -> bool {
    slot.formula.is_some() || !matches!(slot.value, Value::Empty)
}

fn scaled_coordinate(value: u64, zoom: f64) -> u32 {
    (value as f64 * zoom)
        .round()
        .clamp(0.0, f64::from(u32::MAX)) as u32
}

fn apply_edit(ctx: &mut CommandContext<'_>, session: &UiInner) -> Result<Effect, CoreError> {
    let Some(origin) = session.edit.origin else {
        return Ok(Effect::query(serde_json::json!({"changed": 0})));
    };
    let sheet = origin.sheet.unwrap_or(session.selection.sheet);
    if cell_input(ctx, origin)? == session.edit.buffer {
        return Ok(Effect::query(serde_json::json!({"changed": 0})));
    }
    ctx.workbook()
        .set_cell_contents(sheet, origin.row, origin.col, &session.edit.buffer)?;
    Ok(Effect {
        events: vec![Event::CellChanged {
            sheet,
            row: origin.row,
            col: origin.col,
        }],
        summary: ChangeSummary {
            cells: 1,
            text: format!("edit {}", origin.to_a1()),
            ..ChangeSummary::default()
        },
        dirty: vec![CellCoord::new(sheet, origin.row, origin.col)],
        result: serde_json::json!({"changed": 1}),
        auto_recalc: true,
        ..Effect::default()
    })
}

fn cell_input(
    ctx: &CommandContext<'_>,
    cell: omacell_core::addr::CellRef,
) -> Result<String, CoreError> {
    let sheet = cell
        .sheet
        .unwrap_or_else(|| ctx.workbook_ref().active_sheet());
    let Some(slot) = ctx.workbook_ref().get(sheet, cell.row, cell.col)? else {
        return Ok(String::new());
    };
    if let Some(formula) = slot.formula {
        return ctx
            .workbook_ref()
            .intern()
            .formulas
            .get(formula)
            .map(str::to_string)
            .ok_or_else(|| CoreError::new("ui.edit", "cell formula handle is not interned"));
    }
    Ok(match slot.value {
        Value::Text(id) => ctx
            .workbook_ref()
            .intern()
            .strings
            .get(id)
            .map(str::to_string)
            .ok_or_else(|| CoreError::new("ui.edit", "cell text handle is not interned"))?,
        value => value.to_string(),
    })
}

fn handle_zoom(
    ctx: &mut CommandContext<'_>,
    inner: &Arc<Mutex<UiInner>>,
    args: ZoomArgs,
) -> Result<Effect, CoreError> {
    let current = inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .viewport
        .zoom;
    let zoom = match (args.factor, args.delta) {
        (Some(factor), None) => factor,
        (None, Some(delta)) => current + delta,
        _ => {
            return Err(CoreError::new(
                "ui.view",
                "view.zoom requires exactly one of factor or delta",
            ));
        }
    };
    if !zoom.is_finite() || zoom <= 0.0 {
        return Err(CoreError::new(
            "ui.view",
            "view.zoom must resolve to a finite positive factor",
        ));
    }
    if ctx.is_preflight() {
        return Ok(Effect::query(
            serde_json::json!({"zoom": zoom.clamp(0.25, 8.0), "dry_run": ctx.is_dry_run()}),
        ));
    }
    let mut session = inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    session.viewport.set_zoom(zoom);
    Ok(Effect::query(
        serde_json::json!({"zoom": session.viewport.zoom}),
    ))
}

fn handle_select(
    ctx: &mut CommandContext<'_>,
    inner: &Arc<Mutex<UiInner>>,
    args: SelectArgs,
) -> Result<Effect, CoreError> {
    let (sheet, area) = resolve_selection(ctx, &args.range)?;
    if ctx.is_preflight() {
        return Ok(Effect::query(
            serde_json::json!({"range": args.range, "dry_run": ctx.is_dry_run()}),
        ));
    }
    ctx.workbook().set_active_sheet(sheet)?;
    let mut session = inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    session.selection.sheet = sheet;
    session.selection.replace(area);
    let cursor = session.selection.cursor;
    session.viewport.ensure_row_visible(cursor.row);
    session.viewport.ensure_col_visible(cursor.col);
    Ok(Effect::query(serde_json::json!({"range": args.range})))
}

fn resolve_selection(
    ctx: &CommandContext<'_>,
    range: &str,
) -> Result<(omacell_core::addr::SheetId, Area), CoreError> {
    let resolved = ctx.workbook_ref().resolve_parsed(parse_a1(range)?)?;
    let (sheet, mut start, mut end) = match resolved {
        RefKind::Cell(cell) => {
            let sheet = cell
                .sheet
                .unwrap_or_else(|| ctx.workbook_ref().active_sheet());
            (sheet, cell, cell)
        }
        RefKind::Range(range) => {
            if range.sheet_end.is_some() {
                return Err(CoreError::new(
                    "ui.view",
                    "view.select does not support 3-D ranges",
                ));
            }
            let sheet = range
                .start
                .sheet
                .unwrap_or_else(|| ctx.workbook_ref().active_sheet());
            (sheet, range.start, range.end)
        }
    };
    start.sheet = Some(sheet);
    end.sheet = Some(sheet);
    Ok((sheet, Area { start, end }))
}
