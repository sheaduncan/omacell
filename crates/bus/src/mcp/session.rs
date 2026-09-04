//! Sync MCP tool/resource dispatch over [`crate::Bus`].

use omacell_core::addr::{RefKind, col_to_letters, parse_a1, quote_sheet_name};
use omacell_core::changeset::{Changeset, ChangesetId, ChangesetStatus, CommandCall};
use omacell_core::command::{CommandId, Origin};
use omacell_core::error::CoreError;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use serde_json::{Map, Value as Json};

use super::catalog::{
    CardArgs, ChangesetIdArgs, ChangesetProposeArgs, CommandRunArgs, EmptyArgs, ExportArgs,
    FormulaSetArgs, RangeReadArgs, RangeWriteArgs, RecalcArgs, RenderArgs, SheetAddToolArgs,
    SheetRenameToolArgs, WorkbookOpenArgs, WorkbookSaveArgs,
};
use super::uri::{ResourceKind, card_uri, parse_resource_uri, sheet_uri};
use crate::error::codes;
use crate::registry::{CommandKind, Exposure};
use crate::resolve::MAX_RANGE_CELLS;
use crate::session::Bus;

/// Default `range_read` page size (rows).
pub const DEFAULT_PAGE_ROWS: u32 = 256;
/// Maximum `range_read` page size (rows).
pub const MAX_PAGE_ROWS: u32 = 1_024;
/// Maximum serialized MCP argument JSON (bytes).
pub const MAX_MCP_JSON_BYTES: usize = 1_048_576;
/// Maximum JSON nesting for MCP arguments.
pub const MAX_MCP_JSON_DEPTH: u32 = 32;
/// Maximum serialized cell rows returned by one `range_read` page.
pub const MAX_RANGE_READ_BYTES: usize = 1_048_576;

/// Called after an `ExternalAgent` proposal is stored.
pub type ProposeHook = Box<dyn Fn(&Changeset) + Send + Sync>;
/// Optional WP-22 workbook card (defaults to [`stub_card`]).
pub type CardHook = Box<dyn Fn(&Workbook, Option<&str>) -> Json + Send + Sync>;

/// Session-side MCP state (open path, render capability, notify hook).
#[derive(Default)]
pub struct McpCtx {
    /// Path of the workbook opened through `workbook_open`, if any.
    pub open_path: Option<String>,
    /// `true` when a GUI owns this bus (enables `render`).
    pub gui_running: bool,
    /// Called after an `ExternalAgent` proposal is stored.
    pub on_external_propose: Option<ProposeHook>,
    /// Workbook card builder. `None` uses [`stub_card`].
    pub card: Option<CardHook>,
}

impl std::fmt::Debug for McpCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpCtx")
            .field("open_path", &self.open_path)
            .field("gui_running", &self.gui_running)
            .field(
                "on_external_propose",
                &self.on_external_propose.as_ref().map(|_| "set"),
            )
            .field("card", &self.card.as_ref().map(|_| "set"))
            .finish()
    }
}

/// MCP dispatch. Stateless aside from [`McpCtx`].
pub struct McpSession;

impl McpSession {
    /// Invoke a named tool. Unknown names and invalid arguments are errors.
    pub fn call(
        bus: &mut Bus,
        ctx: &mut McpCtx,
        tool: &str,
        args: Json,
    ) -> Result<Json, CoreError> {
        check_args_budget(&args)?;
        match tool {
            "workbook_open" => workbook_open(bus, ctx, parse_args(args)?),
            "workbook_list" => workbook_list(ctx, parse_args::<EmptyArgs>(args)?),
            "workbook_save" => workbook_save(bus, ctx, parse_args(args)?),
            "sheet_list" => sheet_list(bus, parse_args::<EmptyArgs>(args)?),
            "sheet_add" => sheet_add(bus, ctx, parse_args(args)?),
            "sheet_rename" => sheet_rename(bus, ctx, parse_args(args)?),
            "range_read" => range_read(bus, parse_args(args)?),
            "range_write" => range_write(bus, ctx, parse_args(args)?),
            "formula_set" => formula_set(bus, ctx, parse_args(args)?),
            "command_run" => command_run(bus, ctx, parse_args(args)?),
            "commands_list" => commands_list(bus, parse_args::<EmptyArgs>(args)?),
            "recalc" => recalc(bus, parse_args(args)?),
            "audit" => audit(bus, parse_args::<EmptyArgs>(args)?),
            "card" => {
                let _ = parse_args::<CardArgs>(args)?;
                let path = ctx.open_path.clone();
                Ok(card_payload_for(bus.workbook(), ctx, path.as_deref()))
            }
            "changeset_propose" => changeset_propose(bus, ctx, parse_args(args)?),
            "changeset_apply" => changeset_apply(bus, parse_args(args)?),
            "changeset_revert" => changeset_revert(bus, parse_args(args)?),
            "changeset_list" => changeset_list(bus, parse_args::<EmptyArgs>(args)?),
            "export" => export(bus, parse_args(args)?),
            "render" => render(ctx, parse_args(args)?),
            other => Err(
                CoreError::new(codes::MCP_UNKNOWN, format!("unknown MCP tool {other:?}"))
                    .with_hint("call tools/list for the frozen catalog"),
            ),
        }
    }

