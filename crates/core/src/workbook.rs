//! In-memory workbook (spec F-1, §11.3).
//!
//! Single-writer. [`Workbook::snapshot`] is a cheap copy-on-write view for
//! readers (render during recalc, §11.5).

use std::sync::Arc;

use indexmap::IndexMap;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use std::borrow::Cow;

use crate::addr::{CellRef, ParsedRef, RefKind, SheetId, SheetSpec};
use crate::chart::{Chart, ChartId, Sparkline};
pub use crate::date_system::DateSystem;
use crate::error::CoreError;
use crate::intern::{ArrayPayload, FormulaId, Interners, RichTextRun};
use crate::locale::LocaleId;
use crate::names::{DefinedName, NameRegistry, NameScope};
use crate::numfmt;
use crate::pivot::{
    CacheValue, PivotId, PivotRegistry, PivotTable, materialize, materialize_from_cache,
    write_output,
};
use crate::print::PageSetup;
use crate::sheet::{
    Comment, Hyperlink, Note, ProtectionState, Sheet, SheetEditState, SheetVisibility, ViewState,
    validate_sheet_name,
};
use crate::storage::{CellFlags, CellSlot, UsedRange};
use crate::style::{Color, NumFmtId, Style, StyleId};
use crate::tables::{Table, TableId, TableRegistry};
use crate::undo::{AffectedRange, Delta, UndoLog, transaction_affected};
use crate::value::{ArrayId, StrId, Value};

/// First custom `numFmtId` (Excel custom formats start at 164).
const CUSTOM_NUM_FMT_START: u32 = 164;

/// Calculation mode (F-1.6).
///
/// ```
/// use omacell_core::workbook::CalcMode;
/// assert_eq!(CalcMode::default(), CalcMode::Automatic);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalcMode {
    /// Recalc after every change (WP-04).
    #[default]
    Automatic,
    /// Automatic except tables.
    AutomaticExceptTables,
    /// Manual.
    Manual,
}

/// Iterative calculation settings.
///
/// ```
/// use omacell_core::workbook::Iteration;
/// let i = Iteration::default();
/// assert!(!i.enabled);
/// assert_eq!(i.max_iterations, 100);
/// ```
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Iteration {
    /// Iterative calc on.
    pub enabled: bool,
    /// Max iterations (Excel default 100).
    pub max_iterations: u32,
    /// Max change (Excel default 0.001).
    pub max_change: f64,
}

impl Default for Iteration {
    fn default() -> Self {
        Self {
            enabled: false,
            max_iterations: 100,
            max_change: 0.001,
        }
    }
}

impl PartialEq for Iteration {
    fn eq(&self, other: &Self) -> bool {
        self.enabled == other.enabled
            && self.max_iterations == other.max_iterations
            && self.max_change.to_bits() == other.max_change.to_bits()
    }
}

impl Eq for Iteration {}

/// Per-workbook calculation and date settings (F-1.6, F-2.6).
///
/// ```
/// use omacell_core::workbook::WorkbookSettings;
/// assert!(!WorkbookSettings::default().precision_as_displayed);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkbookSettings {
    /// Date serial system.
    pub date_system: DateSystem,
    /// Recalc mode.
    pub calc_mode: CalcMode,
    /// Iterative calculation.
    pub iteration: Iteration,
    /// Display-precision rounding (off by default).
    pub precision_as_displayed: bool,
}

/// Document properties.
///
/// ```
/// use omacell_core::workbook::WorkbookMeta;
/// let m = WorkbookMeta::default();
/// assert!(m.title.is_none());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkbookMeta {
    /// Title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Custom properties (ordered).
    #[serde(default)]
    pub custom: IndexMap<String, String>,
}

/// Legacy workbook-level protection flags (compatibility, not encryption).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkbookProtectionState {
    /// Protection record is enabled.
    pub enabled: bool,
    /// Prevent sheet add/remove/reorder/rename.
    #[serde(default)]
    pub lock_structure: bool,
    /// Prevent workbook-window changes.
    #[serde(default)]
    pub lock_windows: bool,
    /// OOXML legacy password verifier as uppercase ASCII hex bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<Vec<u8>>,
}

/// Cheap reader snapshot (Arc block pages + intern tables).
///
/// Mutating the originating [`Workbook`] does not change this view.
#[derive(Clone, Debug)]
pub struct WorkbookSnapshot {
    sheets: IndexMap<SheetId, Sheet>,
    names: NameRegistry,
    tables: crate::tables::TableRegistry,
    pivots: crate::pivot::PivotRegistry,
    intern: Arc<Interners>,
    settings: WorkbookSettings,
    protection: WorkbookProtectionState,
}

impl WorkbookSnapshot {
    /// Ordered sheets.
    pub fn sheets(&self) -> impl Iterator<Item = &Sheet> {
        self.sheets.values()
    }

    /// Sheet by id.
    #[must_use]
    pub fn sheet(&self, id: SheetId) -> Option<&Sheet> {
        self.sheets.get(&id)
    }

    /// Intern tables as of the snapshot.
    #[must_use]
    pub fn intern(&self) -> &Interners {
        &self.intern
    }

    /// Defined names.
    #[must_use]
    pub fn names(&self) -> &NameRegistry {
        &self.names
    }

    /// Tables as of the snapshot.
    #[must_use]
    pub fn tables(&self) -> &crate::tables::TableRegistry {
        &self.tables
    }

    /// Pivot tables as of the snapshot.
    #[must_use]
    pub fn pivots(&self) -> &crate::pivot::PivotRegistry {
        &self.pivots
    }

    /// Settings.
    #[must_use]
    pub fn settings(&self) -> &WorkbookSettings {
        &self.settings
    }

    /// Workbook protection as of the snapshot.
    #[must_use]
    pub fn protection(&self) -> &WorkbookProtectionState {
        &self.protection
    }

    /// Get a cell slot.
    pub fn get(&self, id: SheetId, row: u32, col: u16) -> Result<Option<&CellSlot>, CoreError> {
        let sheet = self
            .sheets
            .get(&id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))?;
        sheet.store.get(row, col)
    }
}

/// In-memory workbook.
///
/// ```
/// use omacell_core::storage::CellSlot;
/// use omacell_core::workbook::Workbook;
/// let mut wb = Workbook::new();
/// let id = wb.active_sheet();
/// wb.set_number(id, 0, 0, 1.5).unwrap();
/// assert_eq!(wb.get(id, 0, 0).unwrap().unwrap().value, omacell_core::value::Value::Number(1.5));
/// wb.undo().unwrap();
/// assert!(wb.get(id, 0, 0).unwrap().is_none());
/// ```
#[derive(Clone, Debug)]
pub struct Workbook {
    sheets: IndexMap<SheetId, Sheet>,
    names_by_lower: FxHashMap<String, SheetId>,
    intern: Arc<Interners>,
    names: NameRegistry,
    tables: TableRegistry,
    pivots: PivotRegistry,
    settings: WorkbookSettings,
    protection: WorkbookProtectionState,
    meta: WorkbookMeta,
    /// Opaque extra parts for WP-10 (e.g. unused OOXML).
    pub custom_parts: IndexMap<String, Vec<u8>>,
    undo: UndoLog,
    next_sheet: u32,
    active: SheetId,
    /// Custom number-format codes keyed by `numFmtId` (≥ 164).
    num_fmts: IndexMap<u32, String>,
    next_num_fmt: u32,
    ref_errors: u64,
}

impl Default for Workbook {
    fn default() -> Self {
        Self::new()
    }
}

impl Workbook {
    /// One visible sheet named `Sheet1`.
    #[must_use]
    pub fn new() -> Self {
        let id = SheetId::new(0);
        let sheet = match Sheet::new(id, "Sheet1") {
            Ok(s) => s,
            Err(_) => Sheet {
                id,
                name: "Sheet1".into(),
                visibility: SheetVisibility::Visible,
                tab_color: None,
                view: ViewState::default(),
                protection: ProtectionState::default(),
                merges: Vec::new(),
                notes: FxHashMap::default(),
                comments: FxHashMap::default(),
                hyperlinks: FxHashMap::default(),
                store: crate::storage::SheetStore::new(),
                geometry: crate::geometry::SheetGeometry::new(),
                charts: Vec::new(),
                sparklines: Vec::new(),
                page_setup: PageSetup::default(),
                autofilter: None,
                filter_hidden_rows: std::collections::BTreeSet::new(),
                validations: Vec::new(),
                cond_formats: Vec::new(),
            },
        };
        let mut sheets = IndexMap::new();
        sheets.insert(id, sheet);
        let mut names_by_lower = FxHashMap::default();
        names_by_lower.insert("sheet1".into(), id);
        Self {
            sheets,
            names_by_lower,
            intern: Arc::new(Interners::new()),
            names: NameRegistry::new(),
            tables: TableRegistry::new(),
            pivots: PivotRegistry::new(),
            settings: WorkbookSettings::default(),
            protection: WorkbookProtectionState::default(),
            meta: WorkbookMeta::default(),
            custom_parts: IndexMap::new(),
            undo: UndoLog::new(),
            next_sheet: 1,
            active: id,
            num_fmts: IndexMap::new(),
            next_num_fmt: CUSTOM_NUM_FMT_START,
            ref_errors: 0,
        }
    }

    fn intern_mut(&mut self) -> &mut Interners {
        Arc::make_mut(&mut self.intern)
    }

    /// Copy-on-write snapshot for readers.
    #[must_use]
    pub fn snapshot(&self) -> WorkbookSnapshot {
        WorkbookSnapshot {
            sheets: self.sheets.clone(),
            names: self.names.clone(),
            tables: self.tables.clone(),
            pivots: self.pivots.clone(),
            intern: Arc::clone(&self.intern),
            settings: self.settings.clone(),
            protection: self.protection.clone(),
        }
    }

    /// Settings.
    #[must_use]
    pub fn settings(&self) -> &WorkbookSettings {
        &self.settings
    }

    /// Mutable settings (not undo-tracked; WP-07a will wrap this).
    pub fn settings_mut(&mut self) -> &mut WorkbookSettings {
        &mut self.settings
    }

    /// Workbook-level protection flags.
    #[must_use]
    pub fn protection(&self) -> &WorkbookProtectionState {
        &self.protection
    }

    /// Replace workbook-level protection flags with undo tracking.
    pub fn set_workbook_protection(
        &mut self,
        protection: WorkbookProtectionState,
    ) -> Result<(), CoreError> {
        let before = self.protection.clone();
        if before == protection {
            return Ok(());
        }
        self.protection = protection.clone();
        self.undo.record(Delta::WorkbookProtection {
            before,
            after: protection,
        });
        Ok(())
    }

