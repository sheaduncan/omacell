//! In-memory workbook (spec F-1, §11.3).
//!
//! Single-writer. [`Workbook::snapshot`] is a cheap copy-on-write view for
//! readers (render during recalc, §11.5).

use std::sync::Arc;

use indexmap::IndexMap;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::addr::{ParsedRef, RefKind, SheetId, SheetSpec};
use crate::error::CoreError;
use crate::intern::{FormulaId, Interners};
use crate::names::{DefinedName, NameRegistry};
use crate::sheet::{
    Comment, Hyperlink, Note, ProtectionState, Sheet, SheetVisibility, ViewState,
    validate_sheet_name,
};
use crate::storage::{CellSlot, UsedRange};
use crate::style::{Color, Style, StyleId};
use crate::tables::{Table, TableId, TableRegistry};
use crate::undo::{AffectedRange, Delta, UndoLog, transaction_affected};
use crate::value::{StrId, Value};

/// 1900 (Lotus leap-year quirk, WP-06) or 1904 date system.
///
/// ```
/// use omacell_core::workbook::DateSystem;
/// assert_eq!(DateSystem::default(), DateSystem::Date1900);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateSystem {
    /// Excel Windows default (serial 1 = 1899-12-31, with 1900 leap-year quirk).
    #[default]
    Date1900,
    /// Excel Mac 1904 system.
    Date1904,
}

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

/// Cheap reader snapshot (Arc block pages + intern tables).
///
/// Mutating the originating [`Workbook`] does not change this view.
#[derive(Clone, Debug)]
pub struct WorkbookSnapshot {
    sheets: IndexMap<SheetId, Sheet>,
    names: NameRegistry,
    intern: Arc<Interners>,
    settings: WorkbookSettings,
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

