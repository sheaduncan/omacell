//! Parse `.omc` records into a workbook / changeset.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use omacell_core::addr::{RefKind, col_from_letters, parse_a1};
use omacell_core::changeset::{
    ChangeSummary, Changeset, ChangesetId, ChangesetStatus, CommandCall,
};
use omacell_core::command::{CommandId, Origin};
use omacell_core::error::{CoreError, ErrorKind};
use omacell_core::intern::{ArrayPayload, RichTextRun};
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::sheet::{
    Comment, FreezePanes, Hyperlink, Note, ProtectionState, SheetVisibility, SplitView, ViewState,
};
use omacell_core::storage::CellSlot;
use omacell_core::style::{Font, Style};
use omacell_core::tables::{Table, TableColumn, TableId};
use omacell_core::value::Value;
use omacell_core::workbook::{
    CalcMode, DateSystem, Workbook, WorkbookMeta, WorkbookProtectionState, WorkbookSettings,
};

use super::{MAX_OMC_LINE, MAX_OMC_RECORDS, OmcDocument};
use crate::error;
use crate::xlsx::WorksheetExtras;

use super::write::{PivotWire, WireValue, validate_custom_part_name};

const MAX_VALUE_DEPTH: u8 = 16;

#[derive(Clone, Debug)]
struct Field {
    value: String,
    quoted: bool,
}

pub(super) fn parse(text: &str) -> Result<OmcDocument, CoreError> {
    let mut saw_magic = false;
    let mut saw_book = false;
    let mut saw_changeset = false;
    let mut records = 0usize;
    let mut wb = Workbook::new();
    wb.undo_log_mut().set_enabled(false);
    let mut extras: HashMap<String, WorksheetExtras> = HashMap::new();
    let mut style_map: HashMap<u32, Style> = HashMap::new();
    let mut numfmt_map: HashMap<u32, omacell_core::style::NumFmtId> = HashMap::new();
    let mut pending_active: Option<String> = None;
    let mut pending_vis: BTreeMap<String, SheetVisibility> = BTreeMap::new();
    let mut pending_names: Vec<Vec<Field>> = Vec::new();
    let mut pending_pivots: Vec<PivotWire> = Vec::new();
    let mut pending_pivot_ids = BTreeSet::new();
    let mut sheet_records = BTreeSet::new();
    let mut page_setup_records = BTreeSet::new();
    let mut custom_names = BTreeSet::new();
    let mut changeset_id = None;
    let mut changeset_status = ChangesetStatus::Proposed;
    let mut changeset_origin = Origin::User;
    let mut summary = ChangeSummary::default();
    let mut forward = Vec::new();
    let mut inverse = Vec::new();

    for (i, raw) in text.split('\n').enumerate() {
        let line_no = i + 1;
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.len() > MAX_OMC_LINE {
            return Err(error::omc_limit(format!(
                "line {line_no} is {} bytes; maximum is {MAX_OMC_LINE}",
                line.len()
            )));
        }
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        records += 1;
        if records > MAX_OMC_RECORDS {
            return Err(error::omc_limit(format!(
                "more than {MAX_OMC_RECORDS} records"
            )));
        }
        let fields = split_fields_meta(line)
            .map_err(|e| error::omc_parse(format!("line {line_no}: {}", e.message)))?;
        if fields.is_empty() {
            continue;
        }
        if !saw_magic {
            if trimmed == "omc 1"
                || (fields.len() == 2 && fields[0].value == "omc" && fields[1].value == "1")
            {
                saw_magic = true;
                continue;
            }
            return Err(error::omc_format(format!(
                "line {line_no}: expected 'omc 1'"
            )));
        }
        match fields[0].value.as_str() {
            "book" => {
                if saw_book {
                    return Err(error::omc_parse(format!(
                        "line {line_no}: duplicate book record"
                    )));
                }
                saw_book = true;
                apply_book(&mut wb, &fields[1..], &mut pending_active)?;
            }
            "numfmt" => load_numfmt(&mut wb, &mut numfmt_map, &fields[1..])?,
            "style" => load_style(&mut style_map, &numfmt_map, &fields[1..])?,
            "name" => pending_names.push(fields[1..].to_vec()),
            "sheet" => load_sheet(
                &mut wb,
                &mut pending_vis,
                &mut sheet_records,
                &mut page_setup_records,
                &fields[1..],
            )?,
            "cell" => load_cell(&mut wb, &style_map, &fields[1..])?,
            "merge" => load_merge(&mut wb, &fields[1..])?,
            "comment" => load_comment(&mut wb, &fields[1..])?,
            "threaded_comment" => load_threaded_comment(&mut wb, &fields[1..])?,
            "hyperlink" => load_hyperlink(&mut wb, &fields[1..])?,
            "table" => load_table(&mut wb, &fields[1..])?,
            "pivot" => {
                if fields.len() != 2 {
                    return Err(error::omc_parse(format!(
                        "line {line_no}: pivot record needs one JSON field"
                    )));
                }
                if pending_pivots.len() >= usize::from(MAX_COLS) {
                    return Err(error::omc_limit(format!(
                        "more than {MAX_COLS} pivot records"
                    )));
                }
                let pivot: PivotWire = parse_json(&fields[1].value, "pivot")?;
                if !pending_pivot_ids.insert(pivot.table.id) {
                    return Err(error::omc_parse(format!(
                        "duplicate pivot id {}",
                        pivot.table.id.index()
                    )));
                }
                pending_pivots.push(pivot);
            }
            "extra" | "cf" | "validation" => load_extra(&mut wb, &mut extras, &fields)?,
            "custom" => load_custom(&mut wb, &mut custom_names, &fields[1..])?,
            "aicache" => load_aicache(&mut wb, &fields[1..])?,
            "changeset" => {
                if saw_changeset {
                    return Err(error::omc_parse(format!(
                        "line {line_no}: duplicate changeset record"
                    )));
                }
                saw_changeset = true;
                let kv = parse_kv(&fields[1..])?;
                if let Some(id) = kv.get("id") {
                    changeset_id = Some(ChangesetId::new(id.clone())?);
                }
                if let Some(s) = kv.get("status") {
                    changeset_status = parse_status(s)?;
                }
                if let Some(o) = kv.get("origin") {
                    changeset_origin = parse_origin(o)?;
                }
                if let Some(t) = kv.get("text") {
                    summary.text = t.clone();
                }
                summary.cells = parse_u64_kv(&kv, "cells")?;
                summary.rows = parse_u64_kv(&kv, "rows")?;
                summary.columns = parse_u64_kv(&kv, "columns")?;
                summary.sheets = parse_u64_kv(&kv, "sheets")?;
                summary.styles = parse_u64_kv(&kv, "styles")?;
            }
            "change" => load_change(
                &fields[1..],
                &mut forward,
                &mut inverse,
                &mut changeset_origin,
            )?,
            other => {
                return Err(error::omc_parse(format!(
                    "line {line_no}: unknown record {other}"
                )));
            }
        }
    }
    if !saw_magic {
        return Err(error::omc_format("missing 'omc 1' header"));
    }
    for fields in pending_names {
        load_name(&mut wb, &fields)?;
    }
    hydrate_legacy_print_setups(&mut wb, &extras, &page_setup_records)?;
    let sheet_order: Vec<_> = wb.sheets().map(|sheet| sheet.name.clone()).collect();
    for name in sheet_order {
        if let Some(vis) = pending_vis.get(&name) {
            let id = wb.resolve_sheet_name(&name)?;
            wb.set_visibility(id, *vis)?;
        }
    }
    if let Some(name) = pending_active {
        let id = wb.resolve_sheet_name(&name)?;
        wb.set_active_sheet(id)?;
    }
    for wire in pending_pivots {
        let mut pivot = wire.table;
        pivot.source_sheet = wb.resolve_sheet_name(&wire.source_sheet)?;
        pivot.dest_sheet = wb.resolve_sheet_name(&wire.dest_sheet)?;
        if pivot.dest_row >= MAX_ROWS
            || u32::from(pivot.dest_col) >= u32::from(MAX_COLS)
            || pivot.out_end_row >= MAX_ROWS
            || u32::from(pivot.out_end_col) >= u32::from(MAX_COLS)
        {
            return Err(error::omc_parse(format!(
                "pivot {:?} output is outside the worksheet grid",
                pivot.name
            )));
        }
        wb.restore_pivot(pivot)?;
    }
    validate_sheet_links(&wb)?;
    let changeset =
        if forward.is_empty() && inverse.is_empty() && changeset_id.is_none() && !saw_changeset {
            None
        } else {
            let id = match changeset_id {
                Some(id) => id,
                None => ChangesetId::new("cs-omc")?,
            };
            let cs = Changeset {
                id,
                origin: changeset_origin,
                status: changeset_status,
                forward,
                inverse,
                summary,
            };
            cs.validate()?;
            Some(cs)
        };
    Ok(OmcDocument {
        workbook: wb,
        extras,
        changeset,
    })
}

