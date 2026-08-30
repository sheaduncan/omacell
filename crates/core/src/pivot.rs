//! Pivot tables (F-7.1).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::addr::{RangeRef, SheetId};
use crate::date_system::DateSystem;
use crate::dates::serial_to_date;
use crate::error::CoreError;
use crate::storage::{CellFlags, CellSlot};
use crate::style::{Font, Style, StyleId};
use crate::value::Value;
use crate::workbook::Workbook;

/// Handle for a pivot table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PivotId(u32);

impl PivotId {
    /// Wrap an index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Numeric id.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Aggregation for a data field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PivotAgg {
    /// Sum of numbers.
    #[default]
    Sum,
    /// Count of numbers.
    Count,
    /// Average of numbers.
    Average,
    /// Minimum.
    Min,
    /// Maximum.
    Max,
    /// Count of non-empty.
    CountA,
    /// Distinct count of display values.
    DistinctCount,
    /// Sample standard deviation.
    Stdev,
    /// Sample variance.
    Var,
}

impl PivotAgg {
    /// Stable snake_case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Count => "count",
            Self::Average => "average",
            Self::Min => "min",
            Self::Max => "max",
            Self::CountA => "counta",
            Self::DistinctCount => "distinct_count",
            Self::Stdev => "stdev",
            Self::Var => "var",
        }
    }

    /// Parse a snake_case or OOXML subtotal name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "sum" => Self::Sum,
            "count" | "countNums" => Self::Count,
            "average" => Self::Average,
            "min" => Self::Min,
            "max" => Self::Max,
            "counta" => Self::CountA,
            "distinct_count" | "distinctCount" => Self::DistinctCount,
            "stdev" | "stdDev" => Self::Stdev,
            "var" => Self::Var,
            _ => return None,
        })
    }

    /// OOXML `dataField@subtotal`.
    #[must_use]
    pub const fn ooxml_subtotal(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Count => "countNums",
            Self::Average => "average",
            Self::Min => "min",
            Self::Max => "max",
            Self::CountA | Self::DistinctCount => "count",
            Self::Stdev => "stdDev",
            Self::Var => "var",
        }
    }
}

/// Show-values-as transform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowAs {
    /// Raw aggregate.
    #[default]
    Normal,
    /// Percent of grand total (0–100).
    PctOfTotal,
    /// Percent of row total (0–100).
    PctOfRow,
    /// Percent of column total (0–100).
    PctOfCol,
    /// Running total down the rows.
    RunningTotal,
    /// Difference from the previous row.
    DifferenceFrom,
}

impl ShowAs {
    /// Stable snake_case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::PctOfTotal => "pct_of_total",
            Self::PctOfRow => "pct_of_row",
            Self::PctOfCol => "pct_of_col",
            Self::RunningTotal => "running_total",
            Self::DifferenceFrom => "difference_from",
        }
    }

    /// Parse a snake_case or OOXML `showDataAs` name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "normal" => Self::Normal,
            "pct_of_total" | "percentOfTotal" => Self::PctOfTotal,
            "pct_of_row" | "percentOfRow" => Self::PctOfRow,
            "pct_of_col" | "percentOfCol" => Self::PctOfCol,
            "running_total" | "runTotal" => Self::RunningTotal,
            "difference_from" | "difference" => Self::DifferenceFrom,
            _ => return None,
        })
    }

    /// OOXML `showDataAs`.
    #[must_use]
    pub const fn ooxml(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::PctOfTotal => "percentOfTotal",
            Self::PctOfRow => "percentOfRow",
            Self::PctOfCol => "percentOfCol",
            Self::RunningTotal => "runTotal",
            Self::DifferenceFrom => "difference",
        }
    }
}

/// Date grouping grain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateGroup {
    /// Calendar day.
    Days,
    /// Calendar month.
    Months,
    /// Calendar quarter.
    Quarters,
    /// Calendar year.
    Years,
}

impl DateGroup {
    /// Stable snake_case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Days => "days",
            Self::Months => "months",
            Self::Quarters => "quarters",
            Self::Years => "years",
        }
    }

    /// Parse a snake_case or OOXML `groupBy` name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "days" => Self::Days,
            "months" => Self::Months,
            "quarters" => Self::Quarters,
            "years" => Self::Years,
            _ => return None,
        })
    }
}

