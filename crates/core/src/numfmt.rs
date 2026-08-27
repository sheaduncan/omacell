//! Excel number formats, the General algorithm, and `numFmtId` 0–49 (F-2.3, F-2.6).
//!
//! ```
//! use omacell_core::locale::LocaleId;
//! use omacell_core::numfmt::{format, FormatValue};
//! let out = format(FormatValue::Number(1234.5), "#,##0.00", LocaleId::EN_US);
//! assert_eq!(out.text, "1,234.50");
//! ```

mod builtin;
mod fraction;
mod general;
mod number;
mod parse;
mod render;
mod token;

use crate::dates::DateSystem;
use crate::error::{ErrorKind, codes};
use crate::locale::LocaleId;
use crate::value::Value;

pub use builtin::builtin_format;
pub use general::{general, general_for_width};
pub use parse::parse;
pub use token::{ColorHint, LayoutHints, MAX_FORMAT_LEN, NamedColor, ParsedFormat};

/// A value the formatter can render without intern tables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FormatValue<'a> {
    /// Empty cell.
    Empty,
    /// IEEE 754 number (including date serials).
    Number(f64),
    /// Boolean.
    Bool(bool),
    /// Text payload.
    Text(&'a str),
    /// Excel error; always displayed as its canonical string.
    Error(ErrorKind),
}

impl FormatValue<'static> {
    /// Map a [`Value`] that does not need intern tables.
    ///
    /// [`Value::Text`] and [`Value::Array`] become [`FormatValue::Empty`].
    #[must_use]
    pub fn from_value(value: Value) -> Self {
        match value {
            Value::Empty => Self::Empty,
            Value::Number(n) => Self::Number(n),
            Value::Bool(b) => Self::Bool(b),
            Value::Text(_) | Value::Array(_) => Self::Empty,
            Value::Error(e) => Self::Error(e),
        }
    }
}

/// Result of [`format()`] / [`format_with`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Formatted {
    /// Display text.
    pub text: String,
    /// `[Red]` / `[Color n]` if the selected section named a color.
    pub color_hint: Option<ColorHint>,
    /// `*` fill and `_` skip markers.
    pub layout_hints: LayoutHints,
}

impl Formatted {
    pub(crate) fn text(text: String) -> Self {
        Self {
            text,
            color_hint: None,
            layout_hints: LayoutHints::default(),
        }
    }
}

/// Options beyond the three-arg [`format()`] API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatOptions {
    /// Workbook / cell locale.
    pub locale: LocaleId,
    /// 1900 (default) or 1904 date system.
    pub date_system: DateSystem,
    /// Character budget for General and `*` fill.
    pub width: Option<usize>,
}

impl FormatOptions {
    /// `locale`, 1900 date system, no width.
    #[must_use]
    pub fn new(locale: LocaleId) -> Self {
        Self {
            locale,
            date_system: DateSystem::Excel1900,
            width: None,
        }
    }
}

/// Format `value` with an Excel format code in `locale` (1900 date system).
#[must_use]
pub fn format(value: FormatValue<'_>, fmt: &str, locale: LocaleId) -> Formatted {
    format_with(value, fmt, &FormatOptions::new(locale))
}

/// Format with an explicit date system and optional column width.
#[must_use]
pub fn format_with(value: FormatValue<'_>, fmt: &str, opts: &FormatOptions) -> Formatted {
    render::format_value(value, fmt, opts)
}

/// Format using a pre-parsed format code.
#[must_use]
pub fn format_parsed(value: FormatValue<'_>, parsed: &ParsedFormat, locale: LocaleId) -> Formatted {
    render::format_parsed(value, parsed, &FormatOptions::new(locale))
}

/// Parse error machine code (`numfmt.parse`).
#[must_use]
pub fn parse_error_code() -> &'static str {
    codes::NUMFMT_PARSE
}