    /// Metadata.
    #[must_use]
    pub fn meta(&self) -> &WorkbookMeta {
        &self.meta
    }

    /// Mutable metadata.
    pub fn meta_mut(&mut self) -> &mut WorkbookMeta {
        &mut self.meta
    }

    /// Intern tables.
    #[must_use]
    pub fn intern(&self) -> &Interners {
        &self.intern
    }

    /// Defined names.
    #[must_use]
    pub fn names(&self) -> &NameRegistry {
        &self.names
    }

    /// Tables.
    #[must_use]
    pub fn tables(&self) -> &TableRegistry {
        &self.tables
    }

    /// Pivot tables.
    #[must_use]
    pub fn pivots(&self) -> &PivotRegistry {
        &self.pivots
    }

    /// Undo log (budget, enable/disable, stack queries).
    #[must_use]
    pub fn undo_log(&self) -> &UndoLog {
        &self.undo
    }

    /// Undo log (budget, enable/disable).
    pub fn undo_log_mut(&mut self) -> &mut UndoLog {
        &mut self.undo
    }

    /// Active sheet id.
    #[must_use]
    pub fn active_sheet(&self) -> SheetId {
        self.active
    }

    /// Number of stored cells whose current value is `#REF!`.
    #[must_use]
    pub fn ref_error_count(&self) -> u64 {
        self.ref_errors
    }

    /// Set the active sheet.
    pub fn set_active_sheet(&mut self, id: SheetId) -> Result<(), CoreError> {
        if !self.sheets.contains_key(&id) {
            return Err(CoreError::sheet_id(format!("unknown sheet {}", id.index())));
        }
        self.active = id;
        Ok(())
    }

    /// Ordered sheets.
    pub fn sheets(&self) -> impl Iterator<Item = &Sheet> {
        self.sheets.values()
    }

    /// Sheet by id.
    #[must_use]
    pub fn sheet(&self, id: SheetId) -> Option<&Sheet> {
        self.sheets.get(&id)
    }

