//! Pivot tables (F-7.1).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::addr::{RangeRef, SheetId};
use crate::date_system::DateSystem;
use crate::dates::serial_to_date;
use crate::error::CoreError;
use crate::limits::{MAX_COLS, MAX_ROWS};
use crate::storage::{CellFlags, CellSlot};
use crate::style::{Font, Style, StyleId};
use crate::value::Value;
use crate::workbook::Workbook;

const MAX_PIVOT_OUTPUT_CELLS: usize = 1_000_000;

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

/// A calculated cache field (`cacheField@databaseField="0"`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PivotCalcField {
    /// Field name shown in the pivot field list.
    pub name: String,
    /// Excel-style formula over other cache field names (`'Amount'*1.1`).
    pub formula: String,
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
    /// Calculated cache fields (not present on the source worksheet).
    #[serde(default)]
    pub calc_fields: Vec<PivotCalcField>,
    /// When true, `.xlsx` cache/table parts must be regenerated on save.
    #[serde(skip)]
    pub ooxml_dirty: bool,
    /// Original OOXML `cacheId` when loaded from `.xlsx`.
    #[serde(skip)]
    pub ooxml_cache_id: Option<u32>,
    /// Original cache definition part name when loaded from `.xlsx`.
    #[serde(skip)]
    pub ooxml_cache_def: Option<String>,
    /// Original pivot table part name when loaded from `.xlsx`.
    #[serde(skip)]
    pub ooxml_table: Option<String>,
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
            calc_fields: Vec::new(),
            ooxml_dirty: true,
            ooxml_cache_id: None,
            ooxml_cache_def: None,
            ooxml_table: None,
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

    pub(crate) fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.name.len()
            + self.rows.iter().map(String::len).sum::<usize>()
            + self.cols.iter().map(String::len).sum::<usize>()
            + self
                .data
                .iter()
                .map(|field| field.source.len() + std::mem::size_of::<PivotDataField>())
                .sum::<usize>()
            + self
                .filters
                .iter()
                .map(|(name, values)| name.len() + values.iter().map(String::len).sum::<usize>())
                .sum::<usize>()
            + self.groups.keys().map(String::len).sum::<usize>()
            + self
                .calc_fields
                .iter()
                .map(|field| field.name.len() + field.formula.len())
                .sum::<usize>()
            + self.ooxml_cache_def.as_ref().map(String::len).unwrap_or(0)
            + self.ooxml_table.as_ref().map(String::len).unwrap_or(0)
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
        self.validate_insert(&table)?;
        let nk = table.name.to_lowercase();
        let id = PivotId::new(self.next);
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| CoreError::new("pivot.id", "pivot id space is exhausted"))?;
        table.id = id;
        self.by_name.insert(nk, id);
        self.tables.insert(id, table);
        Ok(id)
    }

    pub(crate) fn validate_insert(&self, table: &PivotTable) -> Result<(), CoreError> {
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
        if self.next == u32::MAX {
            return Err(CoreError::new("pivot.id", "pivot id space is exhausted"));
        }
        Ok(())
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
        let next = if table.id.index() >= self.next {
            table
                .id
                .index()
                .checked_add(1)
                .ok_or_else(|| CoreError::new("pivot.id", "pivot id space is exhausted"))?
        } else {
            self.next
        };
        if let Some(existing) = self.tables.get(&table.id)
            && existing.name.to_lowercase() != nk
        {
            self.by_name.remove(&existing.name.to_lowercase());
        }
        self.by_name.insert(nk, table.id);
        self.next = next;
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
    /// Apply a percent number format to a numeric value.
    pub percent: bool,
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

/// Columnar pivot source reusable by analysis consumers.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotColumns {
    headers: Vec<String>,
    columns: Vec<Vec<CacheValue>>,
    by_name: BTreeMap<String, usize>,
    rows: usize,
}

impl PivotColumns {
    /// Build columns from cache rows, padding short rows with blanks.
    pub fn from_rows(headers: &[String], rows: &[Vec<CacheValue>]) -> Result<Self, CoreError> {
        if headers.is_empty() {
            return Err(CoreError::new(
                "pivot.source",
                "pivot source must contain a header row",
            ));
        }
        if headers.len() > usize::from(MAX_COLS) {
            return Err(CoreError::new("pivot.source", "pivot source is too wide"));
        }
        let mut by_name = BTreeMap::new();
        for (index, header) in headers.iter().enumerate() {
            if header.is_empty() || by_name.insert(header.clone(), index).is_some() {
                return Err(CoreError::new(
                    "pivot.field",
                    "pivot source headers must be non-empty and unique",
                ));
            }
        }
        if rows.iter().any(|row| row.len() > headers.len()) {
            return Err(CoreError::new(
                "pivot.source",
                "pivot cache record is wider than its field list",
            ));
        }
        let mut columns = vec![Vec::with_capacity(rows.len()); headers.len()];
        for row in rows {
            for (index, column) in columns.iter_mut().enumerate() {
                column.push(row.get(index).cloned().unwrap_or(CacheValue::Empty));
            }
        }
        Ok(Self {
            headers: headers.to_vec(),
            columns,
            by_name,
            rows: rows.len(),
        })
    }

    /// Source headers in column order.
    #[must_use]
    pub fn headers(&self) -> &[String] {
        &self.headers
    }

    /// Number of source records.
    #[must_use]
    pub const fn row_count(&self) -> usize {
        self.rows
    }

    /// One source column by exact header name.
    #[must_use]
    pub fn column(&self, name: &str) -> Option<&[CacheValue]> {
        self.by_name
            .get(name)
            .and_then(|index| self.columns.get(*index))
            .map(Vec::as_slice)
    }

