//! Sheet metadata, view state, and the cell store (spec F-1.1, F-2.2).

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::addr::{CellRef, RangeRef, SheetId};
use crate::chart::{Chart, Sparkline};
use crate::error::CoreError;
use crate::geometry::{AxisGeometry, SheetGeometry};
use crate::print::PageSetup;
use crate::storage::{SheetStore, UsedRange};
use crate::style::Color;

/// Excel sheet visibility.
///
/// ```
/// use omacell_core::sheet::SheetVisibility;
/// assert_eq!(SheetVisibility::Visible, SheetVisibility::Visible);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SheetVisibility {
    /// Shown in the tab bar.
    #[default]
    Visible,
    /// Hidden; unhide from the UI.
    Hidden,
    /// Very hidden; only an agent/script unhides (Excel VBA `xlSheetVeryHidden`).
    VeryHidden,
}

impl SheetVisibility {
    /// Shown in the tab bar.
    #[must_use]
    pub fn is_visible(self) -> bool {
        matches!(self, Self::Visible)
    }
}

/// Frozen pane counts (first `rows` rows and `cols` columns stay put).
///
/// ```
/// use omacell_core::sheet::FreezePanes;
/// let f = FreezePanes { rows: 1, cols: 0 };
/// assert_eq!(f.rows, 1);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FreezePanes {
    /// Frozen row count.
    pub rows: u32,
    /// Frozen column count.
    pub cols: u16,
}

/// Split view in pixels from the top-left of the sheet pane.
///
/// ```
/// use omacell_core::sheet::SplitView;
/// let s = SplitView { x_px: 100, y_px: 40 };
/// assert_eq!(s.x_px, 100);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SplitView {
    /// Vertical splitter x.
    pub x_px: u32,
    /// Horizontal splitter y.
    pub y_px: u32,
}

/// Per-sheet view (F-1.1).
///
/// ```
/// use omacell_core::sheet::ViewState;
/// let v = ViewState::default();
/// assert!((v.zoom - 1.0).abs() < f64::EPSILON);
/// assert!(v.gridlines);
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    /// 1.0 = 100 %.
    pub zoom: f64,
    /// Frozen panes.
    pub freeze: FreezePanes,
    /// Optional split.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<SplitView>,
    /// First visible row.
    pub scroll_row: u32,
    /// First visible column.
    pub scroll_col: u16,
    /// Current selection.
    pub selection: RangeRef,
    /// Show gridlines.
    pub gridlines: bool,
    /// Show formulas instead of values.
    pub show_formulas: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            freeze: FreezePanes::default(),
            split: None,
            scroll_row: 0,
            scroll_col: 0,
            selection: RangeRef::from_corners(
                CellRef {
                    sheet: None,
                    row: 0,
                    col: 0,
                    row_abs: false,
                    col_abs: false,
                },
                CellRef {
                    sheet: None,
                    row: 0,
                    col: 0,
                    row_abs: false,
                    col_abs: false,
                },
            ),
            gridlines: true,
            show_formulas: false,
        }
    }
}

/// Sheet protection (password blob is opaque for WP-10 round-trip).
///
/// ```
/// use omacell_core::sheet::ProtectionState;
/// assert!(!ProtectionState::default().enabled);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProtectionState {
    /// Protection is on.
    pub enabled: bool,
    /// Opaque hash / verifier bytes (Excel XOR hash, two bytes, not a secret).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<Vec<u8>>,
    /// Actions still allowed while the sheet is protected.
    #[serde(default)]
    pub allow: ProtectionAllow,
    /// Password-editable ranges preserved from OOXML.
    #[serde(default)]
    pub protected_ranges: Vec<ProtectedRange>,
}

/// A sheet range that may use its own legacy protection verifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProtectedRange {
    /// Display name.
    pub name: String,
    /// One or more allowed areas.
    pub ranges: Vec<RangeRef>,
    /// Optional uppercase ASCII legacy verifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<Vec<u8>>,
}

