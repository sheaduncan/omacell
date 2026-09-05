//! Range sort with Excel type ordering (F-6.1).

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::addr::{RangeRef, SheetId};
use crate::condfmt::{CfVisual, ResolvedCfOverlay, resolve_overlay};
use crate::error::CoreError;
use crate::formula::{RewriteOp, rewrite_print};
use crate::storage::CellSlot;
use crate::style::{Color, Fill};
use crate::value::Value;
use crate::workbook::Workbook;

type RowRecord = (u32, usize, Vec<CellSlot>, Vec<Option<u8>>);
type ColRecord = (u16, usize, Vec<CellSlot>, Vec<Option<u8>>);

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
    /// Resolved conditional-format icon bucket.
    Icon,
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

/// Heuristically detect a header row (or column for left-to-right sorting).
///
/// A header is detected when the first record has text over non-text data, a
/// distinct style, or a non-formula label over formula data. Ambiguous all-text
/// data is left as data so callers can override explicitly.
pub fn detect_header(
    wb: &Workbook,
    sheet: SheetId,
    range: RangeRef,
    left_to_right: bool,
) -> Result<bool, CoreError> {
    let (r0, c0, r1, c1) = norm(range);
    if (!left_to_right && r0 == r1) || (left_to_right && c0 == c1) {
        return Ok(false);
    }
    let mut style_signals = 0usize;
    let pair_count = if left_to_right {
        for row in r0..=r1 {
            let (strong, style) = header_pair(wb, sheet, (row, c0), (row, c0 + 1))?;
            if strong {
                return Ok(true);
            }
            style_signals += usize::from(style);
        }
        usize::try_from(r1 - r0 + 1).unwrap_or(usize::MAX)
    } else {
        for col in c0..=c1 {
            let (strong, style) = header_pair(wb, sheet, (r0, col), (r0 + 1, col))?;
            if strong {
                return Ok(true);
            }
            style_signals += usize::from(style);
        }
        usize::from(c1 - c0) + 1
    };
    Ok(style_signals > 0 && style_signals.saturating_mul(2) >= pair_count)
}