    /// `resources/list` for the open workbook.
    pub fn list_resources(bus: &Bus, ctx: &McpCtx) -> Vec<Json> {
        let Some(file) = ctx.open_path.as_deref() else {
            return Vec::new();
        };
        let mut out = vec![serde_json::json!({
            "uri": card_uri(file),
            "name": "card",
            "mimeType": "application/json",
        })];
        for sheet in bus.workbook().sheets() {
            out.push(serde_json::json!({
                "uri": sheet_uri(file, &sheet.name),
                "name": sheet.name,
                "mimeType": "application/json",
            }));
        }
        out
    }

    /// `resources/read`.
    pub fn read_resource(bus: &Bus, ctx: &McpCtx, uri: &str) -> Result<Json, CoreError> {
        let kind = parse_resource_uri(uri)?;
        match kind {
            ResourceKind::Card { file } => {
                require_file(ctx, &file)?;
                Ok(card_payload_for(bus.workbook(), ctx, Some(&file)))
            }
            ResourceKind::Sheet { file, sheet } => {
                require_file(ctx, &file)?;
                sheet_summary(bus.workbook(), &sheet)
            }
        }
    }
}

fn card_payload_for(wb: &Workbook, ctx: &McpCtx, path: Option<&str>) -> Json {
    match &ctx.card {
        Some(hook) => hook(wb, path),
        None => stub_card(wb, path),
    }
}

fn require_file(ctx: &McpCtx, file: &str) -> Result<(), CoreError> {
    match ctx.open_path.as_deref() {
        Some(open) if open == file => Ok(()),
        Some(_) => Err(CoreError::new(
            codes::MCP_URI,
            "resource file does not match the open workbook",
        )),
        None => Err(
            CoreError::new(codes::MCP_URI, "no workbook is open in this MCP session")
                .with_hint("call workbook_open first"),
        ),
    }
}

fn parse_args<T: serde::de::DeserializeOwned>(args: Json) -> Result<T, CoreError> {
    serde_json::from_value(args).map_err(|err| {
        CoreError::new(codes::MCP_ARGS, err.to_string())
            .with_hint("arguments must match the tool schema; unknown fields are rejected")
    })
}

fn check_args_budget(args: &Json) -> Result<(), CoreError> {
    let encoded = serde_json::to_vec(args).unwrap_or_default();
    if encoded.len() > MAX_MCP_JSON_BYTES {
        return Err(CoreError::new(
            codes::MCP_ARGS,
            format!(
                "MCP arguments exceed {MAX_MCP_JSON_BYTES} bytes (got {})",
                encoded.len()
            ),
        ));
    }
    if json_depth(args) > MAX_MCP_JSON_DEPTH {
        return Err(CoreError::new(
            codes::MCP_ARGS,
            format!("MCP arguments exceed nesting depth {MAX_MCP_JSON_DEPTH}"),
        ));
    }
    Ok(())
}

fn json_depth(value: &Json) -> u32 {
    match value {
        Json::Array(items) => 1 + items.iter().map(json_depth).max().unwrap_or(0),
        Json::Object(map) => 1 + map.values().map(json_depth).max().unwrap_or(0),
        _ => 1,
    }
}