fn validate_sheet_links(wb: &Workbook) -> Result<(), CoreError> {
    for sheet in wb.sheets() {
        let selection = sheet.view.selection;
        for id in [
            selection.start.sheet,
            selection.end.sheet,
            selection.sheet_end,
        ]
        .into_iter()
        .flatten()
        {
            if wb.sheet(id).is_none() {
                return Err(error::omc_parse(format!(
                    "sheet {:?} view references unknown sheet id {}",
                    sheet.name,
                    id.index()
                )));
            }
        }
    }
    Ok(())
}

fn apply_book(
    wb: &mut Workbook,
    fields: &[Field],
    pending_active: &mut Option<String>,
) -> Result<(), CoreError> {
    let kv = parse_kv(fields)?;
    if let Some(settings) = kv.get("settings") {
        let parsed: WorkbookSettings = parse_json(settings, "book settings")?;
        validate_settings(&parsed)?;
        *wb.settings_mut() = parsed;
    }
    if let Some(meta) = kv.get("meta") {
        *wb.meta_mut() = parse_json::<WorkbookMeta>(meta, "book metadata")?;
    }
    if let Some(protection) = kv.get("protection") {
        wb.set_workbook_protection(parse_json::<WorkbookProtectionState>(
            protection,
            "workbook protection",
        )?)?;
    }
    if let Some(ds) = kv.get("date_system") {
        wb.settings_mut().date_system = match ds.as_str() {
            "1904" => DateSystem::Excel1904,
            "1900" => DateSystem::Excel1900,
            _ => return Err(error::omc_parse(format!("unknown date system {ds}"))),
        };
    }
    if let Some(c) = kv.get("calc") {
        wb.settings_mut().calc_mode = match c.as_str() {
            "manual" => CalcMode::Manual,
            "autoNoTable" | "automatic_except_tables" => CalcMode::AutomaticExceptTables,
            "automatic" => CalcMode::Automatic,
            _ => return Err(error::omc_parse(format!("unknown calculation mode {c}"))),
        };
    }
    if let Some(name) = kv.get("active") {
        *pending_active = Some(name.clone());
    }
    Ok(())
}

fn load_numfmt(
    wb: &mut Workbook,
    map: &mut HashMap<u32, omacell_core::style::NumFmtId>,
    fields: &[Field],
) -> Result<(), CoreError> {
    if fields.len() != 2 {
        return Err(error::omc_parse("numfmt record needs id and code"));
    }
    let file_id: u32 = fields[0]
        .value
        .parse()
        .map_err(|_| error::omc_parse("numfmt id is not an integer"))?;
    if file_id < 164 {
        return Err(error::omc_parse("custom numfmt id must be at least 164"));
    }
    let actual = wb.intern_num_fmt(&fields[1].value)?;
    if map.insert(file_id, actual).is_some() {
        return Err(error::omc_parse(format!("duplicate numfmt id {file_id}")));
    }
    Ok(())
}

fn load_style(
    map: &mut HashMap<u32, Style>,
    numfmts: &HashMap<u32, omacell_core::style::NumFmtId>,
    fields: &[Field],
) -> Result<(), CoreError> {
    if fields.len() < 2 {
        return Err(error::omc_parse("style record needs id and value"));
    }
    let id: u32 = fields[0]
        .value
        .parse()
        .map_err(|_| error::omc_parse("style id is not an integer"))?;
    if id == 0 {
        return Err(error::omc_parse("style id 0 is reserved for the default"));
    }
    let mut style = if fields.get(1).is_some_and(|s| s.value.starts_with('{')) {
        if fields.len() != 2 {
            return Err(error::omc_parse(
                "JSON style record needs exactly id and JSON",
            ));
        }
        parse_json(&fields[1].value, "style")?
    } else {
        compact_style(&fields[1..])?
    };
    if style.num_fmt.index() >= 164 {
        style.num_fmt = *numfmts.get(&style.num_fmt.index()).ok_or_else(|| {
            error::omc_parse(format!(
                "style {id} references unknown numfmt {}",
                style.num_fmt.index()
            ))
        })?;
    }
    if map.insert(id, style).is_some() {
        return Err(error::omc_parse(format!("duplicate style id {id}")));
    }
    Ok(())
}