/// Excel sheet-protection allow-list (compatibility only).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProtectionAllow {
    /// Select locked cells.
    #[serde(default = "true_bool")]
    pub select_locked: bool,
    /// Select unlocked cells.
    #[serde(default = "true_bool")]
    pub select_unlocked: bool,
    /// Format cells.
    #[serde(default)]
    pub format_cells: bool,
    /// Insert rows.
    #[serde(default)]
    pub insert_rows: bool,
    /// Insert columns.
    #[serde(default)]
    pub insert_cols: bool,
    /// Sort.
    #[serde(default)]
    pub sort: bool,
    /// AutoFilter.
    #[serde(default)]
    pub auto_filter: bool,
}

fn true_bool() -> bool {
    true
}

impl Default for ProtectionAllow {
    fn default() -> Self {
        Self {
            select_locked: true,
            select_unlocked: true,
            format_cells: false,
            insert_rows: false,
            insert_cols: false,
            sort: false,
            auto_filter: false,
        }
    }
}

/// Cell note (legacy comment).
///
/// ```
/// use omacell_core::sheet::Note;
/// let n = Note { author: Some("Ada".into()), text: "check".into() };
/// assert_eq!(n.text, "check");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Note {
    /// Author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Body.
    pub text: String,
}

/// Threaded comment (and replies).
///
/// ```
/// use omacell_core::sheet::Comment;
/// let c = Comment { author: "Ada".into(), text: "hi".into(), replies: vec![], resolved: false };
/// assert!(c.replies.is_empty());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Comment {
    /// Author.
    pub author: String,
    /// Body.
    pub text: String,
    /// Replies.
    #[serde(default)]
    pub replies: Vec<Comment>,
    /// Thread is resolved.
    #[serde(default)]
    pub resolved: bool,
}

/// Hyperlink on a cell.
///
/// ```
/// use omacell_core::sheet::Hyperlink;
/// let h = Hyperlink { target: "https://example.com".into(), tooltip: None, display: None };
/// assert!(h.tooltip.is_none());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hyperlink {
    /// URL or internal location.
    pub target: String,
    /// Tooltip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    /// Display text override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

/// One legacy Ctrl+Shift+Enter formula anchored to a fixed output range.
///
/// The formula source is stored only on [`Self::anchor`]'s cell slot. Every
/// cell in [`Self::range`] owns a cached result and carries `CellFlags::ARRAY`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArrayFormula {
    /// Top-left cell that owns the formula source.
    pub anchor: CellRef,
    /// Fixed output rectangle, local to this sheet.
    pub range: RangeRef,
}

impl ArrayFormula {
    /// Whether the fixed output contains `row`, `col`.
    #[must_use]
    pub fn contains(self, row: u32, col: u16) -> bool {
        row >= self.range.start.row
            && row <= self.range.end.row
            && col >= self.range.start.col
            && col <= self.range.end.col
    }
}

/// Forbidden characters in a sheet name (F-1.1).
const FORBIDDEN: &[char] = &['[', ']', ':', '*', '?', '/', '\\'];

/// Validate F-1.1 sheet naming rules (uniqueness is the workbook's job).
pub fn validate_sheet_name(name: &str) -> Result<(), CoreError> {
    if name.is_empty() {
        return Err(CoreError::sheet_name("sheet name is empty"));
    }
    if name.chars().count() > 31 {
        return Err(CoreError::sheet_name(format!(
            "sheet name {name:?} is longer than 31 characters"
        )));
    }
    if name.starts_with('\'') || name.ends_with('\'') {
        return Err(CoreError::sheet_name(
            "sheet names cannot start or end with an apostrophe",
        ));
    }
    if name.chars().any(|c| FORBIDDEN.contains(&c)) {
        return Err(CoreError::sheet_name(format!(
            "sheet name {name:?} contains a forbidden character []:*?/\\"
        )));
    }
    Ok(())
}