    fn value(&self, row: usize, name: &str) -> Option<&CacheValue> {
        self.column(name).and_then(|column| column.get(row))
    }
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
    let columns = PivotColumns::from_rows(headers, rows)?;
    validate_definition(pivot, &columns)?;
    let filtered: Vec<usize> = (0..columns.row_count())
        .filter(|row| {
            pivot.filters.iter().all(|(name, allowed)| {
                allowed.is_empty()
                    || allowed.contains(&group_key(
                        &columns,
                        *row,
                        name,
                        pivot.groups.get(name).unwrap_or(&PivotGroup::None),
                        date_system,
                    ))
            })
        })
        .collect();
    let row_keys = unique_keys(&columns, &filtered, &pivot.rows, &pivot.groups, date_system);
    let col_keys = unique_keys(&columns, &filtered, &pivot.cols, &pivot.groups, date_system);
    validate_materialized_shape(
        pivot,
        row_keys.len(),
        col_keys.len(),
        compact_group_header_count(&row_keys, pivot),
    )?;
    let data_n = pivot.data.len().max(1);
    let mut raw: BTreeMap<(Vec<String>, Vec<String>, usize), Agg> = BTreeMap::new();
    for row in &filtered {
        let rk = keys_of(&columns, *row, &pivot.rows, &pivot.groups, date_system);
        let ck = keys_of(&columns, *row, &pivot.cols, &pivot.groups, date_system);
        for (di, df) in pivot.data.iter().enumerate() {
            let (num, text) = value_parts(columns.value(*row, &df.source));
            raw.entry((rk.clone(), ck.clone(), di))
                .or_default()
                .add(num, text.as_ref());
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
        u16::try_from(pivot.rows.len())
            .map_err(|_| CoreError::new("pivot.output", "too many pivot row fields"))?
    } else {
        1
    };
    let col_header_rows = if pivot.cols.is_empty() {
        1
    } else {
        u32::try_from(pivot.cols.len())
            .map_err(|_| CoreError::new("pivot.output", "too many pivot column fields"))?
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
    let mut prev_key: Option<&Vec<String>> = None;
    for (ri, rk) in row_keys.iter().enumerate() {
        if tabular {
            for (i, part) in rk.iter().enumerate() {
                let text = if matches!(pivot.layout, PivotLayout::Outline)
                    && i + 1 < rk.len()
                    && prev_key.is_some_and(|prev| prev.get(i) == Some(part))
                {
                    ""
                } else {
                    part.as_str()
                };
                cells.push(label(r, i as u16, text));
            }
        } else if rk.len() > 1 {
            for depth in 0..rk.len().saturating_sub(1) {
                let changed = prev_key.is_none_or(|prev| prev.get(depth) != rk.get(depth));
                if changed {
                    cells.push(label(r, 0, &compact_label(depth, &rk[depth])));
                    r += 1;
                }
            }
            if let Some(leaf) = rk.last() {
                cells.push(label(
                    r,
                    0,
                    &compact_label(rk.len().saturating_sub(1), leaf),
                ));
            }
        } else {
            cells.push(label(r, 0, rk.first().map(String::as_str).unwrap_or("")));
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
                let mut sub_grand = vec![Agg::default(); data_n];
                for ck in &col_keys {
                    for (di, total) in sub_grand.iter_mut().enumerate() {
                        if let Some(agg) = sub_agg.get(&(ck.clone(), di)) {
                            add_into(total, agg);
                        }
                    }
                }
                for (ci, ck) in col_keys.iter().enumerate() {
                    for (di, df) in pivot.data.iter().enumerate() {
                        let n = sub_agg
                            .get(&(ck.clone(), di))
                            .and_then(|a| a.finish(df.agg))
                            .and_then(|n| {
                                show_total(
                                    n,
                                    df.show_as,
                                    sub_grand.get(di).and_then(|a| a.finish(df.agg)),
                                    col_tot
                                        .get(ck)
                                        .and_then(|values| values.get(di))
                                        .and_then(|a| a.finish(df.agg)),
                                    grand.get(di).and_then(|a| a.finish(df.agg)),
                                )
                            });
                        cells.push(PivotCell {
                            row: r,
                            col: row_label_cols + (ci * data_n + di) as u16,
                            value: n.map(PivotValue::Number).unwrap_or(PivotValue::Empty),
                            header: false,
                            percent: is_percent(df.show_as),
                        });
                    }
                }
                if emit_grand_cols {
                    for (di, df) in pivot.data.iter().enumerate() {
                        cells.push(PivotCell {
                            row: r,
                            col: row_label_cols + (col_keys.len() * data_n + di) as u16,
                            value: sub_grand
                                .get(di)
                                .and_then(|total| total.finish(df.agg))
                                .and_then(|n| {
                                    show_total(
                                        n,
                                        df.show_as,
                                        Some(n),
                                        grand.get(di).and_then(|a| a.finish(df.agg)),
                                        grand.get(di).and_then(|a| a.finish(df.agg)),
                                    )
                                })
                                .map(PivotValue::Number)
                                .unwrap_or(PivotValue::Empty),
                            header: true,
                            percent: is_percent(df.show_as),
                        });
                    }
                }
                r += 1;
            }
        }
        prev_key = Some(rk);
    }
    if pivot.grand_rows && !pivot.rows.is_empty() {
        cells.push(label(r, 0, "Grand Total"));
        for (ci, ck) in col_keys.iter().enumerate() {
            for (di, df) in pivot.data.iter().enumerate() {
                let tot = col_tot
                    .get(ck)
                    .and_then(|v| v.get(di))
                    .and_then(|a| a.finish(df.agg))
                    .and_then(|n| {
                        show_total(
                            n,
                            df.show_as,
                            grand.get(di).and_then(|a| a.finish(df.agg)),
                            Some(n),
                            grand.get(di).and_then(|a| a.finish(df.agg)),
                        )
                    });
                cells.push(PivotCell {
                    row: r,
                    col: row_label_cols + (ci * data_n + di) as u16,
                    value: tot.map(PivotValue::Number).unwrap_or(PivotValue::Empty),
                    header: true,
                    percent: is_percent(df.show_as),
                });
            }
        }
        if emit_grand_cols {
            for (di, df) in pivot.data.iter().enumerate() {
                let g = grand
                    .get(di)
                    .and_then(|a| a.finish(df.agg))
                    .and_then(|n| show_total(n, df.show_as, Some(n), Some(n), Some(n)));
                cells.push(PivotCell {
                    row: r,
                    col: row_label_cols + (col_keys.len() * data_n + di) as u16,
                    value: g.map(PivotValue::Number).unwrap_or(PivotValue::Empty),
                    header: true,
                    percent: is_percent(df.show_as),
                });
            }
        }
    }
    if cells
        .iter()
        .any(|cell| matches!(&cell.value, PivotValue::Number(number) if !number.is_finite()))
    {
        return Err(CoreError::new(
            "pivot.number",
            "pivot aggregation produced a non-finite number",
        )
        .with_hint("check the source for values whose sum or variance overflows"));
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
                percent: is_percent(df.show_as),
            });
        }
        if pivot.data.is_empty() {
            cells.push(PivotCell {
                row: r,
                col: row_label_cols + ci as u16,
                value: PivotValue::Empty,
                header: false,
                percent: false,
            });
        }
    }
    if emit_grand_cols {
        for (di, df) in pivot.data.iter().enumerate() {
            let raw_total = row_tot
                .get(rk)
                .and_then(|v| v.get(di))
                .and_then(|a| a.finish(df.agg));
            let tot = raw_total.and_then(|n| {
                show_total(
                    n,
                    df.show_as,
                    Some(n),
                    grand.get(di).and_then(|a| a.finish(df.agg)),
                    grand.get(di).and_then(|a| a.finish(df.agg)),
                )
            });
            cells.push(PivotCell {
                row: r,
                col: row_label_cols + (col_keys.len() * data_n + di) as u16,
                value: tot.map(PivotValue::Number).unwrap_or(PivotValue::Empty),
                header: true,
                percent: is_percent(df.show_as),
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
    if wb.sheet(pivot.source_sheet).is_none() {
        return Err(CoreError::sheet_id(format!(
            "unknown pivot source sheet {}",
            pivot.source_sheet.index()
        )));
    }
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
    apply_calc_fields(pivot, &mut headers, &mut rows)?;
    Ok((headers, rows))
}

fn apply_calc_fields(
    pivot: &PivotTable,
    headers: &mut Vec<String>,
    rows: &mut [Vec<CacheValue>],
) -> Result<(), CoreError> {
    for field in &pivot.calc_fields {
        if field.name.is_empty() {
            return Err(CoreError::new(
                "pivot.field",
                "calculated field name is empty",
            ));
        }
        if headers.iter().any(|header| header == &field.name) {
            continue;
        }
        if headers.len() >= usize::from(MAX_COLS) {
            return Err(CoreError::new(
                "pivot.source",
                "pivot source is too wide to add a calculated field",
            ));
        }
        let by_name: BTreeMap<&str, usize> = headers
            .iter()
            .enumerate()
            .map(|(index, name)| (name.as_str(), index))
            .collect();
        for row in rows.iter_mut() {
            row.resize(headers.len(), CacheValue::Empty);
            let value = eval_calc_field(&field.formula, &by_name, row);
            row.push(value);
        }
        headers.push(field.name.clone());
    }
    Ok(())
}

struct CalcCtx<'a> {
    by_name: &'a BTreeMap<&'a str, usize>,
    row: &'a [CacheValue],
}

fn eval_calc_field(
    formula: &str,
    by_name: &BTreeMap<&str, usize>,
    row: &[CacheValue],
) -> CacheValue {
    let expr = formula.trim().strip_prefix('=').unwrap_or(formula).trim();
    let ctx = CalcCtx { by_name, row };
    match parse_calc_add(&ctx, expr, 0) {
        Ok((value, rest)) if rest.trim().is_empty() => value
            .and_then(|n| n.is_finite().then_some(CacheValue::Number(n)))
            .unwrap_or(CacheValue::Empty),
        _ => CacheValue::Empty,
    }
}

fn parse_calc_add<'a>(
    ctx: &CalcCtx<'_>,
    input: &'a str,
    depth: u32,
) -> Result<(Option<f64>, &'a str), ()> {
    if depth > 16 {
        return Err(());
    }
    let (mut acc, mut rest) = parse_calc_mul(ctx, input, depth + 1)?;
    loop {
        let next = rest.trim_start();
        if let Some(tail) = next.strip_prefix('+') {
            let (rhs, after) = parse_calc_mul(ctx, tail, depth + 1)?;
            acc = acc.zip(rhs).map(|(a, b)| a + b);
            rest = after;
        } else if let Some(tail) = next.strip_prefix('-') {
            let (rhs, after) = parse_calc_mul(ctx, tail, depth + 1)?;
            acc = acc.zip(rhs).map(|(a, b)| a - b);
            rest = after;
        } else {
            return Ok((acc, rest));
        }
    }
}

fn parse_calc_mul<'a>(
    ctx: &CalcCtx<'_>,
    input: &'a str,
    depth: u32,
) -> Result<(Option<f64>, &'a str), ()> {
    if depth > 16 {
        return Err(());
    }
    let (mut acc, mut rest) = parse_calc_atom(ctx, input, depth + 1)?;
    loop {
        let next = rest.trim_start();
        if let Some(tail) = next.strip_prefix('*') {
            let (rhs, after) = parse_calc_atom(ctx, tail, depth + 1)?;
            acc = acc.zip(rhs).map(|(a, b)| a * b);
            rest = after;
        } else if let Some(tail) = next.strip_prefix('/') {
            let (rhs, after) = parse_calc_atom(ctx, tail, depth + 1)?;
            acc = match (acc, rhs) {
                (Some(a), Some(b)) if b != 0.0 => Some(a / b),
                _ => None,
            };
            rest = after;
        } else {
            return Ok((acc, rest));
        }
    }
}

