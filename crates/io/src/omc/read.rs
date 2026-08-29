//! Parse `.omc` records into a workbook / changeset.

use std::collections::HashMap;

use omacell_core::addr::{ParsedRef, RefKind, col_from_letters, parse_a1};
use omacell_core::changeset::{
    ChangeSummary, Changeset, ChangesetId, ChangesetStatus, CommandCall,
};
use omacell_core::command::{CommandId, Origin};
use omacell_core::error::{CoreError, ErrorKind};
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::sheet::{
    FreezePanes, Hyperlink, Note, ProtectionState, SheetVisibility, SplitView,
};
use omacell_core::storage::CellSlot;
use omacell_core::style::{Font, Style};
use omacell_core::tables::{Table, TableColumn, TableId};
use omacell_core::value::Value;
use omacell_core::workbook::{CalcMode, DateSystem, Workbook};

use super::{MAX_OMC_LINE, MAX_OMC_RECORDS, OmcDocument};
use crate::error;
use crate::xlsx::WorksheetExtras;

pub(super) fn parse(text: &str) -> Result<OmcDocument, CoreError> {
    let mut saw_magic = false;
    let mut records = 0usize;
    let mut wb = Workbook::new();
    wb.undo_log_mut().set_enabled(false);
    let mut extras: HashMap<String, WorksheetExtras> = HashMap::new();
    let mut style_map: HashMap<u32, Style> = HashMap::new();
    let mut numfmt_map: HashMap<u32, omacell_core::style::NumFmtId> = HashMap::new();
    let mut pending_active: Option<String> = None;
    let mut pending_vis: HashMap<String, SheetVisibility> = HashMap::new();
    let mut changeset_id = None;
    let mut changeset_status = ChangesetStatus::Proposed;
    let mut changeset_origin = Origin::User;
    let mut summary = ChangeSummary::default();
    let mut forward = Vec::new();
    let mut inverse = Vec::new();
    let mut dropped_aicache = false;

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
        let fields = split_fields(line)?;
        if fields.is_empty() {
            continue;
        }
        if !saw_magic {
            if trimmed == "omc 1" || (fields.len() == 2 && fields[0] == "omc" && fields[1] == "1") {
                saw_magic = true;
                continue;
            }
            return Err(error::omc_format(format!(
                "line {line_no}: expected 'omc 1'"
            )));
        }
        match fields[0].as_str() {
            "book" => apply_book(&mut wb, &fields[1..], &mut pending_active)?,
            "numfmt" => load_numfmt(&mut wb, &mut numfmt_map, &fields[1..])?,
            "style" => load_style(&mut style_map, &numfmt_map, &fields[1..])?,
            "name" => load_name(&mut wb, &fields[1..])?,
            "sheet" => load_sheet(&mut wb, &mut pending_vis, &fields[1..])?,
            "cell" => load_cell(&mut wb, &style_map, &fields[1..])?,
            "merge" => load_merge(&mut wb, &fields[1..])?,
            "comment" => load_comment(&mut wb, &fields[1..])?,
            "hyperlink" => load_hyperlink(&mut wb, &fields[1..])?,
            "table" => load_table(&mut wb, &fields[1..])?,
            "extra" | "cf" | "validation" => load_extra(&mut extras, &fields)?,
            "custom" => load_custom(&mut wb, &fields[1..])?,
            "aicache" => dropped_aicache = true,
            "changeset" => {
                let kv = parse_kv(&fields[1..]);
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
    let _ = dropped_aicache;
    for (name, vis) in pending_vis {
        let id = wb.resolve_sheet_name(&name)?;
        wb.set_visibility(id, vis)?;
    }
    if let Some(name) = pending_active
        && let Ok(id) = wb.resolve_sheet_name(&name)
    {
        wb.set_active_sheet(id)?;
    }
    let changeset = if forward.is_empty() && inverse.is_empty() && changeset_id.is_none() {
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

fn apply_book(
    wb: &mut Workbook,
    fields: &[String],
    pending_active: &mut Option<String>,
) -> Result<(), CoreError> {
    let kv = parse_kv(fields);
    if let Some(ds) = kv.get("date_system") {
        wb.settings_mut().date_system = match ds.as_str() {
            "1904" => DateSystem::Excel1904,
            _ => DateSystem::Excel1900,
        };
    }
    if let Some(c) = kv.get("calc") {
        wb.settings_mut().calc_mode = match c.as_str() {
            "manual" => CalcMode::Manual,
            "autoNoTable" | "automatic_except_tables" => CalcMode::AutomaticExceptTables,
            _ => CalcMode::Automatic,
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
    fields: &[String],
) -> Result<(), CoreError> {
    if fields.len() < 2 {
        return Err(error::omc_parse("numfmt record needs id and code"));
    }
    let file_id: u32 = fields[0]
        .parse()
        .map_err(|_| error::omc_parse("numfmt id is not an integer"))?;
    let actual = wb.intern_num_fmt(&fields[1])?;
    map.insert(file_id, actual);
    Ok(())
}

fn load_style(
    map: &mut HashMap<u32, Style>,
    numfmts: &HashMap<u32, omacell_core::style::NumFmtId>,
    fields: &[String],
) -> Result<(), CoreError> {
    if fields.is_empty() {
        return Err(error::omc_parse("style record missing id"));
    }
    let id: u32 = fields[0]
        .parse()
        .map_err(|_| error::omc_parse("style id is not an integer"))?;
    let mut style = if fields.get(1).is_some_and(|s| s.starts_with('{')) {
        serde_json::from_str(&fields[1]).map_err(|e| error::omc_parse(e.to_string()))?
    } else {
        compact_style(&fields[1..])
    };
    if let Some(mapped) = numfmts.get(&style.num_fmt.index()) {
        style.num_fmt = *mapped;
    }
    map.insert(id, style);
    Ok(())
}

fn compact_style(fields: &[String]) -> Style {
    let kv = parse_kv(fields);
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
    style
}

fn load_name(wb: &mut Workbook, fields: &[String]) -> Result<(), CoreError> {
    if fields.len() < 2 {
        return Err(error::omc_parse("name record needs a name and referent"));
    }
    let kv = parse_kv(&fields[2..]);
    let scope = match kv.get("scope") {
        Some(s) => NameScope::Sheet(wb.resolve_sheet_name(s)?),
        None => NameScope::Workbook,
    };
    let referent = parse_name_referent(wb, &fields[1])?;
    wb.define_name(DefinedName {
        name: fields[0].clone(),
        scope,
        referent,
        comment: kv.get("comment").cloned(),
    })?;
    Ok(())
}

fn parse_name_referent(wb: &mut Workbook, raw: &str) -> Result<NameReferent, CoreError> {
    if let Some(f) = raw.strip_prefix('=') {
        if parse_a1(f).is_ok() {
            if let Ok(ParsedRef {
                kind: RefKind::Range(r),
                ..
            }) = parse_a1(f)
            {
                return Ok(NameReferent::Range(r));
            }
            if let Ok(ParsedRef {
                kind: RefKind::Cell(c),
                ..
            }) = parse_a1(f)
            {
                return Ok(NameReferent::Range(
                    omacell_core::addr::RangeRef::from_corners(c, c),
                ));
            }
        }
        return Ok(NameReferent::Formula(raw.to_string()));
    }
    if let Ok(p) = parse_a1(raw) {
        return match p.kind {
            RefKind::Range(r) => Ok(NameReferent::Range(r)),
            RefKind::Cell(c) => Ok(NameReferent::Range(
                omacell_core::addr::RangeRef::from_corners(c, c),
            )),
        };
    }
    match parse_literal(wb, raw)? {
        Value::Empty => Ok(NameReferent::Formula(raw.to_string())),
        v => Ok(NameReferent::Constant(v)),
    }
}

fn load_sheet(
    wb: &mut Workbook,
    pending_vis: &mut HashMap<String, SheetVisibility>,
    fields: &[String],
) -> Result<(), CoreError> {
    if fields.is_empty() {
        return Err(error::omc_parse("sheet record missing name"));
    }
    let name = &fields[0];
    let id = if wb.sheets().count() == 1 && wb.sheets().next().is_some_and(|s| s.name == "Sheet1") {
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
    let kv = parse_kv(&fields[1..]);
    if kv.contains_key("veryHidden") {
        pending_vis.insert(name.clone(), SheetVisibility::VeryHidden);
    } else if kv.contains_key("hidden") {
        pending_vis.insert(name.clone(), SheetVisibility::Hidden);
    }
    let mut view = wb.sheet(id).map(|s| s.view.clone()).unwrap_or_default();
    if let Some(z) = kv.get("zoom").and_then(|s| s.parse::<f64>().ok()) {
        view.zoom = z / if z > 8.0 { 100.0 } else { 1.0 };
        if view.zoom > 8.0 {
            view.zoom = z / 100.0;
        }
    }
    if let Some(f) = kv.get("freeze") {
        view.freeze = parse_freeze(f)?;
    }
    if let Some(s) = kv.get("split") {
        let mut xy = s.split(',');
        view.split = Some(SplitView {
            x_px: xy.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            y_px: xy.next().and_then(|v| v.parse().ok()).unwrap_or(0),
        });
    }
    wb.set_sheet_view(id, view)?;
    if kv.contains_key("protect") {
        wb.set_sheet_protection(
            id,
            ProtectionState {
                enabled: true,
                password: None,
            },
        )?;
    }
    if let Some(cols) = kv.get("cols") {
        for part in cols.split(',') {
            if let Some((letters, w)) = part.split_once(':')
                && let Ok(col) = col_from_letters(letters)
                && let Ok(width) = w.parse::<f64>()
            {
                let px = (width * f64::from(omacell_core::geometry::DEFAULT_COL_PX) / 8.43)
                    .round()
                    .max(1.0) as u32;
                wb.set_col_width(id, col, px)?;
            }
        }
    }
    Ok(())
}

fn parse_freeze(s: &str) -> Result<FreezePanes, CoreError> {
    if s.contains(',') {
        let mut p = s.split(',');
        return Ok(FreezePanes {
            rows: p.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            cols: p.next().and_then(|v| v.parse().ok()).unwrap_or(0),
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
    fields: &[String],
) -> Result<(), CoreError> {
    if fields.is_empty() {
        return Err(error::omc_parse("cell record missing address"));
    }
    let (id, row, col) = resolve_cell(wb, &fields[0])?;
    let literal = fields.get(1).map(String::as_str).unwrap_or("");
    let kv = parse_kv(&fields[2..]);
    let mut slot = CellSlot::empty();
    if let Some(body) = literal.strip_prefix('=') {
        let src = if body.starts_with('=') {
            literal.to_string()
        } else {
            format!("={body}")
        };
        let fid = wb.intern_formula(&src)?;
        slot.formula = Some(fid);
        if let Some(v) = kv.get("v") {
            slot.value = parse_literal(wb, v)?;
        }
        wb.set_slot(id, row, col, slot)?;
        wb.release_formula(fid);
    } else {
        slot.value = parse_literal(wb, literal)?;
        if let Value::Text(tid) = slot.value {
            wb.set_slot(id, row, col, slot)?;
            wb.release_text(tid);
        } else {
            wb.set_slot(id, row, col, slot)?;
        }
    }
    if let Some(s) = kv.get("s").and_then(|s| s.parse::<u32>().ok())
        && let Some(style) = styles.get(&s)
    {
        wb.set_cell_style(id, row, col, style.clone())?;
    }
    Ok(())
}

fn parse_literal(wb: &mut Workbook, raw: &str) -> Result<Value, CoreError> {
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
        Some(spec) => wb.resolve_sheet_name(&spec.start)?,
        None => wb.active_sheet(),
    };
    match parsed.kind {
        RefKind::Cell(c) => Ok((id, c.row, c.col)),
        RefKind::Range(_) => Err(error::omc_parse(format!(
            "{addr} is a range, expected a cell"
        ))),
    }
}

fn load_merge(wb: &mut Workbook, fields: &[String]) -> Result<(), CoreError> {
    if fields.is_empty() {
        return Err(error::omc_parse("merge record missing range"));
    }
    let parsed = parse_a1(&fields[0]).map_err(|e| error::omc_parse(e.to_string()))?;
    let id = match parsed.sheet {
        Some(spec) => wb.resolve_sheet_name(&spec.start)?,
        None => wb.active_sheet(),
    };
    let rg = match parsed.kind {
        RefKind::Range(r) => r,
        RefKind::Cell(c) => omacell_core::addr::RangeRef::from_corners(c, c),
    };
    let mut merges = wb.sheet(id).map(|s| s.merges.clone()).unwrap_or_default();
    merges.push(rg);
    wb.set_sheet_merges(id, merges)?;
    Ok(())
}

fn load_comment(wb: &mut Workbook, fields: &[String]) -> Result<(), CoreError> {
    if fields.is_empty() {
        return Err(error::omc_parse("comment record missing address"));
    }
    let (id, row, col) = resolve_cell(wb, &fields[0])?;
    let kv = parse_kv(&fields[1..fields.len().saturating_sub(1).max(1)]);
    let text = fields.last().cloned().unwrap_or_default();
    let author = kv.get("author").cloned().or_else(|| {
        fields
            .get(1)
            .and_then(|f| f.strip_prefix("author="))
            .map(str::to_string)
    });
    let body = if fields.len() > 2 {
        fields[fields.len() - 1].clone()
    } else if !text.starts_with("author=") {
        text
    } else {
        String::new()
    };
    wb.set_note(id, row, col, Some(Note { author, text: body }))?;
    Ok(())
}

fn load_hyperlink(wb: &mut Workbook, fields: &[String]) -> Result<(), CoreError> {
    if fields.len() < 2 {
        return Err(error::omc_parse(
            "hyperlink record needs address and target",
        ));
    }
    let (id, row, col) = resolve_cell(wb, &fields[0])?;
    let kv = parse_kv(&fields[2..]);
    wb.set_hyperlink(
        id,
        row,
        col,
        Some(Hyperlink {
            target: fields[1].clone(),
            tooltip: kv.get("tooltip").cloned(),
            display: kv.get("display").cloned(),
        }),
    )?;
    Ok(())
}

fn load_table(wb: &mut Workbook, fields: &[String]) -> Result<(), CoreError> {
    if fields.len() < 2 {
        return Err(error::omc_parse("table record needs name and range"));
    }
    let parsed = parse_a1(&fields[1]).map_err(|e| error::omc_parse(e.to_string()))?;
    let id = match parsed.sheet {
        Some(spec) => wb.resolve_sheet_name(&spec.start)?,
        None => wb.active_sheet(),
    };
    let rg = match parsed.kind {
        RefKind::Range(r) => r,
        RefKind::Cell(c) => omacell_core::addr::RangeRef::from_corners(c, c),
    };
    let kv = parse_kv(&fields[2..]);
    let mut table = Table::new(
        TableId::new(0),
        fields[0].clone(),
        id,
        rg.start.row,
        rg.start.col,
        rg.end.row,
        rg.end.col,
    );
    table.has_header = kv.get("header").is_none_or(|s| s != "0");
    table.has_totals = kv.get("totals").is_some_and(|s| s == "1");
    if let Some(cols) = kv.get("cols") {
        table.columns = cols
            .split(',')
            .map(|n| TableColumn {
                name: n.to_string(),
            })
            .collect();
    }
    wb.add_table(table)?;
    Ok(())
}

fn load_extra(
    extras: &mut HashMap<String, WorksheetExtras>,
    fields: &[String],
) -> Result<(), CoreError> {
    match fields[0].as_str() {
        "extra" => {
            if fields.len() < 4 {
                return Err(error::omc_parse("extra record needs sheet, kind, payload"));
            }
            let sheet = &fields[1];
            let kind = &fields[2];
            let payload = &fields[3];
            let extra = extras.entry(sheet.clone()).or_default();
            match kind.as_str() {
                "autofilter" => extra.autofilter = Some(payload.clone()),
                "cf" => extra.conditional_formatting_xml.push(decode_blob(payload)),
                "dv" => extra.data_validations_xml.push(decode_blob(payload)),
                "print" => extra.print_xml.push(decode_blob(payload)),
                "sparkline" => extra.sparkline_xml.push(decode_blob(payload)),
                _ => {}
            }
        }
        "cf" if fields.len() >= 2 => {
            let extra = extras.entry(sheet_of(&fields[1])).or_default();
            extra
                .conditional_formatting_xml
                .push(fields[1..].join("\t").into_bytes());
        }
        "validation" if fields.len() >= 2 => {
            let extra = extras.entry(sheet_of(&fields[1])).or_default();
            extra
                .data_validations_xml
                .push(fields[1..].join("\t").into_bytes());
        }
        _ => {}
    }
    Ok(())
}

fn sheet_of(addr: &str) -> String {
    addr.split('!')
        .next()
        .unwrap_or("Sheet1")
        .trim_matches('\'')
        .to_string()
}

fn decode_blob(s: &str) -> Vec<u8> {
    if let Ok(v) = serde_json::from_str::<String>(s) {
        v.into_bytes()
    } else {
        s.as_bytes().to_vec()
    }
}

fn load_custom(wb: &mut Workbook, fields: &[String]) -> Result<(), CoreError> {
    if fields.len() < 2 {
        return Err(error::omc_parse("custom record needs name and payload"));
    }
    wb.custom_parts
        .insert(fields[0].clone(), fields[1].as_bytes().to_vec());
    Ok(())
}

fn load_change(
    fields: &[String],
    forward: &mut Vec<CommandCall>,
    inverse: &mut Vec<CommandCall>,
    origin: &mut Origin,
) -> Result<(), CoreError> {
    if fields.len() < 3 {
        return Err(error::omc_parse("change record needs direction/cmd/json"));
    }
    let (dir, cmd, json) = if fields[0] == "forward" || fields[0] == "inverse" {
        (fields[0].as_str(), fields[1].as_str(), fields[2].as_str())
    } else {
        *origin = parse_origin(&fields[0])?;
        ("forward", fields[1].as_str(), fields[2].as_str())
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

fn parse_kv(fields: &[String]) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for f in fields {
        if let Some((k, v)) = f.split_once('=') {
            m.insert(k.to_string(), v.to_string());
        } else if !f.is_empty() {
            m.insert(f.clone(), "1".into());
        }
    }
    m
}

pub(super) fn split_fields(line: &str) -> Result<Vec<String>, CoreError> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while chars.peek().is_some() {
        match chars.peek() {
            Some('\t') => {
                chars.next();
                if out.is_empty() {
                    out.push(String::new());
                }
                if chars.peek().is_none() {
                    out.push(String::new());
                }
            }
            Some('"') => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next() {
                        None => return Err(error::omc_parse("unterminated quoted field")),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('r') => s.push('\r'),
                            Some(c) => s.push(c),
                            None => return Err(error::omc_parse("dangling escape")),
                        },
                        Some(c) => s.push(c),
                    }
                }
                out.push(s);
            }
            Some(_) => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '\t' {
                        break;
                    }
                    s.push(c);
                    chars.next();
                }
                out.push(s);
            }
            None => break,
        }
    }
    Ok(out)
}