fn compact_style(fields: &[Field]) -> Result<Style, CoreError> {
    let kv = parse_kv(fields)?;
    let mut style = Style::default();
    if let Some(font) = kv.get("font") {
        let mut f = Font::default();
        for part in font.split(';') {
            match part {
                "bold" => f.bold = true,
                "italic" => f.italic = true,
                other if !other.is_empty() => f.name = other.into(),
                _ => {}
            }
        }
        style.font = f;
    }
    if let Some(code) = kv.get("numfmt") {
        let _ = code;
        // compact sketch only; full codes go through JSON styles
    }
    Ok(style)
}

fn load_name(wb: &mut Workbook, fields: &[Field]) -> Result<(), CoreError> {
    if fields.len() < 2 {
        return Err(error::omc_parse("name record needs a name and referent"));
    }
    let kv = parse_kv(&fields[2..])?;
    let scope = match kv.get("scope") {
        Some(s) => NameScope::Sheet(wb.resolve_sheet_name(s)?),
        None => NameScope::Workbook,
    };
    if let Some(kind) = kv.get("type")
        && kind != "text"
        && kind != "formula"
    {
        return Err(error::omc_parse(format!(
            "unknown defined-name type {kind:?}"
        )));
    }
    let referent = if let Some(array) = kv.get("array") {
        if !fields[1].value.is_empty() || kv.contains_key("type") {
            return Err(error::omc_parse(
                "defined-name array cannot also have a literal or type",
            ));
        }
        NameReferent::Constant(wire_to_value(
            wb,
            parse_json(array, "defined-name array")?,
            0,
        )?)
    } else {
        parse_name_referent(
            wb,
            &fields[1].value,
            fields[1].quoted,
            kv.get("type").map(String::as_str),
        )?
    };
    wb.define_name(DefinedName {
        name: fields[0].value.clone(),
        scope,
        referent,
        comment: kv.get("comment").cloned(),
    })?;
    Ok(())
}

fn parse_name_referent(
    wb: &mut Workbook,
    raw: &str,
    quoted: bool,
    explicit_type: Option<&str>,
) -> Result<NameReferent, CoreError> {
    if explicit_type == Some("text") || (quoted && explicit_type != Some("formula")) {
        return Ok(NameReferent::Constant(parse_text(wb, raw, None)?));
    }
    if raw.starts_with('=') || explicit_type == Some("formula") {
        return Ok(NameReferent::Formula(raw.to_string()));
    }
    if let Ok(p) = parse_a1(raw) {
        let resolved = wb.resolve_parsed(p)?;
        return match resolved {
            RefKind::Range(r) => Ok(NameReferent::Range(r)),
            RefKind::Cell(c) => Ok(NameReferent::Range(
                omacell_core::addr::RangeRef::from_corners(c, c),
            )),
        };
    }
    Ok(NameReferent::Constant(parse_literal(wb, raw, false)?))
}

fn load_sheet(
    wb: &mut Workbook,
    pending_vis: &mut BTreeMap<String, SheetVisibility>,
    sheet_records: &mut BTreeSet<String>,
    page_setup_records: &mut BTreeSet<String>,
    fields: &[Field],
) -> Result<(), CoreError> {
    if fields.is_empty() {
        return Err(error::omc_parse("sheet record missing name"));
    }
    let name = &fields[0].value;
    let lower = name.to_lowercase();
    if !sheet_records.insert(lower.clone()) {
        return Err(error::omc_parse(format!(
            "duplicate sheet record for {name:?}"
        )));
    }
    let id = if sheet_records.len() == 1
        && wb.sheets().count() == 1
        && wb.sheets().next().is_some_and(|s| s.name == "Sheet1")
    {
        let id = wb.active_sheet();
        if name != "Sheet1" {
            wb.rename_sheet(id, name)?;
        }
        id
    } else if let Ok(id) = wb.resolve_sheet_name(name) {
        id
    } else {
        wb.add_sheet(name)?
    };
    let kv = parse_kv(&fields[1..])?;
    let very_hidden = parse_bool_kv(&kv, "veryHidden", false)?;
    let hidden = parse_bool_kv(&kv, "hidden", false)?;
    if very_hidden && hidden {
        return Err(error::omc_parse(
            "sheet cannot be both hidden and veryHidden",
        ));
    }
    if very_hidden {
        pending_vis.insert(name.clone(), SheetVisibility::VeryHidden);
    } else if hidden {
        pending_vis.insert(name.clone(), SheetVisibility::Hidden);
    }
    let mut view = if let Some(value) = kv.get("view") {
        parse_json::<ViewState>(value, "sheet view")?
    } else {
        wb.sheet(id).map(|s| s.view.clone()).unwrap_or_default()
    };
    if let Some(z) = kv.get("zoom") {
        let z = parse_f64(z, "sheet zoom")?;
        view.zoom = if z > 8.0 { z / 100.0 } else { z };
    }
    if let Some(f) = kv.get("freeze") {
        view.freeze = parse_freeze(f)?;
    }
    if let Some(s) = kv.get("split") {
        let (x, y) = s
            .split_once(',')
            .ok_or_else(|| error::omc_parse("split must be x,y"))?;
        view.split = Some(SplitView {
            x_px: parse_u32(x, "split x")?,
            y_px: parse_u32(y, "split y")?,
        });
    }
    validate_view(&view)?;
    wb.set_sheet_view(id, view)?;
    if let Some(value) = kv.get("protection") {
        let protection = parse_json::<ProtectionState>(value, "sheet protection")?;
        wb.set_sheet_protection(id, protection)?;
    } else if kv.contains_key("protect") {
        wb.set_sheet_protection(
            id,
            ProtectionState {
                enabled: parse_bool_kv(&kv, "protect", false)?,
                password: None,
                allow: Default::default(),
                protected_ranges: Vec::new(),
            },
        )?;
    }
    if let Some(value) = kv.get("tab_color") {
        let color = parse_json(value, "sheet tab color")?;
        wb.set_tab_color(id, color)?;
    }
    if let Some(value) = kv.get("page_setup") {
        let setup = parse_json(value, "sheet page setup")?;
        wb.set_page_setup(id, setup)
            .map_err(|err| error::omc_parse(err.message))?;
        page_setup_records.insert(lower);
    }
    if let Some(cols) = kv.get("cols") {
        for part in cols.split(',') {
            let (letters, w) = part
                .split_once(':')
                .ok_or_else(|| error::omc_parse("cols entries must be LETTERS:width"))?;
            let col = col_from_letters(letters)?;
            let width = parse_f64(w, "column width")?;
            if width <= 0.0 {
                return Err(error::omc_parse("column width must be positive"));
            }
            let px = (width * f64::from(omacell_core::geometry::DEFAULT_COL_PX) / 8.43)
                .round()
                .max(1.0) as u32;
            wb.set_col_width(id, col, px)?;
        }
    }
    for (index, px) in parse_pairs(&kv, "row_sizes")? {
        wb.set_row_height(id, index, px)?;
    }
    for row in parse_indices(&kv, "row_hidden")? {
        wb.set_row_hidden(id, row, true)?;
    }
    for (index, px) in parse_pairs(&kv, "col_sizes")? {
        let col =
            u16::try_from(index).map_err(|_| error::omc_parse("column index is out of range"))?;
        wb.set_col_width(id, col, px)?;
    }
    for index in parse_indices(&kv, "col_hidden")? {
        let col =
            u16::try_from(index).map_err(|_| error::omc_parse("column index is out of range"))?;
        wb.set_col_hidden(id, col, true)?;
    }
    Ok(())
}