fn outcome_json(outcome: omacell_core::command::Outcome) -> Result<Json, CoreError> {
    if outcome.ok {
        Ok(outcome.result.unwrap_or(Json::Null))
    } else {
        Err(outcome
            .error
            .unwrap_or_else(|| CoreError::new(codes::COMMAND_UNKNOWN, "command failed")))
    }
}

fn query_execute(bus: &mut Bus, id: &str, args: Json) -> Result<Json, CoreError> {
    outcome_json(bus.execute(Origin::ExternalAgent, id, args))
}

fn session_execute(bus: &mut Bus, id: &str, args: Json) -> Result<Json, CoreError> {
    outcome_json(bus.execute_mcp_session(id, args))
}

fn has_pending_proposal(bus: &Bus) -> bool {
    bus.list_changesets()
        .iter()
        .any(|changeset| changeset.status == ChangesetStatus::Proposed)
}

fn ensure_no_pending_proposals(bus: &Bus) -> Result<(), CoreError> {
    if has_pending_proposal(bus) {
        return Err(crate::error::denied(
            "cannot replace the workbook while a changeset is still proposed",
        )
        .with_hint(
            "apply or discard the proposal first (`omacell changeset apply|discard <id>`)",
        ));
    }
    Ok(())
}

fn deny_mcp_file_write(command: &str) -> CoreError {
    crate::error::denied(format!("MCP cannot execute {command}"))
        .with_hint("file writes stay with the user CLI/GUI; MCP retains ExternalAgent provenance")
}

fn write_calls(
    bus: &mut Bus,
    ctx: &McpCtx,
    apply: bool,
    calls: Vec<CommandCall>,
) -> Result<Json, CoreError> {
    if apply {
        return Err(CoreError::new(
            codes::COMMAND_DENIED,
            "external agents cannot apply mutations",
        )
        .with_hint("propose, then run omacell changeset apply <id> as the user"));
    }
    let changeset = bus.propose(Origin::ExternalAgent, calls)?;
    if let Some(notify) = ctx.on_external_propose.as_ref() {
        notify(&changeset);
    }
    serde_json::to_value(&changeset).map_err(|err| CoreError::new(codes::MCP_ARGS, err.to_string()))
}

fn workbook_open(
    bus: &mut Bus,
    ctx: &mut McpCtx,
    args: WorkbookOpenArgs,
) -> Result<Json, CoreError> {
    ensure_no_pending_proposals(bus)?;
    let result = session_execute(bus, "file.open", serde_json::json!({"path": args.path}))?;
    ctx.open_path = result
        .get("path")
        .and_then(Json::as_str)
        .map(str::to_string)
        .or(Some(args.path));
    Ok(result)
}

fn workbook_list(ctx: &McpCtx, _args: EmptyArgs) -> Result<Json, CoreError> {
    let files = ctx
        .open_path
        .as_ref()
        .map(|p| vec![p.clone()])
        .unwrap_or_default();
    Ok(serde_json::json!({ "files": files }))
}

fn workbook_save(
    _bus: &mut Bus,
    _ctx: &mut McpCtx,
    _args: WorkbookSaveArgs,
) -> Result<Json, CoreError> {
    Err(deny_mcp_file_write("file.save"))
}

fn sheet_list(bus: &Bus, _args: EmptyArgs) -> Result<Json, CoreError> {
    let names: Vec<String> = bus.workbook().sheets().map(|s| s.name.clone()).collect();
    Ok(serde_json::json!({ "sheets": names }))
}

fn sheet_add(bus: &mut Bus, ctx: &McpCtx, args: SheetAddToolArgs) -> Result<Json, CoreError> {
    let mut payload = Map::new();
    if let Some(name) = args.name {
        payload.insert("name".into(), Json::String(name));
    }
    let call = command_call("sheet.add", Json::Object(payload))?;
    write_calls(bus, ctx, args.apply, vec![call])
}

fn sheet_rename(bus: &mut Bus, ctx: &McpCtx, args: SheetRenameToolArgs) -> Result<Json, CoreError> {
    let call = command_call(
        "sheet.rename",
        serde_json::json!({"sheet": args.sheet, "name": args.name}),
    )?;
    write_calls(bus, ctx, args.apply, vec![call])
}