    /// Ordered tab index for a sheet.
    pub fn sheet_index(&self, id: SheetId) -> Result<usize, CoreError> {
        self.sheets
            .get_index_of(&id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))
    }

    /// Portable snapshot of WP-17 metadata outside the cell store.
    pub fn sheet_edit_state(&self, id: SheetId) -> Result<SheetEditState, CoreError> {
        self.sheet(id)
            .map(SheetEditState::capture)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))
    }

    /// Restore a portable WP-17 metadata snapshot as one undo delta.
    pub fn restore_sheet_edit_state(
        &mut self,
        id: SheetId,
        state: SheetEditState,
    ) -> Result<(), CoreError> {
        self.mutate_sheet_edit(id, |sheet| {
            state.restore(sheet);
            Ok(())
        })
    }

    /// Sheet by name (case-insensitive).
    #[must_use]
    pub fn sheet_by_name(&self, name: &str) -> Option<&Sheet> {
        let id = *self.names_by_lower.get(&name.to_lowercase())?;
        self.sheets.get(&id)
    }

    /// Resolve a parsed sheet name to an id.
    pub fn resolve_sheet_name(&self, name: &str) -> Result<SheetId, CoreError> {
        self.names_by_lower
            .get(&name.to_lowercase())
            .copied()
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {name:?}")))
    }

    /// Resolve [`SheetSpec`] (3-D end optional).
    pub fn resolve_spec(&self, spec: &SheetSpec) -> Result<(SheetId, Option<SheetId>), CoreError> {
        let start = self.resolve_sheet_name(&spec.start)?;
        let end = match &spec.end {
            Some(n) => Some(self.resolve_sheet_name(n)?),
            None => None,
        };
        Ok((start, end))
    }

    /// Attach resolved sheet ids to a [`ParsedRef`].
    pub fn resolve_parsed(&self, parsed: ParsedRef) -> Result<RefKind, CoreError> {
        let (sheet, sheet_end) = match &parsed.sheet {
            Some(spec) => {
                let (a, b) = self.resolve_spec(spec)?;
                (Some(a), b)
            }
            None => (None, None),
        };
        Ok(match parsed.kind {
            RefKind::Cell(mut c) => {
                c.sheet = sheet;
                RefKind::Cell(c)
            }
            RefKind::Range(mut r) => {
                r.start.sheet = sheet;
                r.end.sheet = sheet;
                r.sheet_end = sheet_end;
                RefKind::Range(r)
            }
        })
    }

    pub(crate) fn sheet_mut(&mut self, id: SheetId) -> Result<&mut Sheet, CoreError> {
        self.sheets
            .get_mut(&id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))
    }

    pub(crate) fn mutate_sheet_edit<T>(
        &mut self,
        id: SheetId,
        edit: impl FnOnce(&mut Sheet) -> Result<T, CoreError>,
    ) -> Result<T, CoreError> {
        let before = SheetEditState::capture(
            self.sheet(id)
                .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))?,
        );
        let result = match edit(self.sheet_mut(id)?) {
            Ok(result) => result,
            Err(error) => {
                before.clone().restore(self.sheet_mut(id)?);
                return Err(error);
            }
        };
        let after = SheetEditState::capture(
            self.sheet(id)
                .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))?,
        );
        if before != after {
            self.undo.record(Delta::SheetEdit {
                sheet: id,
                before: Box::new(before),
                after: Box::new(after),
            });
        }
        Ok(result)
    }

    /// Append a chart. Ids are assigned.
    pub fn add_chart(&mut self, mut chart: Chart) -> Result<ChartId, CoreError> {
        chart.values_valid()?;
        let next = self
            .sheets
            .values()
            .flat_map(|sheet| sheet.charts.iter().map(|c| c.id.index()))
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| CoreError::new("chart.id", "chart id space is exhausted"))?;
        let id = ChartId::new(next);
        chart.id = id;
        let sheet = chart.sheet;
        let target = self.sheet_mut(sheet)?;
        let index = target.charts.len();
        target.charts.push(chart.clone());
        self.undo.record(crate::undo::Delta::ChartAdd {
            sheet,
            index,
            chart: Box::new(chart),
        });
        Ok(id)
    }

    /// Remove a chart by id, returning the exact record for inverse commands.
    pub fn remove_chart(&mut self, sheet: SheetId, id: ChartId) -> Result<Chart, CoreError> {
        let target = self.sheet_mut(sheet)?;
        let index = target
            .charts
            .iter()
            .position(|chart| chart.id == id)
            .ok_or_else(|| CoreError::new("chart.id", format!("unknown chart {}", id.index())))?;
        let chart = target.charts.remove(index);
        self.undo.record(crate::undo::Delta::ChartRemove {
            sheet,
            index,
            chart: Box::new(chart.clone()),
        });
        Ok(chart)
    }

    /// Append a sparkline.
    pub fn add_sparkline(&mut self, spark: Sparkline) -> Result<(), CoreError> {
        spark.values_valid()?;
        let sheet = spark.sheet;
        let target = self.sheet_mut(sheet)?;
        let index = target.sparklines.len();
        target.sparklines.push(spark.clone());
        self.undo.record(crate::undo::Delta::SparklineAdd {
            sheet,
            index,
            sparkline: spark,
        });
        Ok(())
    }

    /// Replace a sheet's page setup (print / PDF).
    pub fn set_page_setup(&mut self, id: SheetId, setup: PageSetup) -> Result<(), CoreError> {
        setup.validate()?;
        let before = self.sheet_mut(id)?.page_setup.clone();
        if before == setup {
            return Ok(());
        }
        self.sheet_mut(id)?.page_setup = setup.clone();
        self.undo.record(Delta::PageSetup {
            sheet: id,
            before: Box::new(before),
            after: Box::new(setup),
        });
        Ok(())
    }

    /// Remove a sparkline by its stable list position.
    pub fn remove_sparkline(
        &mut self,
        sheet: SheetId,
        index: usize,
    ) -> Result<Sparkline, CoreError> {
        let target = self.sheet_mut(sheet)?;
        if index >= target.sparklines.len() {
            return Err(CoreError::new(
                "sparkline.id",
                format!("unknown sparkline index {index}"),
            ));
        }
        let sparkline = target.sparklines.remove(index);
        self.undo.record(crate::undo::Delta::SparklineRemove {
            sheet,
            index,
            sparkline: sparkline.clone(),
        });
        Ok(sparkline)
    }

    /// Borrow a cell.
    pub fn get(&self, id: SheetId, row: u32, col: u16) -> Result<Option<&CellSlot>, CoreError> {
        let sheet = self
            .sheets
            .get(&id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))?;
        sheet.store.get(row, col)
    }

    fn hold_slot(&mut self, slot: &CellSlot) {
        let intern = self.intern_mut();
        if let Value::Text(id) = slot.value {
            intern.strings.add_ref(id);
        }
        if let Value::Array(id) = slot.value {
            intern.arrays.add_ref(id);
        }
        if let Some(f) = slot.formula {
            intern.formulas.add_ref(f);
        }
        intern.styles.add_ref(slot.style);
    }

    fn release_slot(&mut self, slot: &CellSlot) {
        let intern = self.intern_mut();
        if let Value::Text(id) = slot.value {
            intern.strings.release(id);
        }
        if let Value::Array(id) = slot.value {
            intern.arrays.release(id);
        }
        if let Some(f) = slot.formula {
            intern.formulas.release(f);
        }
        intern.styles.release(slot.style);
    }

    fn ensure_not_pivot_output(&self, id: SheetId, row: u32, col: u16) -> Result<(), CoreError> {
        self.ensure_range_not_pivot_output(id, row, col, row, col)
    }

    pub(crate) fn ensure_range_not_pivot_output(
        &self,
        id: SheetId,
        min_row: u32,
        min_col: u16,
        max_row: u32,
        max_col: u16,
    ) -> Result<(), CoreError> {
        if self.pivots.iter().any(|pivot| {
            pivot.dest_sheet == id
                && min_row <= pivot.out_end_row
                && pivot.dest_row <= max_row
                && min_col <= pivot.out_end_col
                && pivot.dest_col <= max_col
        }) {
            return Err(
                CoreError::new("pivot.readonly", "pivot output cells are read-only")
                    .with_hint("change the source range and run pivot.refresh"),
            );
        }
        Ok(())
    }

    pub(crate) fn ensure_sheet_not_used_by_pivot(&self, id: SheetId) -> Result<(), CoreError> {
        if let Some(pivot) = self
            .pivots
            .iter()
            .find(|pivot| pivot.source_sheet == id || pivot.dest_sheet == id)
        {
            return Err(CoreError::new(
                "pivot.readonly",
                format!("structural edit would invalidate pivot {:?}", pivot.name),
            )
            .with_hint("remove the pivot before inserting or deleting cells, rows, or columns"));
        }
        Ok(())
    }

    /// Set a plain numeric cell.
    pub fn set_number(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        n: f64,
    ) -> Result<Option<CellSlot>, CoreError> {
        self.ensure_not_pivot_output(id, row, col)?;
        let old = self.replace_slot(id, row, col, Some(CellSlot::number(n)))?;
        self.expand_tables_at(id, row, col);
        Ok(old)
    }

    /// Intern rich text and store it as the cell value.
    pub fn set_rich_text(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        text: &str,
        runs: Vec<RichTextRun>,
    ) -> Result<StrId, CoreError> {
        self.ensure_not_pivot_output(id, row, col)?;
        let sid = self.intern_mut().strings.intern_rich(text, runs);
        let slot = CellSlot {
            value: Value::Text(sid),
            formula: None,
            style: StyleId::DEFAULT,
            flags: crate::storage::CellFlags::DEFAULT,
        };
        self.replace_slot(id, row, col, Some(slot))?;
        self.intern_mut().strings.release(sid);
        self.expand_tables_at(id, row, col);
        Ok(sid)
    }

    /// Intern `text` and store it as the cell value.
    pub fn set_text(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        text: &str,
    ) -> Result<StrId, CoreError> {
        self.ensure_not_pivot_output(id, row, col)?;
        let sid = self.intern_mut().strings.intern(text);
        let slot = CellSlot {
            value: Value::Text(sid),
            formula: None,
            style: StyleId::DEFAULT,
            flags: crate::storage::CellFlags::DEFAULT,
        };
        self.replace_slot(id, row, col, Some(slot))?;
        self.intern_mut().strings.release(sid);
        self.expand_tables_at(id, row, col);
        Ok(sid)
    }

    /// Intern formula *source* (not parsed) and attach it to the cell.
    pub fn set_formula_text(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        source: &str,
    ) -> Result<FormulaId, CoreError> {
        self.ensure_not_pivot_output(id, row, col)?;
        let fid = self.intern_mut().formulas.intern(source)?;
        let slot = CellSlot {
            value: Value::Empty,
            formula: Some(fid),
            style: StyleId::DEFAULT,
            flags: crate::storage::CellFlags::DEFAULT,
        };
        self.replace_slot(id, row, col, Some(slot))?;
        self.intern_mut().formulas.release(fid);
        Ok(fid)
    }

    /// Clear a cell.
    pub fn clear_cell(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
    ) -> Result<Option<CellSlot>, CoreError> {
        self.ensure_not_pivot_output(id, row, col)?;
        self.replace_slot(id, row, col, None)
    }

    /// Write a complete slot without auto-expand (sort / restore).
    pub(crate) fn write_slot(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        slot: Option<CellSlot>,
    ) -> Result<Option<CellSlot>, CoreError> {
        self.replace_slot(id, row, col, slot)
    }

    fn replace_slot(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        slot: Option<CellSlot>,
    ) -> Result<Option<CellSlot>, CoreError> {
        let old = {
            let sheet = self.sheet_mut(id)?;
            match slot {
                Some(s) => sheet.store.set(row, col, s)?,
                None => sheet.store.clear(row, col)?,
            }
        };
        let old_ref = old
            .as_ref()
            .is_some_and(|slot| matches!(slot.value, Value::Error(crate::error::ErrorKind::Ref)));
        let new_ref = slot
            .as_ref()
            .is_some_and(|slot| matches!(slot.value, Value::Error(crate::error::ErrorKind::Ref)));
        match (old_ref, new_ref) {
            (false, true) => self.ref_errors = self.ref_errors.saturating_add(1),
            (true, false) => self.ref_errors = self.ref_errors.saturating_sub(1),
            _ => {}
        }
        if let Some(s) = slot {
            self.hold_slot(&s);
        }
        if let Some(ref old) = old {
            self.release_slot(old);
        }
        if self.undo.is_enabled() {
            if let Some(s) = slot {
                self.hold_slot(&s);
            }
            if let Some(ref old) = old {
                self.hold_slot(old);
            }
            self.undo.record(Delta::Cell {
                sheet: id,
                row,
                col,
                before: old,
                after: slot,
            });
        }
        Ok(old)
    }

    /// Intern a style and assign it to a cell (creates the cell if needed).
    pub fn set_cell_style(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        style: Style,
    ) -> Result<StyleId, CoreError> {
        self.ensure_not_pivot_output(id, row, col)?;
        let sid = self.intern_mut().styles.intern(style);
        let mut slot = self
            .get(id, row, col)?
            .copied()
            .unwrap_or_else(CellSlot::empty);
        slot.style = sid;
        self.replace_slot(id, row, col, Some(slot))?;
        self.intern_mut().styles.release(sid);
        Ok(sid)
    }

    /// Run `f` as one undo unit.
    pub fn transact<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.undo.begin();
        let r = f(self);
        self.undo.commit();
        r
    }

    /// Run `f` as one undo unit, rolling back the open transaction on error.
    ///
    /// Used by the command bus so a live failure after preflight never leaves a
    /// partial mutation as a successful outcome.
    pub fn transact_try<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, CoreError>,
    ) -> Result<T, CoreError> {
        self.undo.begin();
        match f(self) {
            Ok(value) => {
                self.undo.commit();
                Ok(value)
            }
            Err(err) => {
                if let Some(tx) = self.undo.abort() {
                    let undo_on = self.undo.is_enabled();
                    self.undo.set_enabled(false);
                    let rolled = self.apply_transaction(&tx, true);
                    self.undo.set_enabled(undo_on);
                    if let Err(rollback) = rolled {
                        return Err(CoreError::new(
                            "undo.rollback",
                            format!(
                                "command failed ({}); rollback also failed ({})",
                                err.message, rollback.message
                            ),
                        )
                        .with_hint("the workbook may be inconsistent; reload from disk"));
                    }
                }
                Err(err)
            }
        }
    }

    /// Undo the last transaction.
    pub fn undo(&mut self) -> Result<Vec<AffectedRange>, CoreError> {
        let tx = self.undo.pop_undo()?;
        self.apply_transaction(&tx, true)?;
        let affected = transaction_affected(&tx);
        self.undo.push_redo(tx);
        Ok(affected)
    }

    /// Redo the last undone transaction.
    pub fn redo(&mut self) -> Result<Vec<AffectedRange>, CoreError> {
        let tx = self.undo.pop_redo()?;
        self.apply_transaction(&tx, false)?;
        let affected = transaction_affected(&tx);
        self.undo.push_undo(tx);
        Ok(affected)
    }

    fn apply_transaction(
        &mut self,
        tx: &crate::undo::Transaction,
        inverse: bool,
    ) -> Result<(), CoreError> {
        let was = self.undo.is_enabled();
        self.undo.set_enabled(false);
        let deltas: Vec<Delta> = tx.deltas().to_vec();
        let result = if inverse {
            deltas
                .iter()
                .rev()
                .try_for_each(|d| self.apply_delta(d, true))
        } else {
            deltas.iter().try_for_each(|d| self.apply_delta(d, false))
        };
        self.undo.set_enabled(was);
        result
    }

    fn apply_delta(&mut self, delta: &Delta, inverse: bool) -> Result<(), CoreError> {
        match delta {
            Delta::Cell {
                sheet,
                row,
                col,
                before,
                after,
            } => {
                let slot = if inverse { *before } else { *after };
                let _ = self.replace_slot(*sheet, *row, *col, slot)?;
                Ok(())
            }
            Delta::RowGeom {
                sheet,
                row,
                before_px,
                after_px,
                hidden_before,
                hidden_after,
                custom_before,
                custom_after,
            } => {
                let (px, hid, custom) = if inverse {
                    (*before_px, *hidden_before, *custom_before)
                } else {
                    (*after_px, *hidden_after, *custom_after)
                };
                let s = self.sheet_mut(*sheet)?;
                s.geometry.rows.set_hidden(*row, false)?;
                if custom {
                    s.geometry.rows.set_size(*row, px)?;
                } else {
                    s.geometry.rows.clear_size(*row)?;
                }
                s.geometry.rows.set_hidden(*row, hid)?;
                Ok(())
            }
            Delta::ColGeom {
                sheet,
                col,
                before_px,
                after_px,
                hidden_before,
                hidden_after,
                custom_before,
                custom_after,
            } => {
                let (px, hid, custom) = if inverse {
                    (*before_px, *hidden_before, *custom_before)
                } else {
                    (*after_px, *hidden_after, *custom_after)
                };
                let s = self.sheet_mut(*sheet)?;
                s.geometry.cols.set_hidden(u32::from(*col), false)?;
                if custom {
                    s.geometry.cols.set_size(u32::from(*col), px)?;
                } else {
                    s.geometry.cols.clear_size(u32::from(*col))?;
                }
                s.geometry.cols.set_hidden(u32::from(*col), hid)?;
                Ok(())
            }
            Delta::SheetAdd { id, index, sheet } => {
                if inverse {
                    self.unlink_sheet(*id)?;
                } else {
                    self.link_sheet(*index, (**sheet).clone())?;
                }
                Ok(())
            }
            Delta::SheetRemove {
                id,
                index,
                sheet,
                active_before,
                active_after,
            } => {
                if inverse {
                    self.link_sheet(*index, (**sheet).clone())?;
                    self.active = *active_before;
                } else {
                    self.unlink_sheet(*id)?;
                    self.active = *active_after;
                }
                Ok(())
            }
            Delta::SheetReorder { id, before, after } => {
                let index = if inverse { *before } else { *after };
                self.reorder_sheet_inner(*id, index)
            }
            Delta::SheetEdit {
                sheet,
                before,
                after,
            } => {
                let state = if inverse { before } else { after };
                state.as_ref().clone().restore(self.sheet_mut(*sheet)?);
                Ok(())
            }
            Delta::SheetRename { id, before, after } => {
                let name = if inverse { before } else { after };
                self.rename_sheet_inner(*id, name, false)
            }
            Delta::SheetVisibility { id, before, after } => {
                let vis = if inverse { *before } else { *after };
                self.set_visibility_inner(*id, vis, false)
            }
            Delta::TabColor { id, before, after } => {
                let c = if inverse { *before } else { *after };
                self.sheet_mut(*id)?.tab_color = c;
                Ok(())
            }
            Delta::PageSetup {
                sheet,
                before,
                after,
            } => {
                self.sheet_mut(*sheet)?.page_setup = if inverse {
                    (**before).clone()
                } else {
                    (**after).clone()
                };
                Ok(())
            }
            Delta::Name {
                scope,
                name,
                before,
                after,
            } => {
                let target = if inverse { before } else { after };
                let _ = self.names.remove(*scope, name);
                if let Some(n) = target {
                    self.names.upsert(n.clone())?;
                }
                Ok(())
            }
            Delta::Table { before, after } => {
                let target = if inverse { before } else { after };
                if let Some(t) = after.as_ref().or(before.as_ref()) {
                    let _ = self.tables.remove(t.id);
                }
                if let Some(t) = target {
                    self.tables.restore(t.clone())?;
                }
                Ok(())
            }
            Delta::Pivot { before, after } => {
                let target = if inverse { before } else { after };
                if let Some(t) = after.as_ref().or(before.as_ref()) {
                    let _ = self.pivots.remove(t.id);
                }
                if let Some(t) = target {
                    self.pivots.restore((**t).clone())?;
                }
                Ok(())
            }
            Delta::ChartAdd {
                sheet,
                index,
                chart,
            } => {
                let charts = &mut self.sheet_mut(*sheet)?.charts;
                if inverse {
                    let removed = charts.get(*index).ok_or_else(|| {
                        CoreError::new("undo.chart", "chart undo index is out of range")
                    })?;
                    if removed.id != chart.id {
                        return Err(CoreError::new(
                            "undo.chart",
                            "chart undo identity does not match",
                        ));
                    }
                    charts.remove(*index);
                } else if *index <= charts.len() {
                    charts.insert(*index, (**chart).clone());
                } else {
                    return Err(CoreError::new(
                        "undo.chart",
                        "chart redo index is out of range",
                    ));
                }
                Ok(())
            }
            Delta::ChartRemove {
                sheet,
                index,
                chart,
            } => {
                let charts = &mut self.sheet_mut(*sheet)?.charts;
                if inverse {
                    if *index <= charts.len() {
                        charts.insert(*index, (**chart).clone());
                    } else {
                        return Err(CoreError::new(
                            "undo.chart",
                            "chart restore index is out of range",
                        ));
                    }
                } else {
                    let removed = charts.get(*index).ok_or_else(|| {
                        CoreError::new("undo.chart", "chart redo index is out of range")
                    })?;
                    if removed.id != chart.id {
                        return Err(CoreError::new(
                            "undo.chart",
                            "chart redo identity does not match",
                        ));
                    }
                    charts.remove(*index);
                }
                Ok(())
            }
            Delta::SparklineAdd {
                sheet,
                index,
                sparkline,
            } => {
                let sparklines = &mut self.sheet_mut(*sheet)?.sparklines;
                if inverse {
                    let removed = sparklines.get(*index).ok_or_else(|| {
                        CoreError::new("undo.sparkline", "sparkline undo index is out of range")
                    })?;
                    if removed != sparkline {
                        return Err(CoreError::new(
                            "undo.sparkline",
                            "sparkline undo identity does not match",
                        ));
                    }
                    sparklines.remove(*index);
                } else if *index <= sparklines.len() {
                    sparklines.insert(*index, sparkline.clone());
                } else {
                    return Err(CoreError::new(
                        "undo.sparkline",
                        "sparkline redo index is out of range",
                    ));
                }
                Ok(())
            }
            Delta::SparklineRemove {
                sheet,
                index,
                sparkline,
            } => {
                let sparklines = &mut self.sheet_mut(*sheet)?.sparklines;
                if inverse {
                    if *index <= sparklines.len() {
                        sparklines.insert(*index, sparkline.clone());
                    } else {
                        return Err(CoreError::new(
                            "undo.sparkline",
                            "sparkline restore index is out of range",
                        ));
                    }
                } else {
                    let removed = sparklines.get(*index).ok_or_else(|| {
                        CoreError::new("undo.sparkline", "sparkline redo index is out of range")
                    })?;
                    if removed != sparkline {
                        return Err(CoreError::new(
                            "undo.sparkline",
                            "sparkline redo identity does not match",
                        ));
                    }
                    sparklines.remove(*index);
                }
                Ok(())
            }
            Delta::ShiftRows {
                sheet,
                at,
                count,
                removed,
            } => {
                let n = if inverse { -count } else { *count };
                let store = &mut self.sheet_mut(*sheet)?.store;
                store.shift_rows(*at, n)?;
                if inverse && *count < 0 {
                    for (row, col, slot) in removed {
                        store.set(*row, *col, *slot)?;
                    }
                }
                self.recount_ref_errors();
                Ok(())
            }
            Delta::ShiftCols {
                sheet,
                at,
                count,
                removed,
            } => {
                let n = if inverse { -count } else { *count };
                let store = &mut self.sheet_mut(*sheet)?.store;
                store.shift_cols(*at, n)?;
                if inverse && *count < 0 {
                    for (row, col, slot) in removed {
                        store.set(*row, *col, *slot)?;
                    }
                }
                self.recount_ref_errors();
                Ok(())
            }
            Delta::CalcMode { before, after } => {
                self.settings.calc_mode = if inverse { *before } else { *after };
                Ok(())
            }
            Delta::WorkbookProtection { before, after } => {
                self.protection = if inverse {
                    before.clone()
                } else {
                    after.clone()
                };
                Ok(())
            }
        }
    }

    fn unlink_sheet(&mut self, id: SheetId) -> Result<Sheet, CoreError> {
        let pos = self
            .sheets
            .get_index_of(&id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))?;
        let (_, sheet) = self
            .sheets
            .shift_remove_index(pos)
            .ok_or_else(|| CoreError::sheet_id("sheet vanished"))?;
        self.ref_errors = self
            .ref_errors
            .saturating_sub(sheet_ref_error_count(&sheet));
        self.names_by_lower.remove(&sheet.name.to_lowercase());
        if self.active == id {
            self.active = self.sheets.keys().copied().next().unwrap_or(id);
        }
        Ok(sheet)
    }

    fn link_sheet(&mut self, index: usize, sheet: Sheet) -> Result<(), CoreError> {
        self.ref_errors = self
            .ref_errors
            .saturating_add(sheet_ref_error_count(&sheet));
        self.names_by_lower
            .insert(sheet.name.to_lowercase(), sheet.id);
        let idx = index.min(self.sheets.len());
        self.sheets.shift_insert(idx, sheet.id, sheet);
        Ok(())
    }

    /// Add a sheet at the end. Name must be unique ignoring case.
    pub fn add_sheet(&mut self, name: impl Into<String>) -> Result<SheetId, CoreError> {
        let name = name.into();
        validate_sheet_name(&name)?;
        let lower = name.to_lowercase();
        if self.names_by_lower.contains_key(&lower) {
            return Err(CoreError::sheet_name(format!(
                "sheet name {name:?} already exists"
            )));
        }
        let id = SheetId::new(self.next_sheet);
        self.next_sheet += 1;
        let sheet = Sheet::new(id, name)?;
        let index = self.sheets.len();
        self.undo.record(Delta::SheetAdd {
            id,
            index,
            sheet: Box::new(sheet.clone()),
        });
        self.names_by_lower.insert(lower, id);
        self.sheets.insert(id, sheet);
        Ok(id)
    }

    /// Restore an exact sheet snapshot at a tab index.
    ///
    /// This narrow hook exists for the trusted command bus inverse of
    /// `sheet.remove`; ordinary callers should use [`Self::add_sheet`].
    pub fn restore_sheet_at(&mut self, index: usize, sheet: Sheet) -> Result<(), CoreError> {
        validate_sheet_name(&sheet.name)?;
        if self.sheets.contains_key(&sheet.id) {
            return Err(CoreError::sheet_id(format!(
                "sheet {} already exists",
                sheet.id.index()
            )));
        }
        if self.names_by_lower.contains_key(&sheet.name.to_lowercase()) {
            return Err(CoreError::sheet_name(format!(
                "sheet name {:?} already exists",
                sheet.name
            )));
        }
        self.next_sheet = self.next_sheet.max(
            sheet
                .id
                .index()
                .checked_add(1)
                .ok_or_else(|| CoreError::sheet_id("sheet id space is exhausted"))?,
        );
        self.link_sheet(index, sheet.clone())?;
        self.undo.record(Delta::SheetAdd {
            id: sheet.id,
            index: index.min(self.sheets.len().saturating_sub(1)),
            sheet: Box::new(sheet),
        });
        Ok(())
    }

    /// Remove a sheet. The last remaining sheet cannot be removed. The last
    /// visible sheet cannot be removed if it is the only visible one.
    pub fn remove_sheet(&mut self, id: SheetId) -> Result<Sheet, CoreError> {
        if let Some(pivot) = self
            .pivots
            .iter()
            .find(|pivot| pivot.source_sheet == id || pivot.dest_sheet == id)
        {
            return Err(CoreError::new(
                "pivot.sheet",
                format!("sheet is used by pivot {:?}", pivot.name),
            )
            .with_hint("remove the pivot table before deleting its source or output sheet"));
        }
        if self.sheets.len() == 1 {
            return Err(CoreError::sheet_name(
                "a workbook must contain at least one sheet",
            ));
        }
        let vis = self
            .sheets
            .get(&id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))?
            .visibility;
        if vis.is_visible() {
            let visible = self
                .sheets
                .values()
                .filter(|s| s.visibility.is_visible())
                .count();
            if visible == 1 {
                return Err(CoreError::sheet_name(
                    "cannot remove the last visible sheet",
                ));
            }
        }
        let index = self.sheets.get_index_of(&id).unwrap_or(0);
        let active_before = self.active;
        let sheet = self.unlink_sheet(id)?;
        let active_after = self.active;
        self.undo.record(Delta::SheetRemove {
            id,
            index,
            sheet: Box::new(sheet.clone()),
            active_before,
            active_after,
        });
        Ok(sheet)
    }

    fn recount_ref_errors(&mut self) {
        self.ref_errors = self.sheets.values().map(sheet_ref_error_count).sum();
    }

    /// Move `id` to tab index `index` (0-based).
    pub fn reorder_sheet(&mut self, id: SheetId, index: usize) -> Result<(), CoreError> {
        let from = self
            .sheets
            .get_index_of(&id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))?;
        let to = index.min(self.sheets.len().saturating_sub(1));
        if from != to {
            self.reorder_sheet_inner(id, to)?;
            self.undo.record(Delta::SheetReorder {
                id,
                before: from,
                after: to,
            });
        }
        Ok(())
    }

    fn reorder_sheet_inner(&mut self, id: SheetId, index: usize) -> Result<(), CoreError> {
        let from = self
            .sheets
            .get_index_of(&id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))?;
        let to = index.min(self.sheets.len().saturating_sub(1));
        if from != to {
            self.sheets.move_index(from, to);
        }
        Ok(())
    }

    /// Rename a sheet.
    pub fn rename_sheet(&mut self, id: SheetId, name: impl Into<String>) -> Result<(), CoreError> {
        self.rename_sheet_inner(id, &name.into(), true)
    }

    fn rename_sheet_inner(
        &mut self,
        id: SheetId,
        name: &str,
        record: bool,
    ) -> Result<(), CoreError> {
        validate_sheet_name(name)?;
        let lower = name.to_lowercase();
        if let Some(&other) = self.names_by_lower.get(&lower)
            && other != id
        {
            return Err(CoreError::sheet_name(format!(
                "sheet name {name:?} already exists"
            )));
        }
        let sheet = self.sheet_mut(id)?;
        let before = sheet.name.clone();
        if before == name {
            return Ok(());
        }
        sheet.name = name.to_string();
        self.names_by_lower.remove(&before.to_lowercase());
        self.names_by_lower.insert(lower, id);
        if record {
            self.undo.record(Delta::SheetRename {
                id,
                before,
                after: name.to_string(),
            });
        }
        Ok(())
    }

    /// Set visibility. The last visible sheet cannot be hidden.
    pub fn set_visibility(
        &mut self,
        id: SheetId,
        visibility: SheetVisibility,
    ) -> Result<(), CoreError> {
        self.set_visibility_inner(id, visibility, true)
    }

    fn set_visibility_inner(
        &mut self,
        id: SheetId,
        visibility: SheetVisibility,
        record: bool,
    ) -> Result<(), CoreError> {
        let current = self
            .sheets
            .get(&id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))?
            .visibility;
        if current == visibility {
            return Ok(());
        }
        if current.is_visible() && !visibility.is_visible() {
            let visible = self
                .sheets
                .values()
                .filter(|s| s.visibility.is_visible())
                .count();
            if visible == 1 {
                return Err(CoreError::sheet_name("cannot hide the last visible sheet"));
            }
        }
        self.sheet_mut(id)?.visibility = visibility;
        if record {
            self.undo.record(Delta::SheetVisibility {
                id,
                before: current,
                after: visibility,
            });
        }
        Ok(())
    }

    /// Set tab colour.
    pub fn set_tab_color(&mut self, id: SheetId, color: Option<Color>) -> Result<(), CoreError> {
        let before = self.sheet_mut(id)?.tab_color;
        self.sheet_mut(id)?.tab_color = color;
        self.undo.record(Delta::TabColor {
            id,
            before,
            after: color,
        });
        Ok(())
    }

    /// Hide or unhide a row (`SUBTOTAL` / `AGGREGATE` hidden-row semantics).
    pub fn set_row_hidden(&mut self, id: SheetId, row: u32, hidden: bool) -> Result<(), CoreError> {
        let (before_px, hidden_before, custom_before) = {
            let sheet = self.sheet_mut(id)?;
            let before_px = sheet.geometry.rows.size(row)?;
            let hidden_before = sheet.geometry.rows.is_hidden(row)?;
            let custom_before = sheet.geometry.rows.has_custom_size(row);
            if hidden_before == hidden {
                return Ok(());
            }
            sheet.geometry.rows.set_hidden(row, hidden)?;
            (before_px, hidden_before, custom_before)
        };
        self.undo.record(Delta::RowGeom {
            sheet: id,
            row,
            before_px,
            after_px: before_px,
            hidden_before,
            hidden_after: hidden,
            custom_before,
            custom_after: custom_before,
        });
        Ok(())
    }

    /// Set a row height in pixels.
    pub fn set_row_height(&mut self, id: SheetId, row: u32, px: u32) -> Result<(), CoreError> {
        let (before_px, hidden, custom_before) = {
            let sheet = self.sheet_mut(id)?;
            let before_px = sheet.geometry.rows.size(row)?;
            let hidden = sheet.geometry.rows.is_hidden(row)?;
            let custom_before = sheet.geometry.rows.has_custom_size(row);
            if before_px == px && custom_before {
                return Ok(());
            }
            sheet.geometry.rows.set_size(row, px)?;
            (before_px, hidden, custom_before)
        };
        self.undo.record(Delta::RowGeom {
            sheet: id,
            row,
            before_px,
            after_px: px,
            hidden_before: hidden,
            hidden_after: hidden,
            custom_before,
            custom_after: true,
        });
        Ok(())
    }

    /// Hide or unhide a column.
    pub fn set_col_hidden(&mut self, id: SheetId, col: u16, hidden: bool) -> Result<(), CoreError> {
        let (before_px, hidden_before, custom_before) = {
            let sheet = self.sheet_mut(id)?;
            let before_px = sheet.geometry.cols.size(u32::from(col))?;
            let hidden_before = sheet.geometry.cols.is_hidden(u32::from(col))?;
            let custom_before = sheet.geometry.cols.has_custom_size(u32::from(col));
            if hidden_before == hidden {
                return Ok(());
            }
            sheet.geometry.cols.set_hidden(u32::from(col), hidden)?;
            (before_px, hidden_before, custom_before)
        };
        self.undo.record(Delta::ColGeom {
            sheet: id,
            col,
            before_px,
            after_px: before_px,
            hidden_before,
            hidden_after: hidden,
            custom_before,
            custom_after: custom_before,
        });
        Ok(())
    }

    /// Set a column width in pixels.
    pub fn set_col_width(&mut self, id: SheetId, col: u16, px: u32) -> Result<(), CoreError> {
        let (before_px, hidden, custom_before) = {
            let sheet = self.sheet_mut(id)?;
            let before_px = sheet.geometry.cols.size(u32::from(col))?;
            let hidden = sheet.geometry.cols.is_hidden(u32::from(col))?;
            let custom_before = sheet.geometry.cols.has_custom_size(u32::from(col));
            if before_px == px && custom_before {
                return Ok(());
            }
            sheet.geometry.cols.set_size(u32::from(col), px)?;
            (before_px, hidden, custom_before)
        };
        self.undo.record(Delta::ColGeom {
            sheet: id,
            col,
            before_px,
            after_px: px,
            hidden_before: hidden,
            hidden_after: hidden,
            custom_before,
            custom_after: true,
        });
        Ok(())
    }

    /// Row outline level (0–7).
    pub fn row_outline_level(&self, id: SheetId, row: u32) -> Result<u8, CoreError> {
        self.sheet(id)
            .map(|sheet| sheet.geometry.rows.outline_level(row))
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))
    }

    /// Set a row outline level while loading or editing.
    pub fn set_row_outline_level(
        &mut self,
        id: SheetId,
        row: u32,
        level: u8,
    ) -> Result<(), CoreError> {
        self.mutate_sheet_edit(id, |sheet| {
            sheet.geometry.rows.set_outline_level(row, level)
        })
    }

    /// Column outline level (0–7).
    pub fn col_outline_level(&self, id: SheetId, col: u16) -> Result<u8, CoreError> {
        self.sheet(id)
            .map(|sheet| sheet.geometry.cols.outline_level(u32::from(col)))
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))
    }

    /// Set a column outline level while loading or editing.
    pub fn set_col_outline_level(
        &mut self,
        id: SheetId,
        col: u16,
        level: u8,
    ) -> Result<(), CoreError> {
        self.mutate_sheet_edit(id, |sheet| {
            sheet.geometry.cols.set_outline_level(u32::from(col), level)
        })
    }

    /// Set row outline collapse state while loading a file.
    pub fn set_row_collapsed(
        &mut self,
        id: SheetId,
        row: u32,
        collapsed: bool,
    ) -> Result<(), CoreError> {
        self.mutate_sheet_edit(id, |sheet| {
            sheet.geometry.rows.set_collapsed(row, collapsed)
        })
    }

    /// Whether a row outline marker is collapsed.
    pub fn row_collapsed(&self, id: SheetId, row: u32) -> Result<bool, CoreError> {
        self.sheet(id)
            .map(|sheet| sheet.geometry.rows.is_collapsed(row))
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))
    }

    /// Whether a column outline marker is collapsed.
    pub fn col_collapsed(&self, id: SheetId, col: u16) -> Result<bool, CoreError> {
        self.sheet(id)
            .map(|sheet| sheet.geometry.cols.is_collapsed(u32::from(col)))
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))
    }

    /// Set column outline collapse state while loading or editing.
    pub fn set_col_collapsed(
        &mut self,
        id: SheetId,
        col: u16,
        collapsed: bool,
    ) -> Result<(), CoreError> {
        self.mutate_sheet_edit(id, |sheet| {
            sheet.geometry.cols.set_collapsed(u32::from(col), collapsed)
        })
    }

    /// Replace sheet view state while loading a file.
    pub fn set_sheet_view(&mut self, id: SheetId, view: ViewState) -> Result<(), CoreError> {
        self.sheet_mut(id)?.view = view;
        Ok(())
    }

    /// Replace sheet protection metadata while loading a file.
    pub fn set_sheet_protection(
        &mut self,
        id: SheetId,
        protection: ProtectionState,
    ) -> Result<(), CoreError> {
        self.mutate_sheet_edit(id, |sheet| {
            sheet.protection = protection;
            Ok(())
        })
    }

    /// Replace merged ranges while loading a file.
    pub fn set_sheet_merges(
        &mut self,
        id: SheetId,
        merges: Vec<crate::addr::RangeRef>,
    ) -> Result<(), CoreError> {
        self.sheet_mut(id)?.merges = merges;
        Ok(())
    }

    /// Replace AutoFilter.
    pub fn set_autofilter(
        &mut self,
        id: SheetId,
        filter: Option<crate::filter::AutoFilter>,
    ) -> Result<(), CoreError> {
        self.mutate_sheet_edit(id, |sheet| {
            for row in std::mem::take(&mut sheet.filter_hidden_rows) {
                sheet.geometry.rows.set_hidden(row, false)?;
            }
            sheet.autofilter = filter;
            Ok(())
        })
    }

    /// Replace data validations.
    pub fn set_validations(
        &mut self,
        id: SheetId,
        validations: Vec<crate::validation::DataValidation>,
    ) -> Result<(), CoreError> {
        for validation in &validations {
            if validation.kind != crate::validation::DvType::Any
                && validation.formula1.as_deref().is_none_or(str::is_empty)
            {
                return Err(CoreError::new(
                    "validation.formula1",
                    "validation requires formula1",
                ));
            }
            if matches!(
                validation.op,
                crate::validation::DvOp::Between | crate::validation::DvOp::NotBetween
            ) && !matches!(
                validation.kind,
                crate::validation::DvType::Any
                    | crate::validation::DvType::List
                    | crate::validation::DvType::Custom
            ) && validation.formula2.as_deref().is_none_or(str::is_empty)
            {
                return Err(CoreError::new(
                    "validation.formula2",
                    "between validation requires formula2",
                ));
            }
        }
        self.mutate_sheet_edit(id, |sheet| {
            sheet.validations = validations;
            Ok(())
        })
    }

    /// Replace conditional format rules.
    pub fn set_cond_formats(
        &mut self,
        id: SheetId,
        mut rules: Vec<crate::condfmt::CondFormat>,
    ) -> Result<(), CoreError> {
        rules.sort_by_key(|rule| rule.priority);
        for (index, rule) in rules.iter_mut().enumerate() {
            rule.priority = u32::try_from(index)
                .ok()
                .and_then(|index| index.checked_add(1))
                .ok_or_else(|| CoreError::new("condfmt.limit", "too many conditional formats"))?;
            match &rule.kind {
                crate::condfmt::CfKind::CellIs {
                    op,
                    formula1,
                    formula2,
                } if formula1.is_empty()
                    || (matches!(
                        op,
                        crate::condfmt::CfOp::Between | crate::condfmt::CfOp::NotBetween
                    ) && formula2.as_deref().is_none_or(str::is_empty)) =>
                {
                    return Err(CoreError::new(
                        "condfmt.formula",
                        "cell-is rule is missing a required formula",
                    ));
                }
                crate::condfmt::CfKind::Formula(formula) if formula.is_empty() => {
                    return Err(CoreError::new(
                        "condfmt.formula",
                        "formula rule cannot be empty",
                    ));
                }
                crate::condfmt::CfKind::ColorScale { colors }
                    if !(2..=3).contains(&colors.len()) =>
                {
                    return Err(CoreError::new(
                        "condfmt.colors",
                        "color scales require two or three colors",
                    ));
                }
                crate::condfmt::CfKind::IconSet { icons } if !(3..=5).contains(icons) => {
                    return Err(CoreError::new(
                        "condfmt.icons",
                        "icon sets require three to five icons",
                    ));
                }
                crate::condfmt::CfKind::TopN { n, percent, .. }
                    if *n == 0 || (*percent && *n > 100) =>
                {
                    return Err(CoreError::new(
                        "condfmt.top_n",
                        "top-N rank must be positive and percent must be at most 100",
                    ));
                }
                _ => {}
            }
        }
        self.mutate_sheet_edit(id, |sheet| {
            sheet.cond_formats = rules;
            Ok(())
        })
    }

    /// Insert a defined name.
    pub fn define_name(&mut self, name: DefinedName) -> Result<(), CoreError> {
        let scope = name.scope;
        let n = name.name.clone();
        self.names.insert(name.clone())?;
        self.undo.record(Delta::Name {
            scope,
            name: n,
            before: None,
            after: Some(name),
        });
        Ok(())
    }

    /// Remove a defined name (case-insensitive within `scope`).
    pub fn remove_name(&mut self, scope: NameScope, name: &str) -> Result<DefinedName, CoreError> {
        let before = self.names.remove(scope, name).ok_or_else(|| {
            CoreError::name_defined(format!(
                "defined name {name:?} does not exist in this scope"
            ))
        })?;
        self.undo.record(Delta::Name {
            scope,
            name: before.name.clone(),
            before: Some(before.clone()),
            after: None,
        });
        Ok(before)
    }

    /// Set calculation mode (undo-tracked).
    pub fn set_calc_mode(&mut self, mode: CalcMode) -> Result<(), CoreError> {
        let before = self.settings.calc_mode;
        if before == mode {
            return Ok(());
        }
        self.settings.calc_mode = mode;
        self.undo.record(Delta::CalcMode {
            before,
            after: mode,
        });
        Ok(())
    }

    /// Insert a table.
    pub fn add_table(&mut self, table: Table) -> Result<TableId, CoreError> {
        let id = self.tables.insert(table.clone())?;
        let stored = self.tables.get(id).cloned();
        self.undo.record(Delta::Table {
            before: None,
            after: stored,
        });
        Ok(id)
    }

    /// Insert a pivot definition without writing output (xlsx import).
    pub fn import_pivot(&mut self, table: PivotTable) -> Result<PivotId, CoreError> {
        self.pivots.insert(table)
    }

    /// Insert a pivot table and materialize its output region.
    pub fn add_pivot(&mut self, table: PivotTable) -> Result<PivotId, CoreError> {
        self.transact_try(move |workbook| workbook.add_pivot_inner(table))
    }

    fn add_pivot_inner(&mut self, mut table: PivotTable) -> Result<PivotId, CoreError> {
        self.pivots.validate_insert(&table)?;
        let cells = materialize(self, &table)?;
        write_output(self, &mut table, &cells)?;
        let id = self.pivots.insert(table.clone())?;
        let stored = self.pivots.get(id).cloned();
        self.undo.record(Delta::Pivot {
            before: None,
            after: stored.map(Box::new),
        });
        Ok(id)
    }

    /// Rebuild a pivot from its source range.
    pub fn refresh_pivot(&mut self, id: PivotId) -> Result<(), CoreError> {
        self.transact_try(|workbook| workbook.refresh_pivot_inner(id))
    }

    fn refresh_pivot_inner(&mut self, id: PivotId) -> Result<(), CoreError> {
        let mut table =
            self.pivots.get(id).cloned().ok_or_else(|| {
                CoreError::new("pivot.id", format!("unknown pivot {}", id.index()))
            })?;
        let before = table.clone();
        let cells = materialize(self, &table)?;
        write_output(self, &mut table, &cells)?;
        self.pivots.restore(table.clone())?;
        self.undo.record(Delta::Pivot {
            before: Some(Box::new(before)),
            after: Some(Box::new(table)),
        });
        Ok(())
    }

    /// Rebuild a pivot from cached records (source range missing).
    pub fn refresh_pivot_from_cache(
        &mut self,
        id: PivotId,
        headers: &[String],
        rows: &[Vec<CacheValue>],
    ) -> Result<(), CoreError> {
        self.transact_try(|workbook| workbook.refresh_pivot_from_cache_inner(id, headers, rows))
    }

    fn refresh_pivot_from_cache_inner(
        &mut self,
        id: PivotId,
        headers: &[String],
        rows: &[Vec<CacheValue>],
    ) -> Result<(), CoreError> {
        let mut table =
            self.pivots.get(id).cloned().ok_or_else(|| {
                CoreError::new("pivot.id", format!("unknown pivot {}", id.index()))
            })?;
        let before = table.clone();
        let cells = materialize_from_cache(self.settings().date_system, &table, headers, rows)?;
        write_output(self, &mut table, &cells)?;
        self.pivots.restore(table.clone())?;
        self.undo.record(Delta::Pivot {
            before: Some(Box::new(before)),
            after: Some(Box::new(table)),
        });
        Ok(())
    }

    /// Drop a pivot and clear its output region.
    pub fn remove_pivot(&mut self, id: PivotId) -> Result<PivotTable, CoreError> {
        self.transact_try(|workbook| workbook.remove_pivot_inner(id))
    }

    fn remove_pivot_inner(&mut self, id: PivotId) -> Result<PivotTable, CoreError> {
        let mut table = self
            .pivots
            .remove(id)
            .ok_or_else(|| CoreError::new("pivot.id", format!("unknown pivot {}", id.index())))?;
        if table.out_end_row >= table.dest_row && table.out_end_col >= table.dest_col {
            for r in table.dest_row..=table.out_end_row {
                for c in table.dest_col..=table.out_end_col {
                    let _ = self.write_slot(table.dest_sheet, r, c, None);
                }
            }
        }
        table.out_end_row = table.dest_row;
        table.out_end_col = table.dest_col;
        self.undo.record(Delta::Pivot {
            before: Some(Box::new(table.clone())),
            after: None,
        });
        Ok(table)
    }

    /// Restore or replace a pivot while preserving its stable id.
    pub fn restore_pivot(&mut self, table: PivotTable) -> Result<(), CoreError> {
        let before = self.pivots.get(table.id).cloned();
        if before.as_ref() == Some(&table) {
            return Ok(());
        }
        if before.is_some() {
            let _ = self.pivots.remove(table.id);
        }
        if let Err(error) = self.pivots.restore(table.clone()) {
            if let Some(previous) = before.clone() {
                let _ = self.pivots.restore(previous);
            }
            return Err(error);
        }
        self.undo.record(Delta::Pivot {
            before: before.map(Box::new),
            after: Some(Box::new(table)),
        });
        Ok(())
    }

    /// Create a table covering `range`, using the first row as headers.
    pub fn create_table(
        &mut self,
        sheet: SheetId,
        range: crate::addr::RangeRef,
        name: impl Into<String>,
    ) -> Result<TableId, CoreError> {
        let r0 = range.start.row.min(range.end.row);
        let r1 = range.start.row.max(range.end.row);
        let c0 = range.start.col.min(range.end.col);
        let c1 = range.start.col.max(range.end.col);
        self.ensure_range_not_pivot_output(sheet, r0, c0, r1, c1)?;
        if self.tables.iter().any(|existing| {
            existing.sheet == sheet
                && ranges_overlap(
                    (r0, c0, r1, c1),
                    (
                        existing.start_row,
                        existing.start_col,
                        existing.end_row,
                        existing.end_col,
                    ),
                )
        }) {
            return Err(CoreError::table_name(
                "table range overlaps an existing table",
            ));
        }
        let mut table = Table::new(TableId::new(0), name, sheet, r0, c0, r1, c1);
        for (i, col) in (c0..=c1).enumerate() {
            let header = match self.get(sheet, r0, col)? {
                Some(slot) => match slot.value {
                    crate::value::Value::Text(id) => {
                        self.intern().strings.get(id).unwrap_or("").to_string()
                    }
                    crate::value::Value::Number(n) => n.to_string(),
                    _ => format!("Column{}", i + 1),
                },
                None => format!("Column{}", i + 1),
            };
            let base = if header.is_empty() {
                format!("Column{}", i + 1)
            } else {
                header
            };
            let unique = unique_table_column_name(&table.columns, i, &base);
            if let Some(c) = table.columns.get_mut(i) {
                c.name = unique;
            }
        }
        self.add_table(table)
    }

    /// Drop a table, leaving cell values.
    pub fn convert_table(&mut self, id: TableId) -> Result<Table, CoreError> {
        let before = self
            .tables
            .remove(id)
            .ok_or_else(|| CoreError::table_name("unknown table"))?;
        self.undo.record(Delta::Table {
            before: Some(before.clone()),
            after: None,
        });
        Ok(before)
    }

    /// Restore or replace a table while preserving its stable id.
    ///
    /// This is used by trusted undo and changeset restoration payloads. Normal
    /// callers should use [`Self::create_table`], [`Self::resize_table`], or
    /// [`Self::convert_table`].
    pub fn restore_table(&mut self, table: Table) -> Result<(), CoreError> {
        if self.tables.iter().any(|existing| {
            existing.id != table.id
                && existing.sheet == table.sheet
                && ranges_overlap(
                    (
                        table.start_row,
                        table.start_col,
                        table.end_row,
                        table.end_col,
                    ),
                    (
                        existing.start_row,
                        existing.start_col,
                        existing.end_row,
                        existing.end_col,
                    ),
                )
        }) {
            return Err(CoreError::table_name(
                "restored table would overlap an existing table",
            ));
        }
        let before = self.tables.get(table.id).cloned();
        if before.as_ref() == Some(&table) {
            return Ok(());
        }
        if before.is_some() {
            let _ = self.tables.remove(table.id);
        }
        if let Err(error) = self.tables.restore(table.clone()) {
            if let Some(previous) = before.clone() {
                let _ = self.tables.restore(previous);
            }
            return Err(error);
        }
        self.undo.record(Delta::Table {
            before,
            after: Some(table),
        });
        Ok(())
    }

    /// Resize a table's range.
    pub fn resize_table(
        &mut self,
        id: TableId,
        range: crate::addr::RangeRef,
    ) -> Result<(), CoreError> {
        let before = self
            .tables
            .get(id)
            .cloned()
            .ok_or_else(|| CoreError::table_name("unknown table"))?;
        let new_bounds = (
            range.start.row.min(range.end.row),
            range.start.col.min(range.end.col),
            range.start.row.max(range.end.row),
            range.start.col.max(range.end.col),
        );
        self.ensure_range_not_pivot_output(
            before.sheet,
            new_bounds.0,
            new_bounds.1,
            new_bounds.2,
            new_bounds.3,
        )?;
        if self.tables.iter().any(|existing| {
            existing.id != id
                && existing.sheet == before.sheet
                && ranges_overlap(
                    new_bounds,
                    (
                        existing.start_row,
                        existing.start_col,
                        existing.end_row,
                        existing.end_col,
                    ),
                )
        }) {
            return Err(CoreError::table_name(
                "resized table would overlap an existing table",
            ));
        }
        {
            let table = self
                .tables
                .get_mut(id)
                .ok_or_else(|| CoreError::table_name("unknown table"))?;
            table.start_row = range.start.row.min(range.end.row);
            table.end_row = range.start.row.max(range.end.row);
            table.start_col = range.start.col.min(range.end.col);
            table.end_col = range.start.col.max(range.end.col);
            let width = u32::from(table.end_col - table.start_col) + 1;
            while table.columns.len() < width as usize {
                let n = table.columns.len() + 1;
                table.columns.push(crate::tables::TableColumn {
                    name: format!("Column{n}"),
                    totals_fn: None,
                });
            }
            table.columns.truncate(width as usize);
        }
        let after = self.tables.get(id).cloned();
        if Some(&before) != after.as_ref() {
            self.undo.record(Delta::Table {
                before: Some(before),
                after,
            });
        }
        Ok(())
    }

    /// Rename a table and update structured references in workbook formulas.
    pub fn rename_table(&mut self, id: TableId, name: impl Into<String>) -> Result<(), CoreError> {
        let before = self
            .tables
            .get(id)
            .cloned()
            .ok_or_else(|| CoreError::table_name("unknown table"))?;
        let mut after = before.clone();
        after.name = name.into();
        if after.name == before.name {
            return Ok(());
        }
        let mut formulas = Vec::new();
        for sheet in self.sheets() {
            for (row, col, slot) in sheet.store.iter() {
                let Some(formula) = slot.formula else {
                    continue;
                };
                let source = self.intern().formulas.get(formula).unwrap_or("");
                let rewritten = crate::formula::rewrite_print(
                    source,
                    &crate::formula::RewriteOp::TableRename {
                        old: before.name.clone(),
                        new: after.name.clone(),
                    },
                )
                .map_err(|error| {
                    CoreError::new(
                        "table.rename",
                        format!("could not rewrite structured reference: {error}"),
                    )
                })?;
                if rewritten != source {
                    formulas.push((sheet.id, row, col, rewritten));
                }
            }
        }
        self.transact_try(move |workbook| {
            workbook.restore_table(after)?;
            for (sheet, row, col, source) in formulas {
                let formula = workbook.intern_formula(&source)?;
                let mut slot = workbook
                    .get(sheet, row, col)?
                    .copied()
                    .ok_or_else(|| CoreError::new("table.rename", "formula cell vanished"))?;
                slot.formula = Some(formula);
                workbook.write_slot(sheet, row, col, Some(slot))?;
                workbook.release_formula(formula);
            }
            Ok(())
        })
    }

    /// Show or hide a table totals row and set per-column total functions.
    pub fn set_table_totals(
        &mut self,
        id: TableId,
        show: bool,
        functions: Vec<Option<String>>,
    ) -> Result<(), CoreError> {
        let mut table = self
            .tables
            .get(id)
            .cloned()
            .ok_or_else(|| CoreError::table_name("unknown table"))?;
        if functions.len() > table.columns.len() {
            return Err(CoreError::table_name(
                "more totals functions than table columns",
            ));
        }
        for function in functions.iter().flatten() {
            if !matches!(
                function.as_str(),
                "average"
                    | "count"
                    | "countNums"
                    | "max"
                    | "min"
                    | "stdDev"
                    | "sum"
                    | "var"
                    | "custom"
                    | "none"
            ) {
                return Err(CoreError::table_name(format!(
                    "unsupported table totals function {function:?}"
                )));
            }
        }
        if show && !table.has_totals {
            table.end_row = table
                .end_row
                .checked_add(1)
                .filter(|row| *row < crate::limits::MAX_ROWS)
                .ok_or_else(|| CoreError::table_name("table totals row exceeds the grid"))?;
        } else if !show && table.has_totals {
            table.end_row = table.end_row.saturating_sub(1).max(table.start_row);
        }
        table.has_totals = show;
        for (column, function) in table.columns.iter_mut().zip(functions) {
            column.totals_fn = function.filter(|function| function != "none");
        }
        if !show {
            for column in &mut table.columns {
                column.totals_fn = None;
            }
        }
        if self.tables.iter().any(|existing| {
            existing.id != id
                && existing.sheet == table.sheet
                && ranges_overlap(
                    (
                        table.start_row,
                        table.start_col,
                        table.end_row,
                        table.end_col,
                    ),
                    (
                        existing.start_row,
                        existing.start_col,
                        existing.end_row,
                        existing.end_col,
                    ),
                )
        }) {
            return Err(CoreError::table_name(
                "table totals row would overlap an existing table",
            ));
        }
        self.restore_table(table)
    }

    fn expand_tables_at(&mut self, sheet: SheetId, row: u32, col: u16) {
        let ids: Vec<TableId> = self
            .tables
            .iter()
            .filter(|t| t.sheet == sheet && t.auto_expand)
            .map(|t| t.id)
            .collect();
        for id in ids {
            let Some(t) = self.tables.get(id).cloned() else {
                continue;
            };
            let mut end_row = t.end_row;
            let mut end_col = t.end_col;
            if col >= t.start_col && col <= t.end_col && row == t.end_row.saturating_add(1) {
                end_row = row;
            }
            if row >= t.start_row && row <= t.end_row && col == t.end_col.saturating_add(1) {
                end_col = col;
            }
            if end_row != t.end_row || end_col != t.end_col {
                let grew_row = end_row == t.end_row.saturating_add(1);
                let (Ok(start), Ok(end)) = (
                    crate::addr::CellRef::new(t.start_row, t.start_col),
                    crate::addr::CellRef::new(end_row, end_col),
                ) else {
                    continue;
                };
                if self
                    .resize_table(id, crate::addr::RangeRef::from_corners(start, end))
                    .is_ok()
                    && grew_row
                {
                    self.fill_calculated_row(sheet, &t, end_row, col);
                }
            }
        }
    }

    fn fill_calculated_row(&mut self, sheet: SheetId, table: &Table, new_row: u32, skip_col: u16) {
        let src_row = if table.has_totals {
            table.end_row.saturating_sub(1)
        } else {
            table.end_row
        };
        if src_row == new_row {
            return;
        }
        for c in table.start_col..=table.end_col {
            if c == skip_col {
                continue;
            }
            let Ok(Some(slot)) = self.get(sheet, src_row, c) else {
                continue;
            };
            let Some(fid) = slot.formula else {
                continue;
            };
            let src = self.intern().formulas.get(fid).unwrap_or("").to_string();
            if let Ok(rewritten) = crate::formula::rewrite_print(
                &src,
                &crate::formula::RewriteOp::Copy { dcol: 0, drow: 1 },
            ) {
                let _ = self.set_cell_contents(sheet, new_row, c, &rewritten);
            }
        }
    }

    /// Insert rows. Formula rewrite is TODO(WP-03)/TODO(WP-17).
    pub fn insert_rows(&mut self, id: SheetId, at: u32, count: u32) -> Result<(), CoreError> {
        if count == 0 {
            return Ok(());
        }
        self.ensure_sheet_not_used_by_pivot(id)?;
        let n = i32::try_from(count).map_err(|_| CoreError::addr_ref("row count too large"))?;
        self.sheet_mut(id)?.store.shift_rows(at, n)?;
        self.recount_ref_errors();
        self.undo.record(Delta::ShiftRows {
            sheet: id,
            at,
            count: n,
            removed: Vec::new(),
        });
        Ok(())
    }

    /// Delete rows. Formula rewrite is TODO(WP-03)/TODO(WP-17).
    pub fn delete_rows(&mut self, id: SheetId, at: u32, count: u32) -> Result<(), CoreError> {
        if count == 0 {
            return Ok(());
        }
        self.ensure_sheet_not_used_by_pivot(id)?;
        if at >= crate::limits::MAX_ROWS {
            return Err(CoreError::addr_ref("row delete anchor is out of range"));
        }
        let actual = count.min(crate::limits::MAX_ROWS - at);
        let n = i32::try_from(actual).map_err(|_| CoreError::addr_ref("row count too large"))?;
        let removed = self
            .sheet(id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))?
            .store
            .iter_region(at, 0, at + actual - 1, crate::limits::MAX_COLS - 1)
            .collect();
        self.sheet_mut(id)?.store.shift_rows(at, -n)?;
        self.recount_ref_errors();
        self.undo.record(Delta::ShiftRows {
            sheet: id,
            at,
            count: -n,
            removed,
        });
        Ok(())
    }

    /// Insert columns.
    pub fn insert_cols(&mut self, id: SheetId, at: u16, count: u16) -> Result<(), CoreError> {
        if count == 0 {
            return Ok(());
        }
        self.ensure_sheet_not_used_by_pivot(id)?;
        let n = i32::from(count);
        self.sheet_mut(id)?.store.shift_cols(at, n)?;
        self.recount_ref_errors();
        self.undo.record(Delta::ShiftCols {
            sheet: id,
            at,
            count: n,
            removed: Vec::new(),
        });
        Ok(())
    }

    /// Delete columns.
    pub fn delete_cols(&mut self, id: SheetId, at: u16, count: u16) -> Result<(), CoreError> {
        if count == 0 {
            return Ok(());
        }
        self.ensure_sheet_not_used_by_pivot(id)?;
        if u32::from(at) >= u32::from(crate::limits::MAX_COLS) {
            return Err(CoreError::addr_ref("column delete anchor is out of range"));
        }
        let actual = count.min(crate::limits::MAX_COLS - at);
        let n = i32::from(actual);
        let removed = self
            .sheet(id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))?
            .store
            .iter_region(0, at, crate::limits::MAX_ROWS - 1, at + actual - 1)
            .collect();
        self.sheet_mut(id)?.store.shift_cols(at, -n)?;
        self.recount_ref_errors();
        self.undo.record(Delta::ShiftCols {
            sheet: id,
            at,
            count: -n,
            removed,
        });
        Ok(())
    }

    /// Set a note.
    pub fn set_note(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        note: Option<Note>,
    ) -> Result<(), CoreError> {
        CellRef::new(row, col)?;
        self.ensure_not_pivot_output(id, row, col)?;
        self.mutate_sheet_edit(id, |sheet| {
            match note {
                Some(n) => {
                    sheet.notes.insert((row, col), n);
                }
                None => {
                    sheet.notes.remove(&(row, col));
                }
            }
            Ok(())
        })
    }

    /// Set a threaded comment.
    pub fn set_comment(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        comment: Option<Comment>,
    ) -> Result<(), CoreError> {
        CellRef::new(row, col)?;
        self.ensure_not_pivot_output(id, row, col)?;
        self.mutate_sheet_edit(id, |sheet| {
            match comment {
                Some(c) => {
                    sheet.comments.insert((row, col), c);
                }
                None => {
                    sheet.comments.remove(&(row, col));
                }
            }
            Ok(())
        })
    }

    /// Set a hyperlink.
    pub fn set_hyperlink(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        link: Option<Hyperlink>,
    ) -> Result<(), CoreError> {
        CellRef::new(row, col)?;
        self.ensure_not_pivot_output(id, row, col)?;
        self.mutate_sheet_edit(id, |sheet| {
            match link {
                Some(h) => {
                    sheet.hyperlinks.insert((row, col), h);
                }
                None => {
                    sheet.hyperlinks.remove(&(row, col));
                }
            }
            Ok(())
        })
    }

    /// Used range of a sheet.
    pub fn used_range(&self, id: SheetId) -> Result<Option<UsedRange>, CoreError> {
        Ok(self
            .sheets
            .get(&id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))?
            .used_range())
    }

    /// Estimated heap of stores + interners.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        self.sheets
            .values()
            .map(|s| s.store.heap_bytes())
            .sum::<usize>()
            + self.intern.heap_bytes()
    }

    /// Replace a cell slot, preserving intern refcount rules of [`Self::set_text`].
    pub fn set_slot(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        slot: CellSlot,
    ) -> Result<Option<CellSlot>, CoreError> {
        self.ensure_not_pivot_output(id, row, col)?;
        self.replace_slot(id, row, col, Some(slot))
    }

    /// Intern text (refcount +1). Pair with [`Self::release_text`] after the slot holds it.
    pub fn intern_text(&mut self, text: &str) -> StrId {
        self.intern_mut().strings.intern(text)
    }

    /// Intern a style (refcount +1). Pair with [`Self::release_style`] after slots hold it.
    pub fn intern_style(&mut self, style: Style) -> StyleId {
        self.intern_mut().styles.intern(style)
    }

    /// Drop an interned-style refcount.
    pub fn release_style(&mut self, id: StyleId) {
        self.intern_mut().styles.release(id);
    }

    /// Drop an interned-text refcount.
    pub fn release_text(&mut self, id: StrId) {
        self.intern_mut().strings.release(id);
    }

    /// Intern an array payload (refcount +1).
    pub fn intern_array(&mut self, payload: ArrayPayload) -> ArrayId {
        self.intern_mut().arrays.intern(payload)
    }

    /// Drop an interned-array refcount.
    pub fn release_array(&mut self, id: ArrayId) {
        self.intern_mut().arrays.release(id);
    }

    /// Intern formula source (refcount +1). Pair with [`Self::release_formula`]
    /// after the slot holds it.
    pub fn intern_formula(&mut self, source: &str) -> Result<FormulaId, CoreError> {
        self.intern_mut().formulas.intern(source)
    }

    /// Drop an interned-formula refcount.
    pub fn release_formula(&mut self, id: FormulaId) {
        self.intern_mut().formulas.release(id);
    }

    /// Intern a number-format code. Built-in ids 0–49 are reused when the code
    /// matches the en-US builtin table; otherwise a custom id ≥ 164 is allocated.
    pub fn intern_num_fmt(&mut self, code: &str) -> Result<NumFmtId, CoreError> {
        numfmt::parse(code)?;
        for id in 0..=49 {
            if numfmt::builtin_format(id, LocaleId::EN_US).as_deref() == Some(code) {
                return Ok(NumFmtId::new(id));
            }
        }
        if let Some((&id, _)) = self
            .num_fmts
            .iter()
            .find(|(_, existing)| existing.as_str() == code)
        {
            return Ok(NumFmtId::new(id));
        }
        let id = self.next_num_fmt;
        self.next_num_fmt = self.next_num_fmt.saturating_add(1);
        self.num_fmts.insert(id, code.to_string());
        Ok(NumFmtId::new(id))
    }

    /// Format code for a stored [`NumFmtId`].
    #[must_use]
    pub fn num_fmt_code(&self, id: NumFmtId) -> Option<Cow<'_, str>> {
        if let Some(code) = numfmt::builtin_format(id.index(), LocaleId::EN_US) {
            return Some(code);
        }
        self.num_fmts
            .get(&id.index())
            .map(|code| Cow::Borrowed(code.as_str()))
    }

    /// Set cell contents from formula-bar text, preserving style.
    ///
    /// Leading `=` is stored as formula source. Otherwise a finite number,
    /// `TRUE`/`FALSE`, or text. Empty input clears contents and keeps style.
    pub fn set_cell_contents(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        input: &str,
    ) -> Result<Option<CellSlot>, CoreError> {
        self.ensure_not_pivot_output(id, row, col)?;
        let prev = self.get(id, row, col)?.copied();
        let style = prev.map(|slot| slot.style).unwrap_or(StyleId::DEFAULT);
        let flags = content_flags(prev);
        let trimmed = input.trim();
        if trimmed.is_empty() {
            if prev.is_none() {
                return Ok(None);
            }
            if prev.is_some_and(|slot| {
                slot.formula.is_none() && slot.value == Value::Empty && slot.style == style
            }) {
                return Ok(prev);
            }
            return self.replace_slot(
                id,
                row,
                col,
                Some(CellSlot {
                    value: Value::Empty,
                    formula: None,
                    style,
                    flags,
                }),
            );
        }
        let slot = if let Some(stripped) = trimmed.strip_prefix('=') {
            if stripped.is_empty() {
                return Err(CoreError::new(
                    crate::error::codes::FORMULA_LEN,
                    "formula input is empty after '='",
                )
                .with_hint("enter a formula such as =A1+1"));
            }
            let fid = self.intern_formula(trimmed)?;
            let slot = CellSlot {
                value: Value::Empty,
                formula: Some(fid),
                style,
                flags,
            };
            let old = self.replace_slot(id, row, col, Some(slot))?;
            self.release_formula(fid);
            self.expand_tables_at(id, row, col);
            return Ok(old);
        } else if trimmed.eq_ignore_ascii_case("TRUE") {
            CellSlot {
                value: Value::Bool(true),
                formula: None,
                style,
                flags,
            }
        } else if trimmed.eq_ignore_ascii_case("FALSE") {
            CellSlot {
                value: Value::Bool(false),
                formula: None,
                style,
                flags,
            }
        } else if let Some(number) = parse_finite_number(trimmed) {
            CellSlot {
                value: Value::Number(number),
                formula: None,
                style,
                flags,
            }
        } else {
            let sid = self.intern_text(trimmed);
            let slot = CellSlot {
                value: Value::Text(sid),
                formula: None,
                style,
                flags,
            };
            let old = self.replace_slot(id, row, col, Some(slot))?;
            self.release_text(sid);
            self.expand_tables_at(id, row, col);
            return Ok(old);
        };
        let old = self.replace_slot(id, row, col, Some(slot))?;
        self.expand_tables_at(id, row, col);
        Ok(old)
    }
}