fn hydrate_legacy_print_setups(
    wb: &mut Workbook,
    extras: &HashMap<String, WorksheetExtras>,
    page_setup_records: &BTreeSet<String>,
) -> Result<(), CoreError> {
    let sheets: Vec<_> = wb
        .sheets()
        .map(|sheet| (sheet.id, sheet.name.clone()))
        .collect();
    for (sheet_id, sheet_name) in sheets {
        if page_setup_records.contains(&sheet_name.to_lowercase()) {
            continue;
        }
        let mut setup = omacell_core::print::PageSetup::default();
        if let Some(extra) = extras.get(&sheet_name) {
            crate::xlsx::print::apply_print_xml(&mut setup, &extra.print_xml);
        }
        let print_names: Vec<_> = wb
            .names()
            .iter()
            .filter(|name| {
                matches!(name.scope, NameScope::Sheet(id) if id == sheet_id)
                    && crate::xlsx::print::is_print_name(&name.name)
            })
            .map(|name| {
                let referent = match &name.referent {
                    NameReferent::Range(range) => range.to_a1(),
                    NameReferent::Formula(formula) => formula.clone(),
                    NameReferent::Constant(_) => String::new(),
                };
                (name.name.clone(), referent)
            })
            .collect();
        for (name, referent) in print_names {
            crate::xlsx::print::apply_print_name(&mut setup, &name, &referent);
        }
        if !setup.is_default() {
            wb.set_page_setup(sheet_id, setup)
                .map_err(|err| error::omc_parse(err.message))?;
        }
    }
    Ok(())
}

fn parse_freeze(s: &str) -> Result<FreezePanes, CoreError> {
    if s.contains(',') {
        let (rows, cols) = s
            .split_once(',')
            .ok_or_else(|| error::omc_parse("freeze must be rows,cols"))?;
        return Ok(FreezePanes {
            rows: parse_u32(rows, "freeze rows")?,
            cols: cols
                .parse()
                .map_err(|_| error::omc_parse("freeze columns is not a u16"))?,
        });
    }
    let parsed = parse_a1(s)?;
    match parsed.kind {
        RefKind::Cell(c) => Ok(FreezePanes {
            rows: c.row,
            cols: c.col,
        }),
        RefKind::Range(_) => Err(error::omc_parse("freeze expected a cell")),
    }
}

fn load_cell(
    wb: &mut Workbook,
    styles: &HashMap<u32, Style>,
    fields: &[Field],
) -> Result<(), CoreError> {
    if fields.is_empty() {
        return Err(error::omc_parse("cell record missing address"));
    }
    let (id, row, col) = resolve_cell(wb, &fields[0].value)?;
    if wb.get(id, row, col)?.is_some() {
        return Err(error::omc_parse(format!(
            "duplicate cell record for {:?}",
            fields[0].value
        )));
    }
    let literal = fields
        .get(1)
        .map(|field| field.value.as_str())
        .unwrap_or("");
    let quoted = fields.get(1).is_some_and(|field| field.quoted);
    let kv = parse_kv(&fields[2..])?;
    let mut slot = CellSlot::empty();
    let is_formula = match kv.get("type").map(String::as_str) {
        Some("formula") => true,
        Some("text") => false,
        Some(kind) => {
            return Err(error::omc_parse(format!(
                "unknown cell value type {kind:?}"
            )));
        }
        None => !quoted && literal.starts_with('='),
    };
    let cache_count = ["v", "v_text", "v_array"]
        .into_iter()
        .filter(|key| kv.contains_key(*key))
        .count();
    if cache_count > 1 {
        return Err(error::omc_parse(
            "formula cell has more than one cached value",
        ));
    }
    if kv.contains_key("v_rich") && !kv.contains_key("v_text") {
        return Err(error::omc_parse("v_rich requires v_text"));
    }
    if is_formula {
        if kv.contains_key("array") || kv.contains_key("rich") {
            return Err(error::omc_parse(
                "formula cell cannot also have a direct array or rich value",
            ));
        }
        let src = if literal.starts_with('=') {
            literal.to_string()
        } else {
            format!("={literal}")
        };
        let fid = wb.intern_formula(&src)?;
        slot.formula = Some(fid);
        let mut release_value = true;
        if let Some(value) = kv.get("v_text") {
            let rich = parse_optional_rich(&kv, "v_rich")?;
            (slot.value, release_value) = cell_text_value(wb, id, row, col, value, rich)?;
        } else if let Some(value) = kv.get("v_array") {
            slot.value = wire_to_value(wb, parse_json(value, "cached array")?, 0)?;
        } else if let Some(v) = kv.get("v") {
            slot.value = parse_literal(wb, v, false)?;
        }
        wb.set_slot(id, row, col, slot)?;
        wb.release_formula(fid);
        if release_value {
            release_direct_value(wb, slot.value);
        }
    } else if let Some(value) = kv.get("array") {
        if !literal.is_empty()
            || kv.contains_key("rich")
            || kv.contains_key("type")
            || kv.contains_key("v")
            || kv.contains_key("v_text")
            || kv.contains_key("v_array")
            || kv.contains_key("v_rich")
        {
            return Err(error::omc_parse(
                "array cell cannot also have a literal, cache, or rich value",
            ));
        }
        slot.value = wire_to_value(wb, parse_json(value, "cell array")?, 0)?;
        wb.set_slot(id, row, col, slot)?;
        release_direct_value(wb, slot.value);
    } else {
        if cache_count > 0 || kv.contains_key("v_rich") {
            return Err(error::omc_parse(
                "cached values are only valid on formula cells",
            ));
        }
        let explicit_text = kv.get("type").is_some_and(|value| value == "text");
        if kv.contains_key("rich") && !quoted && !explicit_text {
            return Err(error::omc_parse("rich runs require a text cell"));
        }
        let mut release_value = true;
        if quoted || explicit_text {
            (slot.value, release_value) =
                cell_text_value(wb, id, row, col, literal, parse_optional_rich(&kv, "rich")?)?;
        } else {
            slot.value = parse_literal(wb, literal, false)?;
        }
        wb.set_slot(id, row, col, slot)?;
        if release_value {
            release_direct_value(wb, slot.value);
        }
    }
    if let Some(raw) = kv.get("s") {
        let s = raw
            .parse::<u32>()
            .map_err(|_| error::omc_parse("cell style id is not an integer"))?;
        let style = styles
            .get(&s)
            .ok_or_else(|| error::omc_parse(format!("unknown cell style id {s}")))?;
        wb.set_cell_style(id, row, col, style.clone())?;
    }
    Ok(())
}