fn range_read(bus: &Bus, args: RangeReadArgs) -> Result<Json, CoreError> {
    let fields = match args.fields.as_ref() {
        None => vec!["values", "formulas", "formats"],
        Some(list) if list.is_empty() => vec!["values", "formulas", "formats"],
        Some(list) => {
            for field in list {
                if !matches!(field.as_str(), "values" | "formulas" | "formats") {
                    return Err(CoreError::new(
                        codes::MCP_ARGS,
                        format!("unknown range_read field {field:?}"),
                    )
                    .with_hint("fields are values, formulas, formats"));
                }
            }
            list.iter().map(String::as_str).collect()
        }
    };
    let limit = args.limit.unwrap_or(DEFAULT_PAGE_ROWS).min(MAX_PAGE_ROWS);
    if limit == 0 {
        return Err(CoreError::new(
            codes::MCP_ARGS,
            "range_read limit must be > 0",
        ));
    }
    let (sheet, min_row, min_col, max_row, max_col) = resolve_range(bus.workbook(), &args.range)?;
    let width = u64::from(max_col - min_col + 1);
    let height = u64::from(max_row - min_row + 1);
    if width.saturating_mul(height) > MAX_RANGE_CELLS {
        return Err(crate::error::range_size(width.saturating_mul(height)));
    }
    let start_row = min_row.saturating_add(args.offset);
    if start_row > max_row {
        return Ok(serde_json::json!({
            "range": args.range,
            "offset": args.offset,
            "limit": limit,
            "truncated": false,
            "rows": [],
        }));
    }
    let end_row = (start_row.saturating_add(limit) - 1).min(max_row);
    let want_values = fields.contains(&"values");
    let want_formulas = fields.contains(&"formulas");
    let want_formats = fields.contains(&"formats");
    let wb = bus.workbook();
    let mut rows = Vec::new();
    let mut rows_bytes = 2usize;
    let mut byte_limited = false;
    for row in start_row..=end_row {
        let mut cells = Vec::new();
        let mut row_bytes = 2usize;
        for col in min_col..=max_col {
            let slot = wb.get(sheet, row, col)?.cloned();
            let mut cell = Map::new();
            cell.insert("ref".into(), Json::String(a1_of(wb, sheet, row, col)));
            if want_values {
                cell.insert(
                    "value".into(),
                    Json::String(
                        slot.as_ref()
                            .map(|s| format_value(wb, &s.value))
                            .unwrap_or_default(),
                    ),
                );
            }
            if want_formulas {
                let formula = slot
                    .as_ref()
                    .and_then(|s| s.formula)
                    .and_then(|id| wb.intern().formulas.get(id).map(str::to_string))
                    .unwrap_or_default();
                cell.insert("formula".into(), Json::String(formula));
            }
            if want_formats {
                let fmt = slot
                    .as_ref()
                    .and_then(|s| wb.intern().styles.get(s.style))
                    .map(|style| {
                        wb.num_fmt_code(style.num_fmt)
                            .unwrap_or(std::borrow::Cow::Borrowed("General"))
                            .into_owned()
                    })
                    .unwrap_or_else(|| "General".into());
                cell.insert("format".into(), Json::String(fmt));
            }
            let cell = Json::Object(cell);
            let cell_bytes = serde_json::to_vec(&cell)
                .map_err(|err| CoreError::new(codes::MCP_ARGS, err.to_string()))?
                .len()
                .saturating_add(1);
            row_bytes = row_bytes.saturating_add(cell_bytes);
            if row_bytes > MAX_RANGE_READ_BYTES {
                return Err(CoreError::new(
                    codes::MCP_ARGS,
                    format!(
                        "one range_read row exceeds the {MAX_RANGE_READ_BYTES}-byte response budget"
                    ),
                )
                .with_hint("request a narrower column range"));
            }
            cells.push(cell);
        }
        if rows_bytes.saturating_add(row_bytes) > MAX_RANGE_READ_BYTES {
            if rows.is_empty() {
                return Err(CoreError::new(
                    codes::MCP_ARGS,
                    format!(
                        "one range_read row exceeds the {MAX_RANGE_READ_BYTES}-byte response budget"
                    ),
                )
                .with_hint("request a narrower column range"));
            }
            byte_limited = true;
            break;
        }
        rows_bytes = rows_bytes.saturating_add(row_bytes);
        rows.push(Json::Array(cells));
    }
    let returned_rows = u32::try_from(rows.len()).unwrap_or(u32::MAX);
    let truncated = byte_limited || start_row.saturating_add(returned_rows) <= max_row;
    Ok(serde_json::json!({
        "range": args.range,
        "offset": args.offset,
        "limit": limit,
        "truncated": truncated,
        "rows": rows,
    }))
}