    /// Settings.
    #[must_use]
    pub fn settings(&self) -> &WorkbookSettings {
        &self.settings
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
    settings: WorkbookSettings,
    meta: WorkbookMeta,
    /// Opaque extra parts for WP-10 (e.g. unused OOXML).
    pub custom_parts: IndexMap<String, Vec<u8>>,
    undo: UndoLog,
    next_sheet: u32,
    active: SheetId,
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
            settings: WorkbookSettings::default(),
            meta: WorkbookMeta::default(),
            custom_parts: IndexMap::new(),
            undo: UndoLog::new(),
            next_sheet: 1,
            active: id,
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
            intern: Arc::clone(&self.intern),
            settings: self.settings.clone(),
        }
    }

    /// Settings.
    #[must_use]
    pub fn settings(&self) -> &WorkbookSettings {
        &self.settings
    }

    /// Mutable settings (not undo-tracked; WP-07 will wrap this).
    pub fn settings_mut(&mut self) -> &mut WorkbookSettings {
        &mut self.settings
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

    /// Undo log (budget, enable/disable).
    pub fn undo_log_mut(&mut self) -> &mut UndoLog {
        &mut self.undo
    }

    /// Active sheet id.
    #[must_use]
    pub fn active_sheet(&self) -> SheetId {
        self.active
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

    fn sheet_mut(&mut self, id: SheetId) -> Result<&mut Sheet, CoreError> {
        self.sheets
            .get_mut(&id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", id.index())))
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

    /// Set a plain numeric cell.
    pub fn set_number(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        n: f64,
    ) -> Result<Option<CellSlot>, CoreError> {
        self.replace_slot(id, row, col, Some(CellSlot::number(n)))
    }

    /// Intern `text` and store it as the cell value.
    pub fn set_text(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        text: &str,
    ) -> Result<StrId, CoreError> {
        let sid = self.intern_mut().strings.intern(text);
        let slot = CellSlot {
            value: Value::Text(sid),
            formula: None,
            style: StyleId::DEFAULT,
            flags: crate::storage::CellFlags::DEFAULT,
        };
        self.replace_slot(id, row, col, Some(slot))?;
        self.intern_mut().strings.release(sid);
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
        self.replace_slot(id, row, col, None)
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
            } => {
                let (px, hid) = if inverse {
                    (*before_px, *hidden_before)
                } else {
                    (*after_px, *hidden_after)
                };
                let s = self.sheet_mut(*sheet)?;
                s.geometry.rows.set_size(*row, px)?;
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
            } => {
                let (px, hid) = if inverse {
                    (*before_px, *hidden_before)
                } else {
                    (*after_px, *hidden_after)
                };
                let s = self.sheet_mut(*sheet)?;
                s.geometry.cols.set_size(u32::from(*col), px)?;
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
            Delta::SheetRemove { id, index, sheet } => {
                if inverse {
                    self.link_sheet(*index, (**sheet).clone())?;
                } else {
                    self.unlink_sheet(*id)?;
                }
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
            Delta::ShiftRows { sheet, at, count } => {
                let n = if inverse { -count } else { *count };
                self.sheet_mut(*sheet)?.store.shift_rows(*at, n)
            }
            Delta::ShiftCols { sheet, at, count } => {
                let n = if inverse { -count } else { *count };
                self.sheet_mut(*sheet)?.store.shift_cols(*at, n)
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
        self.names_by_lower.remove(&sheet.name.to_lowercase());
        if self.active == id {
            self.active = self.sheets.keys().copied().next().unwrap_or(id);
        }
        Ok(sheet)
    }

    fn link_sheet(&mut self, index: usize, sheet: Sheet) -> Result<(), CoreError> {
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

    /// Remove a sheet. The last remaining sheet cannot be removed. The last
    /// visible sheet cannot be removed if it is the only visible one.
    pub fn remove_sheet(&mut self, id: SheetId) -> Result<Sheet, CoreError> {
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
        let sheet = self.unlink_sheet(id)?;
        self.undo.record(Delta::SheetRemove {
            id,
            index,
            sheet: Box::new(sheet.clone()),
        });
        Ok(sheet)
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

    /// Insert rows. Formula rewrite is TODO(WP-03)/TODO(WP-17).
    pub fn insert_rows(&mut self, id: SheetId, at: u32, count: u32) -> Result<(), CoreError> {
        if count == 0 {
            return Ok(());
        }
        let n = i32::try_from(count).map_err(|_| CoreError::addr_ref("row count too large"))?;
        self.sheet_mut(id)?.store.shift_rows(at, n)?;
        self.undo.record(Delta::ShiftRows {
            sheet: id,
            at,
            count: n,
        });
        Ok(())
    }

    /// Delete rows. Formula rewrite is TODO(WP-03)/TODO(WP-17).
    pub fn delete_rows(&mut self, id: SheetId, at: u32, count: u32) -> Result<(), CoreError> {
        if count == 0 {
            return Ok(());
        }
        let n = i32::try_from(count).map_err(|_| CoreError::addr_ref("row count too large"))?;
        self.sheet_mut(id)?.store.shift_rows(at, -n)?;
        self.undo.record(Delta::ShiftRows {
            sheet: id,
            at,
            count: -n,
        });
        Ok(())
    }

    /// Insert columns.
    pub fn insert_cols(&mut self, id: SheetId, at: u16, count: u16) -> Result<(), CoreError> {
        if count == 0 {
            return Ok(());
        }
        let n = i32::from(count);
        self.sheet_mut(id)?.store.shift_cols(at, n)?;
        self.undo.record(Delta::ShiftCols {
            sheet: id,
            at,
            count: n,
        });
        Ok(())
    }

    /// Delete columns.
    pub fn delete_cols(&mut self, id: SheetId, at: u16, count: u16) -> Result<(), CoreError> {
        if count == 0 {
            return Ok(());
        }
        let n = i32::from(count);
        self.sheet_mut(id)?.store.shift_cols(at, -n)?;
        self.undo.record(Delta::ShiftCols {
            sheet: id,
            at,
            count: -n,
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
        let sheet = self.sheet_mut(id)?;
        match note {
            Some(n) => {
                sheet.notes.insert((row, col), n);
            }
            None => {
                sheet.notes.remove(&(row, col));
            }
        }
        Ok(())
    }

    /// Set a threaded comment.
    pub fn set_comment(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        comment: Option<Comment>,
    ) -> Result<(), CoreError> {
        let sheet = self.sheet_mut(id)?;
        match comment {
            Some(c) => {
                sheet.comments.insert((row, col), c);
            }
            None => {
                sheet.comments.remove(&(row, col));
            }
        }
        Ok(())
    }

    /// Set a hyperlink.
    pub fn set_hyperlink(
        &mut self,
        id: SheetId,
        row: u32,
        col: u16,
        link: Option<Hyperlink>,
    ) -> Result<(), CoreError> {
        let sheet = self.sheet_mut(id)?;
        match link {
            Some(h) => {
                sheet.hyperlinks.insert((row, col), h);
            }
            None => {
                sheet.hyperlinks.remove(&(row, col));
            }
        }
        Ok(())
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
}