/// One sheet: store, geometry, view, and side tables.
///
/// ```
/// use omacell_core::addr::SheetId;
/// use omacell_core::sheet::Sheet;
/// let s = Sheet::new(SheetId::new(0), "Sheet1").unwrap();
/// assert_eq!(s.name, "Sheet1");
/// ```
#[derive(Clone, Debug)]
pub struct Sheet {
    /// Stable id.
    pub id: SheetId,
    /// Display name.
    pub name: String,
    /// Visibility.
    pub visibility: SheetVisibility,
    /// Tab colour.
    pub tab_color: Option<Color>,
    /// View state.
    pub view: ViewState,
    /// Protection.
    pub protection: ProtectionState,
    /// Merged ranges.
    pub merges: Vec<RangeRef>,
    /// Notes keyed by (row, col).
    pub notes: FxHashMap<(u32, u16), Note>,
    /// Threaded comments.
    pub comments: FxHashMap<(u32, u16), Comment>,
    /// Hyperlinks.
    pub hyperlinks: FxHashMap<(u32, u16), Hyperlink>,
    /// Cell store.
    pub store: SheetStore,
    /// Row/column geometry.
    pub geometry: SheetGeometry,
    /// Charts overlaid on this sheet.
    pub charts: Vec<Chart>,
    /// In-cell sparklines.
    pub sparklines: Vec<Sparkline>,
    /// Page setup used by print preview and PDF export.
    pub page_setup: PageSetup,
    /// AutoFilter (WP-18).
    pub autofilter: Option<crate::filter::AutoFilter>,
    /// Rows hidden by the active AutoFilter, distinct from manually hidden rows.
    pub(crate) filter_hidden_rows: std::collections::BTreeSet<u32>,
    /// Data validations (WP-18).
    pub validations: Vec<crate::validation::DataValidation>,
    /// Conditional format rules, low priority number wins (WP-18).
    pub cond_formats: Vec<crate::condfmt::CondFormat>,
    pub(crate) array_formulas: Vec<ArrayFormula>,
}

/// Undo snapshot of the WP-17/WP-18 sheet metadata that lives outside the cell store.
///
/// The fields are intentionally private: callers edit through [`crate::workbook::Workbook`]
/// so mutations remain validated and undo tracked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetEditState {
    rows: AxisEditState,
    cols: AxisEditState,
    protection: ProtectionState,
    merges: Vec<RangeRef>,
    notes: Vec<(u32, u16, Note)>,
    comments: Vec<(u32, u16, Comment)>,
    hyperlinks: Vec<(u32, u16, Hyperlink)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    autofilter: Option<crate::filter::AutoFilter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    filter_hidden_rows: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    validations: Vec<crate::validation::DataValidation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    cond_formats: Vec<crate::condfmt::CondFormat>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    array_formulas: Vec<ArrayFormula>,
}

/// Portable sparse state for one row or column axis.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct AxisEditState {
    custom: Vec<(u32, u32)>,
    hidden: Vec<u32>,
    outline: Vec<(u32, u8)>,
    collapsed: Vec<u32>,
}

impl SheetEditState {
    pub(crate) fn capture(sheet: &Sheet) -> Self {
        Self {
            rows: capture_axis(&sheet.geometry.rows),
            cols: capture_axis(&sheet.geometry.cols),
            protection: sheet.protection.clone(),
            merges: sheet.merges.clone(),
            notes: sorted_map(&sheet.notes),
            comments: sorted_map(&sheet.comments),
            hyperlinks: sorted_map(&sheet.hyperlinks),
            autofilter: sheet.autofilter.clone(),
            filter_hidden_rows: sheet.filter_hidden_rows.iter().copied().collect(),
            validations: sheet.validations.clone(),
            cond_formats: sheet.cond_formats.clone(),
            array_formulas: sheet.array_formulas.clone(),
        }
    }