fn range_write(bus: &mut Bus, ctx: &McpCtx, args: RangeWriteArgs) -> Result<Json, CoreError> {
    let call = command_call(
        "range.set",
        serde_json::json!({"range": args.range, "values": args.values}),
    )?;
    write_calls(bus, ctx, args.apply, vec![call])
}

fn formula_set(bus: &mut Bus, ctx: &McpCtx, args: FormulaSetArgs) -> Result<Json, CoreError> {
    let input = if args.formula.starts_with('=') {
        args.formula
    } else {
        format!("={}", args.formula)
    };
    let call = command_call(
        "cell.set",
        serde_json::json!({"ref": args.cell_ref, "input": input}),
    )?;
    write_calls(bus, ctx, args.apply, vec![call])
}

fn command_run(bus: &mut Bus, ctx: &McpCtx, args: CommandRunArgs) -> Result<Json, CoreError> {
    let spec = bus.registry().get_str(&args.id)?;
    if spec.exposure != Exposure::Public {
        return Err(crate::error::internal(&args.id));
    }
    let mutating = spec.kind == CommandKind::Mutating;
    let eligible = spec.changeset_eligible;
    if mutating && eligible {
        let call = command_call(&args.id, args.args)?;
        return write_calls(bus, ctx, args.apply, vec![call]);
    }
    if mutating {
        return Err(CoreError::new(
            codes::COMMAND_DENIED,
            "external agents cannot run mutations that are not changeset-eligible",
        )
        .with_hint("use a dedicated MCP tool or a changeset-eligible command"));
    }
    query_execute(bus, &args.id, args.args)
}

fn commands_list(bus: &Bus, _args: EmptyArgs) -> Result<Json, CoreError> {
    let text = bus
        .commands_json()
        .map_err(|err| CoreError::new(codes::MCP_ARGS, err.to_string()))?;
    serde_json::from_str(&text).map_err(|err| CoreError::new(codes::MCP_ARGS, err.to_string()))
}

fn recalc(bus: &mut Bus, args: RecalcArgs) -> Result<Json, CoreError> {
    if has_pending_proposal(bus) {
        return Err(crate::error::denied(
            "cannot recalculate while a changeset is still proposed",
        )
        .with_hint(
            "recalc runs on live state, not the proposal; apply or discard first (`omacell changeset apply|discard <id>`)",
        ));
    }
    let result = session_execute(bus, "calc.recalc", serde_json::json!({"mode": "rebuild"}))?;
    Ok(serde_json::json!({
        "recalc": result,
        "wait": args.wait,
        "settled": true,
    }))
}

fn audit(bus: &mut Bus, _args: EmptyArgs) -> Result<Json, CoreError> {
    query_execute(bus, "audit.run", serde_json::json!({}))
}

fn changeset_propose(
    bus: &mut Bus,
    ctx: &McpCtx,
    args: ChangesetProposeArgs,
) -> Result<Json, CoreError> {
    let mut calls = Vec::with_capacity(args.commands.len());
    for command in args.commands {
        calls.push(command_call(&command.id, command.args)?);
    }
    write_calls(bus, ctx, false, calls)
}

fn changeset_apply(bus: &mut Bus, args: ChangesetIdArgs) -> Result<Json, CoreError> {
    let id = ChangesetId::new(args.id)?;
    match bus.apply(Origin::ExternalAgent, &id) {
        Ok(cs) => {
            serde_json::to_value(cs).map_err(|err| CoreError::new(codes::MCP_ARGS, err.to_string()))
        }
        Err(err) => Err(err.with_hint("omacell changeset apply <id>")),
    }
}

fn changeset_revert(bus: &mut Bus, args: ChangesetIdArgs) -> Result<Json, CoreError> {
    let id = ChangesetId::new(args.id)?;
    match bus.revert(Origin::ExternalAgent, &id) {
        Ok(cs) => {
            serde_json::to_value(cs).map_err(|err| CoreError::new(codes::MCP_ARGS, err.to_string()))
        }
        Err(err) => Err(err.with_hint("omacell changeset revert <id>")),
    }
}