fn parse_literal(wb: &mut Workbook, raw: &str, quoted: bool) -> Result<Value, CoreError> {
    if quoted {
        return parse_text(wb, raw, None);
    }
    if raw.is_empty() {
        return Ok(Value::Empty);
    }
    if raw == "TRUE" {
        return Ok(Value::Bool(true));
    }
    if raw == "FALSE" {
        return Ok(Value::Bool(false));
    }
    if let Some(kind) = ErrorKind::from_display(raw) {
        return Ok(Value::Error(kind));
    }
    if let Ok(n) = raw.parse::<f64>()
        && n.is_finite()
    {
        return Ok(Value::Number(n));
    }
    let tid = wb.intern_text(raw);
    Ok(Value::Text(tid))
}

fn resolve_cell(
    wb: &Workbook,
    addr: &str,
) -> Result<(omacell_core::addr::SheetId, u32, u16), CoreError> {
    let parsed = parse_a1(addr).map_err(|e| error::omc_parse(e.to_string()))?;
    let id = match parsed.sheet {
        Some(spec) if spec.end.is_none() => wb.resolve_sheet_name(&spec.start)?,
        Some(_) => return Err(error::omc_parse("cell address cannot span sheets")),
        None => wb.active_sheet(),
    };
    match parsed.kind {
        RefKind::Cell(c) => Ok((id, c.row, c.col)),
        RefKind::Range(_) => Err(error::omc_parse(format!(
            "{addr} is a range, expected a cell"
        ))),
    }
}

fn load_merge(wb: &mut Workbook, fields: &[Field]) -> Result<(), CoreError> {
    if fields.len() != 1 {
        return Err(error::omc_parse("merge record missing range"));
    }
    let parsed = parse_a1(&fields[0].value).map_err(|e| error::omc_parse(e.to_string()))?;
    let id = match parsed.sheet {
        Some(spec) if spec.end.is_none() => wb.resolve_sheet_name(&spec.start)?,
        Some(_) => return Err(error::omc_parse("merge cannot span sheets")),
        None => wb.active_sheet(),
    };
    let rg = match parsed.kind {
        RefKind::Range(r) => r,
        RefKind::Cell(c) => omacell_core::addr::RangeRef::from_corners(c, c),
    };
    let mut merges = wb.sheet(id).map(|s| s.merges.clone()).unwrap_or_default();
    if merges.iter().any(|existing| ranges_overlap(*existing, rg)) {
        return Err(error::omc_parse("merge overlaps an existing merge"));
    }
    merges.push(rg);
    wb.set_sheet_merges(id, merges)?;
    Ok(())
}

fn ranges_overlap(a: omacell_core::addr::RangeRef, b: omacell_core::addr::RangeRef) -> bool {
    let (a_r0, a_r1) = (a.start.row.min(a.end.row), a.start.row.max(a.end.row));
    let (a_c0, a_c1) = (a.start.col.min(a.end.col), a.start.col.max(a.end.col));
    let (b_r0, b_r1) = (b.start.row.min(b.end.row), b.start.row.max(b.end.row));
    let (b_c0, b_c1) = (b.start.col.min(b.end.col), b.start.col.max(b.end.col));
    a_r0 <= b_r1 && b_r0 <= a_r1 && a_c0 <= b_c1 && b_c0 <= a_c1
}

fn load_comment(wb: &mut Workbook, fields: &[Field]) -> Result<(), CoreError> {
    if fields.is_empty() {
        return Err(error::omc_parse("comment record missing address"));
    }
    let (id, row, col) = resolve_cell(wb, &fields[0].value)?;
    if wb
        .sheet(id)
        .is_some_and(|sheet| sheet.notes.contains_key(&(row, col)))
    {
        return Err(error::omc_parse("duplicate comment record"));
    }
    let (metadata, body) = if fields.len() >= 3 {
        (
            &fields[1..fields.len() - 1],
            fields.last().map(|f| f.value.clone()).unwrap_or_default(),
        )
    } else if fields.len() == 2 && !fields[1].value.starts_with("author=") {
        (&fields[1..1], fields[1].value.clone())
    } else {
        (&fields[1..], String::new())
    };
    let kv = parse_kv(metadata)?;
    let author = kv.get("author").cloned();
    wb.set_note(id, row, col, Some(Note { author, text: body }))?;
    Ok(())
}

