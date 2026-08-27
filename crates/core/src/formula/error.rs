//! Parse errors for the formula language.

use crate::error::CoreError;

/// Stable machine codes for formula errors. These are **not** WP-01 `codes::*`.
pub mod codes {
    /// Formula source could not be parsed.
    pub const PARSE: &str = "formula.parse";
    /// Source is longer than [`crate::limits::MAX_FORMULA_LEN`].
    pub const LENGTH: &str = "formula.length";
    /// Nesting exceeded [`crate::formula::MAX_FORMULA_DEPTH`].
    pub const DEPTH: &str = "formula.depth";
}

/// A formula parse failure with a byte offset and an expected-token set.
///
/// ```
/// use omacell_core::formula::{parse, codes};
/// let err = parse("=)").unwrap_err();
/// assert_eq!(err.error.code, codes::PARSE);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Engine error (`{code, message, hint}`).
    pub error: CoreError,
    /// UTF-8 byte offset in the original source.
    pub offset: usize,
    /// Tokens that would have been legal at `offset` (editor / autocomplete).
    pub expected: Vec<String>,
}

impl ParseError {
    pub(crate) fn new(
        code: &'static str,
        message: impl Into<String>,
        offset: usize,
        expected: Vec<String>,
    ) -> Self {
        Self {
            error: CoreError::new(code, message),
            offset,
            expected,
        }
    }

    pub(crate) fn parse(message: impl Into<String>, offset: usize, expected: Vec<String>) -> Self {
        Self::new(codes::PARSE, message, offset, expected)
    }

    pub(crate) fn length(message: impl Into<String>) -> Self {
        Self::new(codes::LENGTH, message, 0, Vec::new())
            .with_hint("Excel formulas are at most 8,192 bytes")
    }

    pub(crate) fn depth(message: impl Into<String>, offset: usize) -> Self {
        Self::new(codes::DEPTH, message, offset, Vec::new())
            .with_hint("Excel allows at most 64 nested functions")
    }

    fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.error = self.error.with_hint(hint);
        self
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}", self.error, self.offset)
    }
}

impl std::error::Error for ParseError {}