    pub(crate) fn restore(self, sheet: &mut Sheet) {
        sheet.geometry.rows = restore_axis(self.rows, true);
        sheet.geometry.cols = restore_axis(self.cols, false);
        sheet.protection = self.protection;
        sheet.merges = self.merges;
        sheet.notes = self
            .notes
            .into_iter()
            .map(|(row, col, value)| ((row, col), value))
            .collect();
        sheet.comments = self
            .comments
            .into_iter()
            .map(|(row, col, value)| ((row, col), value))
            .collect();
        sheet.hyperlinks = self
            .hyperlinks
            .into_iter()
            .map(|(row, col, value)| ((row, col), value))
            .collect();
        sheet.autofilter = self.autofilter;
        sheet.filter_hidden_rows = self.filter_hidden_rows.into_iter().collect();
        sheet.validations = self.validations;
        sheet.cond_formats = self.cond_formats;
        sheet.array_formulas = self.array_formulas;
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        let note_bytes = self
            .notes
            .iter()
            .map(|(_, _, note)| {
                note.text.len() + note.author.as_ref().map(String::len).unwrap_or(0)
            })
            .sum::<usize>();
        let comment_bytes = self
            .comments
            .iter()
            .map(|(_, _, comment)| comment_bytes(comment))
            .sum::<usize>();
        let hyperlink_bytes = self
            .hyperlinks
            .iter()
            .map(|(_, _, link)| {
                link.target.len()
                    + link.tooltip.as_ref().map(String::len).unwrap_or(0)
                    + link.display.as_ref().map(String::len).unwrap_or(0)
            })
            .sum::<usize>();
        256 + self.rows.custom.len() * 16
            + self.rows.hidden.len() * 4
            + self.rows.outline.len() * 8
            + self.rows.collapsed.len() * 4
            + self.cols.custom.len() * 16
            + self.cols.hidden.len() * 4
            + self.cols.outline.len() * 8
            + self.cols.collapsed.len() * 4
            + self.merges.len() * std::mem::size_of::<RangeRef>()
            + note_bytes
            + comment_bytes
            + hyperlink_bytes
            + self.validations.len() * 64
            + self.cond_formats.len() * 64
            + self.array_formulas.len() * std::mem::size_of::<ArrayFormula>()
            + usize::from(self.autofilter.is_some()) * 64
            + self.filter_hidden_rows.len() * std::mem::size_of::<u32>()
    }
}

fn capture_axis(axis: &AxisGeometry) -> AxisEditState {
    AxisEditState {
        custom: axis.iter_custom().collect(),
        hidden: axis.iter_hidden().collect(),
        outline: axis.iter_outline().collect(),
        collapsed: axis.iter_collapsed().collect(),
    }
}

fn restore_axis(state: AxisEditState, rows: bool) -> AxisGeometry {
    let mut axis = if rows {
        AxisGeometry::rows()
    } else {
        AxisGeometry::cols()
    };
    for (index, size) in state.custom {
        let _ = axis.set_size(index, size);
    }
    for index in state.hidden {
        let _ = axis.set_hidden(index, true);
    }
    for (index, level) in state.outline {
        let _ = axis.set_outline_level(index, level);
    }
    for index in state.collapsed {
        let _ = axis.set_collapsed(index, true);
    }
    axis
}

fn sorted_map<T: Clone>(map: &FxHashMap<(u32, u16), T>) -> Vec<(u32, u16, T)> {
    let mut entries: Vec<_> = map
        .iter()
        .map(|(&(row, col), value)| (row, col, value.clone()))
        .collect();
    entries.sort_by_key(|(row, col, _)| (*row, *col));
    entries
}

fn comment_bytes(comment: &Comment) -> usize {
    comment.author.len()
        + comment.text.len()
        + comment.replies.iter().map(comment_bytes).sum::<usize>()
}