/// Field grouping.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PivotGroup {
    /// No grouping.
    #[default]
    None,
    /// Group date serials.
    Date(DateGroup),
    /// Numeric bins `[start, start+size, …)`.
    Numeric {
        /// Bin origin.
        start: f64,
        /// Bin width (must be positive and finite).
        size: f64,
    },
}

/// Report layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PivotLayout {
    /// Compact form (Excel default).
    #[default]
    Compact,
    /// Outline.
    Outline,
    /// Tabular (one column per row field).
    Tabular,
}

impl PivotLayout {
    /// Stable snake_case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Outline => "outline",
            Self::Tabular => "tabular",
        }
    }

    /// Parse a snake_case name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "compact" => Self::Compact,
            "outline" => Self::Outline,
            "tabular" => Self::Tabular,
            _ => return None,
        })
    }
}

/// One data field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PivotDataField {
    /// Source column name.
    pub source: String,
    /// Aggregation.
    pub agg: PivotAgg,
    /// Show-as.
    #[serde(default)]
    pub show_as: ShowAs,
}

impl PivotDataField {
    /// Construct a data field with raw aggregates.
    #[must_use]
    pub fn new(source: impl Into<String>, agg: PivotAgg) -> Self {
        Self {
            source: source.into(),
            agg,
            show_as: ShowAs::Normal,
        }
    }
}

/// Cached source cell used when the live range is missing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CacheValue {
    /// Number.
    Number(f64),
    /// Display text.
    Text(String),
    /// Blank.
    Empty,
}

/// A pivot table definition plus last output rectangle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PivotTable {
    /// Stable id.
    pub id: PivotId,
    /// Display name.
    pub name: String,
    /// Source sheet.
    pub source_sheet: SheetId,
    /// Source range including header row.
    pub source: RangeRef,
    /// Output sheet.
    pub dest_sheet: SheetId,
    /// Output origin.
    pub dest_row: u32,
    /// Output origin column.
    pub dest_col: u16,
    /// Inclusive output end (after last refresh).
    pub out_end_row: u32,
    /// Inclusive output end column.
    pub out_end_col: u16,
    /// Row fields (source names).
    pub rows: Vec<String>,
    /// Column fields.
    pub cols: Vec<String>,
    /// Data fields.
    pub data: Vec<PivotDataField>,
    /// Page/filter fields: `(name, allowed values)`. Empty allow-list means all.
    pub filters: Vec<(String, Vec<String>)>,
    /// Grouping per source field name.
    #[serde(default)]
    pub groups: BTreeMap<String, PivotGroup>,
    /// Layout.
    #[serde(default)]
    pub layout: PivotLayout,
    /// Grand totals for rows.
    #[serde(default = "yes")]
    pub grand_rows: bool,
    /// Grand totals for columns.
    #[serde(default = "yes")]
    pub grand_cols: bool,
    /// Subtotals for outer row fields when more than one row field is present.
    #[serde(default = "yes")]
    pub subtotals: bool,
    /// Refresh when the file opens.
    #[serde(default)]
    pub refresh_on_load: bool,
}

fn yes() -> bool {
    true
}

impl PivotTable {
    /// Construct a compact pivot at `dest` over `source` (header row included).
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        source_sheet: SheetId,
        source: RangeRef,
        dest_sheet: SheetId,
        dest_row: u32,
        dest_col: u16,
    ) -> Self {
        Self {
            id: PivotId::new(0),
            name: name.into(),
            source_sheet,
            source,
            dest_sheet,
            dest_row,
            dest_col,
            out_end_row: dest_row,
            out_end_col: dest_col,
            rows: Vec::new(),
            cols: Vec::new(),
            data: Vec::new(),
            filters: Vec::new(),
            groups: BTreeMap::new(),
            layout: PivotLayout::Compact,
            grand_rows: true,
            grand_cols: true,
            subtotals: true,
            refresh_on_load: false,
        }
    }

    /// Whether `(row, col)` on `sheet` is inside the last rendered region.
    #[must_use]
    pub fn contains(&self, sheet: SheetId, row: u32, col: u16) -> bool {
        sheet == self.dest_sheet
            && row >= self.dest_row
            && row <= self.out_end_row
            && col >= self.dest_col
            && col <= self.out_end_col
    }
}

/// Workbook pivot registry.
#[derive(Clone, Debug, Default)]
pub struct PivotRegistry {
    tables: BTreeMap<PivotId, PivotTable>,
    by_name: BTreeMap<String, PivotId>,
    next: u32,
}