fn header_pair(
    wb: &Workbook,
    sheet: SheetId,
    first: (u32, u16),
    second: (u32, u16),
) -> Result<(bool, bool), CoreError> {
    let first = wb.get(sheet, first.0, first.1)?.copied();
    let second = wb.get(sheet, second.0, second.1)?.copied();
    let first_value = first.map(|slot| slot.value).unwrap_or(Value::Empty);
    let second_value = second.map(|slot| slot.value).unwrap_or(Value::Empty);
    let strong = (matches!(first_value, Value::Text(_))
        && !matches!(second_value, Value::Text(_) | Value::Empty))
        || (first.is_some_and(|slot| slot.formula.is_none())
            && second.is_some_and(|slot| slot.formula.is_some()));
    Ok((
        strong,
        first.map(|slot| slot.style) != second.map(|slot| slot.style),
    ))
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
        wb.ensure_range_not_pivot_output(sheet, r0, c0, r1, c1)?;
        let max_offset = if spec.left_to_right {
            r1 - r0
        } else {
            u32::from(c1 - c0)
        };
        if let Some(key) = spec
            .keys
            .iter()
            .find(|key| u32::from(key.offset) > max_offset)
        {
            return Err(CoreError::new(
                "sort.key",
                format!("sort key offset {} is outside the range", key.offset),
            ));
        }
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
    let icon_overlay = if spec.keys.iter().any(|key| key.by == SortBy::Icon) {
        Some(resolve_overlay(
            wb,
            sheet,
            RangeRef::from_corners(
                crate::addr::CellRef::new(r0, c0)?,
                crate::addr::CellRef::new(r1, c1)?,
            ),
        )?)
    } else {
        None
    };
    let mut decorated: Vec<RowRecord> = Vec::new();
    for (ord, &r) in rows.iter().enumerate() {
        let mut slots = Vec::new();
        for c in c0..=c1 {
            slots.push(wb.get(sheet, r, c)?.copied().unwrap_or(empty_slot()));
        }
        let icons = spec
            .keys
            .iter()
            .map(|key| {
                c0.checked_add(key.offset)
                    .and_then(|col| icon_at(icon_overlay.as_ref(), r, col))
            })
            .collect();
        decorated.push((r, ord, slots, icons));
    }
    let held_slots: Vec<CellSlot> = decorated
        .iter()
        .flat_map(|(_, _, slots, _)| slots.iter().copied())
        .collect();
    wb.with_held_slots(&held_slots, |wb| {
        decorated.sort_by(|a, b| {
            let ord = cmp_records(wb, &a.2, &b.2, &a.3, &b.3, spec);
            if ord == std::cmp::Ordering::Equal {
                a.1.cmp(&b.1)
            } else {
                ord
            }
        });
        let row_map: FxHashMap<u32, u32> = rows
            .iter()
            .zip(&decorated)
            .map(|(destination, (source, _, _, _))| (*source, *destination))
            .collect();
        remap_row_side_records(wb, sheet, &row_map, c0, c1)?;
        let mut moved = 0u32;
        for (dest_row, (src_row, _, slots, _)) in rows.iter().zip(decorated) {
            let drow = *dest_row as i32 - src_row as i32;
            if drow != 0 {
                moved += 1;
            }
            for (i, slot) in slots.into_iter().enumerate() {
                let col = c0 + i as u16;
                write_moved(wb, sheet, *dest_row, col, slot, drow, 0)?;
            }
        }
        crate::ops::rewrite_ai_redact_marks_sort_rows(wb, sheet, &row_map, c0, c1);
        Ok(moved)
    })
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
    let icon_overlay = if spec.keys.iter().any(|key| key.by == SortBy::Icon) {
        Some(resolve_overlay(
            wb,
            sheet,
            RangeRef::from_corners(
                crate::addr::CellRef::new(r0, c0)?,
                crate::addr::CellRef::new(r1, c1)?,
            ),
        )?)
    } else {
        None
    };
    let mut decorated: Vec<ColRecord> = Vec::new();
    for (ord, &c) in cols.iter().enumerate() {
        let mut slots = Vec::new();
        for r in r0..=r1 {
            slots.push(wb.get(sheet, r, c)?.copied().unwrap_or(empty_slot()));
        }
        let icons = spec
            .keys
            .iter()
            .map(|key| {
                r0.checked_add(u32::from(key.offset))
                    .and_then(|row| icon_at(icon_overlay.as_ref(), row, c))
            })
            .collect();
        decorated.push((c, ord, slots, icons));
    }
    let held_slots: Vec<CellSlot> = decorated
        .iter()
        .flat_map(|(_, _, slots, _)| slots.iter().copied())
        .collect();
    wb.with_held_slots(&held_slots, |wb| {
        decorated.sort_by(|a, b| {
            let ord = cmp_records(wb, &a.2, &b.2, &a.3, &b.3, spec);
            if ord == std::cmp::Ordering::Equal {
                a.1.cmp(&b.1)
            } else {
                ord
            }
        });
        let col_map: FxHashMap<u16, u16> = cols
            .iter()
            .zip(&decorated)
            .map(|(destination, (source, _, _, _))| (*source, *destination))
            .collect();
        remap_col_side_records(wb, sheet, &col_map, r0, r1)?;
        let mut moved = 0u32;
        for (dest_col, (src_col, _, slots, _)) in cols.iter().zip(decorated) {
            let dcol = i32::from(*dest_col) - i32::from(src_col);
            if dcol != 0 {
                moved += 1;
            }
            for (i, slot) in slots.into_iter().enumerate() {
                let row = r0 + i as u32;
                write_moved(wb, sheet, row, *dest_col, slot, 0, dcol)?;
            }
        }
        crate::ops::rewrite_ai_redact_marks_sort_cols(wb, sheet, &col_map, r0, r1);
        Ok(moved)
    })
}

fn remap_row_side_records(
    wb: &mut Workbook,
    sheet: SheetId,
    rows: &FxHashMap<u32, u32>,
    c0: u16,
    c1: u16,
) -> Result<(), CoreError> {
    wb.mutate_sheet_edit(sheet, |sheet| {
        sheet.notes = remap_map_rows(&sheet.notes, rows, c0, c1);
        sheet.comments = remap_map_rows(&sheet.comments, rows, c0, c1);
        sheet.hyperlinks = remap_map_rows(&sheet.hyperlinks, rows, c0, c1);
        Ok(())
    })
}

