//! Sheet, cell, and range addresses (spec F-1.2).
//!
//! Indices are **0-based**. A1 `A1` and R1C1 `R1C1` are `(row: 0, col: 0)`.
//! `MAX_ROWS` / `MAX_COLS` are counts; valid indices are `0..MAX_*`.

mod a1;
mod letters;
mod r1c1;
mod scan;

pub use a1::{parse_a1, parse_a1_cell, quote_sheet_name};
pub use letters::{col_from_letters, col_to_letters};
pub use r1c1::{parse_r1c1, parse_r1c1_cell};

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::CoreError;
use crate::limits::{MAX_COLS, MAX_ROWS};

/// Workbook-assigned sheet identity (WP-02 assigns ids).
///
/// ```
/// use omacell_core::addr::SheetId;
/// let id = SheetId::new(0);
/// assert_eq!(id.index(), 0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SheetId(u32);

impl SheetId {
    /// Wrap a workbook sheet id.
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

/// Sheet names as written in A1/R1C1 text, before WP-02 resolves them to [`SheetId`].
///
/// `end` is `Some` for 3-D spans (`Sheet1:Sheet3!A1`).
///
/// ```
/// use omacell_core::addr::SheetSpec;
/// let s = SheetSpec { start: "Data".into(), end: None };
/// assert_eq!(s.to_a1_prefix(), "Data!");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SheetSpec {
    /// First (or only) sheet name.
    pub start: String,
    /// Last sheet name of a 3-D reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
}

/// A cell address with optional sheet id and absolute/relative flags.
///
/// ```
/// use omacell_core::addr::CellRef;
/// let a1 = CellRef::new(0, 0).unwrap();
/// assert_eq!(a1.to_a1(), "A1");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct CellRef {
    /// Resolved sheet, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<SheetId>,
    /// 0-based row (`0..MAX_ROWS`).
    pub row: u32,
    /// 0-based column (`0..MAX_COLS`).
    pub col: u16,
    /// `$` on the row (A1) / no brackets on `R` (R1C1).
    pub row_abs: bool,
    /// `$` on the column (A1) / no brackets on `C` (R1C1).
    pub col_abs: bool,
}

impl CellRef {
    /// Relative cell at `(row, col)`.
    pub fn new(row: u32, col: u16) -> Result<Self, CoreError> {
        Self::with_abs(row, col, false, false)
    }

    /// Cell with explicit absolute flags.
    pub fn with_abs(row: u32, col: u16, row_abs: bool, col_abs: bool) -> Result<Self, CoreError> {
        let cell = Self {
            sheet: None,
            row,
            col,
            row_abs,
            col_abs,
        };
        cell.validate()?;
        Ok(cell)
    }

    /// Attach a resolved sheet id.
    #[must_use]
    pub fn on_sheet(mut self, sheet: SheetId) -> Self {
        self.sheet = Some(sheet);
        self
    }

    /// Validate that the row and column are inside Excel's grid.
    pub fn validate(self) -> Result<(), CoreError> {
        if self.row >= MAX_ROWS || u32::from(self.col) >= u32::from(MAX_COLS) {
            Err(CoreError::addr_ref(format!(
                "cell r{}c{} is out of range",
                self.row, self.col
            )))
        } else {
            Ok(())
        }
    }
}

#[derive(Deserialize)]
struct CellRefWire {
    #[serde(default)]
    sheet: Option<SheetId>,
    row: u32,
    col: u16,
    row_abs: bool,
    col_abs: bool,
}

impl<'de> Deserialize<'de> for CellRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = CellRefWire::deserialize(deserializer)?;
        let mut cell = Self::with_abs(wire.row, wire.col, wire.row_abs, wire.col_abs)
            .map_err(serde::de::Error::custom)?;
        cell.sheet = wire.sheet;
        Ok(cell)
    }
}

/// Rectangular range, including whole-row/column and 3-D forms.
///
/// ```
/// use omacell_core::addr::{CellRef, RangeRef};
/// let r = RangeRef::from_corners(
///     CellRef::new(0, 0).unwrap(),
///     CellRef::new(1, 1).unwrap(),
/// );
/// assert_eq!(r.to_a1(), "A1:B2");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RangeRef {
    /// Top-left (or start) corner. `start.sheet` is the first sheet of a 3-D ref.
    pub start: CellRef,
    /// Bottom-right (or end) corner.
    pub end: CellRef,
    /// Last sheet of a 3-D reference (`Sheet1:Sheet3!A1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet_end: Option<SheetId>,
    /// Print/parse as `1:10` rather than `A1:XFD10`.
    pub whole_row: bool,
    /// Print/parse as `A:C` rather than `A1:C1048576`.
    pub whole_col: bool,
}

impl RangeRef {
    /// Cell-cell range (not whole-row/column).
    #[must_use]
    pub fn from_corners(start: CellRef, end: CellRef) -> Self {
        Self {
            start,
            end,
            sheet_end: None,
            whole_row: false,
            whole_col: false,
        }
    }

    /// Whether this is a 3-D reference.
    #[must_use]
    pub fn is_3d(self) -> bool {
        self.sheet_end.is_some()
    }
}

/// Cell or range after parsing, with optional unresolved sheet names.
///
/// ```
/// use omacell_core::addr::{parse_a1, ParsedRef};
/// let p: ParsedRef = parse_a1("Sheet1!A1").unwrap();
/// assert_eq!(p.sheet.as_ref().map(|s| s.start.as_str()), Some("Sheet1"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParsedRef {
    /// Sheet names from the text; [`CellRef::sheet`] stays `None` until WP-02.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<SheetSpec>,
    /// Cell or range body.
    pub kind: RefKind,
}

/// Body of a parsed reference.
///
/// ```
/// use omacell_core::addr::{parse_a1, RefKind};
/// assert!(matches!(parse_a1("A1").unwrap().kind, RefKind::Cell(_)));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RefKind {
    /// Single cell.
    Cell(CellRef),
    /// Range, whole row/column, or 3-D body.
    Range(RangeRef),
}

pub(crate) fn sheet_name_needs_quote(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    if name.eq_ignore_ascii_case("TRUE") || name.eq_ignore_ascii_case("FALSE") {
        return true;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return true;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return true;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return true;
    }
    a1::parse_a1_body(name).is_ok() || r1c1::parse_r1c1_body(name, 0, 0).is_ok()
}