impl PivotRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert, assigning a fresh id.
    pub fn insert(&mut self, mut table: PivotTable) -> Result<PivotId, CoreError> {
        let nk = table.name.to_lowercase();
        if nk.is_empty() {
            return Err(CoreError::new("pivot.name", "pivot name is empty"));
        }
        if self.by_name.contains_key(&nk) {
            return Err(CoreError::new(
                "pivot.name",
                format!("pivot name {:?} already exists", table.name),
            ));
        }
        let id = PivotId::new(self.next);
        self.next += 1;
        table.id = id;
        self.by_name.insert(nk, id);
        self.tables.insert(id, table);
        Ok(id)
    }

    /// Restore keeping id.
    pub fn restore(&mut self, table: PivotTable) -> Result<(), CoreError> {
        let nk = table.name.to_lowercase();
        if nk.is_empty() {
            return Err(CoreError::new("pivot.name", "pivot name is empty"));
        }
        if let Some(&id) = self.by_name.get(&nk)
            && id != table.id
        {
            return Err(CoreError::new(
                "pivot.name",
                format!("pivot name {:?} already exists", table.name),
            ));
        }
        if let Some(existing) = self.tables.get(&table.id)
            && existing.name.to_lowercase() != nk
        {
            self.by_name.remove(&existing.name.to_lowercase());
        }
        self.by_name.insert(nk, table.id);
        if table.id.index() >= self.next {
            self.next = table.id.index() + 1;
        }
        self.tables.insert(table.id, table);
        Ok(())
    }

    /// Remove by id.
    pub fn remove(&mut self, id: PivotId) -> Option<PivotTable> {
        let t = self.tables.remove(&id)?;
        self.by_name.remove(&t.name.to_lowercase());
        Some(t)
    }

    /// Borrow.
    #[must_use]
    pub fn get(&self, id: PivotId) -> Option<&PivotTable> {
        self.tables.get(&id)
    }

    /// Mutable borrow.
    pub fn get_mut(&mut self, id: PivotId) -> Option<&mut PivotTable> {
        self.tables.get_mut(&id)
    }

    /// Lookup by name.
    #[must_use]
    pub fn get_by_name(&self, name: &str) -> Option<&PivotTable> {
        self.by_name
            .get(&name.to_lowercase())
            .and_then(|id| self.tables.get(id))
    }

    /// Sorted by id.
    pub fn iter(&self) -> impl Iterator<Item = &PivotTable> {
        self.tables.values()
    }

    /// Count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    /// Empty?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }
}

/// One rendered cell of a pivot (offsets from dest origin).
#[derive(Clone, Debug, PartialEq)]
pub struct PivotCell {
    /// Row offset from dest origin.
    pub row: u32,
    /// Column offset from dest origin.
    pub col: u16,
    /// Display/value.
    pub value: PivotValue,
    /// Header / total label.
    pub header: bool,
}

/// Output value.
#[derive(Clone, Debug, PartialEq)]
pub enum PivotValue {
    /// Number.
    Number(f64),
    /// Label.
    Text(String),
    /// Empty.
    Empty,
}

#[derive(Clone, Debug)]
struct Record {
    num: BTreeMap<String, Option<f64>>,
    text: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
struct Agg {
    sum: f64,
    count_n: u32,
    count_a: u32,
    min: Option<f64>,
    max: Option<f64>,
    mean: f64,
    m2: f64,
    distinct: BTreeMap<String, ()>,
}

impl Agg {
    fn add(&mut self, num: Option<f64>, text: &str) {
        if !text.is_empty() {
            self.count_a += 1;
            self.distinct.insert(text.to_string(), ());
        }
        let Some(n) = num.filter(|n| n.is_finite()) else {
            return;
        };
        self.count_n += 1;
        self.sum += n;
        self.min = Some(self.min.map_or(n, |m| m.min(n)));
        self.max = Some(self.max.map_or(n, |m| m.max(n)));
        let delta = n - self.mean;
        self.mean += delta / f64::from(self.count_n);
        self.m2 += delta * (n - self.mean);
    }

