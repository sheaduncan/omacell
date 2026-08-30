//! Range sort with Excel type ordering (F-6.1).

use serde::{Deserialize, Serialize};

use crate::addr::{RangeRef, SheetId};
use crate::error::CoreError;
use crate::formula::{RewriteOp, rewrite_print};
use crate::storage::CellSlot;
use crate::style::{Color, Fill};
use crate::value::Value;
use crate::workbook::Workbook;

/// What a sort key compares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    /// Cell values (Excel type order).
    #[default]
    Value,
    /// Fill colour ARGB.
    FillColor,
    /// Font colour ARGB.
    FontColor,
}

/// One sort key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortKey {
    /// 0-based column (or row, when left-to-right) relative to the range origin.
    pub offset: u16,
    /// Descending.
    #[serde(default)]
    pub descending: bool,
    /// Comparison source.
    #[serde(default)]
    pub by: SortBy,
    /// Custom-list rank (case-insensitive). Empty = none.
    #[serde(default)]
    pub custom_list: Vec<String>,
}

/// Sort options.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortSpec {
    /// Keys in priority order.
    #[serde(default)]
    pub keys: Vec<SortKey>,
    /// First row (or column) is a header and stays put.
    #[serde(default)]
    pub header: bool,
    /// Case-sensitive text.
    #[serde(default)]
    pub case_sensitive: bool,
    /// Sort left-to-right (columns as records).
    #[serde(default)]
    pub left_to_right: bool,
}

/// Sort `range` on `sheet`. Hidden rows/cols are left in place.
pub fn sort_range(
    wb: &mut Workbook,
    sheet: SheetId,
    range: RangeRef,
    spec: &SortSpec,
) -> Result<u32, CoreError> {
    let spec = spec.clone();
    wb.transact_try(move |wb| {
        let (r0, c0, r1, c1) = norm(range);
        if spec.left_to_right {
            sort_columns(wb, sheet, r0, c0, r1, c1, &spec)
        } else {
            sort_rows(wb, sheet, r0, c0, r1, c1, &spec)
        }
    })
}

fn sort_rows(
    wb: &mut Workbook,
    sheet: SheetId,
    r0: u32,
    c0: u16,
    r1: u32,
    c1: u16,
    spec: &SortSpec,
) -> Result<u32, CoreError> {
    let mut rows: Vec<u32> = (r0..=r1)
        .filter(|&r| {
            !wb.sheet(sheet)
                .is_some_and(|s| s.geometry.rows.is_hidden(r).unwrap_or(false))
        })
        .collect();
    if spec.header && !rows.is_empty() {
        rows.remove(0);
    }
    if rows.len() < 2 {
        return Ok(0);
    }
    let mut decorated: Vec<(u32, usize, Vec<CellSlot>)> = Vec::new();
    for (ord, &r) in rows.iter().enumerate() {
        let mut slots = Vec::new();
        for c in c0..=c1 {
            slots.push(wb.get(sheet, r, c)?.copied().unwrap_or(empty_slot()));
        }
        decorated.push((r, ord, slots));
    }
    decorated.sort_by(|a, b| {
        let ord = cmp_records(wb, &a.2, &b.2, spec);
        if ord == std::cmp::Ordering::Equal {
            a.1.cmp(&b.1)
        } else {
            ord
        }
    });
    let mut moved = 0u32;
    for (dest_row, (src_row, _, slots)) in rows.iter().zip(decorated) {
        let drow = *dest_row as i32 - src_row as i32;
        if drow != 0 {
            moved += 1;
        }
        for (i, slot) in slots.into_iter().enumerate() {
            let col = c0 + i as u16;
            write_moved(wb, sheet, *dest_row, col, slot, drow, 0)?;
        }
    }
    Ok(moved)
}

fn sort_columns(
    wb: &mut Workbook,
    sheet: SheetId,
    r0: u32,
    c0: u16,
    r1: u32,
    c1: u16,
    spec: &SortSpec,
) -> Result<u32, CoreError> {
    let mut cols: Vec<u16> = (c0..=c1)
        .filter(|&c| {
            !wb.sheet(sheet)
                .is_some_and(|s| s.geometry.cols.is_hidden(u32::from(c)).unwrap_or(false))
        })
        .collect();
    if spec.header && !cols.is_empty() {
        cols.remove(0);
    }
    if cols.len() < 2 {
        return Ok(0);
    }
    let mut decorated: Vec<(u16, usize, Vec<CellSlot>)> = Vec::new();
    for (ord, &c) in cols.iter().enumerate() {
        let mut slots = Vec::new();
        for r in r0..=r1 {
            slots.push(wb.get(sheet, r, c)?.copied().unwrap_or(empty_slot()));
        }
        decorated.push((c, ord, slots));
    }
    decorated.sort_by(|a, b| {
        let ord = cmp_records(wb, &a.2, &b.2, spec);
        if ord == std::cmp::Ordering::Equal {
            a.1.cmp(&b.1)
        } else {
            ord
        }
    });
    let mut moved = 0u32;
    for (dest_col, (src_col, _, slots)) in cols.iter().zip(decorated) {
        let dcol = i32::from(*dest_col) - i32::from(src_col);
        if dcol != 0 {
            moved += 1;
        }
        for (i, slot) in slots.into_iter().enumerate() {
            let row = r0 + i as u32;
            write_moved(wb, sheet, row, *dest_col, slot, 0, dcol)?;
        }
    }
    Ok(moved)
}

