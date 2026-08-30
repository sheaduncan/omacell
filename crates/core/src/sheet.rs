//! Sheet metadata, view state, and the cell store (spec F-1.1, F-2.2).

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::addr::{CellRef, RangeRef, SheetId};
use crate::chart::{Chart, Sparkline};
use crate::error::CoreError;
use crate::geometry::SheetGeometry;
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
    /// Opaque hash / verifier bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<Vec<u8>>,
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
/// let c = Comment { author: "Ada".into(), text: "hi".into(), replies: vec![] };
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
