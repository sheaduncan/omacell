//! Exact logical inverses for changeset-eligible commands.

use std::collections::{BTreeMap, BTreeSet};

use omacell_core::addr::SheetId;
use omacell_core::changeset::{ChangeSummary, CommandCall};
use omacell_core::chart::{Chart, Sparkline};
use omacell_core::error::CoreError;
use omacell_core::print::PageSetup;
use omacell_core::sheet::{Sheet, SheetEditState, SheetVisibility, ViewState};
use omacell_core::storage::CellSlot;
use omacell_core::style::Color;
use omacell_core::tables::Table;
use omacell_core::workbook::{Workbook, WorkbookProtectionState};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::args::CellRestoreArgs;
use crate::commands::cell_restore;
use crate::error as bus_error;
use crate::handler::{CommandContext, Effect};
use crate::logical::{call, inverse_contents};
use crate::registry::{CommandKind, CommandRegistry, CommandSpec, Exposure};
use crate::resolve::ResolvedCell;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RestoreWireArgs {
    patch: serde_json::Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestorePatch {
    #[serde(default)]
    cells: Vec<CellRestoreArgs>,
    #[serde(default)]
    sheet_states: Vec<(u32, SheetEditState)>,
    #[serde(default)]
    restore_sheets: Vec<SheetSnapshot>,
    #[serde(default)]
    remove_sheets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    order: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workbook_protection: Option<WorkbookProtectionState>,
    #[serde(default)]
    restore_tables: Vec<Table>,
    #[serde(default)]
    remove_tables: Vec<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SheetSnapshot {
    index: usize,
    id: u32,
    name: String,
    visibility: SheetVisibility,
    tab_color: Option<Color>,
    view: ViewState,
    edit: SheetEditState,
    cells: Vec<CellRestoreArgs>,
    charts: Vec<Chart>,
    sparklines: Vec<Sparkline>,
    page_setup: PageSetup,
}

pub(crate) fn register(registry: &mut CommandRegistry) -> Result<(), CoreError> {
    registry.register(
        CommandSpec {
            id: "edit.restore",
            doc: "Internal: restore an exact WP-17 logical patch",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Internal,
            default_keys: &[],
        },
        restore,
    )
}

pub(crate) fn exact_inverse(before: &Workbook, after: &Workbook) -> Result<CommandCall, CoreError> {
    let patch = diff(before, after)?;
    let encoded = serde_json::to_value(patch)
        .map_err(|error| bus_error::args(format!("cannot encode exact inverse: {error}")))?;
    call(
        "edit.restore",
        serde_json::to_value(RestoreWireArgs { patch: encoded })
            .map_err(|error| bus_error::args(format!("cannot encode restore command: {error}")))?,
    )
}

fn diff(before: &Workbook, after: &Workbook) -> Result<RestorePatch, CoreError> {
    let before_ids: BTreeSet<_> = before.sheets().map(|sheet| sheet.id.index()).collect();
    let after_ids: BTreeSet<_> = after.sheets().map(|sheet| sheet.id.index()).collect();
    let mut patch = RestorePatch::default();

    for id in before_ids.difference(&after_ids) {
        patch
            .restore_sheets
            .push(snapshot_sheet(before, SheetId::new(*id))?);
    }
    for id in after_ids.difference(&before_ids) {
        let sheet = after
            .sheet(SheetId::new(*id))
            .ok_or_else(|| CoreError::sheet_id("added sheet vanished"))?;
        patch.remove_sheets.push(sheet.name.clone());
    }
    for id in before_ids.intersection(&after_ids) {
        let id = SheetId::new(*id);
        let before_sheet = before
            .sheet(id)
            .ok_or_else(|| CoreError::sheet_id("source sheet vanished"))?;
        let after_sheet = after
            .sheet(id)
            .ok_or_else(|| CoreError::sheet_id("target sheet vanished"))?;
        let before_state = before.sheet_edit_state(id)?;
        let after_state = after.sheet_edit_state(id)?;
        if before_state != after_state {
            patch.sheet_states.push((id.index(), before_state));
        }
        patch
            .cells
            .extend(diff_cells(before, after, before_sheet, after_sheet)?);
    }

    let before_order: Vec<_> = before.sheets().map(|sheet| sheet.id.index()).collect();
    let after_order: Vec<_> = after.sheets().map(|sheet| sheet.id.index()).collect();
    if before_order != after_order {
        patch.order = Some(before_order);
    }
    if before.active_sheet() != after.active_sheet() {
        patch.active = Some(before.active_sheet().index());
    }
    if before.protection() != after.protection() {
        patch.workbook_protection = Some(before.protection().clone());
    }
    let before_tables: BTreeMap<_, _> = before
        .tables()
        .iter()
        .map(|table| (table.id.index(), table))
        .collect();
    let after_tables: BTreeMap<_, _> = after
        .tables()
        .iter()
        .map(|table| (table.id.index(), table))
        .collect();
    let table_ids: BTreeSet<_> = before_tables
        .keys()
        .chain(after_tables.keys())
        .copied()
        .collect();
    for id in table_ids {
        if before_tables.get(&id) == after_tables.get(&id) {
            continue;
        }
        if let Some(table) = before_tables.get(&id) {
            patch.restore_tables.push((*table).clone());
        } else {
            patch.remove_tables.push(id);
        }
    }
    Ok(patch)
}

fn diff_cells(
    before: &Workbook,
    _after: &Workbook,
    before_sheet: &Sheet,
    after_sheet: &Sheet,
) -> Result<Vec<CellRestoreArgs>, CoreError> {
    let before_cells: BTreeMap<(u32, u16), CellSlot> = before_sheet
        .store
        .iter()
        .map(|(row, col, slot)| ((row, col), slot))
        .collect();
    let after_cells: BTreeMap<(u32, u16), CellSlot> = after_sheet
        .store
        .iter()
        .map(|(row, col, slot)| ((row, col), slot))
        .collect();
    let coords: BTreeSet<_> = before_cells
        .keys()
        .chain(after_cells.keys())
        .copied()
        .collect();
    let mut restores = Vec::new();
    for (row, col) in coords {
        if before_cells.get(&(row, col)) == after_cells.get(&(row, col)) {
            continue;
        }
        let inverse = inverse_contents(
            before,
            ResolvedCell {
                sheet: before_sheet.id,
                row,
                col,
            },
        )?;
        let args: CellRestoreArgs = serde_json::from_value(inverse.args)
            .map_err(|error| bus_error::args(format!("invalid generated cell inverse: {error}")))?;
        restores.push(args);
    }
    Ok(restores)
}

fn snapshot_sheet(workbook: &Workbook, id: SheetId) -> Result<SheetSnapshot, CoreError> {
    let sheet = workbook
        .sheet(id)
        .ok_or_else(|| CoreError::sheet_id("removed sheet vanished"))?;
    let mut cells = Vec::new();
    for (row, col, _) in sheet.store.iter() {
        let inverse = inverse_contents(
            workbook,
            ResolvedCell {
                sheet: id,
                row,
                col,
            },
        )?;
        cells.push(serde_json::from_value(inverse.args).map_err(|error| {
            bus_error::args(format!("invalid generated sheet cell inverse: {error}"))
        })?);
    }
    Ok(SheetSnapshot {
        index: workbook.sheet_index(id)?,
        id: id.index(),
        name: sheet.name.clone(),
        visibility: sheet.visibility,
        tab_color: sheet.tab_color,
        view: sheet.view.clone(),
        edit: workbook.sheet_edit_state(id)?,
        cells,
        charts: sheet.charts.clone(),
        sparklines: sheet.sparklines.clone(),
        page_setup: sheet.page_setup.clone(),
    })
}

fn restore(ctx: &mut CommandContext<'_>, args: RestoreWireArgs) -> Result<Effect, CoreError> {
    let patch: RestorePatch = serde_json::from_value(args.patch)
        .map_err(|error| bus_error::args(format!("invalid exact restore patch: {error}")))?;
    let mut effect = Effect {
        auto_recalc: true,
        rebuild: true,
        summary: ChangeSummary {
            text: "restore exact edit state".into(),
            ..ChangeSummary::default()
        },
        ..Effect::default()
    };

    for id in patch.remove_tables {
        ctx.workbook()
            .convert_table(omacell_core::tables::TableId::new(id))?;
    }
    for name in patch.remove_sheets {
        let id = ctx
            .workbook_ref()
            .sheet_by_name(&name)
            .map(|sheet| sheet.id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {name:?}")))?;
        ctx.workbook().remove_sheet(id)?;
    }
    for snapshot in patch.restore_sheets {
        restore_sheet(ctx, snapshot, &mut effect)?;
    }
    for (id, state) in patch.sheet_states {
        ctx.workbook()
            .restore_sheet_edit_state(SheetId::new(id), state)?;
    }
    for cell in patch.cells {
        effect.append(cell_restore(ctx, cell)?);
    }
    if let Some(order) = patch.order {
        for (index, id) in order.into_iter().enumerate() {
            ctx.workbook().reorder_sheet(SheetId::new(id), index)?;
        }
    }
    if let Some(active) = patch.active {
        ctx.workbook().set_active_sheet(SheetId::new(active))?;
    }
    if let Some(protection) = patch.workbook_protection {
        ctx.workbook().set_workbook_protection(protection)?;
    }
    for table in patch.restore_tables {
        ctx.workbook().restore_table(table)?;
    }
    effect.result = serde_json::json!({"restored": true});
    Ok(effect)
}

fn restore_sheet(
    ctx: &mut CommandContext<'_>,
    snapshot: SheetSnapshot,
    effect: &mut Effect,
) -> Result<(), CoreError> {
    let mut sheet = Sheet::new(SheetId::new(snapshot.id), snapshot.name)?;
    sheet.visibility = snapshot.visibility;
    sheet.tab_color = snapshot.tab_color;
    sheet.view = snapshot.view;
    sheet.charts = snapshot.charts;
    sheet.sparklines = snapshot.sparklines;
    sheet.page_setup = snapshot.page_setup;
    let id = sheet.id;
    ctx.workbook().restore_sheet_at(snapshot.index, sheet)?;
    ctx.workbook().restore_sheet_edit_state(id, snapshot.edit)?;
    for cell in snapshot.cells {
        effect.append(cell_restore(ctx, cell)?);
    }
    effect.summary.sheets += 1;
    Ok(())
}