    fn finish(&self, kind: PivotAgg) -> Option<f64> {
        match kind {
            PivotAgg::Sum => (self.count_n > 0 || self.count_a > 0).then_some(self.sum),
            PivotAgg::Count => Some(f64::from(self.count_n)),
            PivotAgg::CountA => Some(f64::from(self.count_a)),
            PivotAgg::Average => (self.count_n > 0).then_some(self.sum / f64::from(self.count_n)),
            PivotAgg::Min => self.min,
            PivotAgg::Max => self.max,
            PivotAgg::DistinctCount => Some(self.distinct.len() as f64),
            PivotAgg::Stdev if self.count_n >= 2 => {
                Some((self.m2 / f64::from(self.count_n - 1)).sqrt())
            }
            PivotAgg::Var if self.count_n >= 2 => Some(self.m2 / f64::from(self.count_n - 1)),
            PivotAgg::Stdev | PivotAgg::Var => None,
        }
    }
}

/// Refresh `pivot` from its source into an in-memory grid (does not write the sheet).
pub fn materialize(wb: &Workbook, pivot: &PivotTable) -> Result<Vec<PivotCell>, CoreError> {
    let (headers, rows) = cache_table(wb, pivot)?;
    materialize_from_cache(wb.settings().date_system, pivot, &headers, &rows)
}

/// Materialize from cached source rows (used when the live range is missing).
pub fn materialize_from_cache(
    date_system: DateSystem,
    pivot: &PivotTable,
    headers: &[String],
    rows: &[Vec<CacheValue>],
) -> Result<Vec<PivotCell>, CoreError> {
    let records: Vec<Record> = rows
        .iter()
        .map(|row| record_from_cache(headers, row))
        .collect();
    let filtered: Vec<&Record> = records
        .iter()
        .filter(|row| {
            pivot.filters.iter().all(|(name, allowed)| {
                allowed.is_empty()
                    || allowed.contains(&group_key(
                        row,
                        name,
                        pivot.groups.get(name).unwrap_or(&PivotGroup::None),
                        date_system,
                    ))
            })
        })
        .collect();
    let row_keys = unique_keys(&filtered, &pivot.rows, &pivot.groups, date_system);
    let col_keys = unique_keys(&filtered, &pivot.cols, &pivot.groups, date_system);
    let data_n = pivot.data.len().max(1);
    let mut raw: BTreeMap<(Vec<String>, Vec<String>, usize), Agg> = BTreeMap::new();
    for rec in &filtered {
        let rk = keys_of(rec, &pivot.rows, &pivot.groups, date_system);
        let ck = keys_of(rec, &pivot.cols, &pivot.groups, date_system);
        for (di, df) in pivot.data.iter().enumerate() {
            let num = rec.num.get(&df.source).copied().flatten();
            let text = rec.text.get(&df.source).cloned().unwrap_or_default();
            raw.entry((rk.clone(), ck.clone(), di))
                .or_default()
                .add(num, &text);
        }
    }
    let mut row_tot: BTreeMap<Vec<String>, Vec<Agg>> = BTreeMap::new();
    let mut col_tot: BTreeMap<Vec<String>, Vec<Agg>> = BTreeMap::new();
    let mut grand = vec![Agg::default(); data_n];
    for ((rk, ck, di), agg) in &raw {
        let row_slot = row_tot
            .entry(rk.clone())
            .or_insert_with(|| vec![Agg::default(); data_n]);
        if let Some(dst) = row_slot.get_mut(*di) {
            add_into(dst, agg);
        }
        let col_slot = col_tot
            .entry(ck.clone())
            .or_insert_with(|| vec![Agg::default(); data_n]);
        if let Some(dst) = col_slot.get_mut(*di) {
            add_into(dst, agg);
        }
        if let Some(g) = grand.get_mut(*di) {
            add_into(g, agg);
        }
    }
    let tabular = matches!(pivot.layout, PivotLayout::Tabular | PivotLayout::Outline);
    let row_label_cols = if pivot.rows.is_empty() {
        1
    } else if tabular {
        u16::try_from(pivot.rows.len()).unwrap_or(1)
    } else {
        1
    };
    let col_header_rows = if pivot.cols.is_empty() {
        1
    } else {
        u32::try_from(pivot.cols.len()).unwrap_or(1)
    };
    let mut cells = Vec::new();
    if pivot.cols.is_empty() {
        if pivot.data.is_empty() {
            cells.push(label(0, row_label_cols, "Values"));
        } else {
            for (di, df) in pivot.data.iter().enumerate() {
                cells.push(label(
                    0,
                    row_label_cols + di as u16,
                    &format!("{} {}", agg_name(df.agg), df.source),
                ));
            }
        }
    } else {
        for (ci, ck) in col_keys.iter().enumerate() {
            for (depth, part) in ck.iter().enumerate() {
                cells.push(label(
                    depth as u32,
                    row_label_cols + (ci * data_n) as u16,
                    part,
                ));
            }
            if pivot.cols.len() < data_n.max(1) && pivot.data.len() > 1 {
                for (di, df) in pivot.data.iter().enumerate() {
                    cells.push(label(
                        col_header_rows.saturating_sub(1),
                        row_label_cols + (ci * data_n + di) as u16,
                        &format!("{} {}", agg_name(df.agg), df.source),
                    ));
                }
            }
        }
    }
    let emit_grand_cols = pivot.grand_cols && !pivot.cols.is_empty();
    if emit_grand_cols {
        cells.push(label(
            0,
            row_label_cols + (col_keys.len() * data_n) as u16,
            "Grand Total",
        ));
    }
    let body0 = col_header_rows;
    let mut r = body0;
    for (ri, rk) in row_keys.iter().enumerate() {
        if tabular {
            for (i, part) in rk.iter().enumerate() {
                cells.push(label(r, i as u16, part));
            }
        } else {
            cells.push(label(r, 0, &rk.join(" | ")));
        }
        push_data_row(
            &mut cells,
            r,
            row_label_cols,
            data_n,
            rk,
            ri,
            &row_keys,
            &col_keys,
            pivot,
            &raw,
            &row_tot,
            &col_tot,
            &grand,
            emit_grand_cols,
        );
        r += 1;
        if pivot.subtotals && pivot.rows.len() > 1 {
            let last_of_group = ri + 1 == row_keys.len()
                || row_keys
                    .get(ri + 1)
                    .is_none_or(|next| next.first() != rk.first());
            if last_of_group {
                let prefix = rk.first().cloned().unwrap_or_default();
                cells.push(label(r, 0, &format!("{prefix} Total")));
                let sub_agg = prefix_totals(&raw, &prefix, data_n, &col_keys);
                for (ci, ck) in col_keys.iter().enumerate() {
                    for (di, df) in pivot.data.iter().enumerate() {
                        let n = sub_agg
                            .get(&(ck.clone(), di))
                            .and_then(|a| a.finish(df.agg));
                        cells.push(PivotCell {
                            row: r,
                            col: row_label_cols + (ci * data_n + di) as u16,
                            value: n.map(PivotValue::Number).unwrap_or(PivotValue::Empty),
                            header: false,
                        });
                    }
                }
                if emit_grand_cols {
                    for (di, df) in pivot.data.iter().enumerate() {
                        let mut tot = Agg::default();
                        for ck in &col_keys {
                            if let Some(a) = sub_agg.get(&(ck.clone(), di)) {
                                add_into(&mut tot, a);
                            }
                        }
                        cells.push(PivotCell {
                            row: r,
                            col: row_label_cols + (col_keys.len() * data_n + di) as u16,
                            value: tot
                                .finish(df.agg)
                                .map(PivotValue::Number)
                                .unwrap_or(PivotValue::Empty),
                            header: true,
                        });
                    }
                }
                r += 1;
            }
        }
    }
    if pivot.grand_rows && !pivot.rows.is_empty() {
        cells.push(label(r, 0, "Grand Total"));
        for (ci, ck) in col_keys.iter().enumerate() {
            for (di, df) in pivot.data.iter().enumerate() {
                let tot = col_tot
                    .get(ck)
                    .and_then(|v| v.get(di))
                    .and_then(|a| a.finish(df.agg));
                cells.push(PivotCell {
                    row: r,
                    col: row_label_cols + (ci * data_n + di) as u16,
                    value: tot.map(PivotValue::Number).unwrap_or(PivotValue::Empty),
                    header: true,
                });
            }
        }
        if emit_grand_cols {
            for (di, df) in pivot.data.iter().enumerate() {
                let g = grand.get(di).and_then(|a| a.finish(df.agg));
                cells.push(PivotCell {
                    row: r,
                    col: row_label_cols + (col_keys.len() * data_n + di) as u16,
                    value: g.map(PivotValue::Number).unwrap_or(PivotValue::Empty),
                    header: true,
                });
            }
        }
    }
    Ok(cells)
}

#[allow(clippy::too_many_arguments)]
fn push_data_row(
    cells: &mut Vec<PivotCell>,
    r: u32,
    row_label_cols: u16,
    data_n: usize,
    rk: &[String],
    ri: usize,
    row_keys: &[Vec<String>],
    col_keys: &[Vec<String>],
    pivot: &PivotTable,
    raw: &BTreeMap<(Vec<String>, Vec<String>, usize), Agg>,
    row_tot: &BTreeMap<Vec<String>, Vec<Agg>>,
    col_tot: &BTreeMap<Vec<String>, Vec<Agg>>,
    grand: &[Agg],
    emit_grand_cols: bool,
) {
    for (ci, ck) in col_keys.iter().enumerate() {
        for (di, df) in pivot.data.iter().enumerate() {
            let n = show_value(
                raw.get(&(rk.to_vec(), ck.clone(), di))
                    .and_then(|a| a.finish(df.agg)),
                df.show_as,
                rk,
                ck,
                di,
                df.agg,
                ri,
                row_keys,
                raw,
                row_tot,
                col_tot,
                grand.get(di).and_then(|a| a.finish(df.agg)),
            );
            cells.push(PivotCell {
                row: r,
                col: row_label_cols + (ci * data_n + di) as u16,
                value: n.map(PivotValue::Number).unwrap_or(PivotValue::Empty),
                header: false,
            });
        }
        if pivot.data.is_empty() {
            cells.push(PivotCell {
                row: r,
                col: row_label_cols + ci as u16,
                value: PivotValue::Empty,
                header: false,
            });
        }
    }
    if emit_grand_cols {
        for (di, df) in pivot.data.iter().enumerate() {
            let tot = row_tot
                .get(rk)
                .and_then(|v| v.get(di))
                .and_then(|a| a.finish(df.agg));
            cells.push(PivotCell {
                row: r,
                col: row_label_cols + (col_keys.len() * data_n + di) as u16,
                value: tot.map(PivotValue::Number).unwrap_or(PivotValue::Empty),
                header: true,
            });
        }
    }
}

fn prefix_totals(
    raw: &BTreeMap<(Vec<String>, Vec<String>, usize), Agg>,
    prefix: &str,
    data_n: usize,
    col_keys: &[Vec<String>],
) -> BTreeMap<(Vec<String>, usize), Agg> {
    let mut out: BTreeMap<(Vec<String>, usize), Agg> = BTreeMap::new();
    for ((rk, ck, di), agg) in raw {
        if rk.first().map(String::as_str) != Some(prefix) {
            continue;
        }
        add_into(out.entry((ck.clone(), *di)).or_default(), agg);
    }
    for ck in col_keys {
        for di in 0..data_n {
            out.entry((ck.clone(), di)).or_default();
        }
    }
    out
}

/// Snapshot the source range as cache records (header row plus data rows).
pub fn cache_table(
    wb: &Workbook,
    pivot: &PivotTable,
) -> Result<(Vec<String>, Vec<Vec<CacheValue>>), CoreError> {
    let (r0, c0, r1, c1) = norm(pivot.source);
    if r1 < r0 {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut headers = Vec::new();
    for c in c0..=c1 {
        let text = cell_text(wb, pivot.source_sheet, r0, c);
        headers.push(if text.is_empty() {
            format!("Column{}", u32::from(c - c0) + 1)
        } else {
            text
        });
    }
    let mut rows = Vec::new();
    if r1 == r0 {
        return Ok((headers, rows));
    }
    for r in r0 + 1..=r1 {
        let mut row = Vec::with_capacity(headers.len());
        for c in c0..=c1 {
            row.push(cell_cache(wb, pivot.source_sheet, r, c));
        }
        rows.push(row);
    }
    Ok((headers, rows))
}

/// Write materialized cells onto the destination sheet and update the output rectangle.
pub fn write_output(
    wb: &mut Workbook,
    pivot: &mut PivotTable,
    cells: &[PivotCell],
) -> Result<(), CoreError> {
    if pivot.out_end_row >= pivot.dest_row && pivot.out_end_col >= pivot.dest_col {
        for r in pivot.dest_row..=pivot.out_end_row {
            for c in pivot.dest_col..=pivot.out_end_col {
                let _ = wb.write_slot(pivot.dest_sheet, r, c, None);
            }
        }
    }
    let header_sid = wb.intern_style(Style {
        font: Font {
            bold: true,
            ..Font::default()
        },
        ..Style::default()
    });
    let mut max_r = pivot.dest_row;
    let mut max_c = pivot.dest_col;
    for cell in cells {
        let row = pivot.dest_row + cell.row;
        let col = pivot.dest_col + cell.col;
        max_r = max_r.max(row);
        max_c = max_c.max(col);
        let style = if cell.header {
            header_sid
        } else {
            StyleId::DEFAULT
        };
        match &cell.value {
            PivotValue::Number(n) => {
                wb.write_slot(
                    pivot.dest_sheet,
                    row,
                    col,
                    Some(CellSlot {
                        value: Value::Number(*n),
                        formula: None,
                        style,
                        flags: CellFlags::DEFAULT,
                    }),
                )?;
            }
            PivotValue::Text(t) => {
                let sid = wb.intern_text(t);
                wb.write_slot(
                    pivot.dest_sheet,
                    row,
                    col,
                    Some(CellSlot {
                        value: Value::Text(sid),
                        formula: None,
                        style,
                        flags: CellFlags::DEFAULT,
                    }),
                )?;
                wb.release_text(sid);
            }
            PivotValue::Empty => {
                let _ = wb.write_slot(pivot.dest_sheet, row, col, None);
            }
        }
    }
    wb.release_style(header_sid);
    pivot.out_end_row = max_r;
    pivot.out_end_col = max_c;
    Ok(())
}

fn label(row: u32, col: u16, text: &str) -> PivotCell {
    PivotCell {
        row,
        col,
        value: if text.is_empty() {
            PivotValue::Empty
        } else {
            PivotValue::Text(text.to_string())
        },
        header: true,
    }
}

fn agg_name(agg: PivotAgg) -> &'static str {
    match agg {
        PivotAgg::Sum => "Sum of",
        PivotAgg::Count => "Count of",
        PivotAgg::Average => "Average of",
        PivotAgg::Min => "Min of",
        PivotAgg::Max => "Max of",
        PivotAgg::CountA => "CountA of",
        PivotAgg::DistinctCount => "Distinct count of",
        PivotAgg::Stdev => "Stdev of",
        PivotAgg::Var => "Var of",
    }
}

fn add_into(dst: &mut Agg, src: &Agg) {
    for k in src.distinct.keys() {
        dst.distinct.insert(k.clone(), ());
    }
    dst.count_a += src.count_a;
    if src.count_n == 0 {
        return;
    }
    if dst.count_n == 0 {
        dst.sum = src.sum;
        dst.count_n = src.count_n;
        dst.min = src.min;
        dst.max = src.max;
        dst.mean = src.mean;
        dst.m2 = src.m2;
        return;
    }
    let n1 = dst.count_n;
    let n2 = src.count_n;
    let n = n1 + n2;
    let delta = src.mean - dst.mean;
    dst.m2 += src.m2 + delta * delta * f64::from(n1) * f64::from(n2) / f64::from(n);
    dst.mean = (dst.mean * f64::from(n1) + src.mean * f64::from(n2)) / f64::from(n);
    dst.sum += src.sum;
    dst.count_n = n;
    dst.min = match (dst.min, src.min) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    dst.max = match (dst.max, src.max) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
}

#[allow(clippy::too_many_arguments)]
fn show_value(
    value: Option<f64>,
    mode: ShowAs,
    rk: &[String],
    ck: &[String],
    di: usize,
    agg: PivotAgg,
    ri: usize,
    row_keys: &[Vec<String>],
    raw: &BTreeMap<(Vec<String>, Vec<String>, usize), Agg>,
    row_tot: &BTreeMap<Vec<String>, Vec<Agg>>,
    col_tot: &BTreeMap<Vec<String>, Vec<Agg>>,
    grand: Option<f64>,
) -> Option<f64> {
    let n = value?;
    match mode {
        ShowAs::Normal => Some(n),
        ShowAs::PctOfTotal => grand.filter(|g| *g != 0.0).map(|g| 100.0 * n / g),
        ShowAs::PctOfRow => row_tot
            .get(rk)
            .and_then(|v| v.get(di))
            .and_then(|a| a.finish(agg))
            .filter(|g| *g != 0.0)
            .map(|g| 100.0 * n / g),
        ShowAs::PctOfCol => col_tot
            .get(ck)
            .and_then(|v| v.get(di))
            .and_then(|a| a.finish(agg))
            .filter(|g| *g != 0.0)
            .map(|g| 100.0 * n / g),
        ShowAs::RunningTotal => {
            let mut acc = 0.0;
            for key in row_keys.iter().take(ri + 1) {
                acc += raw
                    .get(&(key.clone(), ck.to_vec(), di))
                    .and_then(|a| a.finish(agg))
                    .unwrap_or(0.0);
            }
            Some(acc)
        }
        ShowAs::DifferenceFrom => {
            let prev = if ri == 0 {
                0.0
            } else {
                raw.get(&(row_keys[ri - 1].clone(), ck.to_vec(), di))
                    .and_then(|a| a.finish(agg))
                    .unwrap_or(0.0)
            };
            Some(n - prev)
        }
    }
}

fn record_from_cache(headers: &[String], row: &[CacheValue]) -> Record {
    let mut rec = Record {
        num: BTreeMap::new(),
        text: BTreeMap::new(),
    };
    for (i, header) in headers.iter().enumerate() {
        let value = row.get(i).cloned().unwrap_or(CacheValue::Empty);
        match value {
            CacheValue::Number(n) => {
                rec.num.insert(header.clone(), Some(n));
                rec.text.insert(header.clone(), n.to_string());
            }
            CacheValue::Text(t) => {
                rec.num.insert(header.clone(), None);
                rec.text.insert(header.clone(), t);
            }
            CacheValue::Empty => {
                rec.num.insert(header.clone(), None);
                rec.text.insert(header.clone(), String::new());
            }
        }
    }
    rec
}

fn cell_cache(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> CacheValue {
    match wb.get(sheet, row, col).ok().flatten().map(|s| s.value) {
        Some(Value::Number(n)) => CacheValue::Number(n),
        Some(Value::Bool(true)) => CacheValue::Number(1.0),
        Some(Value::Bool(false)) => CacheValue::Number(0.0),
        Some(Value::Text(id)) => {
            CacheValue::Text(wb.intern().strings.get(id).unwrap_or("").to_string())
        }
        Some(Value::Error(k)) => CacheValue::Text(k.as_str().to_string()),
        _ => CacheValue::Empty,
    }
}

fn cell_text(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> String {
    match cell_cache(wb, sheet, row, col) {
        CacheValue::Number(n) => n.to_string(),
        CacheValue::Text(t) => t,
        CacheValue::Empty => String::new(),
    }
}

fn unique_keys(
    rows: &[&Record],
    fields: &[String],
    groups: &BTreeMap<String, PivotGroup>,
    date_system: DateSystem,
) -> Vec<Vec<String>> {
    let mut set = BTreeMap::new();
    for rec in rows {
        set.insert(keys_of(rec, fields, groups, date_system), ());
    }
    set.into_keys().collect()
}

fn keys_of(
    rec: &Record,
    fields: &[String],
    groups: &BTreeMap<String, PivotGroup>,
    date_system: DateSystem,
) -> Vec<String> {
    if fields.is_empty() {
        return vec![String::new()];
    }
    fields
        .iter()
        .map(|f| {
            group_key(
                rec,
                f,
                groups.get(f).unwrap_or(&PivotGroup::None),
                date_system,
            )
        })
        .collect()
}

fn group_key(rec: &Record, field: &str, group: &PivotGroup, date_system: DateSystem) -> String {
    let num = rec.num.get(field).copied().flatten();
    let text = rec.text.get(field).cloned().unwrap_or_default();
    match group {
        PivotGroup::None => {
            if let Some(n) = num {
                n.to_string()
            } else {
                text
            }
        }
        PivotGroup::Date(g) => {
            let Some(n) = num else {
                return text;
            };
            let Some(d) = serial_to_date(n.trunc() as i64, date_system) else {
                return n.to_string();
            };
            match g {
                DateGroup::Years => format!("{}", d.year),
                DateGroup::Quarters => format!("{} Q{}", d.year, (d.month - 1) / 3 + 1),
                DateGroup::Months => format!("{}-{:02}", d.year, d.month),
                DateGroup::Days => format!("{}-{:02}-{:02}", d.year, d.month, d.day),
            }
        }
        PivotGroup::Numeric { start, size } => {
            let Some(n) = num else {
                return text;
            };
            if size.is_finite() && *size > 0.0 && start.is_finite() {
                let i = ((n - start) / size).floor();
                format!("{}", start + i * size)
            } else {
                n.to_string()
            }
        }
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