fn load_threaded_comment(wb: &mut Workbook, fields: &[Field]) -> Result<(), CoreError> {
    if fields.len() != 2 {
        return Err(error::omc_parse(
            "threaded_comment record needs address and JSON",
        ));
    }
    let (id, row, col) = resolve_cell(wb, &fields[0].value)?;
    if wb
        .sheet(id)
        .is_some_and(|sheet| sheet.comments.contains_key(&(row, col)))
    {
        return Err(error::omc_parse("duplicate threaded_comment record"));
    }
    let comment: Comment = parse_json(&fields[1].value, "threaded comment")?;
    wb.set_comment(id, row, col, Some(comment))?;
    Ok(())
}

fn load_hyperlink(wb: &mut Workbook, fields: &[Field]) -> Result<(), CoreError> {
    if fields.len() < 2 {
        return Err(error::omc_parse(
            "hyperlink record needs address and target",
        ));
    }
    let (id, row, col) = resolve_cell(wb, &fields[0].value)?;
    if wb
        .sheet(id)
        .is_some_and(|sheet| sheet.hyperlinks.contains_key(&(row, col)))
    {
        return Err(error::omc_parse("duplicate hyperlink record"));
    }
    let kv = parse_kv(&fields[2..])?;
    wb.set_hyperlink(
        id,
        row,
        col,
        Some(Hyperlink {
            target: fields[1].value.clone(),
            tooltip: kv.get("tooltip").cloned(),
            display: kv.get("display").cloned(),
        }),
    )?;
    Ok(())
}

fn load_table(wb: &mut Workbook, fields: &[Field]) -> Result<(), CoreError> {
    if fields.len() < 2 {
        return Err(error::omc_parse("table record needs name and range"));
    }
    let parsed = parse_a1(&fields[1].value).map_err(|e| error::omc_parse(e.to_string()))?;
    let id = match parsed.sheet {
        Some(spec) if spec.end.is_none() => wb.resolve_sheet_name(&spec.start)?,
        Some(_) => return Err(error::omc_parse("table cannot span sheets")),
        None => wb.active_sheet(),
    };
    let rg = match parsed.kind {
        RefKind::Range(r) => r,
        RefKind::Cell(c) => omacell_core::addr::RangeRef::from_corners(c, c),
    };
    if rg.start.row > rg.end.row || rg.start.col > rg.end.col {
        return Err(error::omc_parse(
            "table range must run from top-left to bottom-right",
        ));
    }
    let kv = parse_kv(&fields[2..])?;
    let mut table = Table::new(
        TableId::new(0),
        fields[0].value.clone(),
        id,
        rg.start.row,
        rg.start.col,
        rg.end.row,
        rg.end.col,
    );
    table.has_header = parse_bool_kv(&kv, "header", table.has_header)?;
    table.has_totals = parse_bool_kv(&kv, "totals", table.has_totals)?;
    table.banded_rows = parse_bool_kv(&kv, "banded_rows", table.banded_rows)?;
    table.banded_cols = parse_bool_kv(&kv, "banded_cols", table.banded_cols)?;
    table.auto_expand = parse_bool_kv(&kv, "auto_expand", table.auto_expand)?;
    if let Some(columns) = kv.get("columns") {
        table.columns = parse_json::<Vec<TableColumn>>(columns, "table columns")?;
    } else if let Some(cols) = kv.get("cols") {
        table.columns = cols
            .split(',')
            .map(|n| TableColumn {
                name: n.to_string(),
                totals_fn: None,
            })
            .collect();
    }
    let expected_columns = usize::from(rg.end.col - rg.start.col) + 1;
    if table.columns.len() != expected_columns {
        return Err(error::omc_parse(format!(
            "table has {} columns but its range has {expected_columns}",
            table.columns.len()
        )));
    }
    wb.add_table(table)?;
    Ok(())
}

fn load_extra(
    wb: &mut Workbook,
    extras: &mut HashMap<String, WorksheetExtras>,
    fields: &[Field],
) -> Result<(), CoreError> {
    match fields[0].value.as_str() {
        "extra" => {
            if fields.len() != 4 {
                return Err(error::omc_parse("extra record needs sheet, kind, payload"));
            }
            let sheet = &fields[1].value;
            let sheet_id = wb.resolve_sheet_name(sheet)?;
            let kind = &fields[2].value;
            let payload = &fields[3].value;
            match kind.as_str() {
                "autofilter_model" => {
                    let filter = serde_json::from_str(payload)
                        .map_err(|e| error::omc_parse(format!("invalid autofilter model: {e}")))?;
                    wb.set_autofilter(sheet_id, Some(filter))?;
                }
                "validation_model" => {
                    let rules = serde_json::from_str(payload)
                        .map_err(|e| error::omc_parse(format!("invalid validation model: {e}")))?;
                    wb.set_validations(sheet_id, rules)?;
                }
                "condfmt_model" => {
                    let rules = serde_json::from_str(payload)
                        .map_err(|e| error::omc_parse(format!("invalid condfmt model: {e}")))?;
                    wb.set_cond_formats(sheet_id, rules)?;
                }
                "autofilter" => {
                    let extra = extras.entry(sheet.clone()).or_default();
                    extra.autofilter = Some(
                        String::from_utf8(decode_blob(payload))
                            .map_err(|_| error::omc_parse("autofilter is not UTF-8"))?,
                    );
                }
                "autofilter_xml" => {
                    extras.entry(sheet.clone()).or_default().autofilter_xml = decode_blob(payload);
                }
                "cf" => extras
                    .entry(sheet.clone())
                    .or_default()
                    .conditional_formatting_xml
                    .push(decode_blob(payload)),
                "dv" => extras
                    .entry(sheet.clone())
                    .or_default()
                    .data_validations_xml
                    .push(decode_blob(payload)),
                "print" => extras
                    .entry(sheet.clone())
                    .or_default()
                    .print_xml
                    .push(decode_blob(payload)),
                "sparkline" => extras
                    .entry(sheet.clone())
                    .or_default()
                    .sparkline_xml
                    .push(decode_blob(payload)),
                _ => return Err(error::omc_parse(format!("unknown extra kind {kind}"))),
            }
        }
        "cf" if fields.len() >= 2 => {
            let sheet = sheet_of(wb, &fields[1].value)?;
            let extra = extras.entry(sheet).or_default();
            extra
                .conditional_formatting_xml
                .push(join_values(&fields[1..]).into_bytes());
        }
        "validation" if fields.len() >= 2 => {
            let sheet = sheet_of(wb, &fields[1].value)?;
            let extra = extras.entry(sheet).or_default();
            extra
                .data_validations_xml
                .push(join_values(&fields[1..]).into_bytes());
        }
        _ => return Err(error::omc_parse("malformed extra record")),
    }
    Ok(())
}

