//! Compact cell values. Text and arrays are intern handles (WP-02 owns the interners).

use std::fmt;
use std::mem::size_of;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{CoreError, ErrorKind, codes};

const _: () = assert!(
    size_of::<Value>() <= 16,
    "Value must stay a 16-byte tagged union (spec §11.3)"
);

/// Interned string handle. The workbook (WP-02) owns the shared-string table.
///
/// ```
/// use omacell_core::value::StrId;
/// let id = StrId::new(0);
/// assert_eq!(id.index(), 0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StrId(u32);

impl StrId {
    /// Wrap a workbook-assigned intern index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Intern index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Interned array handle. The workbook (WP-02) owns array payloads.
///
/// ```
/// use omacell_core::value::ArrayId;
/// let id = ArrayId::new(7);
/// assert_eq!(id.index(), 7);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArrayId(u32);

impl ArrayId {
    /// Wrap a workbook-assigned intern index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Intern index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Shape of a spilled or literal array. Values live behind [`ArrayId`].
///
/// ```
/// use omacell_core::value::Array2D;
/// let shape = Array2D::new(2, 3).expect("shape");
/// assert_eq!(shape.len(), 6);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Array2D {
    /// Number of rows (at least 1).
    pub rows: u32,
    /// Number of columns (at least 1).
    pub cols: u32,
}

impl Array2D {
    /// Construct a non-empty shape whose `rows * cols` fits in `u32`.
    pub fn new(rows: u32, cols: u32) -> Result<Self, CoreError> {
        let shape = Self { rows, cols };
        shape.validate()?;
        Ok(shape)
    }

    /// Validate that the shape is non-empty and its cell count fits in `u32`.
    pub fn validate(self) -> Result<(), CoreError> {
        if self.rows == 0 || self.cols == 0 {
            return Err(CoreError::new(
                codes::ARRAY_SHAPE,
                "array dimensions must be at least 1×1",
            ));
        }
        self.rows
            .checked_mul(self.cols)
            .ok_or_else(|| CoreError::new(codes::ARRAY_SHAPE, "array rows * cols overflows u32"))?;
        Ok(())
    }

    /// Total number of cells.
    #[must_use]
    pub fn len(self) -> u32 {
        self.rows.saturating_mul(self.cols)
    }

    /// Always `false` for a value produced by [`Array2D::new`].
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.rows == 0 || self.cols == 0
    }
}

#[derive(Deserialize)]
struct Array2DWire {
    rows: u32,
    cols: u32,
}

impl<'de> Deserialize<'de> for Array2D {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = Array2DWire::deserialize(deserializer)?;
        Self::new(wire.rows, wire.cols).map_err(serde::de::Error::custom)
    }
}

/// 16-byte tagged cell value (spec §11.3, F-2.1).
///
/// Dates are numbers with a date number format (WP-06), not a separate variant.
///
/// ```
/// use omacell_core::value::Value;
/// let n = Value::Number(1.5);
/// assert!(matches!(n, Value::Number(_)));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    /// Empty cell.
    #[default]
    Empty,
    /// IEEE 754 double (Excel number, including date serials).
    Number(f64),
    /// Boolean.
    Bool(bool),
    /// Interned text.
    Text(StrId),
    /// Excel error value.
    Error(ErrorKind),
    /// Interned array (dynamic-array spill payload).
    Array(ArrayId),
}

impl Value {
    /// `TRUE` / `FALSE` / empty / number / error display. Text and arrays show handles.
    #[must_use]
    pub fn is_error(self) -> bool {
        matches!(self, Self::Error(_))
    }

    /// Excel error, if any.
    #[must_use]
    pub fn error(self) -> Option<ErrorKind> {
        match self {
            Self::Error(e) => Some(e),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => Ok(()),
            Self::Number(n) => write!(f, "{n}"),
            Self::Bool(true) => f.write_str("TRUE"),
            Self::Bool(false) => f.write_str("FALSE"),
            Self::Text(id) => write!(f, "Text({})", id.index()),
            Self::Error(e) => write!(f, "{e}"),
            Self::Array(id) => write!(f, "Array({})", id.index()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_is_at_most_16_bytes() {
        assert!(
            size_of::<Value>() <= 16,
            "Value is {} bytes; spec §11.3 requires ≤ 16",
            size_of::<Value>()
        );
        assert!(size_of::<Value>() > 0);
    }

    #[test]
    fn value_is_copy() {
        let a = Value::Number(1.0);
        let b = a;
        assert_eq!(a, b);
    }
}