impl Sheet {
    /// Empty visible sheet.
    pub fn new(id: SheetId, name: impl Into<String>) -> Result<Self, CoreError> {
        let name = name.into();
        validate_sheet_name(&name)?;
        Ok(Self {
            id,
            name,
            visibility: SheetVisibility::Visible,
            tab_color: None,
            view: ViewState::default(),
            protection: ProtectionState::default(),
            merges: Vec::new(),
            notes: FxHashMap::default(),
            comments: FxHashMap::default(),
            hyperlinks: FxHashMap::default(),
            store: SheetStore::new(),
            geometry: SheetGeometry::new(),
            charts: Vec::new(),
            sparklines: Vec::new(),
            page_setup: PageSetup::default(),
            autofilter: None,
            filter_hidden_rows: std::collections::BTreeSet::new(),
            validations: Vec::new(),
            cond_formats: Vec::new(),
            array_formulas: Vec::new(),
        })
    }

    /// Occupied bounding box.
    #[must_use]
    pub fn used_range(&self) -> Option<UsedRange> {
        self.store.used_range()
    }

    /// Add a merge. Overlap is rejected.
    pub fn add_merge(&mut self, range: RangeRef) -> Result<(), CoreError> {
        range.start.validate()?;
        range.end.validate()?;
        for existing in &self.merges {
            if ranges_overlap(*existing, range) {
                return Err(CoreError::new(
                    crate::error::codes::ADDR_REF,
                    "merged range overlaps an existing merge",
                ));
            }
        }
        self.merges.push(range);
        Ok(())
    }

    /// Notes in sorted cell order.
    pub fn notes_sorted(&self) -> Vec<((u32, u16), &Note)> {
        let mut v: Vec<_> = self.notes.iter().map(|(k, n)| (*k, n)).collect();
        v.sort_by_key(|(k, _)| *k);
        v
    }

    /// Legacy fixed-range array formulas, sorted by anchor.
    pub fn array_formulas(&self) -> impl Iterator<Item = &ArrayFormula> {
        self.array_formulas.iter()
    }

    /// Legacy array formula containing a cell, if any.
    #[must_use]
    pub fn array_formula_at(&self, row: u32, col: u16) -> Option<&ArrayFormula> {
        self.array_formulas
            .iter()
            .find(|formula| formula.contains(row, col))
    }

    pub(crate) fn replace_array_formula(&mut self, formula: ArrayFormula) {
        self.array_formulas
            .retain(|existing| existing.anchor != formula.anchor);
        self.array_formulas.push(formula);
        self.array_formulas
            .sort_by_key(|formula| (formula.anchor.row, formula.anchor.col));
    }

    pub(crate) fn remove_array_formula(&mut self, anchor: CellRef) {
        self.array_formulas
            .retain(|formula| formula.anchor != anchor);
    }
}

fn ranges_overlap(a: RangeRef, b: RangeRef) -> bool {
    let a_r0 = a.start.row.min(a.end.row);
    let a_r1 = a.start.row.max(a.end.row);
    let a_c0 = a.start.col.min(a.end.col);
    let a_c1 = a.start.col.max(a.end.col);
    let b_r0 = b.start.row.min(b.end.row);
    let b_r1 = b.start.row.max(b.end.row);
    let b_c0 = b.start.col.min(b.end.col);
    let b_c1 = b.start.col.max(b.end.col);
    a_r0 <= b_r1 && b_r0 <= a_r1 && a_c0 <= b_c1 && b_c0 <= a_c1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_rules() {
        assert!(validate_sheet_name("Sheet1").is_ok());
        assert!(validate_sheet_name("").is_err());
        assert!(validate_sheet_name("Bad:name").is_err());
        assert!(validate_sheet_name(&"x".repeat(32)).is_err());
        assert!(validate_sheet_name(&"x".repeat(31)).is_ok());
    }
}