fn sheet_ref_error_count(sheet: &Sheet) -> u64 {
    sheet
        .store
        .iter()
        .filter(|(_, _, slot)| matches!(slot.value, Value::Error(crate::error::ErrorKind::Ref)))
        .count() as u64
}

fn content_flags(prev: Option<CellSlot>) -> CellFlags {
    let mut flags = prev.map(|slot| slot.flags).unwrap_or(CellFlags::DEFAULT);
    flags = flags.with(CellFlags::DIRTY, false);
    flags = flags.with(CellFlags::SPILL, false);
    flags = flags.with(CellFlags::ARRAY, false);
    flags = flags.with(CellFlags::STALE, false);
    flags
}

fn parse_finite_number(text: &str) -> Option<f64> {
    let number: f64 = text.parse().ok()?;
    number.is_finite().then_some(number)
}

fn ranges_overlap(a: (u32, u16, u32, u16), b: (u32, u16, u32, u16)) -> bool {
    a.0 <= b.2 && b.0 <= a.2 && a.1 <= b.3 && b.1 <= a.3
}

fn unique_table_column_name(
    columns: &[crate::tables::TableColumn],
    before: usize,
    base: &str,
) -> String {
    if !columns[..before.min(columns.len())]
        .iter()
        .any(|column| column.name.eq_ignore_ascii_case(base))
    {
        return base.to_string();
    }
    for suffix in 2u32..=u32::MAX {
        let candidate = format!("{base}{suffix}");
        if !columns[..before.min(columns.len())]
            .iter()
            .any(|column| column.name.eq_ignore_ascii_case(&candidate))
        {
            return candidate;
        }
    }
    // Every possible suffix being occupied is only theoretical, but returning a
    // deterministic fallback keeps this input-facing path panic-free.
    format!("{base}_unique")
}
