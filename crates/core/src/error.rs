//! Excel cell errors and engine errors with stable machine codes.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// Stable machine codes for [`CoreError::code`].
///
/// CLI, IPC, and MCP mirror these strings. Do not rename after Gate G0.
pub mod codes {
    /// Address is outside `A1:XFD1048576` (Excel `#REF!`).
    pub const ADDR_REF: &str = "addr.ref";
    /// Address text could not be parsed.
    pub const ADDR_PARSE: &str = "addr.parse";
    /// Command id is not a dotted lowercase identifier.
    pub const COMMAND_ID: &str = "command.id";
    /// Changeset id is empty.
    pub const CHANGESET_ID: &str = "changeset.id";
    /// Changeset inverse commands do not match its lifecycle status.
    pub const CHANGESET_INVERSE: &str = "changeset.inverse";
    /// Array shape is empty or overflows.
    pub const ARRAY_SHAPE: &str = "value.array_shape";
}

/// Excel cell error value.
///
/// Display strings match Excel exactly. [`ErrorKind::error_type`] returns the
/// `ERROR.TYPE` code from Microsoft’s documentation, or `None` when Excel
/// returns `#N/A`.
///
/// ```
/// use omacell_core::error::ErrorKind;
/// assert_eq!(ErrorKind::Div0.as_str(), "#DIV/0!");
/// assert_eq!(ErrorKind::Div0.error_type(), Some(2));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// `#NULL!` — intersecting ranges do not intersect.
    Null,
    /// `#DIV/0!` — division by zero.
    Div0,
    /// `#VALUE!` — wrong type of argument or operand.
    Value,
    /// `#REF!` — invalid cell reference.
    Ref,
    /// `#NAME?` — unrecognized name.
    Name,
    /// `#NUM!` — invalid numeric value.
    Num,
    /// `#N/A` — value not available.
    Na,
    /// `#GETTING_DATA` — data is still being retrieved.
    GettingData,
    /// `#SPILL!` — dynamic array could not spill.
    Spill,
    /// `#CALC!` — calculation error in an array.
    Calc,
    /// `#FIELD!` — linked data type field is missing.
    Field,
    /// `#CONNECT!` — connected data is unavailable.
    Connect,
    /// `#BLOCKED!` — the operation is blocked.
    Blocked,
    /// `#UNKNOWN!` — unrecognized error.
    Unknown,
}

struct ErrorMeta {
    kind: ErrorKind,
    display: &'static str,
    error_type: Option<u8>,
}

const ERROR_TABLE: &[ErrorMeta] = &[
    ErrorMeta {
        kind: ErrorKind::Null,
        display: "#NULL!",
        error_type: Some(1),
    },
    ErrorMeta {
        kind: ErrorKind::Div0,
        display: "#DIV/0!",
        error_type: Some(2),
    },
    ErrorMeta {
        kind: ErrorKind::Value,
        display: "#VALUE!",
        error_type: Some(3),
    },
    ErrorMeta {
        kind: ErrorKind::Ref,
        display: "#REF!",
        error_type: Some(4),
    },
    ErrorMeta {
        kind: ErrorKind::Name,
        display: "#NAME?",
        error_type: Some(5),
    },
    ErrorMeta {
        kind: ErrorKind::Num,
        display: "#NUM!",
        error_type: Some(6),
    },
    ErrorMeta {
        kind: ErrorKind::Na,
        display: "#N/A",
        error_type: Some(7),
    },
    ErrorMeta {
        kind: ErrorKind::GettingData,
        display: "#GETTING_DATA",
        error_type: Some(8),
    },
    ErrorMeta {
        kind: ErrorKind::Spill,
        display: "#SPILL!",
        error_type: None,
    },
    ErrorMeta {
        kind: ErrorKind::Calc,
        display: "#CALC!",
        error_type: None,
    },
    ErrorMeta {
        kind: ErrorKind::Field,
        display: "#FIELD!",
        error_type: None,
    },
    ErrorMeta {
        kind: ErrorKind::Connect,
        display: "#CONNECT!",
        error_type: None,
    },
    ErrorMeta {
        kind: ErrorKind::Blocked,
        display: "#BLOCKED!",
        error_type: None,
    },
    ErrorMeta {
        kind: ErrorKind::Unknown,
        display: "#UNKNOWN!",
        error_type: None,
    },
];

