//! Serde import/export plans shared by CLI, TUI/GUI preview, and WP-23.

use omacell_core::date_system::DateSystem;
use omacell_core::error::CoreError;
use omacell_core::locale::LocaleId;
use serde::{Deserialize, Serialize};

use crate::error;

/// Bytes sampled when sniffing a stream or path.
pub const MAX_SNIFF_BYTES: usize = 64 * 1024;

/// Default number of preview rows.
pub const DEFAULT_PREVIEW_ROWS: usize = 50;

/// Largest preview request accepted by the materializing preview API.
pub const MAX_PREVIEW_ROWS: usize = 10_000;

/// Largest single field the reader will accept.
pub const MAX_FIELD_BYTES: usize = 8 * 1024 * 1024;

/// Largest clipboard payload accepted by the materializing clipboard API.
pub const MAX_CLIPBOARD_BYTES: usize = 16 * 1024 * 1024;

/// Largest number of rows accepted by the materializing clipboard API.
pub const MAX_CLIPBOARD_ROWS: usize = 100_000;

/// Largest total cell count accepted by the materializing clipboard API.
pub const MAX_CLIPBOARD_CELLS: usize = 1_000_000;

/// Largest output retained by the convenience [`super::export`] API.
///
/// Use [`super::export_write`] for larger exports.
pub const MAX_BUFFERED_EXPORT_BYTES: usize = 256 * 1024 * 1024;

/// Largest total UTF-8 field payload retained for one export record.
pub const MAX_EXPORT_RECORD_BYTES: usize = 16 * 1024 * 1024;

/// Text encoding of a delimited file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextEncoding {
    /// UTF-8 (default). Invalid sequences are an error on load.
    #[default]
    Utf8,
    /// UTF-16 little-endian.
    Utf16Le,
    /// UTF-16 big-endian.
    Utf16Be,
    /// ISO-8859-1 (byte ↔ U+00xx), not windows-1252.
    Latin1,
}

impl TextEncoding {
    /// Parse a sniff/corpus tag.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag.trim().to_ascii_lowercase().as_str() {
            "utf-8" | "utf8" => Some(Self::Utf8),
            "utf-16le" | "utf16le" => Some(Self::Utf16Le),
            "utf-16be" | "utf16be" => Some(Self::Utf16Be),
            "latin-1" | "latin1" | "iso-8859-1" => Some(Self::Latin1),
            _ => None,
        }
    }
}

/// Record terminator used on export (import accepts CR, LF, or CRLF).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LineEnding {
    /// Unix / default on Linux.
    #[default]
    Lf,
    /// Windows.
    CrLf,
    /// Classic Mac.
    Cr,
}

impl LineEnding {
    /// Parse a corpus tag.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag.trim().to_ascii_lowercase().as_str() {
            "lf" => Some(Self::Lf),
            "crlf" => Some(Self::CrLf),
            "cr" => Some(Self::Cr),
            _ => None,
        }
    }

    /// The terminator bytes.
    #[must_use]
    pub fn as_bytes(self) -> &'static [u8] {
        match self {
            Self::Lf => b"\n",
            Self::CrLf => b"\r\n",
            Self::Cr => b"\r",
        }
    }
}

/// Per-column conversion.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ColumnType {
    /// Conservative per-cell inference (default).
    #[default]
    Auto,
    /// Parse as a number with the plan's separators. Failed cells stay text.
    Number,
    /// Store the raw string.
    Text,
    /// Parse with `format` (Excel-like `yyyy`/`mm`/`dd` tokens). Empty format
    /// uses Auto date rules but will not fall back to number/bool.
    Date {
        /// Date format tokens, or empty for locale/ISO Auto dates.
        #[serde(default)]
        format: String,
    },
    /// Only `TRUE` / `FALSE` (case-insensitive).
    Boolean,
    /// Store the raw string; sniff uses this for ambiguous columns.
    KeepAsText,
}

impl ColumnType {
    /// Parse a corpus tag (`auto`, `date:mm/dd/yyyy`, …).
    #[must_use]
    pub fn from_corpus(tag: &str) -> Option<Self> {
        let t = tag.trim();
        if let Some(fmt) = t.strip_prefix("date:") {
            return Some(Self::Date {
                format: fmt.to_string(),
            });
        }
        match t {
            "auto" => Some(Self::Auto),
            "number" => Some(Self::Number),
            "text" => Some(Self::Text),
            "boolean" => Some(Self::Boolean),
            "keep_as_text" => Some(Self::KeepAsText),
            _ => None,
        }
    }

    /// Whether this type always stores text.
    #[must_use]
    pub fn is_text_only(&self) -> bool {
        matches!(self, Self::Text | Self::KeepAsText)
    }
}

/// One column in an [`ImportPlan`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColumnPlan {
    /// Header name when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Conversion for this column.
    #[serde(default)]
    pub ty: ColumnType,
}

/// How a delimited file should be read. Shared JSON for CLI `--plan`, UIs, and AI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportPlan {
    /// Field delimiter (ASCII).
    pub delimiter: char,
    /// Quote character (ASCII). Default `"`.
    #[serde(default = "default_quote")]
    pub quote: char,
    /// File encoding.
    #[serde(default)]
    pub encoding: TextEncoding,
    /// Input started with a BOM / output should write one.
    #[serde(default)]
    pub bom: bool,
    /// First non-skipped record is a header.
    #[serde(default)]
    pub has_header: bool,
    /// Records to skip before the header/data.
    #[serde(default)]
    pub skip_rows: u32,
    /// Locale for separators and date order (Excel LCID).
    #[serde(default)]
    pub locale: LocaleId,
    /// Decimal separator used when parsing numbers.
    #[serde(default = "default_decimal")]
    pub decimal: char,
    /// Thousands grouping character, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thousands: Option<char>,
    /// Line ending observed / to emit on a matching export.
    #[serde(default)]
    pub line_ending: LineEnding,
    /// Workbook date system for serials.
    #[serde(default)]
    pub date_system: DateSystem,
    /// Per-column types. Extra file columns are [`ColumnType::Auto`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<ColumnPlan>,
}