fn parse_calc_atom<'a>(
    ctx: &CalcCtx<'_>,
    input: &'a str,
    depth: u32,
) -> Result<(Option<f64>, &'a str), ()> {
    if depth > 16 {
        return Err(());
    }
    let input = input.trim_start();
    if let Some(tail) = input.strip_prefix('(') {
        let (value, rest) = parse_calc_add(ctx, tail, depth + 1)?;
        let rest = rest.trim_start().strip_prefix(')').ok_or(())?;
        return Ok((value, rest));
    }
    if let Some(tail) = input.strip_prefix('-')
        && (tail.starts_with(|c: char| {
            c.is_ascii_digit()
                || c == '.'
                || c == '('
                || c == '\''
                || c.is_ascii_alphabetic()
                || c == '_'
        }))
    {
        let (value, rest) = parse_calc_atom(ctx, tail, depth + 1)?;
        return Ok((value.map(|n| -n), rest));
    }
    if let Some(tail) = input.strip_prefix('\'') {
        let end = tail.find('\'').ok_or(())?;
        let name = &tail[..end];
        return Ok((lookup_calc_number(ctx, name), &tail[end + 1..]));
    }
    if input.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        let bytes = input.as_bytes();
        let mut end = 0usize;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b'.' {
            end += 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
        }
        let number: f64 = input.get(..end).ok_or(())?.parse().map_err(|_| ())?;
        return Ok((number.is_finite().then_some(number), &input[end..]));
    }
    if input.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        let end = input
            .char_indices()
            .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_')
            .map(|(i, c)| i + c.len_utf8())
            .last()
            .unwrap_or(0);
        let name = &input[..end];
        return Ok((lookup_calc_number(ctx, name), &input[end..]));
    }
    Err(())
}