fn remap_col_side_records(
    wb: &mut Workbook,
    sheet: SheetId,
    cols: &FxHashMap<u16, u16>,
    r0: u32,
    r1: u32,
) -> Result<(), CoreError> {
    wb.mutate_sheet_edit(sheet, |sheet| {
        sheet.notes = remap_map_cols(&sheet.notes, cols, r0, r1);
        sheet.comments = remap_map_cols(&sheet.comments, cols, r0, r1);
        sheet.hyperlinks = remap_map_cols(&sheet.hyperlinks, cols, r0, r1);
        Ok(())
    })
}

fn remap_map_rows<T: Clone>(
    records: &FxHashMap<(u32, u16), T>,
    rows: &FxHashMap<u32, u32>,
    c0: u16,
    c1: u16,
) -> FxHashMap<(u32, u16), T> {
    records
        .iter()
        .map(|(&(row, col), value)| {
            let row = if col >= c0 && col <= c1 {
                rows.get(&row).copied().unwrap_or(row)
            } else {
                row
            };
            ((row, col), value.clone())
        })
        .collect()
}

fn remap_map_cols<T: Clone>(
    records: &FxHashMap<(u32, u16), T>,
    cols: &FxHashMap<u16, u16>,
    r0: u32,
    r1: u32,
) -> FxHashMap<(u32, u16), T> {
    records
        .iter()
        .map(|(&(row, col), value)| {
            let col = if row >= r0 && row <= r1 {
                cols.get(&col).copied().unwrap_or(col)
            } else {
                col
            };
            ((row, col), value.clone())
        })
        .collect()
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
    a_icons: &[Option<u8>],
    b_icons: &[Option<u8>],
    spec: &SortSpec,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    for (key_index, key) in spec.keys.iter().enumerate() {
        let idx = usize::from(key.offset);
        let oa = a.get(idx);
        let ob = b.get(idx);
        let ord = match key.by {
            SortBy::Value => cmp_value(
                wb,
                oa,
                ob,
                spec.case_sensitive,
                &key.custom_list,
                key.descending,
            ),
            SortBy::FillColor => cmp_u32(fill_argb(wb, oa), fill_argb(wb, ob)),
            SortBy::FontColor => cmp_u32(font_argb(wb, oa), font_argb(wb, ob)),
            SortBy::Icon => cmp_icon(
                a_icons.get(key_index).copied().flatten(),
                b_icons.get(key_index).copied().flatten(),
                key.descending,
            ),
        };
        let ord = if key.descending && !matches!(key.by, SortBy::Value | SortBy::Icon) {
            ord.reverse()
        } else {
            ord
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

fn icon_at(cache: Option<&ResolvedCfOverlay>, row: u32, col: u16) -> Option<u8> {
    match cache?.get(row, col)?.visual {
        Some(CfVisual::Icon { index, .. }) => Some(index),
        _ => None,
    }
}

fn cmp_icon(a: Option<u8>, b: Option<u8>, descending: bool) -> std::cmp::Ordering {
    let order = match (a, b) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    };
    if descending && a.is_some() && b.is_some() {
        order.reverse()
    } else {
        order
    }
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
    descending: bool,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let va = a.map(|s| s.value).unwrap_or(Value::Empty);
    let vb = b.map(|s| s.value).unwrap_or(Value::Empty);
    let ra = type_rank(&va);
    let rb = type_rank(&vb);
    // Excel keeps blanks at the bottom for both ascending and descending
    // sorts. Reversing the complete type comparator would put them first.
    if ra == 4 || rb == 4 {
        return ra.cmp(&rb);
    }
    if ra != rb {
        let order = ra.cmp(&rb);
        return if descending { order.reverse() } else { order };
    }
    if !list.is_empty() {
        let sa = display(wb, &va);
        let sb = display(wb, &vb);
        let ia = list_rank(list, &sa);
        let ib = list_rank(list, &sb);
        if ia != ib {
            let matched = match (ia == usize::MAX, ib == usize::MAX) {
                (false, true) => return Ordering::Less,
                (true, false) => return Ordering::Greater,
                _ => ia.cmp(&ib),
            };
            return if descending {
                matched.reverse()
            } else {
                matched
            };
        }
    }
    let order = match (va, vb) {
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
    };
    if descending { order.reverse() } else { order }
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