fn default_quote() -> char {
    '"'
}

fn default_decimal() -> char {
    '.'
}

impl Default for ImportPlan {
    fn default() -> Self {
        let loc = LocaleId::EN_US;
        let sep = loc.separators();
        Self {
            delimiter: ',',
            quote: '"',
            encoding: TextEncoding::Utf8,
            bom: false,
            has_header: false,
            skip_rows: 0,
            locale: loc,
            decimal: sep.decimal,
            thousands: Some(sep.thousands),
            line_ending: LineEnding::Lf,
            date_system: DateSystem::Excel1900,
            columns: Vec::new(),
        }
    }
}

impl ImportPlan {
    /// Default plan with `locale` separators.
    #[must_use]
    pub fn with_locale(locale: LocaleId) -> Self {
        let sep = locale.separators();
        Self {
            locale,
            decimal: sep.decimal,
            thousands: Some(sep.thousands),
            ..Self::default()
        }
    }

    /// ASCII delimiter byte.
    pub fn delimiter_byte(&self) -> Result<u8, CoreError> {
        ascii_byte(self.delimiter, "delimiter")
    }

    /// ASCII quote byte.
    pub fn quote_byte(&self) -> Result<u8, CoreError> {
        ascii_byte(self.quote, "quote")
    }

    /// Type for column `idx`, or Auto if the plan is shorter.
    #[must_use]
    pub fn column_type(&self, idx: usize) -> &ColumnType {
        self.columns
            .get(idx)
            .map(|c| &c.ty)
            .unwrap_or(&ColumnType::Auto)
    }

    /// Reject inconsistent separators or non-ASCII delimiter/quote.
    pub fn validate(&self) -> Result<(), CoreError> {
        self.delimiter_byte()?;
        self.quote_byte()?;
        if self.delimiter == self.quote {
            return Err(error::plan("delimiter and quote must differ"));
        }
        if let Some(th) = self.thousands
            && th == self.decimal
        {
            return Err(error::plan(
                "thousands separator must differ from the decimal separator",
            ));
        }
        Ok(())
    }
}

fn ascii_byte(c: char, what: &str) -> Result<u8, CoreError> {
    if c.is_ascii() && c != '\0' {
        Ok(c as u8)
    } else {
        Err(error::plan(format!(
            "{what} must be a non-NUL ASCII character"
        )))
    }
}

/// When to wrap fields in quotes on export.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quoting {
    /// Quote when the field contains delimiter, quote, or a line break (RFC 4180).
    #[default]
    Necessary,
    /// Quote every field.
    Always,
    /// Never quote; error if a field would require it.
    Never,
}

/// Whether export writes cached values or formula source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueMode {
    /// Displayed / stored values.
    #[default]
    Values,
    /// Formula source when the cell has one, otherwise the value.
    Formulas,
}

/// Handling for text that spreadsheet programs may execute as a formula.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormulaTextPolicy {
    /// Reject the export. This is the safe default for untrusted workbook text.
    #[default]
    Reject,
    /// Preserve text exactly. Use only when the recipient will not execute CSV formulas.
    Preserve,
    /// Prefix an apostrophe so common spreadsheet programs treat the field as text.
    Escape,
}

/// How a delimited file should be written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportPlan {
    /// Field delimiter (ASCII).
    #[serde(default = "default_comma")]
    pub delimiter: char,
    /// Quote character (ASCII).
    #[serde(default = "default_quote")]
    pub quote: char,
    /// Quoting policy.
    #[serde(default)]
    pub quoting: Quoting,
    /// Output encoding.
    #[serde(default)]
    pub encoding: TextEncoding,
    /// Write a BOM (always written for UTF-16; optional for UTF-8).
    #[serde(default)]
    pub bom: bool,
    /// Record terminator.
    #[serde(default)]
    pub line_ending: LineEnding,
    /// Sheet name; default is the active sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    /// A1 range (`A1:C10`); default is the used range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    /// Values or formulas.
    #[serde(default)]
    pub values: ValueMode,
    /// Policy for formula-like text when exporting values.
    #[serde(default)]
    pub formula_text: FormulaTextPolicy,
    /// Locale for number-format rendering.
    #[serde(default)]
    pub locale: LocaleId,
}

fn default_comma() -> char {
    ','
}

impl Default for ExportPlan {
    fn default() -> Self {
        Self {
            delimiter: ',',
            quote: '"',
            quoting: Quoting::Necessary,
            encoding: TextEncoding::Utf8,
            bom: false,
            line_ending: LineEnding::Lf,
            sheet: None,
            range: None,
            values: ValueMode::Values,
            formula_text: FormulaTextPolicy::Reject,
            locale: LocaleId::EN_US,
        }
    }
}

impl ExportPlan {
    /// ASCII delimiter byte.
    pub fn delimiter_byte(&self) -> Result<u8, CoreError> {
        ascii_byte(self.delimiter, "delimiter")
    }

    /// ASCII quote byte.
    pub fn quote_byte(&self) -> Result<u8, CoreError> {
        ascii_byte(self.quote, "quote")
    }

    /// Reject non-ASCII delimiter/quote.
    pub fn validate(&self) -> Result<(), CoreError> {
        self.delimiter_byte()?;
        self.quote_byte()?;
        if self.delimiter == self.quote {
            return Err(error::plan("delimiter and quote must differ"));
        }
        Ok(())
    }
}