fn lookup_calc_number(ctx: &CalcCtx<'_>, name: &str) -> Option<f64> {
    let index = ctx.by_name.get(name).copied()?;
    match ctx.row.get(index) {
        Some(CacheValue::Number(n)) if n.is_finite() => Some(*n),
        _ => None,
    }
}

/// Write materialized cells onto the destination sheet and update the output rectangle.
pub fn write_output(
    wb: &mut Workbook,
    pivot: &mut PivotTable,
    cells: &[PivotCell],
) -> Result<(), CoreError> {
    let registered = validate_output(wb, pivot, cells)?;
    wb.transact_try(|workbook| write_output_inner(workbook, pivot, cells, registered))
}

fn write_output_inner(
    wb: &mut Workbook,
    pivot: &mut PivotTable,
    cells: &[PivotCell],
    registered: bool,
) -> Result<(), CoreError> {
    if registered && pivot.out_end_row >= pivot.dest_row && pivot.out_end_col >= pivot.dest_col {
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
    let percent_fmt = wb.intern_num_fmt("0.00%")?;
    let percent_sid = wb.intern_style(Style {
        num_fmt: percent_fmt,
        ..Style::default()
    });
    let header_percent_sid = wb.intern_style(Style {
        font: Font {
            bold: true,
            ..Font::default()
        },
        num_fmt: percent_fmt,
        ..Style::default()
    });
    let mut max_r = pivot.dest_row;
    let mut max_c = pivot.dest_col;
    for cell in cells {
        let row = pivot.dest_row + cell.row;
        let col = pivot.dest_col + cell.col;
        max_r = max_r.max(row);
        max_c = max_c.max(col);
        let style = match (cell.header, cell.percent) {
            (true, true) => header_percent_sid,
            (true, false) => header_sid,
            (false, true) => percent_sid,
            (false, false) => StyleId::DEFAULT,
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
    wb.release_style(percent_sid);
    wb.release_style(header_percent_sid);
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
        percent: false,
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
        ShowAs::PctOfTotal => grand.filter(|g| *g != 0.0).map(|g| n / g),
        ShowAs::PctOfRow => row_tot
            .get(rk)
            .and_then(|v| v.get(di))
            .and_then(|a| a.finish(agg))
            .filter(|g| *g != 0.0)
            .map(|g| n / g),
        ShowAs::PctOfCol => col_tot
            .get(ck)
            .and_then(|v| v.get(di))
            .and_then(|a| a.finish(agg))
            .filter(|g| *g != 0.0)
            .map(|g| n / g),
        ShowAs::RunningTotal => {
            let mut acc = 0.0;
            let prefix = rk.split_last().map(|(_, prefix)| prefix).unwrap_or(&[]);
            for key in row_keys.iter().take(ri + 1) {
                if key
                    .split_last()
                    .map(|(_, candidate)| candidate)
                    .unwrap_or(&[])
                    != prefix
                {
                    continue;
                }
                acc += raw
                    .get(&(key.clone(), ck.to_vec(), di))
                    .and_then(|a| a.finish(agg))
                    .unwrap_or(0.0);
            }
            Some(acc)
        }
        ShowAs::DifferenceFrom => {
            let previous = ri.checked_sub(1).and_then(|index| row_keys.get(index))?;
            if previous.split_last().map(|(_, prefix)| prefix)
                != rk.split_last().map(|(_, prefix)| prefix)
            {
                return None;
            }
            let prev = raw
                .get(&(previous.clone(), ck.to_vec(), di))
                .and_then(|a| a.finish(agg))
                .unwrap_or(0.0);
            Some(n - prev)
        }
    }
}

fn is_percent(mode: ShowAs) -> bool {
    matches!(
        mode,
        ShowAs::PctOfTotal | ShowAs::PctOfRow | ShowAs::PctOfCol
    )
}

fn show_total(
    value: f64,
    mode: ShowAs,
    row_total: Option<f64>,
    col_total: Option<f64>,
    grand: Option<f64>,
) -> Option<f64> {
    let denominator = match mode {
        ShowAs::PctOfTotal => grand,
        ShowAs::PctOfRow => row_total,
        ShowAs::PctOfCol => col_total,
        ShowAs::Normal | ShowAs::RunningTotal | ShowAs::DifferenceFrom => return Some(value),
    }?;
    (denominator != 0.0).then_some(value / denominator)
}

fn validate_definition(pivot: &PivotTable, columns: &PivotColumns) -> Result<(), CoreError> {
    let available: BTreeSet<&str> = columns.headers().iter().map(String::as_str).collect();
    let mut axes = BTreeSet::new();
    for name in pivot
        .rows
        .iter()
        .chain(&pivot.cols)
        .chain(pivot.filters.iter().map(|(name, _)| name))
    {
        ensure_field(&available, name)?;
        if !axes.insert(name.as_str()) {
            return Err(CoreError::new(
                "pivot.field",
                format!("pivot field {name:?} is assigned to more than one axis"),
            ));
        }
    }
    for field in &pivot.data {
        ensure_field(&available, &field.source)?;
    }
    for (name, group) in &pivot.groups {
        ensure_field(&available, name)?;
        if let PivotGroup::Numeric { start, size } = group
            && (!start.is_finite() || !size.is_finite() || *size <= 0.0)
        {
            return Err(CoreError::new(
                "pivot.group",
                format!("numeric grouping for {name:?} needs a finite start and positive size"),
            ));
        }
    }
    Ok(())
}

fn ensure_field(available: &BTreeSet<&str>, name: &str) -> Result<(), CoreError> {
    if available.contains(name) {
        Ok(())
    } else {
        Err(CoreError::new(
            "pivot.field",
            format!("unknown pivot source field {name:?}"),
        ))
    }
}

fn compact_label(depth: usize, text: &str) -> String {
    format!("{}{text}", "  ".repeat(depth))
}

fn compact_group_header_count(row_keys: &[Vec<String>], pivot: &PivotTable) -> usize {
    if !matches!(pivot.layout, PivotLayout::Compact) || pivot.rows.len() <= 1 {
        return 0;
    }
    let mut count = 0;
    let mut prev: Option<&Vec<String>> = None;
    for rk in row_keys {
        if rk.len() <= 1 {
            continue;
        }
        for depth in 0..rk.len().saturating_sub(1) {
            if prev.is_none_or(|p| p.get(depth) != rk.get(depth)) {
                count += 1;
            }
        }
        prev = Some(rk);
    }
    count
}

fn validate_materialized_shape(
    pivot: &PivotTable,
    row_keys: usize,
    col_keys: usize,
    extra_row_headers: usize,
) -> Result<(), CoreError> {
    let row_label_cols = if pivot.rows.is_empty() {
        1
    } else if matches!(pivot.layout, PivotLayout::Tabular | PivotLayout::Outline) {
        pivot.rows.len()
    } else {
        1
    };
    let data_n = pivot.data.len().max(1);
    let data_cols = col_keys
        .checked_mul(data_n)
        .and_then(|n| {
            n.checked_add(if pivot.grand_cols && !pivot.cols.is_empty() {
                data_n
            } else {
                0
            })
        })
        .ok_or_else(|| CoreError::new("pivot.output", "pivot output width overflows"))?;
    let cols = row_label_cols
        .checked_add(data_cols.max(data_n))
        .ok_or_else(|| CoreError::new("pivot.output", "pivot output width overflows"))?;
    let header_rows = pivot.cols.len().max(1);
    let subtotal_rows = if pivot.subtotals && pivot.rows.len() > 1 {
        row_keys
    } else {
        0
    };
    let rows = header_rows
        .checked_add(row_keys)
        .and_then(|n| n.checked_add(extra_row_headers))
        .and_then(|n| n.checked_add(subtotal_rows))
        .and_then(|n| n.checked_add(usize::from(pivot.grand_rows && !pivot.rows.is_empty())))
        .ok_or_else(|| CoreError::new("pivot.output", "pivot output height overflows"))?;
    let cells = rows
        .checked_mul(cols)
        .ok_or_else(|| CoreError::new("pivot.output", "pivot output area overflows"))?;
    if cols > usize::from(MAX_COLS) || rows > MAX_ROWS as usize {
        return Err(CoreError::new(
            "pivot.output",
            "pivot output exceeds the worksheet grid",
        ));
    }
    if cells > MAX_PIVOT_OUTPUT_CELLS {
        return Err(CoreError::new(
            "pivot.output",
            format!("pivot output has {cells} cells; maximum is {MAX_PIVOT_OUTPUT_CELLS}"),
        )
        .with_hint("filter the source or use fewer row and column fields"));
    }
    Ok(())
}

fn validate_output(
    wb: &Workbook,
    pivot: &PivotTable,
    cells: &[PivotCell],
) -> Result<bool, CoreError> {
    let sheet = wb.sheet(pivot.dest_sheet).ok_or_else(|| {
        CoreError::sheet_id(format!(
            "unknown pivot output sheet {}",
            pivot.dest_sheet.index()
        ))
    })?;
    let mut end_row = pivot.dest_row;
    let mut end_col = pivot.dest_col;
    for cell in cells {
        let row = pivot.dest_row.checked_add(cell.row).ok_or_else(|| {
            CoreError::new(
                "pivot.output",
                "pivot output row exceeds the worksheet grid",
            )
        })?;
        let col = pivot.dest_col.checked_add(cell.col).ok_or_else(|| {
            CoreError::new(
                "pivot.output",
                "pivot output column exceeds the worksheet grid",
            )
        })?;
        if row >= MAX_ROWS || col >= MAX_COLS {
            return Err(CoreError::new(
                "pivot.output",
                "pivot output exceeds the worksheet grid",
            ));
        }
        end_row = end_row.max(row);
        end_col = end_col.max(col);
    }
    let registered = wb
        .pivots()
        .get(pivot.id)
        .is_some_and(|current| current.name.eq_ignore_ascii_case(&pivot.name));
    if pivot.dest_sheet == pivot.source_sheet {
        let (sr0, sc0, sr1, sc1) = norm(pivot.source);
        if rectangles_overlap(
            pivot.dest_row,
            pivot.dest_col,
            end_row,
            end_col,
            sr0,
            sc0,
            sr1,
            sc1,
        ) {
            return Err(CoreError::new(
                "pivot.output",
                "pivot output overlaps its source range",
            ));
        }
    }
    for existing in wb.pivots().iter() {
        if registered && existing.id == pivot.id {
            continue;
        }
        if existing.dest_sheet == pivot.dest_sheet
            && rectangles_overlap(
                pivot.dest_row,
                pivot.dest_col,
                end_row,
                end_col,
                existing.dest_row,
                existing.dest_col,
                existing.out_end_row,
                existing.out_end_col,
            )
        {
            return Err(CoreError::new(
                "pivot.output",
                format!("pivot output overlaps pivot {:?}", existing.name),
            ));
        }
    }
    for table in wb.tables().iter() {
        if table.sheet == pivot.dest_sheet
            && rectangles_overlap(
                pivot.dest_row,
                pivot.dest_col,
                end_row,
                end_col,
                table.start_row,
                table.start_col,
                table.end_row,
                table.end_col,
            )
        {
            return Err(CoreError::new(
                "pivot.output",
                format!("pivot output overlaps table {:?}", table.name),
            ));
        }
    }
    if sheet.merges.iter().any(|range| {
        let (r0, c0, r1, c1) = norm(*range);
        rectangles_overlap(
            pivot.dest_row,
            pivot.dest_col,
            end_row,
            end_col,
            r0,
            c0,
            r1,
            c1,
        )
    }) {
        return Err(CoreError::new(
            "pivot.output",
            "pivot output overlaps merged cells",
        ));
    }
    if let Some((row, col, _)) = sheet
        .store
        .iter_region(pivot.dest_row, pivot.dest_col, end_row, end_col)
        .find(|(row, col, _)| {
            !registered
                || *row < pivot.dest_row
                || *row > pivot.out_end_row
                || *col < pivot.dest_col
                || *col > pivot.out_end_col
        })
    {
        return Err(CoreError::new(
            "pivot.output",
            format!("pivot output would overwrite occupied cell at row {row}, column {col}"),
        ));
    }
    Ok(registered)
}

#[allow(clippy::too_many_arguments)]
fn rectangles_overlap(
    ar0: u32,
    ac0: u16,
    ar1: u32,
    ac1: u16,
    br0: u32,
    bc0: u16,
    br1: u32,
    bc1: u16,
) -> bool {
    ar0 <= br1 && br0 <= ar1 && ac0 <= bc1 && bc0 <= ac1
}

#[derive(Clone, Copy)]
pub(crate) enum PivotShiftBound {
    SourceHeader,
    OutputOrigin,
}

pub(crate) fn shift_closed_interval(
    start: u32,
    end: u32,
    at: u32,
    count: i32,
    bound: PivotShiftBound,
    max: u32,
) -> Result<(u32, u32), CoreError> {
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    if count == 0 {
        return Ok((start, end));
    }
    if count > 0 {
        let n = u32::try_from(count).map_err(|_| struct_err("insert count is too large"))?;
        let (next_start, next_end) = if at <= start {
            (
                start
                    .checked_add(n)
                    .ok_or_else(|| struct_err("row insert overflows the grid"))?,
                end.checked_add(n)
                    .ok_or_else(|| struct_err("row insert overflows the grid"))?,
            )
        } else if at <= end {
            (
                start,
                end.checked_add(n)
                    .ok_or_else(|| struct_err("row insert overflows the grid"))?,
            )
        } else {
            (start, end)
        };
        if next_end >= max {
            return Err(struct_err(
                "structural edit would push a pivot off the grid",
            ));
        }
        return Ok((next_start, next_end));
    }
    let mag = count.unsigned_abs();
    let deleted_end = at.saturating_add(mag);
    if deleted_end <= start {
        return Ok((start.saturating_sub(mag), end.saturating_sub(mag)));
    }
    if at > end {
        return Ok((start, end));
    }
    let origin_deleted = start >= at && start < deleted_end;
    if origin_deleted {
        return Err(match bound {
            PivotShiftBound::SourceHeader => {
                struct_err("cannot delete the header row or column of a pivot source")
            }
            PivotShiftBound::OutputOrigin => {
                struct_err("cannot delete the origin of a pivot output")
            }
        });
    }
    if at <= start && deleted_end > end {
        return Err(struct_err("structural edit would delete a pivot range"));
    }
    let overlap = {
        let lo = start.max(at);
        let hi = (end + 1).min(deleted_end);
        hi.saturating_sub(lo)
    };
    let next_start = if start >= deleted_end {
        start - mag
    } else {
        start
    };
    let next_end = end.saturating_sub(overlap);
    if next_end < next_start {
        return Err(struct_err("structural edit would delete a pivot range"));
    }
    Ok((next_start, next_end))
}

pub(crate) fn shift_closed_interval_u16(
    start: u16,
    end: u16,
    at: u16,
    count: i32,
    bound: PivotShiftBound,
    max: u16,
) -> Result<(u16, u16), CoreError> {
    let (s, e) = shift_closed_interval(
        u32::from(start),
        u32::from(end),
        u32::from(at),
        count,
        bound,
        u32::from(max),
    )?;
    Ok((
        u16::try_from(s).map_err(|_| struct_err("column shift overflows"))?,
        u16::try_from(e).map_err(|_| struct_err("column shift overflows"))?,
    ))
}

fn struct_err(message: &str) -> CoreError {
    CoreError::new("pivot.struct", message)
        .with_hint("insert or delete fully before, inside, or after the pivot source and output")
}

pub(crate) fn band_covers_cols(bc0: u16, bc1: u16, c0: u16, c1: u16) -> bool {
    bc0 <= c0 && bc1 >= c1
}

pub(crate) fn band_covers_rows(br0: u32, br1: u32, r0: u32, r1: u32) -> bool {
    br0 <= r0 && br1 >= r1
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BandAction {
    None,
    Shift,
    Refuse,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn row_band_action(
    r0: u32,
    c0: u16,
    r1: u32,
    c1: u16,
    at: u32,
    count: i32,
    bc0: u16,
    bc1: u16,
) -> BandAction {
    let cols_overlap = c0 <= bc1 && bc0 <= c1;
    if !cols_overlap {
        return BandAction::None;
    }
    let mag = count.unsigned_abs();
    let rows_overlap = if count >= 0 {
        r1 >= at
    } else {
        let deleted_end = at.saturating_add(mag).saturating_sub(1);
        r0 <= deleted_end && at <= r1
    };
    if !rows_overlap {
        return BandAction::None;
    }
    if band_covers_cols(bc0, bc1, c0, c1) {
        BandAction::Shift
    } else {
        BandAction::Refuse
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn col_band_action(
    r0: u32,
    c0: u16,
    r1: u32,
    c1: u16,
    at: u16,
    count: i32,
    br0: u32,
    br1: u32,
) -> BandAction {
    let rows_overlap = r0 <= br1 && br0 <= r1;
    if !rows_overlap {
        return BandAction::None;
    }
    let mag = count.unsigned_abs() as u16;
    let cols_overlap = if count >= 0 {
        c1 >= at
    } else {
        let deleted_end = at.saturating_add(mag).saturating_sub(1);
        c0 <= deleted_end && at <= c1
    };
    if !cols_overlap {
        return BandAction::None;
    }
    if band_covers_rows(br0, br1, r0, r1) {
        BandAction::Shift
    } else {
        BandAction::Refuse
    }
}

pub(crate) fn split_err(name: &str) -> CoreError {
    CoreError::new(
        "pivot.struct",
        format!("structural edit would split pivot {name:?}"),
    )
    .with_hint("insert or delete the full rows or columns of the pivot source and output")
}

pub(crate) fn shift_pivot_rows(
    pivot: &PivotTable,
    sheet: SheetId,
    at: u32,
    count: i32,
    c0: u16,
    c1: u16,
) -> Result<Option<PivotTable>, CoreError> {
    let mut next = pivot.clone();
    let mut changed = false;
    if pivot.source_sheet == sheet {
        let (sr0, sc0, sr1, sc1) = norm(pivot.source);
        match row_band_action(sr0, sc0, sr1, sc1, at, count, c0, c1) {
            BandAction::None => {}
            BandAction::Refuse => return Err(split_err(&pivot.name)),
            BandAction::Shift => {
                let (nr0, nr1) = shift_closed_interval(
                    sr0,
                    sr1,
                    at,
                    count,
                    PivotShiftBound::SourceHeader,
                    MAX_ROWS,
                )?;
                next.source.start.row = nr0;
                next.source.end.row = nr1;
                changed = true;
            }
        }
    }
    if pivot.dest_sheet == sheet {
        match row_band_action(
            pivot.dest_row,
            pivot.dest_col,
            pivot.out_end_row,
            pivot.out_end_col,
            at,
            count,
            c0,
            c1,
        ) {
            BandAction::None => {}
            BandAction::Refuse => return Err(split_err(&pivot.name)),
            BandAction::Shift => {
                let (nr0, nr1) = shift_closed_interval(
                    pivot.dest_row,
                    pivot.out_end_row,
                    at,
                    count,
                    PivotShiftBound::OutputOrigin,
                    MAX_ROWS,
                )?;
                next.dest_row = nr0;
                next.out_end_row = nr1;
                changed = true;
            }
        }
    }
    if changed {
        next.ooxml_dirty = true;
        Ok(Some(next))
    } else {
        Ok(None)
    }
}

pub(crate) fn shift_pivot_cols(
    pivot: &PivotTable,
    sheet: SheetId,
    at: u16,
    count: i32,
    r0: u32,
    r1: u32,
) -> Result<Option<PivotTable>, CoreError> {
    let mut next = pivot.clone();
    let mut changed = false;
    if pivot.source_sheet == sheet {
        let (sr0, sc0, sr1, sc1) = norm(pivot.source);
        match col_band_action(sr0, sc0, sr1, sc1, at, count, r0, r1) {
            BandAction::None => {}
            BandAction::Refuse => return Err(split_err(&pivot.name)),
            BandAction::Shift => {
                let (nc0, nc1) = shift_closed_interval_u16(
                    sc0,
                    sc1,
                    at,
                    count,
                    PivotShiftBound::SourceHeader,
                    MAX_COLS,
                )?;
                next.source.start.col = nc0;
                next.source.end.col = nc1;
                changed = true;
            }
        }
    }
    if pivot.dest_sheet == sheet {
        match col_band_action(
            pivot.dest_row,
            pivot.dest_col,
            pivot.out_end_row,
            pivot.out_end_col,
            at,
            count,
            r0,
            r1,
        ) {
            BandAction::None => {}
            BandAction::Refuse => return Err(split_err(&pivot.name)),
            BandAction::Shift => {
                let (nc0, nc1) = shift_closed_interval_u16(
                    pivot.dest_col,
                    pivot.out_end_col,
                    at,
                    count,
                    PivotShiftBound::OutputOrigin,
                    MAX_COLS,
                )?;
                next.dest_col = nc0;
                next.out_end_col = nc1;
                changed = true;
            }
        }
    }
    if changed {
        next.ooxml_dirty = true;
        Ok(Some(next))
    } else {
        Ok(None)
    }
}

fn value_parts(value: Option<&CacheValue>) -> (Option<f64>, std::borrow::Cow<'_, str>) {
    match value {
        Some(CacheValue::Number(number)) => (Some(*number), number.to_string().into()),
        Some(CacheValue::Text(text)) => (None, text.as_str().into()),
        Some(CacheValue::Empty) | None => (None, "".into()),
    }
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
    columns: &PivotColumns,
    rows: &[usize],
    fields: &[String],
    groups: &BTreeMap<String, PivotGroup>,
    date_system: DateSystem,
) -> Vec<Vec<String>> {
    let mut set = BTreeMap::new();
    for row in rows {
        let display = keys_of(columns, *row, fields, groups, date_system);
        let sort = fields
            .iter()
            .map(|field| {
                group_sort_key(
                    columns,
                    *row,
                    field,
                    groups.get(field).unwrap_or(&PivotGroup::None),
                    date_system,
                )
            })
            .collect::<Vec<_>>();
        set.entry(display).or_insert(sort);
    }
    let mut keys: Vec<_> = set.into_iter().collect();
    keys.sort_by(|(left_display, left_sort), (right_display, right_sort)| {
        for ((left, left_text), (right, right_text)) in left_sort
            .iter()
            .zip(left_display)
            .zip(right_sort.iter().zip(right_display))
        {
            let ordering = match (left, right) {
                (Some(a), Some(b)) => a.total_cmp(b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => left_text.cmp(right_text),
            };
            if !ordering.is_eq() {
                return ordering;
            }
        }
        left_display.cmp(right_display)
    });
    keys.into_iter().map(|(display, _)| display).collect()
}

fn keys_of(
    columns: &PivotColumns,
    row: usize,
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
                columns,
                row,
                f,
                groups.get(f).unwrap_or(&PivotGroup::None),
                date_system,
            )
        })
        .collect()
}

fn group_key(
    columns: &PivotColumns,
    row: usize,
    field: &str,
    group: &PivotGroup,
    date_system: DateSystem,
) -> String {
    let (num, text) = value_parts(columns.value(row, field));
    match group {
        PivotGroup::None => {
            if let Some(n) = num {
                n.to_string()
            } else {
                text.into_owned()
            }
        }
        PivotGroup::Date(g) => {
            let Some(n) = num else {
                return text.into_owned();
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
                return text.into_owned();
            };
            if size.is_finite() && *size > 0.0 && start.is_finite() {
                let i = ((n - start) / size).floor();
                let bin = start + i * size;
                if bin.is_finite() {
                    bin.to_string()
                } else {
                    n.to_string()
                }
            } else {
                n.to_string()
            }
        }
    }
}

fn group_sort_key(
    columns: &PivotColumns,
    row: usize,
    field: &str,
    group: &PivotGroup,
    date_system: DateSystem,
) -> Option<f64> {
    let (number, _) = value_parts(columns.value(row, field));
    let number = number?;
    match group {
        PivotGroup::None => Some(number),
        PivotGroup::Numeric { start, size }
            if start.is_finite() && size.is_finite() && *size > 0.0 =>
        {
            let bin = start + ((number - start) / size).floor() * size;
            bin.is_finite().then_some(bin)
        }
        PivotGroup::Date(group) => {
            let date = serial_to_date(number.trunc() as i64, date_system)?;
            Some(match group {
                DateGroup::Years => f64::from(date.year),
                DateGroup::Quarters => f64::from(date.year) * 4.0 + f64::from((date.month - 1) / 3),
                DateGroup::Months => f64::from(date.year) * 12.0 + f64::from(date.month - 1),
                DateGroup::Days => number.trunc(),
            })
        }
        PivotGroup::Numeric { .. } => None,
    }
}

pub(crate) fn norm(r: RangeRef) -> (u32, u16, u32, u16) {
    (
        r.start.row.min(r.end.row),
        r.start.col.min(r.end.col),
        r.start.row.max(r.end.row),
        r.start.col.max(r.end.col),
    )
}