fn sheet_of(wb: &Workbook, addr: &str) -> Result<String, CoreError> {
    let parsed = parse_a1(addr).map_err(|e| error::omc_parse(e.to_string()))?;
    let name = parsed
        .sheet
        .map(|spec| {
            if spec.end.is_some() {
                Err(error::omc_parse("extra range cannot span sheets"))
            } else {
                Ok(spec.start)
            }
        })
        .transpose()?
        .or_else(|| wb.sheet(wb.active_sheet()).map(|sheet| sheet.name.clone()))
        .ok_or_else(|| error::omc_parse("extra record has no sheet"))?;
    wb.resolve_sheet_name(&name)?;
    Ok(name)
}

fn decode_blob(s: &str) -> Vec<u8> {
    if let Ok(v) = serde_json::from_str::<String>(s) {
        v.into_bytes()
    } else {
        s.as_bytes().to_vec()
    }
}

fn load_aicache(wb: &mut Workbook, fields: &[Field]) -> Result<(), CoreError> {
    if fields.len() != 1 {
        return Err(error::omc_parse("aicache record needs a JSON payload"));
    }
    wb.custom_parts.insert(
        "xl/omacell/aicache.json".into(),
        fields[0].value.as_bytes().to_vec(),
    );
    Ok(())
}

fn load_custom(
    wb: &mut Workbook,
    names: &mut BTreeSet<String>,
    fields: &[Field],
) -> Result<(), CoreError> {
    if fields.len() != 2 {
        return Err(error::omc_parse("custom record needs name and payload"));
    }
    validate_custom_part_name(&fields[0].value)?;
    if !names.insert(fields[0].value.to_ascii_lowercase()) {
        return Err(error::omc_parse(format!(
            "duplicate custom part {:?}",
            fields[0].value
        )));
    }
    wb.custom_parts
        .insert(fields[0].value.clone(), fields[1].value.as_bytes().to_vec());
    Ok(())
}

fn load_change(
    fields: &[Field],
    forward: &mut Vec<CommandCall>,
    inverse: &mut Vec<CommandCall>,
    origin: &mut Origin,
) -> Result<(), CoreError> {
    if fields.len() != 3 {
        return Err(error::omc_parse(
            "change record needs exactly direction/cmd/json",
        ));
    }
    let (dir, cmd, json) = if fields[0].value == "forward" || fields[0].value == "inverse" {
        (
            fields[0].value.as_str(),
            fields[1].value.as_str(),
            fields[2].value.as_str(),
        )
    } else {
        *origin = parse_origin(&fields[0].value)?;
        (
            "forward",
            fields[1].value.as_str(),
            fields[2].value.as_str(),
        )
    };
    let call = CommandCall {
        id: CommandId::new(cmd)?,
        args: serde_json::from_str(json).map_err(|e| error::omc_parse(e.to_string()))?,
    };
    if dir == "inverse" {
        inverse.push(call);
    } else {
        forward.push(call);
    }
    Ok(())
}

fn parse_status(s: &str) -> Result<ChangesetStatus, CoreError> {
    match s {
        "proposed" => Ok(ChangesetStatus::Proposed),
        "applied" => Ok(ChangesetStatus::Applied),
        "reverted" => Ok(ChangesetStatus::Reverted),
        _ => Err(error::omc_parse(format!("unknown changeset status {s}"))),
    }
}

fn parse_origin(s: &str) -> Result<Origin, CoreError> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
        .map_err(|_| error::omc_parse(format!("unknown origin {s}")))
}

fn parse_kv(fields: &[Field]) -> Result<BTreeMap<String, String>, CoreError> {
    let mut m = BTreeMap::new();
    for f in fields {
        let (key, value) = if let Some((k, v)) = f.value.split_once('=') {
            (k.to_string(), v.to_string())
        } else if !f.value.is_empty() {
            (f.value.clone(), "1".into())
        } else {
            continue;
        };
        if key.is_empty() {
            return Err(error::omc_parse("empty metadata key"));
        }
        if m.insert(key.clone(), value).is_some() {
            return Err(error::omc_parse(format!("duplicate metadata key {key:?}")));
        }
    }
    Ok(m)
}

fn split_fields_meta(line: &str) -> Result<Vec<Field>, CoreError> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut start = 0usize;
    while start <= bytes.len() {
        if start == bytes.len() {
            if line.ends_with('\t') {
                out.push(Field {
                    value: String::new(),
                    quoted: false,
                });
            }
            break;
        }
        if bytes[start] == b'"' {
            let mut value = String::new();
            let mut index = start + 1;
            let mut segment = index;
            let end = loop {
                if index >= bytes.len() {
                    return Err(error::omc_parse("unterminated quoted field"));
                }
                match bytes[index] {
                    b'"' => {
                        value.push_str(&line[segment..index]);
                        break index + 1;
                    }
                    b'\\' => {
                        value.push_str(&line[segment..index]);
                        index += 1;
                        if index >= bytes.len() {
                            return Err(error::omc_parse("dangling escape"));
                        }
                        value.push(match bytes[index] {
                            b'\\' => '\\',
                            b'"' => '"',
                            b'n' => '\n',
                            b't' => '\t',
                            b'r' => '\r',
                            other => {
                                return Err(error::omc_parse(format!(
                                    "unknown escape \\{}",
                                    char::from(other)
                                )));
                            }
                        });
                        index += 1;
                        segment = index;
                    }
                    _ => index += 1,
                }
            };
            if end < bytes.len() && bytes[end] != b'\t' {
                return Err(error::omc_parse(
                    "quoted field must end at a tab or end of line",
                ));
            }
            out.push(Field {
                value,
                quoted: true,
            });
            start = end.saturating_add(1);
        } else {
            let end = bytes[start..]
                .iter()
                .position(|byte| *byte == b'\t')
                .map_or(bytes.len(), |offset| start + offset);
            let value = &line[start..end];
            if value.contains('"') {
                return Err(error::omc_parse("quote in raw field"));
            }
            out.push(Field {
                value: value.to_string(),
                quoted: false,
            });
            start = end.saturating_add(1);
        }
    }
    Ok(out)
}