fn changeset_list(bus: &Bus, _args: EmptyArgs) -> Result<Json, CoreError> {
    serde_json::to_value(bus.list_changesets())
        .map_err(|err| CoreError::new(codes::MCP_ARGS, err.to_string()))
}

fn export(_bus: &mut Bus, _args: ExportArgs) -> Result<Json, CoreError> {
    Err(deny_mcp_file_write("file.export"))
}

fn render(ctx: &McpCtx, args: RenderArgs) -> Result<Json, CoreError> {
    let _ = args;
    if ctx.gui_running {
        return Err(
            CoreError::new(codes::MCP_RENDER, "render is not wired in this session")
                .with_hint("WP-21 headless MCP always reports GUI not running"),
        );
    }
    Err(CoreError::new(codes::MCP_RENDER, "GUI not running")
        .with_hint("start the Omacell GUI for vision render, or use range_read"))
}

fn command_call(id: &str, args: Json) -> Result<CommandCall, CoreError> {
    Ok(CommandCall {
        id: CommandId::new(id)?,
        args,
    })
}

fn resolve_range(
    wb: &Workbook,
    range: &str,
) -> Result<(omacell_core::addr::SheetId, u32, u16, u32, u16), CoreError> {
    let parsed = parse_a1(range)?;
    let kind = wb.resolve_parsed(parsed)?;
    match kind {
        RefKind::Cell(cell) => {
            let sheet = cell.sheet.unwrap_or_else(|| wb.active_sheet());
            Ok((sheet, cell.row, cell.col, cell.row, cell.col))
        }
        RefKind::Range(r) => {
            let sheet = r.start.sheet.unwrap_or_else(|| wb.active_sheet());
            Ok((
                sheet,
                r.start.row.min(r.end.row),
                r.start.col.min(r.end.col),
                r.start.row.max(r.end.row),
                r.start.col.max(r.end.col),
            ))
        }
    }
}

fn a1_of(wb: &Workbook, sheet: omacell_core::addr::SheetId, row: u32, col: u16) -> String {
    let letters = col_to_letters(col).unwrap_or_else(|_| "A".into());
    let name = wb.sheet(sheet).map(|s| s.name.as_str()).unwrap_or("Sheet1");
    format!("{}!{}{}", quote_sheet_name(name), letters, row + 1)
}

fn format_value(wb: &Workbook, value: &Value) -> String {
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

fn sheet_summary(wb: &Workbook, name: &str) -> Result<Json, CoreError> {
    let sheet = wb
        .sheet_by_name(name)
        .ok_or_else(|| CoreError::new(codes::MCP_URI, format!("unknown sheet {name}")))?;
    let used = sheet.used_range();
    let formulas = sheet
        .store
        .iter()
        .filter(|(_, _, slot)| slot.formula.is_some())
        .count();
    Ok(serde_json::json!({
        "name": sheet.name,
        "rows": used.map(|u| u.max_row.saturating_sub(u.min_row) + 1).unwrap_or(0),
        "cols": used.map(|u| u.max_col.saturating_sub(u.min_col) + 1).unwrap_or(0),
        "formulas": formulas,
    }))
}

/// Summary-level workbook card (WP-22 replaces this payload).
#[must_use]
pub fn stub_card(wb: &Workbook, path: Option<&str>) -> Json {
    let mut formula_count = 0u64;
    let mut sheets = Vec::new();
    for sheet in wb.sheets() {
        let used = sheet.used_range();
        let formulas = sheet
            .store
            .iter()
            .filter(|(_, _, slot)| slot.formula.is_some())
            .count() as u64;
        formula_count += formulas;
        sheets.push(serde_json::json!({
            "name": sheet.name,
            "rows": used.map(|u| u.max_row.saturating_sub(u.min_row) + 1).unwrap_or(0),
            "cols": used.map(|u| u.max_col.saturating_sub(u.min_col) + 1).unwrap_or(0),
            "formulas": formulas,
        }));
    }
    let mut names: Vec<String> = wb.names().iter().map(|n| n.name.clone()).collect();
    names.sort();
    let mut tables: Vec<String> = wb.tables().iter().map(|t| t.name.clone()).collect();
    tables.sort();
    serde_json::json!({
        "schema": 1,
        "kind": "summary",
        "file": path,
        "sheets": sheets,
        "names": names,
        "tables": tables,
        "formula_count": formula_count,
    })
}