fn write_moved(
    wb: &mut Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    mut slot: CellSlot,
    drow: i32,
    dcol: i32,
) -> Result<(), CoreError> {
    if let Some(fid) = slot.formula
        && (drow != 0 || dcol != 0)
    {
        let src = wb.intern().formulas.get(fid).unwrap_or("").to_string();
        if let Ok(rewritten) = rewrite_print(&src, &RewriteOp::Copy { dcol, drow }) {
            let new_id = wb.intern_formula(&rewritten)?;
            slot.formula = Some(new_id);
            wb.write_slot(sheet, row, col, Some(slot))?;
            wb.release_formula(new_id);
            return Ok(());
        }
    }
    if slot.formula.is_none()
        && matches!(slot.value, Value::Empty)
        && slot.style == crate::style::StyleId::DEFAULT
    {
        wb.write_slot(sheet, row, col, None)?;
        return Ok(());
    }
    wb.write_slot(sheet, row, col, Some(slot))?;
    Ok(())
}

fn cmp_records(
    wb: &Workbook,
    a: &[CellSlot],
    b: &[CellSlot],
    spec: &SortSpec,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    for key in &spec.keys {
        let idx = usize::from(key.offset);
        let oa = a.get(idx);
        let ob = b.get(idx);
        let ord = match key.by {
            SortBy::Value => cmp_value(wb, oa, ob, spec.case_sensitive, &key.custom_list),
            SortBy::FillColor => cmp_u32(fill_argb(wb, oa), fill_argb(wb, ob)),
            SortBy::FontColor => cmp_u32(font_argb(wb, oa), font_argb(wb, ob)),
        };
        let ord = if key.descending { ord.reverse() } else { ord };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

fn cmp_u32(a: u32, b: u32) -> std::cmp::Ordering {
    a.cmp(&b)
}

fn fill_argb(wb: &Workbook, slot: Option<&CellSlot>) -> u32 {
    let Some(slot) = slot else {
        return 0;
    };
    match wb.intern().styles.get(slot.style).map(|s| &s.fill) {
        Some(Fill::Solid {
            fg: Color::Rgb { argb },
        }) => *argb,
        Some(Fill::Pattern {
            fg: Color::Rgb { argb },
            ..
        }) => *argb,
        _ => 0,
    }
}

fn font_argb(wb: &Workbook, slot: Option<&CellSlot>) -> u32 {
    let Some(slot) = slot else {
        return 0;
    };
    match wb.intern().styles.get(slot.style).map(|s| s.font.color) {
        Some(Color::Rgb { argb }) => argb,
        _ => 0,
    }
}

/// Excel type rank: number, text, logical, error, blank last.
fn type_rank(v: &Value) -> u8 {
    match v {
        Value::Number(_) => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) => 3,
        Value::Empty | Value::Array(_) => 4,
    }
}

fn cmp_value(
    wb: &Workbook,
    a: Option<&CellSlot>,
    b: Option<&CellSlot>,
    case: bool,
    list: &[String],
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let va = a.map(|s| s.value).unwrap_or(Value::Empty);
    let vb = b.map(|s| s.value).unwrap_or(Value::Empty);
    let ra = type_rank(&va);
    let rb = type_rank(&vb);
    if ra != rb {
        return ra.cmp(&rb);
    }
    if !list.is_empty() {
        let sa = display(wb, &va);
        let sb = display(wb, &vb);
        let ia = list_rank(list, &sa);
        let ib = list_rank(list, &sb);
        if ia != ib {
            return ia.cmp(&ib);
        }
    }
    match (va, vb) {
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(&y),
        (Value::Text(i), Value::Text(j)) => {
            let sa = wb.intern().strings.get(i).unwrap_or("");
            let sb = wb.intern().strings.get(j).unwrap_or("");
            if case {
                sa.cmp(sb)
            } else {
                sa.to_lowercase().cmp(&sb.to_lowercase())
            }
        }
        (Value::Error(x), Value::Error(y)) => x.as_str().cmp(y.as_str()),
        _ => Ordering::Equal,
    }
}

fn list_rank(list: &[String], s: &str) -> usize {
    list.iter()
        .position(|x| x.eq_ignore_ascii_case(s))
        .unwrap_or(usize::MAX)
}

fn display(wb: &Workbook, v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::Bool(true) => "TRUE".into(),
        Value::Bool(false) => "FALSE".into(),
        Value::Text(id) => wb.intern().strings.get(*id).unwrap_or("").to_string(),
        Value::Error(k) => k.as_str().to_string(),
        _ => String::new(),
    }
}

fn empty_slot() -> CellSlot {
    CellSlot {
        value: Value::Empty,
        formula: None,
        style: crate::style::StyleId::DEFAULT,
        flags: crate::storage::CellFlags::DEFAULT,
    }
}

fn norm(r: RangeRef) -> (u32, u16, u32, u16) {
    (
        r.start.row.min(r.end.row),
        r.start.col.min(r.end.col),
        r.start.row.max(r.end.row),
        r.start.col.max(r.end.col),
    )
}