fn parse_json<T: serde::de::DeserializeOwned>(raw: &str, what: &str) -> Result<T, CoreError> {
    serde_json::from_str(raw).map_err(|e| error::omc_parse(format!("invalid {what} JSON: {e}")))
}

fn parse_u32(raw: &str, what: &str) -> Result<u32, CoreError> {
    raw.parse()
        .map_err(|_| error::omc_parse(format!("{what} is not a u32")))
}

fn parse_f64(raw: &str, what: &str) -> Result<f64, CoreError> {
    let value: f64 = raw
        .parse()
        .map_err(|_| error::omc_parse(format!("{what} is not a number")))?;
    if !value.is_finite() {
        return Err(error::omc_parse(format!("{what} must be finite")));
    }
    Ok(value)
}

fn parse_u64_kv(values: &BTreeMap<String, String>, key: &str) -> Result<u64, CoreError> {
    values.get(key).map_or(Ok(0), |value| {
        value
            .parse()
            .map_err(|_| error::omc_parse(format!("changeset {key} is not a u64")))
    })
}

fn parse_bool_kv(
    values: &BTreeMap<String, String>,
    key: &str,
    default: bool,
) -> Result<bool, CoreError> {
    match values.get(key).map(String::as_str) {
        None => Ok(default),
        Some("0" | "false") => Ok(false),
        Some("1" | "true") => Ok(true),
        Some(value) => Err(error::omc_parse(format!(
            "{key} must be 0/1, not {value:?}"
        ))),
    }
}

fn validate_settings(settings: &WorkbookSettings) -> Result<(), CoreError> {
    if !settings.iteration.max_change.is_finite() || settings.iteration.max_change < 0.0 {
        return Err(error::omc_parse(
            "iteration max_change must be finite and non-negative",
        ));
    }
    Ok(())
}

fn validate_view(view: &ViewState) -> Result<(), CoreError> {
    if !view.zoom.is_finite() || view.zoom <= 0.0 || view.zoom > 8.0 {
        return Err(error::omc_parse("sheet zoom must be finite and in (0, 8]"));
    }
    view.selection.start.validate()?;
    view.selection.end.validate()?;
    if view.freeze.rows > MAX_ROWS || view.freeze.cols > MAX_COLS {
        return Err(error::omc_parse("freeze panes are out of range"));
    }
    if view.scroll_row >= MAX_ROWS || view.scroll_col >= MAX_COLS {
        return Err(error::omc_parse("sheet scroll position is out of range"));
    }
    Ok(())
}

fn parse_pairs(values: &BTreeMap<String, String>, key: &str) -> Result<Vec<(u32, u32)>, CoreError> {
    values
        .get(key)
        .map_or(Ok(Vec::new()), |raw| parse_json(raw, key))
}

fn parse_indices(values: &BTreeMap<String, String>, key: &str) -> Result<Vec<u32>, CoreError> {
    values
        .get(key)
        .map_or(Ok(Vec::new()), |raw| parse_json(raw, key))
}

fn parse_optional_rich(
    values: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<Vec<RichTextRun>>, CoreError> {
    values
        .get(key)
        .map(|raw| parse_json(raw, "rich text runs"))
        .transpose()
}

fn parse_text(
    wb: &mut Workbook,
    text: &str,
    rich: Option<Vec<RichTextRun>>,
) -> Result<Value, CoreError> {
    if rich.is_some_and(|runs| !runs.is_empty()) {
        return Err(error::omc_parse("rich text is only valid on a cell value"));
    }
    let id = wb.intern_text(text);
    Ok(Value::Text(id))
}

fn cell_text_value(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    row: u32,
    col: u16,
    text: &str,
    rich: Option<Vec<RichTextRun>>,
) -> Result<(Value, bool), CoreError> {
    if let Some(runs) = rich.filter(|runs| !runs.is_empty()) {
        validate_rich_runs(text, &runs)?;
        let id = wb.set_rich_text(sheet, row, col, text, runs)?;
        Ok((Value::Text(id), false))
    } else {
        Ok((parse_text(wb, text, None)?, true))
    }
}

fn validate_rich_runs(text: &str, runs: &[RichTextRun]) -> Result<(), CoreError> {
    for run in runs {
        let start = usize::try_from(run.start)
            .map_err(|_| error::omc_parse("rich-text start is out of range"))?;
        let len = usize::try_from(run.len)
            .map_err(|_| error::omc_parse("rich-text length is out of range"))?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| error::omc_parse("rich-text range overflow"))?;
        if end > text.len() || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            return Err(error::omc_parse(
                "rich-text run is outside the text or splits UTF-8",
            ));
        }
    }
    Ok(())
}

fn wire_to_value(wb: &mut Workbook, value: WireValue, depth: u8) -> Result<Value, CoreError> {
    if depth >= MAX_VALUE_DEPTH {
        return Err(error::omc_limit("array nesting exceeds 16 levels"));
    }
    Ok(match value {
        WireValue::Empty => Value::Empty,
        WireValue::Number { value } if value.is_finite() => Value::Number(value),
        WireValue::Number { .. } => {
            return Err(error::omc_parse("array number must be finite"));
        }
        WireValue::Bool { value } => Value::Bool(value),
        WireValue::Text { value, rich } if rich.is_empty() => parse_text(wb, &value, None)?,
        WireValue::Text { .. } => {
            return Err(error::omc_parse(
                "rich text is not supported inside an array",
            ));
        }
        WireValue::Error { value } => Value::Error(value),
        WireValue::Array { rows, cols, values } => {
            let shape = omacell_core::value::Array2D::new(rows, cols)?;
            if values.len() != shape.len() as usize {
                return Err(error::omc_parse(
                    "array value count does not match rows * columns",
                ));
            }
            let values = values
                .into_iter()
                .map(|value| wire_to_value(wb, value, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            let id = wb.intern_array(ArrayPayload::new(shape, values)?);
            Value::Array(id)
        }
    })
}

fn release_direct_value(wb: &mut Workbook, value: Value) {
    match value {
        Value::Text(id) => wb.release_text(id),
        Value::Array(id) => wb.release_array(id),
        Value::Empty | Value::Number(_) | Value::Bool(_) | Value::Error(_) => {}
    }
}

fn join_values(fields: &[Field]) -> String {
    fields
        .iter()
        .map(|field| field.value.as_str())
        .collect::<Vec<_>>()
        .join("\t")
}