impl ErrorKind {
    /// Every Excel error this crate knows about, in `ERROR.TYPE` then WP-01 order.
    #[must_use]
    pub const fn all() -> [ErrorKind; 14] {
        [
            Self::Null,
            Self::Div0,
            Self::Value,
            Self::Ref,
            Self::Name,
            Self::Num,
            Self::Na,
            Self::GettingData,
            Self::Spill,
            Self::Calc,
            Self::Field,
            Self::Connect,
            Self::Blocked,
            Self::Unknown,
        ]
    }

    fn meta(self) -> &'static ErrorMeta {
        match ERROR_TABLE.iter().find(|m| m.kind == self) {
            Some(m) => m,
            None => &ERROR_TABLE[ERROR_TABLE.len() - 1],
        }
    }

    /// Exact Excel display string (`#DIV/0!`, `#NAME?`, `#N/A`, …).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.meta().display
    }

    /// `ERROR.TYPE` numeric code, or `None` if Excel returns `#N/A`.
    #[must_use]
    pub fn error_type(self) -> Option<u8> {
        self.meta().error_type
    }

    /// Parse an Excel display string.
    #[must_use]
    pub fn from_display(s: &str) -> Option<Self> {
        ERROR_TABLE.iter().find(|m| m.display == s).map(|m| m.kind)
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ErrorKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_display(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown Excel error {s:?}")))
    }
}

/// Engine / API error with a stable `{code, message, hint}` shape.
///
/// ```
/// use omacell_core::error::{codes, CoreError};
/// let err = CoreError::addr_ref("column XFE is out of range");
/// assert_eq!(err.code, codes::ADDR_REF);
/// assert_eq!(err.excel_error(), Some(omacell_core::error::ErrorKind::Ref));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("{message}")]
pub struct CoreError {
    /// Stable dotted machine code (see [`codes`]).
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Optional hint for CLI / UI recovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl CoreError {
    /// Construct a code + message error with no hint.
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: None,
        }
    }

    /// Attach a hint.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Out-of-range address (`#REF!`-class).
    #[must_use]
    pub fn addr_ref(message: impl Into<String>) -> Self {
        Self::new(codes::ADDR_REF, message)
            .with_hint("Excel addresses must be within A1:XFD1048576")
    }

    /// Address syntax error.
    #[must_use]
    pub fn addr_parse(message: impl Into<String>) -> Self {
        Self::new(codes::ADDR_PARSE, message)
    }

    /// Maps `#REF!`-class engine errors onto the Excel cell error they should
    /// produce when a formula hits them.
    #[must_use]
    pub fn excel_error(&self) -> Option<ErrorKind> {
        if self.code == codes::ADDR_REF {
            Some(ErrorKind::Ref)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus;

    #[test]
    fn error_table_matches_corpus() {
        let path = corpus::path("errors/error_type.tsv");
        let rows = corpus::read_tsv(&path);
        assert_eq!(rows.len(), ErrorKind::all().len());
        for row in rows {
            assert!(row.len() >= 4, "row {row:?}");
            let display = &row[1];
            let code = &row[2];
            let kind = ErrorKind::from_display(display)
                .unwrap_or_else(|| panic!("missing display {display}"));
            assert_eq!(kind.as_str(), display);
            match code.as_str() {
                "" => assert_eq!(kind.error_type(), None, "{display}"),
                n => {
                    let n: u8 = n.parse().expect("error_type");
                    assert_eq!(kind.error_type(), Some(n), "{display}");
                }
            }
        }
    }

    #[test]
    fn display_strings_are_exact() {
        assert_eq!(ErrorKind::Null.as_str(), "#NULL!");
        assert_eq!(ErrorKind::Div0.as_str(), "#DIV/0!");
        assert_eq!(ErrorKind::Value.as_str(), "#VALUE!");
        assert_eq!(ErrorKind::Ref.as_str(), "#REF!");
        assert_eq!(ErrorKind::Name.as_str(), "#NAME?");
        assert_eq!(ErrorKind::Num.as_str(), "#NUM!");
        assert_eq!(ErrorKind::Na.as_str(), "#N/A");
        assert_eq!(ErrorKind::GettingData.as_str(), "#GETTING_DATA");
        assert_eq!(ErrorKind::Spill.as_str(), "#SPILL!");
        assert_eq!(ErrorKind::Calc.as_str(), "#CALC!");
        assert_eq!(ErrorKind::Field.as_str(), "#FIELD!");
        assert_eq!(ErrorKind::Connect.as_str(), "#CONNECT!");
        assert_eq!(ErrorKind::Blocked.as_str(), "#BLOCKED!");
        assert_eq!(ErrorKind::Unknown.as_str(), "#UNKNOWN!");
    }
}
